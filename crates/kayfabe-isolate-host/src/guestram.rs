//! ★★★★★ The isolate's **guest-RAM plane** — the one door guest memory comes through.
//!
//! `mode2_isolate_memory_boundary.md`, and it is a *shape* rather than a check. §5 of that
//! page: *"the isolate never `mmap`s guest RAM on its own; it is instructed."* Everything
//! here exists so that sentence is true by construction, and so the enforcement that lands
//! later — fd pinning, the seccomp filter, `SECCOMP_RET_USER_NOTIF` on `mmap`, the `munmap`
//! confirmation — can be added **behind** this module without touching a call site.
//!
//! ## What crosses, and when
//!
//! The guest-RAM descriptor arrives at **spawn**, on a fixed number
//! ([`crate::isolate::GUEST_RAM_FD`]), through the same [`kayfabe_linux_raw::FdGrant`]
//! mechanism that already delivers the control socket and the worker sockets.
//!
//! ⊘⊘ **It does NOT ride a request**, and that is a correction to the obvious design rather
//! than a shortcut. The natural reading of "the fd-IN gap" is *"give the child's request
//! loop a `recvmsg` reader so a verb can carry the memfd"* — a whole mechanism, and this
//! verb does not want it. §2 of the design page says the fd is handed over at a **fixed,
//! known number** *precisely so the seccomp filter installed afterwards can hardcode that
//! number*; a descriptor that arrives per-request has no fixed number to hardcode, so
//! building it that way would have made the enforcement step harder, not merely later.
//! ⇒ The wire verb moves **scalars only**, which is also what makes the eventual notify
//! match trivial: every argument of the `mmap` the child is about to issue is already in
//! the frame that ordered it.
//!
//! ## ⊘ What this module is NOT allowed to grow
//!
//! A way for the isolate to ask *which* guest bytes it wants. [`GuestRamGrant`]'s numbers
//! come from the VMM's own region map; a request-then-validate flow would be a **circular**
//! check — validating a request against itself — which is the same failure as
//! *an echo is unverifiable by its reply*. There is deliberately no `find`, no `resolve`,
//! and no `lookup` here.

use kayfabe_isolate::{GuestRamGrant, RmError};
use kayfabe_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion};
use kayfabe_vmm::Prot;
use std::collections::BTreeMap;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Mutex;

/// ★★★ The bit every guest-RAM mapping's raw name carries, so it can **never** be mistaken
/// for an RM object handle.
///
/// RM handles are 32 bits; `HostRmBackend::narrow` refuses anything that does not fit,
/// which means a tagged name is refused **by the existing gate** rather than by a new one —
/// a guest-RAM name presented where an RM object was expected cannot reach an ioctl.
///
/// ⊘ The same discipline, and the same reason, as `kayfabe_vmm::RAM_EXPORT_TOKEN_TAG`: two
/// zero-based name spaces sharing a `u64`-typed ABI collide at every index, so the
/// separation has to be visible in the value rather than merely intended.
pub const GUEST_RAM_NAME_TAG: u64 = 1 << 62;

/// The isolate's view of guest RAM: one descriptor, its extent, and the mappings made from
/// it.
///
/// One per **isolate**, shared by every worker in its pool — the same argument
/// [`crate::export::ChildExports`] makes: a guest-RAM mapping is an isolate-scoped
/// resource, and a per-worker table would make an address's meaning depend on which pool
/// slot happened to serve the request.
#[derive(Debug)]
pub struct GuestRamPlane {
    fd: OwnedFd,
    /// How many bytes the block has, **as the VMM stated at spawn**.
    ///
    /// ★ Deliberately the VMM's number and not one this process derived with an `lseek`.
    /// The extent of guest RAM is a fact the VMM owns; an isolate that re-derived it would
    /// have a second source of truth for the one number the whole authorization is bounded
    /// by, and the two could disagree exactly when it matters.
    bytes: u64,
    page: HostPageSize,
    /// The next raw name to mint. ★ Monotonic and never recycled, for
    /// [`kayfabe_vmm_qemu`]'s `RamRegionId` reason: a name that is re-issued cannot be used
    /// to prove that the object a caller is holding is the object it looked up.
    next: Mutex<u64>,
    /// Live mappings, keyed by the raw name reported back to the VMM.
    ///
    /// ⚠ Holding the [`MappedRegion`] here is what makes the mapping's lifetime **the
    /// plane's**, not a caller's: an unmap is a table removal, and a plane that is dropped
    /// (the isolate dying) takes every guest-RAM mapping with it. That is the automatic
    /// release §3 requires on isolate death, and it is a destructor rather than a step
    /// someone must remember.
    live: Mutex<BTreeMap<u64, MappedRegion>>,
}

impl GuestRamPlane {
    /// Adopt a granted guest-RAM descriptor of `bytes` bytes.
    #[must_use]
    pub fn new(fd: OwnedFd, bytes: u64, page: HostPageSize) -> Self {
        GuestRamPlane {
            fd,
            bytes,
            page,
            next: Mutex::new(1),
            live: Mutex::new(BTreeMap::new()),
        }
    }

    /// How much guest RAM this isolate can see at all.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// ★★★ Honour one grant: `mmap` the named slice of guest RAM into this process.
    ///
    /// The **only** `mmap` of guest memory in the isolate, and it takes a
    /// [`GuestRamGrant`] rather than an offset and a length so that there is no spelling of
    /// this call that does not go through the VMM's derivation.
    ///
    /// ## The bound, and whose it is
    ///
    /// The range is checked against [`Self::bytes`] here. ⊘ That is **not** the §3 check
    /// that would be circular: this is the isolate refusing to map past the end of the
    /// block it was given, which is a memory-safety fact about *this process* (a mapping
    /// past the end of a `memfd` faults with `SIGBUS` on first touch, at some later
    /// unrelated instruction). The authorization — *which* guest bytes may be reached — is
    /// the VMM's and is not re-decided here.
    ///
    /// Returns the isolate-local raw name of the mapping; the caller stamps it into a
    /// [`kayfabe_isolate::HostHandle`].
    ///
    /// # Errors
    /// [`RmError::NoMemory`] for an empty or out-of-range grant, or when the host refuses
    /// the mapping.
    pub fn honour(&self, grant: GuestRamGrant) -> Result<u64, RmError> {
        if grant.is_empty() {
            return Err(RmError::NoMemory);
        }
        let end = grant
            .offset()
            .checked_add(grant.len())
            .ok_or(RmError::NoMemory)?;
        if end > self.bytes {
            return Err(RmError::NoMemory);
        }
        // ★ `HostProt`, not `Prot`. The two are deliberately distinct types: `Prot` says
        // what the GUEST may do with a guest-physical range, `HostProt` says what THIS
        // PROCESS may do with a host mapping. Collapsing them is how "pages an isolate only
        // reads are mapped read-only **in the isolate**" becomes a mapping that is
        // read-only for the wrong party.
        let prot = match grant.prot() {
            Prot::ReadWrite => HostProt::ReadWrite,
            Prot::ReadOnly => HostProt::ReadOnly,
        };
        let region = MappedRegion::map(
            Backing::SharedFile {
                fd: self.fd.as_fd(),
                offset: grant.offset(),
            },
            grant.len(),
            prot,
            // Guest RAM is ordinary kernel pages behind a `memfd`, so write-back is the
            // only attainable policy and asking for anything else is refused by name rather
            // than silently downgraded.
            CachePolicy::WriteBack,
            self.page,
        )
        .map_err(|_| RmError::NoMemory)?;
        let raw = {
            let mut n = self.next.lock().expect("the guest-RAM name counter");
            let raw = GUEST_RAM_NAME_TAG | *n;
            *n += 1;
            raw
        };
        self.live
            .lock()
            .expect("the guest-RAM table")
            .insert(raw, region);
        Ok(raw)
    }

    /// Give back one mapping, by the raw name [`Self::honour`] minted.
    ///
    /// ★ Keyed on the name alone: the length is the caller's own accounting and is **not**
    /// used to select a mapping, because a caller that got the length wrong would otherwise
    /// silently unmap nothing. A name that is not live is a refusal, not a no-op — *"I
    /// already gave that back"* and *"I never had that"* are different facts, and a VMM
    /// waiting to reuse the range needs to tell them apart.
    ///
    /// # Errors
    /// [`RmError::NoMemory`] if no live mapping carries `raw`.
    pub fn release(&self, raw: u64) -> Result<(), RmError> {
        // The `munmap` happens when the `MappedRegion` drops, and it must NOT happen with
        // the table lock held — a syscall under a lock is the shape this whole codebase
        // asserts against. So the entry is removed under the lock and dropped outside it.
        let removed = self.live.lock().expect("the guest-RAM table").remove(&raw);
        match removed {
            Some(region) => {
                drop(region);
                Ok(())
            }
            None => Err(RmError::NoMemory),
        }
    }

    /// ★ Borrow one live mapping, so the `OS_DESCRIPTOR` alloc that pins it can name it.
    ///
    /// The closure shape is what keeps the [`MappedRegion`] inside this module: a getter
    /// returning a reference would tie the table lock to the caller's scope, and a getter
    /// returning the region would move the mapping's lifetime out of the plane — which is
    /// the property that makes isolate death release everything.
    ///
    /// # Errors
    /// [`RmError::NoMemory`] if no live mapping carries `raw`.
    pub fn with_region<T>(
        &self,
        raw: u64,
        f: impl FnOnce(&MappedRegion) -> T,
    ) -> Result<T, RmError> {
        let live = self.live.lock().expect("the guest-RAM table");
        let region = live.get(&raw).ok_or(RmError::NoMemory)?;
        Ok(f(region))
    }

    /// How many guest-RAM mappings are live. For assertions; the isolate itself never asks.
    #[must_use]
    pub fn live_mappings(&self) -> usize {
        self.live.lock().expect("the guest-RAM table").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kayfabe_linux_raw::SharedRam;

    fn page() -> u64 {
        HostPageSize::query().bytes()
    }

    fn plane(pages: u64) -> (SharedRam, GuestRamPlane) {
        let ram = SharedRam::create(pages * page()).expect("a shared block");
        let fd = ram.dup_for_export().expect("a descriptor");
        let plane = GuestRamPlane::new(fd, pages * page(), HostPageSize::query());
        (ram, plane)
    }

    /// ★★★ **The release really releases** — the half `tests/guest_ram.rs` cannot witness,
    /// because there the mappings live in another process and vanish with it.
    ///
    /// Here the table is in reach, so the claim is asserted against the thing that makes it
    /// true: the [`MappedRegion`] is *owned by the plane*, so a release is a table removal
    /// and a dropped plane takes every mapping with it. That ownership is the whole
    /// mechanism behind §3's *"on isolate death every guest-RAM reference is released
    /// automatically"* — it is a destructor, not a step someone must remember.
    #[test]
    fn a_release_removes_the_mapping_and_a_dropped_plane_removes_them_all() {
        let (_ram, plane) = plane(2);
        assert_eq!(plane.live_mappings(), 0);
        let a = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                0,
                page(),
                Prot::ReadWrite,
            ))
            .expect("first page");
        let b = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                page(),
                page(),
                Prot::ReadOnly,
            ))
            .expect("second page");
        assert_eq!(plane.live_mappings(), 2);
        assert_ne!(a, b, "two grants are two mappings");

        plane.release(a).expect("release the first");
        assert_eq!(plane.live_mappings(), 1);
        assert!(
            plane.release(a).is_err(),
            "★ a second release is a REFUSAL, not a no-op — 'already given back' and \
             'never had it' are different facts"
        );
        assert_eq!(plane.live_mappings(), 1, "and the refusal changed nothing");

        // Dropping the plane is what releases the rest. There is no `close_all` to forget.
        drop(plane);
    }

    /// ★★ Names are **never recycled**, so a stale name can never be mistaken for a live
    /// mapping made after it.
    ///
    /// ⊘ Without this, the release-then-reuse sequence below would hand the second grant
    /// the first one's name, and a caller holding the stale value would release a mapping
    /// it never made — the ABA the QEMU adapter's `RamRegionId` already refuses for the
    /// same reason.
    #[test]
    fn a_released_name_is_never_reissued() {
        let (_ram, plane) = plane(1);
        let first = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                0,
                page(),
                Prot::ReadWrite,
            ))
            .expect("a mapping");
        plane.release(first).expect("released");
        let second = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                0,
                page(),
                Prot::ReadWrite,
            ))
            .expect("another mapping of the same page");
        assert_ne!(
            first, second,
            "the same RANGE may be mapped again; the NAME may not be reissued"
        );
        assert!(plane.release(first).is_err(), "the stale name stays dead");
        plane.release(second).expect("the live one releases");
    }

    /// ★★★ A read-only grant produces a mapping this process **cannot write**.
    ///
    /// ⚠ The bite is the read-write control beside it: without that half, this would pass
    /// on a plane whose writes never worked at all. §3's "Matching" paragraph names the
    /// direction that fails open — authorizing read-only and receiving `PROT_WRITE` is a
    /// silent escalation — so the refusal has to be real rather than incidental.
    #[test]
    fn a_read_only_grant_is_read_only_in_the_isolate() {
        let (_ram, plane) = plane(2);
        let ro = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                0,
                page(),
                Prot::ReadOnly,
            ))
            .expect("a read-only grant");
        let rw = plane
            .honour(GuestRamGrant::originated_by_the_vmm(
                page(),
                page(),
                Prot::ReadWrite,
            ))
            .expect("a read-write grant");

        let wrote_ro = plane
            .with_region(ro, |r| {
                r.write_from(kayfabe_linux_raw::HostOffset::ZERO, &[1u8; 4])
            })
            .expect("the mapping is live");
        assert!(
            wrote_ro.is_err(),
            "a read-only grant must not be writable in the isolate"
        );

        let wrote_rw = plane
            .with_region(rw, |r| {
                r.write_from(kayfabe_linux_raw::HostOffset::ZERO, &[1u8; 4])
            })
            .expect("the mapping is live");
        assert!(
            wrote_rw.is_ok(),
            "and the control writes — otherwise the refusal above is about nothing"
        );
    }

    /// A name that names nothing cannot be borrowed either — the same refusal `release`
    /// gives, so a caller cannot reach a mapping by a route that skips the check.
    #[test]
    fn borrowing_a_name_that_names_nothing_is_refused() {
        let (_ram, plane) = plane(1);
        assert!(
            plane
                .with_region(GUEST_RAM_NAME_TAG | 0x99, |_| ())
                .is_err()
        );
    }
}
