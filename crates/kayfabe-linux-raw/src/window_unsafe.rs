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
use kayfabe_util::lockwitness;
use std::os::fd::AsRawFd;

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
    len: usize,
    page: HostPageSize,
}

// SAFETY: `GuestWindow` owns a process-wide mapping, not a thread-affine resource: the
// three fields are a pointer to that mapping, its length and the page size, all
// established by `create` and never mutated afterwards. `munmap` from a different thread
// than `mmap` is valid, so moving the owner between threads is sound.
unsafe impl Send for GuestWindow {}

// SAFETY: every method takes `&self`. `base`, `len` and `page` are immutable for the
// object's life, so there is no field a concurrent pair of `&self` methods can tear. The
// two methods that change what the address range *contains* (`place`, `restore`) do so by
// `mmap(MAP_FIXED)`, which the kernel applies to the range atomically under its own
// `mmap_lock` — the range is never transiently unmapped, so a concurrent `read_into`
// observes old bytes, new bytes or a mixture, and never a fault. That residual mixture is
// a data race in the letter of the abstract machine and is NOT claimed away: §4.2.2's
// ruling (keep the memcpy; a torn read is indistinguishable from a hostile guest write,
// and the API cannot re-read the source so the double fetch is excluded structurally)
// applies verbatim, one layer out. See the module docs.
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
            len: len_host,
            page,
        })
    }

    /// The window's length in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.len as u64
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
        let (fd, file_offset, share_flags) = match backing {
            Backing::PrivateAnonymous => (-1, 0, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS),
            Backing::SharedFile { fd, offset } => {
                geometry::require_aligned(offset, self.page, "file offset")?;
                let off = libc::off_t::try_from(offset)
                    .map_err(|_| RawError::TooLargeForHost { value: offset })?;
                (fd.as_raw_fd(), off, libc::MAP_SHARED)
            }
            // ★★★ Refused by variant, at the door. See `RawError::DeviceBackingNotPlaceable`:
            // a window placement is MAP_FIXED into a range a guest memslot already names, and
            // a device backing is a window onto hardware. There is no call for which the
            // composition is right, so there is no check — there is a refusal.
            Backing::DeviceFile { .. } => return Err(RawError::DeviceBackingNotPlaceable),
        };
        self.fixed_map(offset, len, fd, file_offset, share_flags, "placement")
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
            -1,
            0,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            "restore",
        )
    }

    /// The one `MAP_FIXED` in this file. Both doors above validate their own arguments
    /// through it, so neither has a precondition the other is trusted to have met (the
    /// crate's house rule).
    fn fixed_map(
        &self,
        offset: HostOffset,
        len: u64,
        fd: libc::c_int,
        file_offset: libc::off_t,
        share_flags: libc::c_int,
        what: &'static str,
    ) -> Result<(), RawError> {
        if len == 0 {
            return Err(RawError::ZeroLength { what });
        }
        geometry::require_aligned(offset.get(), self.page, what)?;
        geometry::require_aligned(len, self.page, what)?;
        let (start, len_host) = bounds::checked_span(self.len_bytes(), offset, len, what)?;

        // SAFETY: this is the `MAP_FIXED` call, and its argument is that the target range
        // is one this process already owns and is still holding:
        //  (a) `self.base` is the type invariant — a live mapping of exactly `self.len`
        //      bytes, created at a KERNEL-CHOSEN address (so it displaced nothing) and not
        //      released until `Drop`;
        //  (b) `checked_span` on the line above proved `start + len_host <= self.len` with
        //      the overflow checked BEFORE the bound, so `base.add(start)` is in bounds of
        //      that allocation (the precondition of `add`) and the whole range lies inside
        //      the window — `MAP_FIXED` can therefore replace only our own window filler,
        //      never the hypervisor's heap, our stack or our text;
        //  (c) `offset` and `len` are host-page-aligned (checked above) and `file_offset`
        //      was validated by the caller two frames up in the same expression, so `mmap`
        //      has no rounding to do;
        //  (d) no reference into the range can be outstanding: this type hands out no
        //      borrows at all — `read_into`/`write_from` copy and return.
        // The result is compared against the requested address below, so a kernel that
        // honoured the request elsewhere cannot be mistaken for success.
        let ret = unsafe {
            let target = self.base.as_ptr().add(start).cast::<libc::c_void>();
            libc::mmap(
                target,
                len_host,
                libc::PROT_READ | libc::PROT_WRITE,
                share_flags | libc::MAP_FIXED,
                fd,
                file_offset,
            )
        };
        if ret == libc::MAP_FAILED {
            return Err(last_syscall_error("mmap"));
        }
        Ok(())
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
    pub(crate) fn userspace_addr_at(&self, offset: u64, len: u64) -> Result<u64, RawError> {
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

impl Drop for GuestWindow {
    fn drop(&mut self) {
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
        let base = w
            .userspace_addr_at(0, p.bytes())
            .expect("the window's own base");
        for k in 1..4u64 {
            assert_eq!(
                w.userspace_addr_at(k * p.bytes(), p.bytes())
                    .expect("a sub-range inside the window")
                    - base,
                k * p.bytes(),
                "a memslot over the k-th page must name the k-th page"
            );
        }
        assert_eq!(
            w.userspace_addr_at(3 * p.bytes(), 2 * p.bytes()),
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
            w.userspace_addr_at(8, p.bytes()),
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
}
