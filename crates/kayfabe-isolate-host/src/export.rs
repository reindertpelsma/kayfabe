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
//! ## ⊘ What is NOT here
//!
//! No device descriptor, in either table. [`ChildExports::mint`] can only ever produce a
//! `memfd`, and [`ExportRegistry::adopt`] refuses anything that is not a regular file
//! **before** it is reachable. The class that cannot be exported is refused one layer up,
//! in the backend, by name — see `kayfabe_isolate::RmError::NotExportableAsMemory`.

use crate::fdcross::{CrossedFd, FdOrigin};
use kayfabe_isolate::IsolateId;
use kayfabe_linux_raw::{DescriptorKind, RawError, SharedRam};
use std::os::fd::OwnedFd;
use std::sync::Mutex;

/// The isolate's own table of backings it minted for the VMM.
///
/// One per child process, shared by every worker thread: a backing is an isolate-scoped
/// resource, not a worker-scoped one, and a worker-scoped table would make the token's
/// meaning depend on which pool slot happened to serve the request.
#[derive(Debug, Default)]
pub struct ChildExports {
    backings: Mutex<Vec<SharedRam>>,
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
        t.push(ram);
        Ok(t.len() as u64 - 1)
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
        let ram = usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .ok_or(RawError::UnknownExport { token })?;
        ram.dup_for_export()
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
    adopted: Mutex<Vec<CrossedFd>>,
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
    /// # Errors
    /// `RawError::DescriptorKindRefused` when the descriptor is not a regular file.
    pub fn adopt(&self, fd: OwnedFd, from: IsolateId) -> Result<u64, RawError> {
        let crossed = CrossedFd::adopt(fd, FdOrigin::Isolate(from), DescriptorKind::RegularFile)?;
        let mut t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        t.push(crossed);
        Ok(t.len() as u64 - 1)
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
            .map(CrossedFd::kind)
    }

    /// Which isolate handed `token` over.
    #[must_use]
    pub fn origin(&self, token: u64) -> Option<FdOrigin> {
        let t = self.adopted.lock().unwrap_or_else(|e| e.into_inner());
        usize::try_from(token)
            .ok()
            .and_then(|i| t.get(i))
            .map(CrossedFd::origin)
    }

    /// How many backings this registry holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.adopted.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// The registries are stored inside an `Isolate`, which the core stores in a `Sync` `Proc`
// (decision #17). Asserted here so the bound cannot be dropped without this line failing.
kayfabe_util::assert_send_sync!(ChildExports, ExportRegistry);
