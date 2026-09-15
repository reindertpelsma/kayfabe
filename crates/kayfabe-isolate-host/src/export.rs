//! ★★★ The two ends of `RmBackend::export_backing` — **what crosses is memory**.
//!
//! `isolate_vmm_fd_crossing.md` §12, the owner's decision (b) for `#133`/`#128`.
//!
//! `fdcross` built the transport and said, in as many words, that no verb used it and that
//! the verb was an owner decision (§10 item 1). This is that verb's plumbing: a table on
//! each side of the boundary, and nothing else.
//!
//! | side | type | what it holds |
//! |---|---|---|
//! | child (isolate) | [`ChildExports`] | the sealed `memfd`s this isolate minted |
//! | parent (VMM) | [`ExportRegistry`] | the [`CrossedFd`]s it adopted, indexed by token |
//!
//! ## ★★ Why the token is minted TWICE, and never carried across
//!
//! The child indexes its own table; the parent indexes its own. The child's index does
//! **not** travel on the wire ([`crate::proto::Reply::Backing`] has no token field), and
//! the reason is the one [`crate::proto::WireError::into_rm_error`] already applies to a
//! `BadHandle`: a value the peer supplies must never name a slot in *our* registry. A
//! child that could choose the parent's token could make the VMM install one backing where
//! it asked for another — a mapping of the wrong bytes, which is the single failure this
//! whole design ranks worst.
//!
//! The association survives without it because the channel is **1-deep**: the descriptor
//! rides the reply to the request that asked for it, on a socket with exactly one verb in
//! flight (`proto`'s module docs — *"no demux, no pending list, and no `txn_id` to
//! confuse"*). There is nothing to correlate.
//!
//! ## ⊘⊘ CORRECTED 2026-08-27 — A DEVICE DESCRIPTOR **CAN** NOW CROSS, AND ONLY ON THE
//! ## VMM'S OWN SAY-SO. The paragraph below described the state until this date.
//!
//! ★ [`ChildExports::mint_armed_node`] adds a second kind of export: **a device node whose
//! RM `mmap` context the isolate has already armed** with `NV_ESC_RM_MAP_MEMORY` (`0x4E`).
//! That is not *"exporting device memory"* — it is exporting **an armed context**, which is
//! precisely what the sibling `nvkvm-pv` passes to its VMM
//! (`src/qemu/nvkvm_isolate_handlers.c:3618`).
//!
//! ★★★ **The property that makes it safe is structural, not policy.** The VMM never issues
//! an RM *escape* on that descriptor — it only `mmap`s it, and `mmap` is not an escape, so
//! `secInfo.privLevel` (recomputed from the caller on every escape,
//! `ogkm-580: escape.c:304`) is never recomputed in a privileged VMM's favour. The
//! privileged half stays in the unprivileged isolate.
//!
//! ★★ **And the peer is still not trusted.** [`ExportRegistry::adopt`] now takes the kind
//! the **caller asked for** and still establishes the actual kind from the *kernel*. The
//! child cannot widen it by claiming anything: a `CharDevice` crosses only when the VMM's
//! own request was for one, and a child answering a fabricated-memory request with a device
//! node is refused exactly as before.
//!
//! ⊘ Unchanged, deliberately: the isolate still names **no GPA**; `GuestWindow::place` still
//! refuses `Backing::DeviceFile`, so none of this reaches a guest memslot; and
//! `kayfabe_isolate::ExportSource::HostDeviceMemory` — *"export this RM object's pages"* —
//! is still refused by name. Those are different requests from this one.
//!
//! ⚠ **The residual risk, named rather than buried:** the VMM now trusts the isolate about
//! *what is behind* the fd. The bound is RM's own ownership check — an unprivileged isolate
//! can only map objects its own client owns — so a compromised isolate can surface its own
//! VRAM, never another tenant's. That bound is RM's, not ours.
//!
//! ## ⊘ What is NOT here (as of the correction above)
//!
//! No *unrequested* device descriptor. [`ChildExports::mint`] can only ever produce a
//! `memfd`, and [`ExportRegistry::adopt`] refuses anything that is not the kind the caller
//! named **before** it is reachable. The class that cannot be exported is refused one layer
//! up, in the backend, by name — see `kayfabe_isolate::RmError::NotExportableAsMemory`.

use crate::fdcross::{CrossedFd, FdOrigin};
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::{CharDevice, DescriptorKind, RawError, SharedRam};
use std::os::fd::OwnedFd;
use std::sync::Mutex;

/// The isolate's own table of backings it minted for the VMM.
///
/// One per child process, shared by every worker thread: a backing is an isolate-scoped
/// resource, not a worker-scoped one, and a worker-scoped table would make the token's
/// meaning depend on which pool slot happened to serve the request.
/// ★★★ **What one child-side export IS** — and the two arms are not interchangeable.
///
/// The distinction is the whole of the 2026-08-27 correction in this module's docs: one arm
/// is memory this isolate *fabricated* and can serve; the other is a **device node whose RM
/// `mmap` context this isolate armed**, which the isolate does not author the bytes of.
#[derive(Debug)]
enum ChildBacking {
    /// A sealed `memfd` — bytes the isolate wrote and can serve. See [`ChildExports::mint`].
    Fabricated(SharedRam),
    /// ★ A device node carrying a live `NV_ESC_RM_MAP_MEMORY` context. See
    /// [`ChildExports::mint_armed_node`].
    ///
    /// ⚠ **The context is one-shot per `struct file`** — a second `0x4E` against a node that
    /// already carries one is `NV_ERR_STATE_IN_USE` (`ogkm-580: nv-usermap.c:53-57`), so the
    /// node stored here must be one freshly opened for this mapping and never reused.
    ArmedNode {
        /// The node whose `struct file` carries the armed context.
        node: CharDevice,
        /// ★ What [`NV_ESC_RM_UNMAP_MEMORY`] needs to identify this mapping. See
        /// [`CpuViewRelease`].
        release: CpuViewRelease,
    },
    /// ⊘ A token whose view has been released. **A tombstone, not a removal**: the token IS the
    /// index, so compacting the vector would silently re-point every later token at the wrong
    /// backing.
    Released,
}

/// ★★★★★ **What it takes to give a CPU view's BAR1 aperture back.**
///
/// `[measured w722]` `munmap` + `close` returns **nothing** to the aperture pool — a second round
/// of mappings at fresh offsets gets **zero**. Only `NV_ESC_RM_UNMAP_MEMORY` (`0x4F`) releases it,
/// and it identifies the mapping by the **`pLinearAddress` cookie RM minted at map time**, which
/// therefore has to be captured rather than status-checked and dropped.
///
/// ⊘ Held here rather than recomputed because by release time the caller no longer has the
/// handles: the token is all it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuViewRelease {
    /// The object the view is of.
    pub h_memory: u32,
    /// ★ RM's own cookie for this mapping, from the map's reply. Opaque; never dereferenced.
    pub p_linear_address: u64,
}

// ⊘⊘⊘ **`h_client` AND `h_device` ARE DELIBERATELY ABSENT, and the tree's own guard is why.**
//
// They were here first, carried from map time. `own_client_invariant.rs` (**F11**) refused it:
// *"an RM escape in rm.rs names a client that is not this isolate's own minted `OwnClient`"*.
// ⇒ An escape must name **this isolate's** client, always — a handle carried in a struct is a
// handle that could have come from somewhere else, and the invariant is that none ever does.
//
// ★ So the release reads them from the connection at issue time instead. That is not merely
// equivalent: it makes the invariant hold **by construction** rather than by every future caller
// populating this struct correctly.

#[derive(Debug, Default)]
pub struct ChildExports {
    backings: Mutex<Vec<ChildBacking>>,
}

impl ChildExports {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ★ Mint `len` bytes of shareable backing and remember it; returns the child-scoped
    /// token.
    ///
    /// [`SharedRam`] rather than a bare `memfd` because its seals are load-bearing at
    /// exactly this seam: `F_SEAL_SHRINK` stops a descriptor holder shortening the file
    /// under a live mapping, which would turn every mapping of it — the VMM's included —
    /// into `SIGBUS`. Its type name says *guest RAM*; the mechanism is the mechanism, and
    /// the direction it is used in here is the other one.
    ///
    /// ★★ The backing is created **outside** the table lock. `SharedRam::create` asserts
    /// R1 for itself, and holding any lock of ours across three syscalls is the shape this
    /// workspace's leaf-witness exists to catch even when the lock happens to be unranked.
    ///
    /// # Errors
    /// Whatever `memfd_create`/`ftruncate`/`fcntl` refused with.
    pub fn mint(&self, len: u64) -> Result<u64, RawError> {
        let ram = SharedRam::create(len)?;
        let mut t = self.backings.lock().unwrap_or_else(|e| e.into_inner());
        t.push(ChildBacking::Fabricated(ram));
        Ok(t.len() as u64 - 1)
    }

    /// ★★★★★ **Remember a device node whose RM `mmap` context is already armed**, so the
    /// VMM can `mmap` the same object; returns the child-scoped token.
    ///
    /// This is the second export kind (see this module's 2026-08-27 correction). The caller
    /// must have obtained `node` from `RmConnection::map_cpu*`, which issues
    /// `NV_ESC_RM_MAP_MEMORY` against it — an unarmed node would hand the VMM an `mmap` that
    /// fails rather than a mapping, and the failure would arrive nowhere near here.
    ///
    /// ⊘ **Takes the node by value on purpose.** The armed context lives on the `struct
    /// file` and is released by `nv_free_file_private`, so the node must outlive every
    /// mapping of it. A borrowed node would be closed by its owner while the VMM's mapping
    /// was live.
    ///
    /// ⚠ It records **no length**. Length is the caller's, carried in the reply beside the
    /// token, because the `mmap` the VMM performs must use the length RM registered — not a
    /// number this table re-derived. `ogkm-580: nv-mmap.c:562-565` refuses any other.
    pub fn mint_armed_node(&self, node: CharDevice, release: CpuViewRelease) -> u64 {
        let mut t = self.backings.lock().unwrap_or_else(|e| e.into_inner());
        t.push(ChildBacking::ArmedNode { node, release });
        t.len() as u64 - 1
    }

    /// ★★★ **Retire `token`'s armed node and hand back what its release needs.**
    ///
    /// Drops the [`CharDevice`] — which closes the descriptor and frees the armed context via
    /// `nv_free_file_private` — and leaves a [`ChildBacking::Released`] tombstone so later tokens
    /// keep their indices.
    ///
    /// ⊘ **Closing the node is NOT the release.** The aperture comes back only when the caller
    /// issues `NV_ESC_RM_UNMAP_MEMORY` with the returned [`CpuViewRelease`]; this table cannot do
    /// it itself because it holds no control descriptor. ⇒ A caller that takes the key and does
    /// not issue the ioctl has leaked the aperture, silently.
    ///
    /// Returns `None` for an unknown token, or one already released — so a double release is a
    /// no-op rather than a second ioctl against a stale cookie.
    pub fn take_cpu_view_release(&self, token: u64) -> Option<CpuViewRelease> {
        let mut t = self.backings.lock().unwrap_or_else(|e| e.into_inner());
        let slot = usize::try_from(token).ok().and_then(|i| t.get_mut(i))?;
        match core::mem::replace(slot, ChildBacking::Released) {
            ChildBacking::ArmedNode { node, release } => {
                drop(node);
                Some(release)
            }
            // ⊘ Put back what we took: neither a fabricated backing nor an already-released
            // token may be turned into a tombstone by asking about it.
            other => {
                *slot = other;
                None
            }
        }
    }

    /// A duplicate of `token`'s descriptor, for attaching to a reply.
    ///
    /// A duplicate rather than the descriptor itself: the child keeps its own end, because
    /// the isolate is the party that writes fabricated bytes and a table that gave its
    /// backing away would have exported memory it can no longer serve.
    ///
    /// # Errors
    /// [`RawError::UnknownExport`] for a token this table never minted; otherwise `dup`'s
    /// refusal.
    pub fn lend(&self, token: u64) -> Result<OwnedFd, RawError> {
        let t = self.backings.lock().unwrap_or_else(|e| e.into_inner());
        let backing = usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .ok_or(RawError::UnknownExport { token })?;
        match backing {
            ChildBacking::Fabricated(ram) => ram.dup_for_export(),
            // ★ The armed node is duplicated, not surrendered: the child keeps its end for
            // the same reason it keeps a fabricated backing's — and here there is a second
            // reason, sharper. Closing the isolate's last reference would run
            // `nv_free_file_private` and **tear down the very mmap context the VMM is about
            // to consume**, turning a correct-looking export into a failing `mmap`.
            ChildBacking::ArmedNode { node, .. } => {
                node.as_fd()
                    .try_clone_to_owned()
                    .map_err(|e| RawError::Syscall {
                        call: "dup",
                        errno: e.raw_os_error(),
                    })
            }
            // ⊘ A released token is not an unknown one, and saying so matters: an unknown token
            // is a bug in the caller, a released one is a use-after-release. Same refusal type,
            // but the token is real and the distinction is visible in the message.
            ChildBacking::Released => Err(RawError::UnknownExport { token }),
        }
    }

    /// How many backings this isolate has minted. Diagnostics and tests only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.backings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Whether this isolate has minted none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The VMM's table of backings an isolate handed it.
///
/// One per isolate, shared by that isolate's pool workers — see [`ChildExports`] for why
/// the scope is the isolate rather than the worker.
#[derive(Debug, Default)]
pub struct ExportRegistry {
    /// ★★★★★ **w734 — `Option`, and the `None` is a TOMBSTONE, not an absence.**
    ///
    /// The token IS the index, so a retired crossing cannot be removed — `Vec::remove` would
    /// renumber every later token under live mappings. ⊘ It is replaced in place, which drops
    /// the [`CrossedFd`] and closes this process's descriptor while leaving every other
    /// token where it was. Exactly the shape [`ChildExports::take_cpu_view_release`] already
    /// uses one process over, and for the same reason.
    adopted: Mutex<Vec<Option<CrossedFd>>>,
}

impl ExportRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ★★★ Adopt a descriptor that arrived on an export reply, **checking it is memory**,
    /// and mint the parent-scoped token for it.
    ///
    /// [`CrossedFd::adopt`] is given [`DescriptorKind::RegularFile`] as the promise, so a
    /// child that answers with a character device — its own `/dev/nvidia0`, or anything
    /// else it can open — is refused by name with `RawError::DescriptorKindRefused`
    /// **and the descriptor is closed on the way out**, because `adopt` takes it by value.
    ///
    /// ★ This is the enforcement point for the property the whole verb exists for. The
    /// backend refusing [`kayfabe_isolate::ExportSource::HostDeviceMemory`] is a decision
    /// made by *our* code inside the child; this check does not trust the child at all.
    /// A compromised isolate is inside the threat model (`l1_os_shell.md` §11) and the two
    /// mechanisms are deliberately independent.
    ///
    /// ★★★ **`want` is the kind the CALLER asked for — never the kind the child claims.**
    /// Added 2026-08-27 with [`ChildExports::mint_armed_node`]; before it, this was hard-wired
    /// to [`DescriptorKind::RegularFile`].
    ///
    /// The peer-trust property is unchanged and that is the point: the *actual* kind is still
    /// established from the **kernel** inside [`CrossedFd::adopt`]. `want` only narrows what
    /// this particular crossing will accept. A child answering a fabricated-memory request
    /// with a device node is refused exactly as it was before this parameter existed, because
    /// that caller passes `RegularFile`.
    ///
    /// # Errors
    /// `RawError::DescriptorKindRefused` when the descriptor is not of kind `want`.
    pub fn adopt(
        &self,
        fd: OwnedFd,
        from: IsolateId,
        want: DescriptorKind,
    ) -> Result<u64, RawError> {
        let crossed = CrossedFd::adopt(fd, FdOrigin::Isolate(from), want)?;
        let mut t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        t.push(Some(crossed));
        Ok(t.len() as u64 - 1)
    }

    /// ★★★★★ **w734 — GIVE THIS PROCESS'S DESCRIPTOR BACK.** Returns whether a live crossing
    /// was there to retire.
    ///
    /// # ⊘⊘⊘ Why this had to exist before §3 could recycle anything
    ///
    /// `[surveyed w734]` this registry was **append-only**: `adopt` pushed and nothing ever
    /// removed. Every crossing therefore retained one adopted descriptor in the VMM **for the
    /// life of the isolate**, and for an armed `/dev/nvidia<N>` node that is not merely an fd:
    /// the armed `mmap` context lives on the `struct file` and is freed by
    /// `nv_free_file_private` only when the **last** reference goes. ⇒ the child's own
    /// `take_cpu_view_release` closing its end frees nothing while this process still holds a
    /// duplicate.
    ///
    /// ⚠ The BAR1 **aperture** was never the leak — `[measured w722]` that comes back through
    /// the explicit `NV_ESC_RM_UNMAP_MEMORY`, 224 MiB a round where `munmap` + `close`
    /// returns zero. What grows without bound is the **descriptor count**, and
    /// `SINGLE_STORE_PLAN.md` §3 is a design in which the BAR mirror arms and releases views
    /// in a loop. A default `RLIMIT_NOFILE` of 1024 is reached by a few hundred views.
    ///
    /// ★ **Retiring does not invalidate a live mapping.** A VMA holds its own reference to the
    /// `struct file`, which is the same property `install_device_window`'s *"the caller may
    /// close it on return — and under decision (b) it SHOULD"* rests on. This closes OUR
    /// descriptor; it does not unmap anything.
    ///
    /// ⊘ A token this registry never minted, or one already retired, is `false` — **not a
    /// panic and not a silent `true`**. A double retire must be a no-op that says so, for
    /// `take_cpu_view_release`'s reason: a caller that could not tell *"I closed it"* from
    /// *"it was already gone"* cannot detect a double release at all.
    pub fn retire(&self, token: u64) -> bool {
        let mut t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = usize::try_from(token).ok().and_then(|i| t.get_mut(i)) else {
            return false;
        };
        // ⊘ `.take()` drops the `CrossedFd` here, under the lock. That is a `close(2)`, which
        // is not a blocking call and takes no ranked lock — the `munmap` that WOULD be one is
        // not ours to do: this registry never mapped anything.
        slot.take().is_some()
    }

    /// A duplicate of `token`'s descriptor, for the VMM's own `mmap` and memslot install.
    ///
    /// A duplicate so the caller may hold it across an unmap of the registry, and so that
    /// no borrow of the table outlives its lock — the same argument
    /// `KvmMachine::map_guest` already makes for its own exports.
    ///
    /// # Errors
    /// [`RawError::UnknownExport`] for a token this registry never minted.
    pub fn dup(&self, token: u64) -> Result<OwnedFd, RawError> {
        let t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        let crossed = usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .and_then(Option::as_ref)
            // ⊘ A RETIRED token and one that was never minted are the same refusal here on
            // purpose: both mean "this registry has no descriptor for you", and a caller that
            // distinguished them would be reasoning about a slot's history rather than about
            // what it can map.
            .ok_or(RawError::UnknownExport { token })?;
        crossed
            .as_local_fd()
            .try_clone_to_owned()
            .map_err(|e| RawError::Syscall {
                call: "dup",
                errno: e.raw_os_error(),
            })
    }

    /// ★ What the **kernel** says `token`'s descriptor is — established at
    /// [`ExportRegistry::adopt`], never claimed by the peer.
    ///
    /// Exists so the property *"what crossed is memory"* is assertable from outside this
    /// module without a descriptor changing hands to ask.
    #[must_use]
    pub fn kind(&self, token: u64) -> Option<DescriptorKind> {
        let t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .and_then(Option::as_ref)
            .map(CrossedFd::kind)
    }

    /// Which isolate handed `token` over.
    #[must_use]
    pub fn origin(&self, token: u64) -> Option<FdOrigin> {
        let t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .and_then(Option::as_ref)
            .map(CrossedFd::origin)
    }

    /// How many **live** backings this registry holds — retired slots do not count.
    ///
    /// ⊘ w734: this used to be the vector's length, which after [`ExportRegistry::retire`]
    /// exists would be *"how many tokens were ever minted"*. Both are real numbers and they
    /// answer different questions; this one is *"what is still held"*, which is what its
    /// callers (`is_empty`, the teardown assertions) are asking. [`ExportRegistry::minted`]
    /// is the other.
    #[must_use]
    pub fn len(&self) -> usize {
        self.adopted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|s| s.is_some())
            .count()
    }

    /// How many tokens this registry has ever minted, retired ones included — the token
    /// space's high-water.
    ///
    /// ★ Worth its own number because *"nothing is held"* and *"nothing ever crossed"* are
    /// different facts, and after `retire` exists [`ExportRegistry::len`] alone cannot tell
    /// them apart. A teardown census reporting `len=0 minted=0` is a boot where the crossing
    /// never happened; `len=0 minted=57` is one where it happened 57 times and was cleaned up.
    #[must_use]
    pub fn minted(&self) -> usize {
        self.adopted.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether it holds none **live**.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// The registries are stored inside an `Isolate`, which the core stores in a `Sync` `Proc`
// (decision #17). Asserted here so the bound cannot be dropped without this line failing.
kayfabe_util::assert_send_sync!(ChildExports, ExportRegistry);

#[cfg(test)]
mod release_tests {
    use super::*;

    /// ⊘⊘⊘ **ASKING MUST NOT DESTROY.** `take_cpu_view_release` replaces the slot to read it, so
    /// a token that is *not* an armed node has to be **put back**. Get this wrong and querying a
    /// fabricated backing silently tombstones it — the export still has a token, the token still
    /// looks valid, and the bytes are gone. ⚠ Nothing else in this table would report that.
    #[test]
    fn asking_a_fabricated_token_for_a_release_does_not_destroy_it() {
        let t = ChildExports::new();
        let token = t.mint(4096).expect("mint");
        assert!(
            t.take_cpu_view_release(token).is_none(),
            "a fabricated backing has no CPU view to release"
        );
        // ★ The real assertion: it must still be lendable afterwards.
        assert!(
            t.lend(token).is_ok(),
            "asking about a fabricated token destroyed it — the 'put back' arm is missing"
        );
        // And asking twice is still harmless.
        assert!(t.take_cpu_view_release(token).is_none());
        assert!(t.lend(token).is_ok(), "a second ask destroyed it");
    }

    /// An unknown token is `None`, not a panic and not an index into somebody else's slot.
    #[test]
    fn an_unknown_token_has_nothing_to_release() {
        let t = ChildExports::new();
        assert!(t.take_cpu_view_release(0).is_none(), "empty table");
        let token = t.mint(4096).expect("mint");
        assert!(t.take_cpu_view_release(token + 1).is_none(), "past the end");
        assert!(t.take_cpu_view_release(u64::MAX).is_none(), "wildly past the end");
    }

    /// ★ **The token is the INDEX**, which is why a release tombstones rather than removes.
    /// This pins the consequence: a release must not shift any other token's meaning.
    #[test]
    fn a_release_does_not_renumber_the_tokens_around_it() {
        let t = ChildExports::new();
        let a = t.mint(4096).expect("a");
        let b = t.mint(4096).expect("b");
        let c = t.mint(4096).expect("c");
        assert_eq!((a, b, c), (0, 1, 2), "tokens are dense indices");
        // Nothing here is an armed node, so nothing is released — but the shape is what matters:
        // every token must still name what it named.
        let _ = t.take_cpu_view_release(b);
        assert!(t.lend(a).is_ok() && t.lend(b).is_ok() && t.lend(c).is_ok());
    }
}
