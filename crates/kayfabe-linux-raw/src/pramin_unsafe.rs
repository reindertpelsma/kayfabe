//! ★★★★★ **THE MOVING WINDOW — one reservation, re-pointed by one `mmap`.**
//!
//! Owner, 2026-09-12: *"pramin offset change needs to be synchronous in vcpu thread, is just
//! one mmap remap in vmm va, no bql lock, no kvm memslot update"*, and separately: *"safe code
//! may not touch raw vmm va pointers unchecked"*.
//!
//! # What this is for
//!
//! `PRAMIN` is a 1 MiB aperture in the register BAR that the guest points at an arbitrary
//! offset of the framebuffer by writing a latch register. `[measured w542]` it carries **24
//! reads and 69 730 writes** in a boot — 78% of every write trap — and the guest re-points it
//! **42** times.
//!
//! ⇒ The whole window becomes one guest-physical range backed by host memory, and moving it is
//! **one `mmap(MAP_FIXED)`** over an address-space reservation this process owns. The hypervisor
//! is not told, because nothing it knows has changed: the guest-physical range and the host
//! virtual range are both exactly where they were. Only what lies *behind* the host virtual
//! range moved, and the kernel's own MMU notifier is what makes the guest's stale entries go.
//!
//! ⊘ That is why it may run **synchronously on the vCPU**: it is one syscall, with no lock and
//! no round trip. There is nothing to defer it behind — RM writes the latch and uses the window
//! immediately, with no completion to wait on.
//!
//! # ⚠ The safety property, and why it is a type rather than a rule
//!
//! A raw host address is exactly the thing safe code must never hold: an address that outlives
//! its mapping, or that is off by a page, is a write into whatever this process mapped next.
//! So the address never leaves this module as a number. [`PraminWindow`] owns the reservation,
//! and [`ReservedVa`] — the only thing it hands out — can be constructed **nowhere else**, so a
//! caller cannot fabricate one or keep one past the window's life.

use crate::bounds::HostOffset;
use crate::cache::CachePolicy;
use crate::error::RawError;
use crate::page_size::HostPageSize;
use crate::mapping_unsafe::{Backing, HostProt, Reservation};
use std::os::fd::BorrowedFd;

/// ★★★★★ **A host virtual range this process owns, as a CAPABILITY rather than a number.**
///
/// Handed to the hypervisor layer so it can place one memory slot over the window. ⊘ Its
/// fields are private and its only constructor is inside this module, so safe code cannot
/// invent one, cannot widen one, and cannot hold one that names a mapping that is gone —
/// [`PraminWindow`] is the owner and this borrows its lifetime.
#[derive(Debug, Clone, Copy)]
pub struct ReservedVa<'w> {
    addr: u64,
    len: u64,
    _window: core::marker::PhantomData<&'w PraminWindow>,
}

impl ReservedVa<'_> {
    /// The host virtual address, for the one caller that must issue a memory-slot ioctl.
    ///
    /// ⚠ Named `for_memslot` and not `addr` so that a reader of a call site sees WHY a raw
    /// address is being taken. There is exactly one legitimate reason.
    #[must_use]
    pub fn for_memslot(self) -> u64 {
        self.addr
    }

    /// The window's length in bytes.
    #[must_use]
    pub fn len(self) -> u64 {
        self.len
    }

    /// ⊘ Clippy's companion; a window is never zero-length (the constructor refuses it).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// ★★★★★ **The window itself: an address-space reservation, and whatever is currently behind it.**
#[derive(Debug)]
pub struct PraminWindow {
    reservation: Reservation,
    len: u64,
    /// The file offset currently placed, if any. ⊘ Kept so a re-point to the SAME offset can
    /// be skipped: `[measured w542]` the guest writes the latch far more often than it moves
    /// it, and an `mmap` that changes nothing still costs a TLB shootdown across every vCPU.
    at: Option<u64>,
    moves: u64,
    skipped: u64,
}

impl PraminWindow {
    /// Reserve `len` bytes of address space for the window. Nothing is mapped yet: until
    /// [`PraminWindow::point_at`] runs, the range is `PROT_NONE` and any access to it faults
    /// in THIS process — which is the correct state for a window the guest has not aimed.
    ///
    /// # Errors
    /// Whatever [`Reservation::new`] refuses with — a zero or non-page-multiple length, or the
    /// kernel declining the address space.
    pub fn new(len: u64, page: HostPageSize) -> Result<Self, RawError> {
        Ok(PraminWindow {
            reservation: Reservation::new(len, page)?,
            len,
            at: None,
            moves: 0,
            skipped: 0,
        })
    }

    /// ★★★★★ **Point the window at `offset` in `fd` — the one `mmap` the whole design rests on.**
    ///
    /// ⊘ Idempotent by measurement, not by luck: re-pointing to the offset already placed is
    /// skipped and counted. The guest writes this latch on a read-modify-write cycle, so the
    /// same value arrives repeatedly, and each redundant `MAP_FIXED` would shoot down every
    /// vCPU's TLB for no change.
    ///
    /// # Errors
    /// [`RawError`] from the placement. ⚠ On failure the window keeps whatever it had: a
    /// refused move must not leave the guest reading a range that is half re-pointed.
    pub fn point_at(&mut self, fd: BorrowedFd<'_>, offset: u64) -> Result<(), RawError> {
        if self.at == Some(offset) {
            self.skipped += 1;
            return Ok(());
        }
        self.reservation.replace_fixed_in(
            HostOffset::ZERO,
            self.len,
            Backing::SharedFile { fd, offset },
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
        )?;
        self.at = Some(offset);
        self.moves += 1;
        Ok(())
    }

    /// The reserved range, as the capability the hypervisor layer needs.
    #[must_use]
    pub fn reserved(&self) -> ReservedVa<'_> {
        ReservedVa {
            addr: self.reservation.base_addr(),
            len: self.len,
            _window: core::marker::PhantomData,
        }
    }

    /// Where the window currently points, or `None` before the guest has aimed it.
    #[must_use]
    pub fn pointing_at(&self) -> Option<u64> {
        self.at
    }

    /// `(moves performed, re-points skipped as redundant)` — the census line.
    ///
    /// ⚠ Two numbers, because *"the guest never moved it"* and *"the guest wrote the same
    /// value forty times"* are different facts and a single total cannot tell them apart.
    #[must_use]
    pub fn census(&self) -> (u64, u64) {
        (self.moves, self.skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SharedRam;

    const WIN: u64 = 0x10_0000; // 1 MiB, PRAMIN's own size

    fn backing(len: u64) -> SharedRam {
        SharedRam::create_named(c"kayfabe-pramin-test", len).expect("memfd")
    }

    /// ★★★★★ **The window shows whatever it is pointed at, and re-pointing REPLACES.**
    ///
    /// ⊘ The bytes are the assertion. A test that only checked `pointing_at()` would pass on a
    /// window that recorded the move and never performed it — which is exactly the failure a
    /// guest would see as stale framebuffer contents, with no fault and no counter.
    #[test]
    fn the_window_shows_what_it_points_at_and_a_move_replaces_it() {
        let page = crate::page_size::HostPageSize::query();
        let ram = backing(4 * WIN);
        // Two distinct payloads, one per window position.
        let view = crate::mapping_unsafe::MappedRegion::map(
            Backing::SharedFile {
                fd: ram.as_backing_fd(),
                offset: 0,
            },
            4 * WIN,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .expect("view");
        view.write_from(HostOffset::ZERO, &[0xAA; 8]).expect("a");
        view.write_from(HostOffset::new(WIN), &[0xBB; 8])
            .expect("b");

        let mut w = PraminWindow::new(WIN, page).expect("window");
        assert_eq!(w.pointing_at(), None, "unaimed before the guest writes the latch");

        w.point_at(ram.as_backing_fd(), 0).expect("aim at 0");
        assert_eq!(w.pointing_at(), Some(0));

        // ⊘ Read through the RESERVATION, not through `view` — that is the whole point: the
        // guest's memory slot will sit on this range.
        let seen = w.reservation.placement_bytes_for_test(8).expect("bytes");
        assert_eq!(seen, [0xAA; 8], "the window shows the first payload");

        w.point_at(ram.as_backing_fd(), WIN).expect("aim at +1MiB");
        let seen = w.reservation.placement_bytes_for_test(8).expect("bytes");
        assert_eq!(seen, [0xBB; 8], "re-pointing REPLACED what the window shows");
    }

    /// ★★★★★ **A re-point to the same offset is SKIPPED, and counted separately.**
    ///
    /// `[measured w542]` the guest writes this latch as a read-modify-write, so the same value
    /// arrives repeatedly. Each redundant `MAP_FIXED` would shoot down every vCPU's TLB for no
    /// change at all — on the vCPU thread, inside the guest's own MMIO exit.
    #[test]
    fn a_redundant_repoint_is_skipped_not_performed() {
        let page = crate::page_size::HostPageSize::query();
        let ram = backing(2 * WIN);
        let mut w = PraminWindow::new(WIN, page).expect("window");

        w.point_at(ram.as_backing_fd(), 0).expect("first");
        for _ in 0..40 {
            w.point_at(ram.as_backing_fd(), 0).expect("same again");
        }
        w.point_at(ram.as_backing_fd(), WIN).expect("a real move");

        let (moves, skipped) = w.census();
        assert_eq!(moves, 2, "only the two DISTINCT positions were mapped");
        assert_eq!(skipped, 40, "the forty redundant writes were skipped, and counted");
    }

    /// ⊘ **A refused move leaves the window where it was.** Half a re-point is a guest reading
    /// a range that is partly the old framebuffer and partly the new one.
    #[test]
    fn a_refused_move_does_not_disturb_the_current_placement() {
        let page = crate::page_size::HostPageSize::query();
        let ram = backing(2 * WIN);
        let mut w = PraminWindow::new(WIN, page).expect("window");
        w.point_at(ram.as_backing_fd(), 0).expect("aim");

        // Past the end of the backing file: the kernel accepts the mapping but the pages
        // fault on touch, so refuse it up front by asking for an offset the file cannot hold.
        let bad = w.point_at(ram.as_backing_fd(), u64::MAX & !0xfff);
        assert!(bad.is_err(), "an impossible offset must be refused");
        assert_eq!(
            w.pointing_at(),
            Some(0),
            "a refused move left the window exactly where it was"
        );
    }
}
