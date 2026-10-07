//! ★★ [`GuestWindow`] — the **coarse region with an allocator inside** (§6.7), as an
//! address range that is mapped for its whole life.
//!
//! `l1_os_shell.md` §6.7 is the rule this type is the mechanism for:
//!
//! > **One memslot per window (or per arena grant) — never one per published object.**
//! > Making a backing guest-visible is a `MAP_FIXED` placement inside an already-installed
//! > window and performs **no KVM ioctl at all**.
//!
//! ## Why this is not [`crate::Reservation`], and why that is not duplication
//!
//! They answer opposite questions and have opposite invariants, and collapsing them would
//! destroy the property each exists for.
//!
//! | | `Reservation` | `GuestWindow` |
//! |---|---|---|
//! | initial state | `PROT_NONE` — **nothing is readable** | fully mapped anonymous RW — **everything is readable** |
//! | who may hold it | one owner (`&mut self` to place) | many threads (`&self` throughout) |
//! | a hole in it | impossible: placements may not overlap | the normal state, restored to anonymous |
//! | what it protects | the `MAP_FIXED` breakout surface (§4.4) | a guest-physical range that must **never stop resolving** |
//!
//! The second row is the whole design. A hypervisor's guest RAM is live to other threads
//! at every instant, so the window's own address range is established once and is never
//! unmapped until the object dies. §6.7 item 3 states the consequence as a rule — *"an
//! unmap inside a window is `mmap(MAP_FIXED|MAP_ANONYMOUS|MAP_NORESERVE)` restoring
//! anonymous backing, **never a plain `munmap`**, which punches a hole in the window's VMA
//! and leaves the live memslot pointing at a gap"* — and [`GuestWindow::restore`] is that
//! rule with no way to express the alternative: there is no `munmap`-a-sub-range door on
//! this type at all.
//!
//! ★ **That rule is also what makes concurrent access sound**, which the doc does not say
//! and which is the reason this type can be `Sync` at all. Because the address range is
//! mapped from construction to `Drop`, a reader that resolved an offset and then lost its
//! race against a concurrent [`GuestWindow::restore`] reads **anonymous zeroes**, not a
//! use-after-`munmap`. The failure mode of losing the race is a value, not a fault.
//!
//! ## ★ The `Send`/`Sync` grant — per type and argued, as §14.5 item 1 asked for
//!
//! M2-b granted neither to any region type, and predicted that the temptation would be
//! *"one blanket `unsafe impl` in the dangerous file"*. This is the per-type, argued form:
//!
//! - **`Send`** — the object is `(base, len, page)`, none of which is thread-affine. The
//!   mapping belongs to the *process*, not to the thread that made it, and `munmap` from
//!   another thread is as valid as from this one.
//! - **`Sync`** — every method takes `&self`, and the two that mutate the address range
//!   ([`GuestWindow::place`], [`GuestWindow::restore`]) mutate it **through the kernel**,
//!   which replaces the page-table entries for the range atomically under its own
//!   `mmap_lock`. There is no field to tear: `base` and `len` are established by the
//!   constructor and never written again.
//! - **What is deliberately NOT claimed:** that a concurrent copy is free of a data race
//!   in the letter of the abstract machine. It is not, and §4.2.2 already ruled on exactly
//!   this for [`crate::MappedRegion`] — *keep the memcpy*, because a torn read here is
//!   indistinguishable from a hostile guest writing those bytes (a case we are already
//!   required to survive) and the API offers no way to re-read the source, which excludes
//!   the double fetch structurally. The same ruling, the same reason, one layer out.
//!
//! ## ★ A placement that fails leaves no hole — or the window refuses everything (a8)
//!
//! `mmap(MAP_FIXED)` over a live range is not atomic on failure: for a file-backed placement
//! (a memfd, a device node) the kernel clears the old pages before it calls the file's `mmap`
//! handler, and a handler that refuses leaves a gap. A gap is the one thing this type promises
//! never to have, and a `read_into` into it is a `SIGSEGV` from safe code. So a failed
//! placement re-plugs the anonymous filler with `MAP_FIXED_NOREPLACE` — which maps only where
//! nothing is mapped, so it can never overwrite a mapping another thread placed in the gap in
//! the meantime. If the re-plug cannot succeed, the window is **poisoned**: every accessor
//! returns [`RawError::WindowPoisoned`], and `Drop` leaves the range mapped rather than
//! unmap what may not be ours (V3_SEC_PERIMETER.md §4.1, a8). The success path is unchanged:
//! one `mmap`, which the PRAMIN move needs on the vCPU (OWNER_RULINGS: one host map + one
//! `mmap`).
//!
//! ⚠ **OPEN (review 2026-10-04): the gap is visible to CONCURRENT accessors.** Poisoning is
//! check-then-act. Between the kernel clearing the old pages and the re-plug, an accessor on
//! another thread that already passed its `live()` check can copy from the gap (`SIGSEGV`), or
//! — if an unrelated `mmap(NULL)` lands in the gap first — read or write that foreign mapping,
//! and a concurrent placement that passed `live()` can `MAP_FIXED` over it. Concurrent use is
//! the intended model (racing PRAMIN placers on vCPUs). Closing it needs either exclusion
//! between accessors and a placement's `MAP_FIXED` (a lock or seqlock on the copy path, which
//! this type's no-lock design and the vCPU rules forbid today) or an atomic replace (a staging
//! map plus `mremap(MREMAP_FIXED)`, one more syscall per PRAMIN move than OWNER_RULINGS item
//! 4 allows). Both are an owner decision; the hazard is on the failure path of a file-backed
//! placement only (a refusing device or file `mmap` handler).
//!
//! ## Read/write are `&self` and take **no lock**
//!
//! This is the load-bearing difference from the mock harness's shape. `Vmm::gpa_read` is
//! in-lock legal (§6.1) and is entered with one of our ranked locks held; if serving it
//! took a lock of the adapter's that a memory-plane syscall also holds, the adapter would
//! have re-created §6.3's inversion with its *own* lock. Here the copy needs no
//! synchronisation at all, so the adapter's map lock covers only the *lookup* — a bounded
//! `BTreeMap` probe — and never the copy.

use crate::bounds::{self, HostOffset};
use crate::error::{RawError, last_syscall_error};
use crate::geometry;
use crate::mapping_unsafe::Backing;
use crate::page_size::HostPageSize;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use kf_util::lockwitness;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};

/// A guest-physical window's host backing: an address range that is mapped from
/// construction until `Drop`, into which page-granular backings are placed and restored.
///
/// See the module docs for the invariants and for the `Send`/`Sync` argument.
#[derive(Debug)]
pub struct GuestWindow {
    /// **Type invariant:** the start of a live mapping of exactly `len` bytes, for this
    /// object's whole life. Established by the single `mmap` in [`GuestWindow::create`],
    /// never written again, and released by exactly one `munmap` in `Drop`.
    base: NonNull<u8>,
    /// Set when a failed placement could not be undone (a8): from then on every accessor
    /// refuses with [`RawError::WindowPoisoned`] and `Drop` leaks the range.
    poisoned: AtomicBool,
    len: usize,
    page: HostPageSize,
}

// SAFETY: `GuestWindow` owns a process-wide mapping, not a thread-affine resource: the
// fields are a pointer to that mapping, its length and the page size, all established by
// `create` and never mutated afterwards, and the `poisoned` flag, an `AtomicBool`. `munmap` from a different thread
// than `mmap` is valid, so moving the owner between threads is sound.
unsafe impl Send for GuestWindow {}

// SAFETY: every method takes `&self`. `base`, `len` and `page` are immutable for the
// object's life, and `poisoned` is an atomic, so there is no field a concurrent pair of
// `&self` methods can tear. The doors that change what the address range *contains*
// (`place`, `place_sealed`, `place_device_view`, `restore`) do so by `mmap(MAP_FIXED)`, which
// the kernel applies under its own `mmap_lock`; a SUCCESSFUL placement replaces the range
// without unmapping it in between, so a concurrent `read_into` observes old bytes, new bytes or
// a mixture. That mixture is a data race in the letter of the abstract machine and is NOT
// claimed away: §4.2.2's ruling (keep the memcpy; a torn read is indistinguishable from a
// hostile guest write, and the API cannot re-read the source so the double fetch is excluded
// structurally) applies verbatim, one layer out.
// ⚠ RESIDUAL, NOT CLAIMED AWAY (review 2026-10-04): a FAILED file-backed `MAP_FIXED` leaves a
// gap until `replug` runs (a8). An accessor on another thread that passed `live()` before the
// poison can fault in that gap, or touch a foreign mapping that an unrelated `mmap` placed
// there; a concurrent placement can `MAP_FIXED` over such a mapping. The module docs state the
// two ways to close it; until an owner chooses one, this impl's row is OPEN with that reason.
unsafe impl Sync for GuestWindow {}

impl GuestWindow {
    /// Map `len` bytes of anonymous, read-write, `MAP_NORESERVE` address space at a
    /// **kernel-chosen** address — the window.
    ///
    /// `MAP_NORESERVE` because a window is mostly holes: committing swap for a range that
    /// exists to be overwritten by placements is the opposite of what it is for. The
    /// address is never requested, so this call cannot displace an existing mapping —
    /// §4.4's clobber class is unreachable from here, exactly as for
    /// [`crate::Reservation`].
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::Misaligned`] (length not a whole number of
    /// host pages), [`RawError::TooLargeForHost`], [`RawError::Syscall`] — an `mmap` that
    /// the kernel refuses (a window larger than the address space is `ENOMEM`, and that
    /// is a **real** refusal a mock cannot produce).
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create(len: u64, page: HostPageSize) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("mmap (creating a guest-physical window)");
        if len == 0 {
            return Err(RawError::ZeroLength {
                what: "window length",
            });
        }
        geometry::require_aligned(len, page, "window length")?;
        let len_host =
            usize::try_from(len).map_err(|_| RawError::TooLargeForHost { value: len })?;

        // SAFETY: a NULL address requests a kernel-chosen placement, so this call cannot
        // displace any existing mapping in this process. It dereferences no caller memory:
        // `len_host` is a checked `usize` conversion of a non-zero, page-aligned length
        // (three lines above) and every other argument is a constant assembled here. The
        // return value is checked for `MAP_FAILED` immediately below, which is what
        // establishes the type invariant for the value stored.
        let ret = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                len_host,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if ret == libc::MAP_FAILED {
            return Err(last_syscall_error("mmap"));
        }
        let base = NonNull::new(ret.cast::<u8>()).ok_or(RawError::Syscall {
            call: "mmap",
            errno: None,
        })?;
        Ok(GuestWindow {
            base,
            poisoned: AtomicBool::new(false),
            len: len_host,
            page,
        })
    }

    /// `Err(WindowPoisoned)` once a failed placement could not be undone (a8).
    fn live(&self) -> Result<(), RawError> {
        if self.poisoned.load(Ordering::Acquire) {
            Err(RawError::WindowPoisoned)
        } else {
            Ok(())
        }
    }

    /// The window's length in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len as u64
    }

    /// ★ P4: `[off, off + len)` of the window as an opaque, bounds-checked [`HostSpan`] — what a
    /// hypervisor registers as the guest-physical range's backing (QEMU's
    /// `memory_region_init_ram_device_ptr`). `None` if it leaves the window. The window is mapped
    /// from construction until `Drop`; only `unsafe` code can open the span.
    #[must_use]
    pub fn host_span(&self, off: usize, len: usize) -> Option<crate::HostSpan> {
        self.live().ok()?;
        crate::HostSpan::within(self.base, self.len, off, len)
    }

    /// The host page size this window's geometry is expressed in.
    #[must_use]
    pub fn page_size(&self) -> HostPageSize {
        self.page
    }

    /// ★ Place `backing` over `[offset, offset + len)` of the window — the **fine tier**
    /// (§6.7): an `mmap(MAP_FIXED)` inside a range we already own, taking `mmap_lock`, no
    /// hypervisor ioctl, no memslot update, no SRCU grace period.
    ///
    /// **Host protection is always read-write, and that is §6.7 item 4 rather than an
    /// oversight.** KVM's read-only flag is a *slot* property, so guest-visible
    /// read-only-ness is a property of the **window**, not of a placement inside one; a
    /// host-side `PROT_READ` placement would only make *our own* writes fault, which is a
    /// different subject with the same two words (see [`HostProt`]'s docs).
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::Misaligned`], [`RawError::OutOfRange`],
    /// [`RawError::LengthOverflow`], [`RawError::TooLargeForHost`], [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn place(
        &self,
        offset: HostOffset,
        len: u64,
        backing: Backing<'_>,
    ) -> Result<(), RawError> {
        lockwitness::assert_lock_free("mmap MAP_FIXED (placing a backing inside a window)");
        let source = match backing {
            Backing::PrivateAnonymous => Source::Anonymous { noreserve: false },
            Backing::SharedFile { fd, offset } => Source::File { fd, offset },
            // ★★★ Refused by variant, at the door. See `RawError::DeviceBackingNotPlaceable`:
            // a window placement is MAP_FIXED into a range a guest memslot already names, and
            // a device backing is a window onto hardware. There is no call for which the
            // composition is right, so there is no check — there is a refusal.
            Backing::DeviceFile { .. } => return Err(RawError::DeviceBackingNotPlaceable),
        };
        self.fixed_map(offset, len, source, "placement")
    }

    /// ★ Place `[file_offset, file_offset + len)` of a [`SealedFile`] over `[offset, offset +
    /// len)` of the window: [`GuestWindow::place`]'s `SharedFile` arm with NO syscall but the
    /// `mmap` itself.
    ///
    /// The a10 bound (every page of the placement exists in the file) is checked against the
    /// size the [`SealedFile`] recorded when it was adopted, off the vCPU, and that size cannot
    /// fall afterwards (`F_SEAL_SHRINK`). `place` must `fstat` every time, which made a PRAMIN
    /// move two syscalls on the vCPU where OWNER_RULINGS item 4 allows one host map plus one
    /// `mmap` (review 2026-10-04); this door keeps it at one, and it also closes a10's
    /// truncation residual for the files that use it.
    ///
    /// # Errors
    /// As [`GuestWindow::place`]; [`RawError::OutOfRange`] when the placement runs past the
    /// sealed size.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn place_sealed(
        &self,
        offset: HostOffset,
        len: u64,
        file: &SealedFile,
        file_offset: u64,
    ) -> Result<(), RawError> {
        lockwitness::assert_lock_free("mmap MAP_FIXED (placing a sealed file inside a window)");
        self.fixed_map(
            offset,
            len,
            Source::Sealed {
                file,
                offset: file_offset,
            },
            "placement",
        )
    }

    /// ★★★★★ **w393 — place an ARMED DEVICE NODE over `[offset, offset + len)` of the
    /// window**: the guest-side half of `DEVICE_LOCAL | HOST_VISIBLE`.
    ///
    /// # Why this is a second door and not a fourth arm of [`GuestWindow::place`]
    ///
    /// [`GuestWindow::place`] refuses [`Backing::DeviceFile`] **by variant**
    /// ([`RawError::DeviceBackingNotPlaceable`]), and its reason is worth keeping true: a
    /// window placement is `MAP_FIXED` into a range a guest memslot names, so a device
    /// backing placed there is *hardware in the guest's physical address space*. That is
    /// exactly the thing the owner's BAR1 target asks for (2026-09-09: *"bar1/2 should not
    /// have traps and just passthrough"* — the Vulkan pair `DEVICE_LOCAL | HOST_VISIBLE`),
    /// and exactly the thing every other caller of `place` must never do by accident. A
    /// separate verb keeps the refusal where it is and makes the one legitimate use a
    /// **named** act at its call site rather than a matched arm anyone can reach.
    ///
    /// The `RawError::DeviceBackingNotPlaceable` doc says: *"If a legitimate need for a
    /// device mapping inside a window ever appears, it is a design change with an owner
    /// ruling, not a matched arm."* This is that design change, and the ruling it needs is
    /// decision (b)'s scope (`isolate_vmm_fd_crossing.md` §12): whether the VMM may hold —
    /// even transiently, never escaping on it — the `/dev/nvidia<N>` node whose `mmap`
    /// context the isolate armed. ⚠ **Not reachable from any production path until that
    /// ruling lands**; the only caller is the installer verb built for it.
    ///
    /// # What the driver decides, and what it does not
    ///
    /// - File offset is **zero**, always: `nvidia_mmap_helper` refuses any other `vm_pgoff`
    ///   (`ogkm-580: kernel-open/nvidia/nv-mmap.c:533-536`), and *what* is mapped was fixed by
    ///   the `NV_ESC_RM_MAP_MEMORY` that armed the node. So there is no offset parameter.
    /// - `len` must equal the driver's page-rounded registered size or the `mmap` is refused
    ///   with `ENXIO` (`nv-mmap.c:560-565`); the isolate reports that rounded length beside
    ///   the node for this reason.
    /// - The VMA comes back `VM_IO | VM_PFNMAP | VM_DONTEXPAND` (`nv-mmap.c:641`) — the same
    ///   shape a VFIO BAR mapping has, which is what a hypervisor memslot over it relies on.
    ///   ⊘ The **memory type** the guest sees through such a slot is the hypervisor's
    ///   decision for a non-RAM pfn, not the driver's write-combining — a measurement this
    ///   crate cannot make (see [`Backing::attainable_cache_policy`]'s `None`).
    ///
    /// # Errors
    /// As [`GuestWindow::place`], plus whatever the driver's `mmap` handler refuses with.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    /// ⊘⊘⊘ **`writable` is NOT cosmetic, and getting it wrong is an `EACCES` at runtime
    /// (w633).** This used to map `PROT_READ | PROT_WRITE` unconditionally. `[measured w600,
    /// unprivileged]` a node opened `O_RDONLY` — which is the containment the counter-page
    /// crossing depends on — **refuses exactly that mmap with `EACCES`**, so an armed
    /// read-only view could never have been placed by this function.
    ///
    /// ⚠ **Nobody had run it on a read-only node.** The only measurement of this path on a
    /// device node is `rmladder --bar1-crossing` LEG B, whose node is armed
    /// `ViewAccess::ReadWrite`; legs R and R2 map read-only but do so with their own `mmap`,
    /// not through here. ⇒ The two halves were each measured and the COMBINATION was not,
    /// which is the gap a passing test suite is least able to see.
    pub fn place_device_view(
        &self,
        offset: HostOffset,
        len: u64,
        fd: BorrowedFd<'_>,
        writable: bool,
    ) -> Result<(), RawError> {
        lockwitness::assert_lock_free("mmap MAP_FIXED (placing an armed device node)");
        // ★ a9: refused by name. A read-only range inside a window that `write_from` and
        // `store_u32` write through is a SIGSEGV from safe code; the only caller asks for a
        // writable view (kf-qemu `mem.rs`, `place_view`).
        if !writable {
            return Err(RawError::Unsupported {
                what: "a read-only device view inside a guest window",
                detail: "the window's write doors would fault on it; place a writable view",
            });
        }
        self.fixed_map(offset, len, Source::Device { fd }, "device view")
    }

    /// ★★ Restore `[offset, offset + len)` to anonymous zero-fill — the **unmap** of the
    /// fine tier, and the reason there is no `munmap`-a-sub-range door on this type.
    ///
    /// §6.7 item 3, verbatim: *"an unmap inside a window is
    /// `mmap(MAP_FIXED|MAP_ANONYMOUS|MAP_NORESERVE)` restoring anonymous backing, **never
    /// a plain `munmap`**, which punches a hole in the window's VMA and leaves the live
    /// memslot pointing at a gap."* On KVM that gap is not a benign hole: the memslot's
    /// `userspace_addr` still names it, so a guest access to it faults inside the kernel's
    /// `gfn_to_pfn` rather than reading zeroes.
    ///
    /// # Errors
    /// As [`GuestWindow::place`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn restore(&self, offset: HostOffset, len: u64) -> Result<(), RawError> {
        lockwitness::assert_lock_free("mmap MAP_FIXED (restoring anonymous backing)");
        self.fixed_map(
            offset,
            len,
            Source::Anonymous { noreserve: true },
            "restore",
        )
    }

    /// The one `MAP_FIXED` in this file, and EVERY check it relies on: the window's own range,
    /// alignment, and — for a file — that each page of the placement exists in the file now
    /// (a10). No door above hands it a precondition: each passes what it was given, as a
    /// [`Source`], and this fn validates it (rule (a); ⊘ until 2026-10-04 the file offset and
    /// size were "validated by the caller two frames up", review).
    fn fixed_map(
        &self,
        offset: HostOffset,
        len: u64,
        source: Source<'_>,
        what: &'static str,
    ) -> Result<(), RawError> {
        self.live()?;
        if len == 0 {
            return Err(RawError::ZeroLength { what });
        }
        geometry::require_aligned(offset.get(), self.page, what)?;
        geometry::require_aligned(len, self.page, what)?;
        let (start, len_host) = bounds::checked_span(self.len_bytes(), offset, len, what)?;
        let anon = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;
        let (fd, file_offset, share_flags) = match source {
            Source::Anonymous { noreserve: false } => (-1, 0, anon),
            Source::Anonymous { noreserve: true } => (-1, 0, anon | libc::MAP_NORESERVE),
            Source::File { fd, offset } => {
                // ★ a10: every page of the placement must exist in the file NOW. A page past
                // end-of-file is mapped without complaint and then raises SIGBUS on the first
                // access, which `read_into` would make from safe code. A file truncated AFTER
                // placement is the residual (OPEN); `place_sealed` closes it.
                let off = self.file_extent(offset, len, file_size(fd)?)?;
                (fd.as_raw_fd(), off, libc::MAP_SHARED)
            }
            Source::Sealed { file, offset } => {
                let off = self.file_extent(offset, len, file.len)?;
                (file.fd.as_raw_fd(), off, libc::MAP_SHARED)
            }
            // A device node's `mmap` handler bounds what it maps (the NVIDIA driver refuses any
            // `vm_pgoff` but zero and any length but the armed one, `nv-mmap.c:533-565`).
            Source::Device { fd } => (fd.as_raw_fd(), 0, libc::MAP_SHARED),
        };

        // SAFETY: the address computation only. `start + len_host <= self.len` was proven
        // above by `checked_span`, with the overflow checked BEFORE the bound, and the
        // type invariant says `base` is a live mapping of `self.len` bytes, so `base.add(start)`
        // stays inside that allocation (the precondition of `add`).
        let target = unsafe { self.base.as_ptr().add(start).cast::<libc::c_void>() };
        // SAFETY: `map_fixed`'s contract, each half established in this fn:
        //  (a) `target..target + len_host` lies inside this window's own live mapping, which was
        //      created at a KERNEL-CHOSEN address (so it displaced nothing) and is not released
        //      until `Drop` — `MAP_FIXED` can replace only our own pages, never the
        //      hypervisor's heap, our stack or our text;
        //  (b) offset and length are host-page-aligned (checked above), and a file's
        //      `[file_offset, file_offset + len)` exists in it (a10, `file_extent`) or is the
        //      device driver's to bound;
        //  (c) no reference into the range can be outstanding: this type hands out no borrows
        //      at all — `read_into`/`write_from` copy and return.
        let ret = unsafe {
            map_fixed(
                target,
                len_host,
                libc::PROT_READ | libc::PROT_WRITE,
                share_flags,
                fd,
                file_offset,
            )
        };
        if ret == libc::MAP_FAILED {
            let err = last_syscall_error("mmap");
            // SAFETY: `replug`'s contract: `target`/`len_host` are exactly the in-bounds,
            // page-aligned range of this window the failed `MAP_FIXED` above named.
            unsafe { self.replug(target, len_host) };
            return Err(err);
        }
        Ok(())
    }

    /// a10: `[offset, offset + len)` of a file of `size` bytes, page-aligned and inside it; the
    /// offset as the `off_t` `mmap` takes.
    fn file_extent(&self, offset: u64, len: u64, size: u64) -> Result<libc::off_t, RawError> {
        geometry::require_aligned(offset, self.page, "file offset")?;
        let off = libc::off_t::try_from(offset)
            .map_err(|_| RawError::TooLargeForHost { value: offset })?;
        let end = offset
            .checked_add(len)
            .ok_or(RawError::LengthOverflow { offset, len })?;
        if end > size {
            return Err(RawError::OutOfRange {
                offset,
                len,
                object_len: size,
            });
        }
        Ok(off)
    }

    /// ★ a8: after a failed `MAP_FIXED`, put the anonymous filler back where the kernel may
    /// have left a gap — with `MAP_FIXED_NOREPLACE`, which maps only where nothing is mapped,
    /// so it cannot overwrite a mapping another thread placed in the gap. If anything is
    /// mapped there (`EEXIST`: the old placement, which a failed anonymous `MAP_FIXED` keeps,
    /// or somebody else's), or the kernel ignores the flag, the range cannot be proved whole:
    /// the window is poisoned. ⚠ OPEN: a foreign anonymous mapping that merged into the
    /// filler's VMA is indistinguishable from the filler (V3_SEC_PERIMETER.md §4.1, a8).
    ///
    /// # Safety
    ///
    /// `target..target + len_host` is the page-aligned range of THIS window that a failed
    /// `MAP_FIXED` named. On success the re-plug is recorded as the window's filler again; given
    /// any other range, a successful `MAP_FIXED_NOREPLACE` would heal nothing and the window
    /// would go on reporting a gap as whole (rule (a): an unchecked precondition, so `unsafe`).
    unsafe fn replug(&self, target: *mut libc::c_void, len_host: usize) {
        // SAFETY: `MAP_FIXED_NOREPLACE` creates a mapping only if NO page of the range is
        // mapped, and fails with `EEXIST` otherwise, so this call can displace nothing — not
        // ours, not another thread's. `target`/`len_host` are this window's page-aligned range
        // by this fn's contract; the flags and protections are constants.
        let ret = unsafe {
            libc::mmap(
                target,
                len_host,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE
                    | libc::MAP_ANONYMOUS
                    | libc::MAP_NORESERVE
                    | libc::MAP_FIXED_NOREPLACE,
                -1,
                0,
            )
        };
        if ret == target {
            return;
        }
        if ret != libc::MAP_FAILED {
            // A kernel without MAP_FIXED_NOREPLACE treats it as a hint and maps elsewhere.
            // SAFETY: `ret` is the `len_host`-byte mapping this call just created, recorded
            // nowhere; unmapping it releases exactly that.
            unsafe { libc::munmap(ret, len_host) };
        }
        self.poisoned.store(true, Ordering::Release);
    }

    /// Copy `dst.len()` bytes out of the window, starting at `offset`.
    ///
    /// Takes **no lock and performs no syscall** — see the module docs for why that is the
    /// property the whole adapter design rests on.
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::OutOfRange`], [`RawError::LengthOverflow`],
    /// [`RawError::TooLargeForHost`].
    pub fn read_into(&self, offset: HostOffset, dst: &mut [u8]) -> Result<(), RawError> {
        self.live()?;
        let (start, len) =
            bounds::checked_span(self.len_bytes(), offset, dst.len() as u64, "read length")?;

        // SAFETY: `checked_span` above proved `start + len <= self.len` with the overflow
        // checked before the bound, and the type invariant says `base` is live for
        // `self.len` bytes — so the source range is inside one live allocation and
        // `base.add(start)` satisfies `add`'s precondition. `dst` is a live `&mut [u8]`
        // of exactly `len` bytes (the length came from `dst.len()` on the line above), so
        // the destination is in bounds by construction. The two ranges cannot overlap: the
        // source is inside a mapping this object owns and `dst` is a caller's slice, and
        // no borrow into this mapping is ever handed out. The residual — that the guest
        // may write the source concurrently — is §4.2.2's ruled residual, argued in the
        // module docs, not an unnoticed one.
        unsafe {
            core::ptr::copy_nonoverlapping(self.base.as_ptr().add(start), dst.as_mut_ptr(), len);
        }
        Ok(())
    }

    /// Copy `src` into the window at `offset`.
    ///
    /// # Errors
    /// As [`GuestWindow::read_into`].
    pub fn write_from(&self, offset: HostOffset, src: &[u8]) -> Result<(), RawError> {
        self.live()?;
        let (start, len) =
            bounds::checked_span(self.len_bytes(), offset, src.len() as u64, "write length")?;

        // SAFETY: the mirror of `read_into`, with the same two facts. `checked_span`
        // proved `start + len <= self.len` (overflow checked before the bound) against the
        // type invariant's live `self.len`-byte mapping, so the destination is in bounds
        // and `base.add(start)` satisfies `add`'s precondition; `src` is a live `&[u8]` of
        // exactly `len` bytes, taken from `src.len()` on the line above. The ranges cannot
        // overlap, for the same reason.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.base.as_ptr().add(start), len);
        }
        Ok(())
    }

    /// ★★★★★ **ONE NATURALLY-ALIGNED DWORD STORE — the hardware doorbell ring.**
    ///
    /// Owner, 2026-09-13: *"passthrough doorbells are inline in vcpu, no queue, no worker"* and
    /// *"you need have write access to it, and vmm only read, write is trapped so you can
    /// translate doorbell token"*.
    ///
    /// # ⊘ Why this is a separate door from [`GuestWindow::write_from`]
    ///
    /// `write_from` is a `copy_nonoverlapping`, which the compiler may lower to any sequence of
    /// accesses it likes. A **doorbell is a register**: it must be reached by exactly one
    /// aligned 32-bit store, because the device samples the word, and a `memcpy` that split it
    /// into two halves would ring twice with garbage in between. ⇒ `write_volatile` of a `u32`,
    /// at a checked, **4-byte-aligned** offset.
    ///
    /// ⚠ Alignment is **refused, not rounded**. An unaligned register store is a different
    /// access than the one intended, and silently fixing the caller's offset would hide a
    /// wrong-offset bug behind a working ring.
    ///
    /// ⊘ This is the only write door that may be used from a vCPU thread: it takes no lock,
    /// allocates nothing, and is a single instruction after the bound. Everything else on this
    /// type is a copy whose cost is the caller's length.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] / [`RawError::LengthOverflow`] as [`GuestWindow::write_from`],
    /// and [`RawError::Unsupported`] for an offset that is not 4-byte aligned.
    pub fn store_u32(&self, offset: HostOffset, value: u32) -> Result<(), RawError> {
        self.live()?;
        let (start, _len) = bounds::checked_span(self.len_bytes(), offset, 4, "register store")?;
        if start % 4 != 0 {
            return Err(RawError::Unsupported {
                what: "an aligned register store",
                detail: "a device register must be reached by ONE naturally-aligned access; an \
                         unaligned offset is a different access than the caller intended, and \
                         rounding it would hide a wrong-offset bug behind a working store",
            });
        }
        // SAFETY: `checked_span` proved `start + 4 <= self.len` (overflow checked before the
        // bound) against the type invariant's live `self.len`-byte mapping, so `base.add(start)`
        // satisfies `add`'s precondition and the four bytes at that address are inside the
        // mapping. `start % 4 == 0` was just checked and `base` is page-aligned by `mmap`, so
        // the destination is a correctly-aligned `u32`. `write_volatile` is used rather than a
        // plain store because the destination may be device memory whose write has an effect
        // the compiler cannot see, and must be neither elided nor split.
        unsafe {
            self.base
                .as_ptr()
                .add(start)
                .cast::<u32>()
                .write_volatile(value);
        }
        Ok(())
    }

    /// The host address of `[offset, offset + len)`, as the integer a memslot install
    /// must carry — **bounds-checked here**, where the object that owns the range is.
    ///
    /// ★ `pub(crate)`, and that is refusal 10 of §4.6 holding: *no host CPU address in the
    /// signature of any item reachable from outside this crate.* The only caller is
    /// [`crate::kvm_unsafe`], which passes it straight into the kernel's memslot struct
    /// and never stores it. The bounds check lives here rather than there because the
    /// house rule is that no relaxation has a precondition established in another file —
    /// and a memslot whose length ran off the end of its window is precisely the
    /// "semantically unbounded bounded object" the crate docs name as the un-mechanisable
    /// refusal, made mechanisable for this one case.
    ///
    /// # Safety
    ///
    /// The returned integer is a host address of this window's mapping (a7, V3_SEC_PERIMETER.md
    /// §4.1). The caller must hand it only to the kernel as the backing of a memslot whose
    /// lifetime this `GuestWindow` outlives, and never turn it into a pointer: the bounds are
    /// checked here, the lifetime cannot be.
    pub(crate) unsafe fn userspace_addr_at(&self, offset: u64, len: u64) -> Result<u64, RawError> {
        self.live()?;
        geometry::require_aligned(offset, self.page, "memslot offset")?;
        geometry::require_aligned(len, self.page, "memslot length")?;
        let (start, _) = bounds::checked_span(
            self.len_bytes(),
            HostOffset::new(offset),
            len,
            "memslot length",
        )?;
        // Address arithmetic, not a dereference: the sum is in bounds of the mapping by
        // the check above, and it is handed to the kernel rather than to a load.
        Ok(self.base.as_ptr() as u64 + start as u64)
    }
}

/// What [`GuestWindow::fixed_map`] places, as each door received it; `fixed_map` checks every
/// part of it (no door hands it a precondition).
#[derive(Debug, Clone, Copy)]
enum Source<'a> {
    /// The anonymous filler (`restore` asks for `MAP_NORESERVE`, `place` does not).
    Anonymous { noreserve: bool },
    /// A shared file: its size is read with `fstat` at placement (a10).
    File { fd: BorrowedFd<'a>, offset: u64 },
    /// A file whose size was recorded at adoption and cannot shrink ([`SealedFile`]).
    Sealed { file: &'a SealedFile, offset: u64 },
    /// An armed device node at offset 0; its driver bounds the mapping.
    Device { fd: BorrowedFd<'a> },
}

/// ★ A shared file that cannot SHRINK, adopted once (off the vCPU): the placement source
/// [`GuestWindow::place_sealed`] bounds with no syscall.
///
/// [`SealedFile::adopt`] duplicates the descriptor (this value owns its own reference), checks
/// that `F_SEAL_SHRINK` is set, and only THEN records the size: from that point the size can
/// only grow, so every page below the recorded size exists for this value's whole life and a
/// placement inside it can never become a `SIGBUS` mapping (a10's truncation residual, closed
/// for this source). `SharedRam` seals itself at creation; QEMU's `memory-backend-memfd` seals
/// by default (`seal=on`).
#[derive(Debug)]
pub struct SealedFile {
    fd: OwnedFd,
    len: u64,
}

impl SealedFile {
    /// Adopt (a duplicate of) `fd`.
    ///
    /// # Errors
    /// [`RawError::Unsupported`] when the file can shrink (no `F_SEAL_SHRINK`, or a file that
    /// takes no seals at all); [`RawError::Syscall`] when it cannot be duplicated or sized.
    pub fn adopt(fd: BorrowedFd<'_>) -> Result<Self, RawError> {
        let fd = fd.try_clone_to_owned().map_err(|e| RawError::Syscall {
            call: "dup(a file to seal-check)",
            errno: e.raw_os_error(),
        })?;
        // SAFETY: `fcntl(F_GET_SEALS)` reads no memory of this process: both arguments are
        // integers, and `fd` is the live descriptor this value owns. The result is checked.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        if seals < 0 || seals & libc::F_SEAL_SHRINK == 0 {
            return Err(RawError::Unsupported {
                what: "a sealed placement source",
                detail: "the file can shrink (no F_SEAL_SHRINK): a page placed from it could vanish \
                         and fault the next access; place it with `GuestWindow::place`, which sizes \
                         it at every placement",
            });
        }
        let len = file_size(fd.as_fd())?;
        Ok(SealedFile { fd, len })
    }

    /// The size recorded at adoption: a lower bound for the file's whole life.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len
    }
}

/// The size of the file behind `fd`, by `fstat` (a10).
fn file_size(fd: BorrowedFd<'_>) -> Result<u64, RawError> {
    let mut st = core::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `fd` is a live borrowed descriptor for the call, and `st` is a writable
    // `struct stat`-sized local that `fstat` fills on success; it is read (`assume_init`)
    // only when the return value says it was filled.
    let size = unsafe {
        if libc::fstat(fd.as_raw_fd(), st.as_mut_ptr()) != 0 {
            None
        } else {
            Some(st.assume_init().st_size)
        }
    };
    let size = size.ok_or_else(|| last_syscall_error("fstat"))?;
    u64::try_from(size).map_err(|_| RawError::Syscall {
        call: "fstat",
        errno: None,
    })
}

/// The one `mmap(MAP_FIXED)` of [`GuestWindow::fixed_map`], split out so the a8 fault seam can
/// stand in for it in tests.
///
/// # Safety
///
/// `target..target + len` is a page-aligned range inside a mapping this process owns and may
/// replace (a [`GuestWindow`]'s own range), with no reference into it outstanding; and for a
/// file (`fd >= 0`, not `MAP_ANONYMOUS`), `[file_offset, file_offset + len)` exists in it now,
/// or the file is a device whose `mmap` handler bounds the mapping. `MAP_FIXED` replaces
/// whatever is mapped at `target` without asking (rule (a): `unsafe`, review 2026-10-04).
unsafe fn map_fixed(
    target: *mut libc::c_void,
    len: usize,
    prot: libc::c_int,
    share_flags: libc::c_int,
    fd: libc::c_int,
    file_offset: libc::off_t,
) -> *mut libc::c_void {
    #[cfg(test)]
    if let Some(ret) = seam::fail_fixed(target, len) {
        return ret;
    }
    // SAFETY: this fn's contract, verbatim: the range is the caller's own replaceable,
    // page-aligned mapping with no borrow into it, and a file's pages exist, so `MAP_FIXED`
    // replaces only those pages and the result is never a SIGBUS mapping.
    unsafe {
        libc::mmap(
            target,
            len,
            prot,
            share_flags | libc::MAP_FIXED,
            fd,
            file_offset,
        )
    }
}

/// ★ The a8 fault seam (tests only): make the next `MAP_FIXED` fail the way a refusing
/// file `mmap` handler does — the old pages gone, the call failed — and, optionally, let
/// "another thread" map something into the gap before the re-plug runs.
#[cfg(test)]
mod seam {
    use core::cell::Cell;

    thread_local! {
        static FAIL: Cell<bool> = const { Cell::new(false) };
        static PLANT: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn arm(plant_foreign: bool) {
        FAIL.with(|c| c.set(true));
        PLANT.with(|c| c.set(plant_foreign));
    }

    pub(super) fn fail_fixed(target: *mut libc::c_void, len: usize) -> Option<*mut libc::c_void> {
        if !FAIL.with(|c| c.replace(false)) {
            return None;
        }
        // SAFETY: test-only; `target..target + len` is a range of a window this test owns.
        // Unmapping it is the kernel's own failure mode being reproduced: the gap.
        unsafe { libc::munmap(target, len) };
        if PLANT.with(|c| c.replace(false)) {
            // SAFETY: test-only; a fresh anonymous mapping exactly in the gap, standing in for
            // a mapping another thread made there (`MAP_FIXED_NOREPLACE` cannot displace), and
            // its first byte written only once the mapping is known to be at `target`.
            let planted = unsafe {
                let foreign = libc::mmap(
                    target,
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE,
                    -1,
                    0,
                );
                if foreign == target {
                    target.cast::<u8>().write(0x5A);
                }
                foreign == target
            };
            assert!(planted, "the fault seam could not plant its mapping");
        }
        Some(libc::MAP_FAILED)
    }
}

impl Drop for GuestWindow {
    fn drop(&mut self) {
        // ★ a8: a poisoned window's range may hold a mapping that is not ours, which an
        // `munmap` of the whole range would destroy. Leaking the address space is the
        // memory-safe failure.
        if *self.poisoned.get_mut() {
            return;
        }
        // SAFETY: the type invariant says `base` is a live mapping of exactly `len` bytes,
        // and this is its unique owner — `GuestWindow` is not `Clone`/`Copy`, nothing else
        // constructs one, and no accessor yields the pointer, so no second `munmap` of
        // this range can exist. Every accessor borrows `&self`, so no copy can be in
        // progress on this thread; a copy on ANOTHER thread would need a `&GuestWindow`,
        // which `Drop` proves does not exist. Both arguments are exactly the pair `mmap`
        // returned.
        let rc = unsafe { libc::munmap(self.base.as_ptr().cast::<libc::c_void>(), self.len) };

        // ★ R1 is asserted AFTER the call, deliberately (§4.5, finding F4): asserting
        // first would panic out of a `Drop` before the resource was released, trading a
        // rule violation for a real leak. The syscall has happened either way; what the
        // assert is for is making it loud.
        if !std::thread::panicking() {
            assert!(rc == 0, "munmap failed: {}", last_syscall_error("munmap"));
            lockwitness::assert_lock_free("munmap (dropping a guest-physical window)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping_unsafe::HostProt;

    fn page() -> HostPageSize {
        HostPageSize::query()
    }

    /// Read the first `n` bytes of a backing WITHOUT going through any window — the only way
    /// to tell *"the window shows B"* from *"the window shows A and A happens to hold B's
    /// bytes"*. ⊘ A probe that shared the window would not be an observer.
    fn read_backing_directly(
        fd: std::os::fd::BorrowedFd<'_>,
        p: HostPageSize,
        n: usize,
    ) -> Vec<u8> {
        let m = crate::MappedRegion::map(
            Backing::SharedFile { fd, offset: 0 },
            p.bytes(),
            HostProt::ReadWrite,
            crate::CachePolicy::WriteBack,
            p,
        )
        .expect("map the backing directly");
        let mut out = vec![0u8; n];
        m.read_into(HostOffset::ZERO, &mut out)
            .expect("read the backing");
        out
    }

    #[test]
    fn a_window_is_readable_everywhere_before_anything_is_placed_in_it() {
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("a four-page window");
        let mut got = vec![0xFFu8; 64];
        w.read_into(HostOffset::new(3 * p.bytes()), &mut got)
            .expect("the last page of an empty window still reads");
        assert_eq!(
            got,
            vec![0u8; 64],
            "a window is anonymous zero-fill from construction — 'nothing placed here' \
             must be a VALUE, never a fault, because a live memslot names it either way"
        );
    }

    /// ★ The property §6.7 item 3 is a rule about, asserted directly: a restored range is
    /// still mapped, still readable, and reads as zeroes.
    #[test]
    fn restoring_a_placement_yields_zeroes_and_never_a_hole() {
        let p = page();
        let ram = crate::SharedRam::create(p.bytes()).expect("memfd");
        let w = GuestWindow::create(2 * p.bytes(), p).expect("a two-page window");
        w.place(
            HostOffset::new(p.bytes()),
            p.bytes(),
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
        )
        .expect("place the shared backing on the second page");
        w.write_from(HostOffset::new(p.bytes()), &[0x5A; 16])
            .expect("write through the placement");
        let mut got = [0u8; 16];
        w.read_into(HostOffset::new(p.bytes()), &mut got)
            .expect("read it back");
        assert_eq!(got, [0x5A; 16], "the placement is live");

        w.restore(HostOffset::new(p.bytes()), p.bytes())
            .expect("restore");
        w.read_into(HostOffset::new(p.bytes()), &mut got).expect(
            "a RESTORED range must still resolve — this read is the whole point of \
                 §6.7 item 3, and it is what a plain munmap would have turned into a \
                 SIGSEGV here and a kernel fault for the guest",
        );
        assert_eq!(got, [0u8; 16], "and it reads as anonymous zeroes");

        // The backing itself is untouched — restore detached it, it did not erase it.
        let check = crate::MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
            p.bytes(),
            HostProt::ReadWrite,
            crate::CachePolicy::WriteBack,
            p,
        )
        .expect("map the backing directly");
        let mut direct = [0u8; 16];
        check
            .read_into(HostOffset::ZERO, &mut direct)
            .expect("read the backing");
        assert_eq!(
            direct, [0x5A; 16],
            "restore detaches a backing from the window; it must not destroy it, or a \
             re-place would resurrect zeroes instead of the object's contents"
        );
    }

    /// ★★★★★ **w752 — THE IN-PLACE RE-POINT SENTINEL.** The brief's required known-positive:
    /// *"write a known value through the window at base A, re-point to base B, prove the window
    /// shows B's bytes AND that A's are no longer reachable through it."*
    ///
    /// This is the property the whole of cut P1 rests on. `barmirror.rs` carried *"a device
    /// view cannot be re-pointed — each arming is its own fd at offset 0"* as the reason
    /// PRAMIN's move became release-and-re-arm (20 arms x 7 vCPU doors + 19 retirements x 3 =
    /// **197 blocking crossings, worst trap 44 ms**, `[measured w742]`). The sentence is true of
    /// the **node** and false of the **window**, and this test is that distinction as a
    /// measurement rather than an argument.
    ///
    /// ⚠ **Why a memfd stands in for `/dev/nvidia<N>`.** [`GuestWindow::place_device_view`] is
    /// `fixed_map` with `MAP_SHARED` at file offset 0 — the driver's constraint (it refuses any
    /// `vm_pgoff` but zero) is on the *node*, not on this call, and the kernel's `MAP_FIXED`
    /// replacement semantics are what is under test here. ⊘ What a memfd CANNOT stand in for is
    /// the `VM_IO | VM_PFNMAP` VMA a real node produces; that half is exercised only on the
    /// bench, and the boot's `PRAMIN-INPLACE` census is where it is read.
    #[test]
    fn a_device_view_is_re_pointed_in_place_and_the_old_backing_goes_unreachable() {
        let p = page();
        let a = crate::SharedRam::create(p.bytes()).expect("backing A");
        let b = crate::SharedRam::create(p.bytes()).expect("backing B");
        let w = GuestWindow::create(p.bytes(), p).expect("a one-page window");

        w.place_device_view(HostOffset::ZERO, p.bytes(), a.as_backing_fd(), true)
            .expect("place A");
        w.write_from(HostOffset::ZERO, b"AAAAAAAA")
            .expect("write A");

        // ★ THE RE-POINT. One `MAP_FIXED`, into the window that is already there: no new
        // window, no memslot, no munmap.
        w.place_device_view(HostOffset::ZERO, p.bytes(), b.as_backing_fd(), true)
            .expect("re-point the SAME window at B, in place");

        let mut got = [0u8; 8];
        w.read_into(HostOffset::ZERO, &mut got).expect("read");
        assert_eq!(
            &got, b"\0\0\0\0\0\0\0\0",
            "the window must show B (untouched, so zeroes) the instant the re-point returns. \
             Seeing A's bytes here is the failure the trap contract calls the one that cannot \
             be contained: the guest re-aims the aperture and reads the PREVIOUS framebuffer, \
             with no trap and no error"
        );

        // ★★ And the stronger half: A is no longer REACHABLE through the window. A write here
        // must land in B and must not touch A — otherwise the old node's aperture is still
        // live behind our PTEs, which is exactly what makes an early release cross-tenant.
        w.write_from(HostOffset::ZERO, b"BBBBBBBB")
            .expect("write B");
        let in_a = read_backing_directly(a.as_backing_fd(), p, 8);
        assert_eq!(
            &in_a[..],
            b"AAAAAAAA",
            "a write through the re-pointed window reached the OLD backing — the window is \
             still mapping A, so the re-point did not take and the two backings are one memory"
        );
        let in_b = read_backing_directly(b.as_backing_fd(), p, 8);
        assert_eq!(
            &in_b[..],
            b"BBBBBBBB",
            "a write through the re-pointed window must reach B: `MAP_FIXED` replaced the \
             mapping, it did not shadow it"
        );
    }

    /// ★★★★★ **w752 — WHY `repoint_device_window` CHECKS THE LENGTH.** The half-re-pointed
    /// aperture, produced on purpose.
    ///
    /// A placement shorter than the window leaves the tail showing the backing it replaced:
    /// **two framebuffers behind one guest-physical range, with no trap and no error.** That is
    /// the `[w582-w586]` class — *zero traps proves the slot INTERCEPTS the access, never that
    /// it shows the same bytes* — and it is why `QemuMachine::repoint_device_window` refuses a
    /// length that is not exactly the window's rather than placing what it was given.
    #[test]
    fn a_short_re_point_leaves_the_windows_tail_showing_the_backing_it_replaced() {
        let p = page();
        let a = crate::SharedRam::create(2 * p.bytes()).expect("backing A");
        let b = crate::SharedRam::create(2 * p.bytes()).expect("backing B");
        let w = GuestWindow::create(2 * p.bytes(), p).expect("a two-page window");

        w.place_device_view(HostOffset::ZERO, 2 * p.bytes(), a.as_backing_fd(), true)
            .expect("place A whole");
        w.write_from(HostOffset::ZERO, b"A0").expect("page 0");
        w.write_from(HostOffset::new(p.bytes()), b"A1")
            .expect("page 1");

        // The mistake, made deliberately: replace only the first page.
        w.place_device_view(HostOffset::ZERO, p.bytes(), b.as_backing_fd(), true)
            .expect("a SHORT placement is accepted by the window — it is in bounds");

        let mut tail = [0u8; 2];
        w.read_into(HostOffset::new(p.bytes()), &mut tail)
            .expect("read the tail");
        assert_eq!(
            &tail, b"A1",
            "this assertion is the DEFECT, asserted so it cannot be argued away: the window's \
             tail still shows the PREVIOUS backing while its head shows the new one. The guest \
             sees one aperture over two framebuffers, and nothing anywhere reports it. \
             `REPOINT_LENGTH_IS_NOT_THE_WINDOWS` is the refusal that keeps this unreachable \
             from the mirror — if that check is ever removed, THIS is what ships"
        );
        let mut head = [0u8; 2];
        w.read_into(HostOffset::ZERO, &mut head)
            .expect("read the head");
        assert_eq!(&head, b"\0\0", "and the head really did move to B");
    }

    #[test]
    fn a_placement_that_leaves_the_window_is_refused_by_exact_variant() {
        let p = page();
        let w = GuestWindow::create(2 * p.bytes(), p).expect("window");
        assert_eq!(
            w.place(
                HostOffset::new(p.bytes()),
                2 * p.bytes(),
                Backing::PrivateAnonymous
            ),
            Err(RawError::OutOfRange {
                offset: p.bytes(),
                len: 2 * p.bytes(),
                object_len: 2 * p.bytes(),
            })
        );
        assert_eq!(
            w.place(
                HostOffset::new(!p.mask()),
                p.bytes(),
                Backing::PrivateAnonymous
            ),
            Err(RawError::LengthOverflow {
                offset: !p.mask(),
                len: p.bytes(),
            }),
            "an overflowing offset+len must be refused as an OVERFLOW — the check that \
             silently succeeds is the one that wrapped first"
        );
    }

    #[test]
    fn a_misaligned_or_empty_placement_is_refused_by_exact_variant() {
        let p = page();
        let w = GuestWindow::create(p.bytes(), p).expect("window");
        assert_eq!(
            w.place(HostOffset::new(8), p.bytes(), Backing::PrivateAnonymous),
            Err(RawError::Misaligned {
                what: "placement",
                value: 8,
                required: p.bytes(),
            })
        );
        assert_eq!(
            w.place(HostOffset::ZERO, 0, Backing::PrivateAnonymous),
            Err(RawError::ZeroLength { what: "placement" })
        );
        assert_eq!(
            GuestWindow::create(p.bytes() + 1, p).unwrap_err(),
            RawError::Misaligned {
                what: "window length",
                value: p.bytes() + 1,
                required: p.bytes(),
            }
        );
    }

    /// ★ A **real** syscall refusal the mock harness could never produce: a window larger
    /// than the address space is `ENOMEM` from the kernel, reported by exact variant.
    #[test]
    fn an_impossible_window_is_a_real_kernel_refusal_not_a_panic() {
        let p = page();
        let absurd = (1u64 << 62) & !p.mask();
        assert_eq!(
            GuestWindow::create(absurd, p).map(|_| ()),
            Err(RawError::Syscall {
                call: "mmap",
                errno: Some(libc::ENOMEM),
            }),
            "a 4 EiB window must come back as the KERNEL's ENOMEM — the point of building \
             against a real OS is that this arm exists at all"
        );
    }

    /// ★ The sub-range memslot's address arithmetic, asserted as a **difference** — the
    /// only property of a host address that may be written down (§4.2.1 refusal 3: an
    /// integer host address is a pointer with the checks filed off, and a difference is
    /// not one).
    ///
    /// This exists because a bite-check found the gap: neutering
    /// `userspace_addr_at` to ignore its offset — so every sub-range memslot of a
    /// read-native overlay would point at the window's base — broke **nothing**. Without a
    /// vCPU nothing observes where a memslot points, so the harness could not tell a
    /// correct address from a constant one. This is the mechanical half that can be
    /// checked without a guest; the rest is honestly M2-d's.
    #[test]
    fn a_sub_range_memslot_address_advances_by_exactly_its_offset() {
        crate::require_kvm!("a_sub_range_memslot_address_advances_by_exactly_its_offset");
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("window");
        let at = |off: u64, len: u64| {
            // SAFETY: the addresses are compared as integers and never handed to the kernel or
            // turned into pointers.
            unsafe { w.userspace_addr_at(off, len) }
        };
        let base = at(0, p.bytes()).expect("the window's own base");
        for k in 1..4u64 {
            assert_eq!(
                at(k * p.bytes(), p.bytes()).expect("a sub-range inside the window") - base,
                k * p.bytes(),
                "a memslot over the k-th page must name the k-th page"
            );
        }
        assert_eq!(
            at(3 * p.bytes(), 2 * p.bytes()),
            Err(RawError::OutOfRange {
                offset: 3 * p.bytes(),
                len: 2 * p.bytes(),
                object_len: 4 * p.bytes(),
            }),
            "a memslot whose length runs off the end of its window is the \"semantically \
             unbounded bounded object\" the crate docs name — refused here, where the \
             object that owns the range is"
        );
        assert_eq!(
            at(8, p.bytes()),
            Err(RawError::Misaligned {
                what: "memslot offset",
                value: 8,
                required: p.bytes(),
            })
        );
    }

    /// The window is `Sync`, so two threads may copy through it concurrently. This is the
    /// grant above being exercised rather than merely written.
    #[test]
    fn two_threads_may_copy_through_one_window_concurrently() {
        let p = page();
        let w = std::sync::Arc::new(GuestWindow::create(2 * p.bytes(), p).expect("window"));
        std::thread::scope(|s| {
            for t in 0..2u64 {
                let w = std::sync::Arc::clone(&w);
                s.spawn(move || {
                    for i in 0..256u64 {
                        let at = HostOffset::new(t * p.bytes() + i * 8);
                        w.write_from(at, &i.to_le_bytes()).expect("write");
                        let mut got = [0u8; 8];
                        w.read_into(at, &mut got).expect("read");
                        assert_eq!(u64::from_le_bytes(got), i, "disjoint ranges never mix");
                    }
                });
            }
        });
    }
    /// ★★★★★ **THE DOORBELL STORE — one aligned dword, and a refusal for anything else.**
    ///
    /// Owner, 2026-09-13: *"passthrough doorbells are inline in vcpu, no queue, no worker."*
    /// The whole vCPU-side act is this call, so its bound and its alignment are the only things
    /// standing between a guest-supplied offset and a write into the VMM's own mapping.
    #[test]
    fn an_aligned_dword_lands_exactly_where_it_was_asked_and_nowhere_else() {
        let p = page();
        let w = GuestWindow::create(2 * p.bytes(), p).expect("a two-page window");
        w.store_u32(HostOffset::new(0x90), 0xDEAD_BEEF)
            .expect("an aligned store inside the window");

        let mut got = [0u8; 16];
        w.read_into(HostOffset::new(0x88), &mut got)
            .expect("read back");
        // ⊘ The neighbours matter as much as the target: a store that also disturbed the bytes
        // around it would ring correctly and corrupt the counter 16 bytes away — which is the
        // page's actual layout (`TIME_0` at +0x80, `DOORBELL` at +0x90).
        assert_eq!(&got[..8], &[0u8; 8], "the 8 bytes BELOW the register moved");
        assert_eq!(
            u32::from_ne_bytes(got[8..12].try_into().unwrap()),
            0xDEAD_BEEF
        );
        assert_eq!(
            &got[12..],
            &[0u8; 4],
            "the 4 bytes ABOVE the register moved"
        );
    }

    /// ⚠ **Refused, never rounded.** An unaligned register store is a different access than the
    /// caller intended; quietly fixing the offset would hide a wrong-offset bug behind a ring
    /// that appears to work.
    #[test]
    fn an_unaligned_register_store_is_refused_by_name() {
        let p = page();
        let w = GuestWindow::create(p.bytes(), p).expect("a one-page window");
        for off in [0x91u64, 0x92, 0x93] {
            let e = w.store_u32(HostOffset::new(off), 1).expect_err("unaligned");
            assert!(
                matches!(e, RawError::Unsupported { what, .. } if what == "an aligned register store"),
                "{off:#x} refused as {e:?}"
            );
        }
        // ⊘ And the refusal must not have written anything on its way out.
        let mut got = [0xFFu8; 8];
        w.read_into(HostOffset::new(0x90), &mut got)
            .expect("read back");
        assert_eq!(got, [0u8; 8], "a refused store still touched the window");
    }

    /// ⊘ The bound is the window's own length. A four-byte store whose LAST byte falls outside
    /// is out of range — the off-by-one that a `offset < len` test would pass.
    #[test]
    fn a_dword_that_straddles_the_end_is_out_of_range() {
        let p = page();
        let w = GuestWindow::create(p.bytes(), p).expect("a one-page window");
        let len = p.bytes();
        w.store_u32(HostOffset::new(len - 4), 7)
            .expect("the LAST aligned dword is inside");
        assert!(
            w.store_u32(HostOffset::new(len), 7).is_err(),
            "a store starting at the end was accepted"
        );
        assert!(
            w.store_u32(HostOffset::new(u64::MAX - 1), 7).is_err(),
            "an offset that overflows when 4 is added was accepted"
        );
    }

    /// ★ a8: a placement whose `MAP_FIXED` fails the way a refusing file handler does (the old
    /// pages already gone) leaves the window READABLE: the filler is re-plugged. On the code
    /// before a8 the gap stayed, and the `read_into` below was a SIGSEGV from safe code.
    #[test]
    fn a_failed_placement_re_plugs_the_filler_and_the_window_stays_readable() {
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("window");
        w.write_from(HostOffset::new(p.bytes()), &[7u8; 8])
            .expect("seed");
        seam::arm(false);
        let r = w.restore(HostOffset::new(p.bytes()), p.bytes());
        assert!(
            matches!(r, Err(RawError::Syscall { call: "mmap", .. })),
            "{r:?}"
        );
        let mut got = [0xFFu8; 8];
        w.read_into(HostOffset::new(p.bytes()), &mut got)
            .expect("the re-plugged range reads");
        assert_eq!(got, [0u8; 8], "the filler is back: anonymous zeroes");
        w.restore(HostOffset::new(p.bytes()), p.bytes())
            .expect("and the window still places");
    }

    /// ★ a8: if something else got into the gap before the re-plug, the window cannot be made
    /// whole without overwriting it: it is POISONED. Every accessor refuses by name, and Drop
    /// leaves the range mapped (the planted page is still readable after the window is gone).
    #[test]
    fn a_foreign_mapping_planted_in_the_gap_poisons_the_window() {
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("window");
        seam::arm(true);
        let at = HostOffset::new(2 * p.bytes());
        assert!(w.restore(at, p.bytes()).is_err());
        let mut b = [0u8; 4];
        assert_eq!(w.read_into(at, &mut b), Err(RawError::WindowPoisoned));
        assert_eq!(w.write_from(at, &b), Err(RawError::WindowPoisoned));
        assert_eq!(w.store_u32(at, 1), Err(RawError::WindowPoisoned));
        assert_eq!(w.restore(at, p.bytes()), Err(RawError::WindowPoisoned));
        assert!(w.host_span(0, 8).is_none());
        // SAFETY: test-only; the address is only compared, never dereferenced.
        let addr = unsafe { w.userspace_addr_at(0, p.bytes()) };
        assert_eq!(addr, Err(RawError::WindowPoisoned));
        let base = w.base.as_ptr();
        let len = w.len;
        drop(w);
        // SAFETY: test-only; Drop left the range mapped, so the planted page (whose first byte
        // the seam wrote) is still readable; then the range Drop deliberately leaked is freed.
        let planted = unsafe {
            let b = base.add(2 * p.bytes() as usize).read();
            libc::munmap(base.cast(), len);
            b
        };
        assert_eq!(
            planted, 0x5A,
            "Drop must not unmap a mapping that is not the window's"
        );
    }

    /// ★ a9: a read-only device view is refused by name, before any `mmap`: the window's write
    /// doors would fault on it.
    #[test]
    fn a_read_only_device_view_is_refused_by_name() {
        let p = page();
        let w = GuestWindow::create(2 * p.bytes(), p).expect("window");
        let f = std::fs::File::open("/dev/zero").expect("/dev/zero");
        use std::os::fd::AsFd;
        let r = w.place_device_view(HostOffset::new(0), p.bytes(), f.as_fd(), false);
        assert!(
            matches!(r, Err(RawError::Unsupported { what, .. }) if what.contains("read-only device view")),
            "{r:?}"
        );
    }

    /// ★ a10: a shared-file placement past the file's end is refused by name. On the code
    /// before a10 it mapped, and the first read of the missing page raised SIGBUS.
    #[test]
    fn a_short_memfd_is_refused() {
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("window");
        let one_page = memfd_of(p.bytes());
        use std::os::fd::AsFd;
        let r = w.place(
            HostOffset::new(0),
            2 * p.bytes(),
            Backing::SharedFile {
                fd: one_page.as_fd(),
                offset: 0,
            },
        );
        assert_eq!(
            r,
            Err(RawError::OutOfRange {
                offset: 0,
                len: 2 * p.bytes(),
                object_len: p.bytes(),
            })
        );
        let r = w.place(
            HostOffset::new(0),
            p.bytes(),
            Backing::SharedFile {
                fd: one_page.as_fd(),
                offset: p.bytes(),
            },
        );
        assert!(matches!(r, Err(RawError::OutOfRange { .. })), "{r:?}");
        w.place(
            HostOffset::new(0),
            p.bytes(),
            Backing::SharedFile {
                fd: one_page.as_fd(),
                offset: 0,
            },
        )
        .expect("the page the file does have places");
    }

    /// ★ a10 without the per-placement `fstat` (review 2026-10-04): a [`SealedFile`] is
    /// refused unless it cannot shrink, a placement past its recorded size is refused by name,
    /// and one inside it places and reads through.
    #[test]
    fn a_sealed_file_places_without_a_syscall_and_refuses_past_its_size() {
        use std::os::fd::AsFd;
        let p = page();
        let w = GuestWindow::create(4 * p.bytes(), p).expect("window");
        // A memfd that takes seals but has none: `F_GET_SEALS` answers 0, so only the
        // `F_SEAL_SHRINK` test can refuse it (a plain file answers EINVAL, which proves less).
        // SAFETY: test-only; the name is a NUL-terminated literal and the result is checked
        // before `OwnedFd` takes ownership of it.
        let unsealed = unsafe {
            let raw = libc::memfd_create(c"kf-a10-unsealed".as_ptr(), libc::MFD_ALLOW_SEALING);
            assert!(raw >= 0, "memfd_create");
            <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw)
        };
        for f in [memfd_of(p.bytes()).into(), unsealed] {
            let f: OwnedFd = f;
            assert!(
                matches!(
                    SealedFile::adopt(f.as_fd()),
                    Err(RawError::Unsupported {
                        what: "a sealed placement source",
                        ..
                    })
                ),
                "a file that can shrink is refused"
            );
        }
        let ram = crate::SharedRam::create(2 * p.bytes()).expect("a sealed memfd");
        let sealed = SealedFile::adopt(ram.as_backing_fd()).expect("SharedRam is sealed");
        assert_eq!(sealed.len_bytes(), 2 * p.bytes());
        assert_eq!(
            w.place_sealed(HostOffset::ZERO, 2 * p.bytes(), &sealed, p.bytes()),
            Err(RawError::OutOfRange {
                offset: p.bytes(),
                len: 2 * p.bytes(),
                object_len: 2 * p.bytes(),
            })
        );
        assert_eq!(
            w.place_sealed(HostOffset::ZERO, p.bytes(), &sealed, !p.mask()),
            Err(RawError::TooLargeForHost { value: !p.mask() }),
            "a file offset no `off_t` can hold is refused by name before any arithmetic"
        );
        w.place_sealed(HostOffset::new(p.bytes()), p.bytes(), &sealed, p.bytes())
            .expect("the second page of the file, inside it");
        w.write_from(HostOffset::new(p.bytes()), b"sealed")
            .expect("write through it");
        let m = crate::MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: p.bytes(),
            },
            p.bytes(),
            HostProt::ReadWrite,
            crate::CachePolicy::WriteBack,
            p,
        )
        .expect("map the file's second page directly");
        let mut direct = [0u8; 6];
        m.read_into(HostOffset::ZERO, &mut direct).expect("read it");
        assert_eq!(
            &direct, b"sealed",
            "the window showed the file's second page"
        );
    }

    /// A file of exactly `len` bytes (unlinked at once; the descriptor keeps it).
    fn memfd_of(len: u64) -> std::fs::File {
        let path = std::env::temp_dir().join(format!("kf-window-a10-{}", std::process::id()));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("a temporary file");
        std::fs::remove_file(&path).expect("unlink");
        f.set_len(len).expect("size it");
        f
    }
}
