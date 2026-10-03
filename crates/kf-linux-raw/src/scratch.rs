// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★★★ [`ScratchTile`] — what an UNMAPPED page of a guest window shows, with the host RAM
//! behind it bounded by ONE TILE per window instead of by the window (2026-10-03).
//!
//! ## Why a window needs scratch at all
//!
//! A guest window is one memslot over one host range ([`crate::GuestWindow`]). A hole in it, or a
//! `PROT_NONE` page, is `KVM_RUN → EFAULT` and kills the guest. So every page the guest has not
//! mapped must still show *something*, and that something is the window's scratch.
//!
//! ## ⊘ Why it is TILED — the owner's question of 2026-10-03
//!
//! *"The memfd is only scratch — isn't that a DoS target?"* It was. Until 2026-10-03 each window's
//! scratch was ONE memfd of the WHOLE window's size, placed 1:1 (window offset `o` showed file
//! offset `o`). A shared-file page has no zero page: a guest READ of a never-touched page faults
//! through `shmem_fault`, which asks for the page with `SGP_CACHE` and so allocates it (Linux 7.1
//! `mm/shmem.c:2765`, `:2527-2564`). The page is charged to the VMM's memory cgroup and is never
//! freed while the VM lives. Guest root that touched every unmapped page of BAR1, BAR2 and PRAMIN
//! therefore made the host allocate `bar1 + bar2 + 1 MiB` of RAM beyond the VM's configured memory:
//! 289 MiB at the device's defaults, and up to the host card's whole BAR1 per device (the whole
//! VRAM, on a resizable-BAR host).
//!
//! Now each window's scratch is ONE tile of `T` bytes ([`scratch_tile_len`]), mapped again and
//! again: window offset `o` shows tile byte `o % T`. A guest can still touch every page, and it can
//! make the host allocate at most `T` per window.
//!
//! | window | `T` | scratch mappings when nothing is mapped | host RAM bound |
//! |---|---|---|---|
//! | PRAMIN, 1 MiB | 1 MiB | 1 | 1 MiB |
//! | 32 MiB | 2 MiB | 16 | 2 MiB |
//! | 256 MiB | 2 MiB | 128 | 2 MiB |
//! | 16 GiB | 16 MiB | 1024 | 16 MiB |
//! | 128 GiB | 128 MiB | 1024 | 128 MiB |
//!
//! The tests below hold both halves: the old shape allocates the whole window on reads alone
//! (`scratch.rs::the_old_whole_window_scratch_allocates_the_whole_window_on_reads_alone`), and the
//! tiled one stays within one tile under reads and writes of every page
//! (`scratch.rs::a_tiled_scratch_holds_a_guest_that_touches_every_page_to_one_tile`).
//!
//! ## Why aliasing is a legal semantics
//!
//! Nothing reads scratch expecting what was written there. It is never copied to or synced with
//! anything (`THE_CONSTRAINTS.md` §18.2: it is not a shadow), and an unmapped BAR page on a real
//! card has no defined contents either. With tiling, a guest write into one unmapped page shows up
//! in the unmapped pages `T` bytes away, in the SAME window of the SAME VM. That is self-corruption
//! only: no tile is shared between windows, devices or VMs.
//!
//! ## ⊘ Corrected 2026-10-03 (review of `v3-scratch-bound`): the mapping count is NOT `≤ 1024`
//!
//! The next section's *"at most 1024"* is the count of an UNMAPPED window's cover, and it was read
//! as a bound on the window. It was not one in production. QEMU advises each window once when it
//! registers it (`MADV_HUGEPAGE`, `MADV_DONTFORK`, and `MADV_DONTDUMP` with `dump-guest-core=off`;
//! QEMU 10.2.4 `system/physmem.c:2294-2304`), AFTER this cover, and a later `MAP_FIXED` sink comes
//! back without those flags. Linux merges neighbouring mappings only when their flags match (7.1
//! `mm/vma.c:84-96`), so a sink beside the advised tiling never merged back, and every guest-driven
//! place-then-sink cycle left two more mappings for the VM's life (the review measured 16 384
//! after 8192 one-page cycles in a 64 MiB window whose tiling is 32). Each one is kernel slab
//! (`vm_area_struct` + a maple-tree node, about 212 B) charged to QEMU's cgroup, outside `-m`.
//!
//! The fix: every off-vCPU sink, and the initial cover, carries [`WINDOW_ADVICE`]
//! ([`ScratchTile::cover_advised`]), a superset of QEMU's flags set BEFORE QEMU registers the
//! window, so QEMU's advice changes nothing and a sink merges back. The count is then:
//! - `ceil(window / T)` (≤ 1024) once nothing is placed;
//! - plus at most two per LIVE placement (the placement itself, and the tile it splits). Live
//!   placements never overlap, so they are at most `window / page`; no cap below that is built
//!   (`V3_P4_PORT_MAP.md` Q3 says why), and the memory cgroup has to cover that slab term.
//! - PRAMIN (sunk on the vCPU, with no advice): at most its 16 slots, since every move re-places
//!   the whole window at slot granularity.
//!
//! The test is `scratch.rs::a_sink_restores_the_canonical_tiling_after_qemu_has_advised_the_window`,
//! and its known-positive is `scratch.rs::an_unadvised_sink_beside_qemus_advice_never_merges_back`.
//!
//! ## Why `T` is not a constant
//!
//! RSS × mappings ≈ window, so one of the two must grow with the window. 4 KiB tiles over 256 MiB
//! would be 65 536 mappings, above the kernel's default `vm.max_map_count` of 65 530. A fixed 2 MiB
//! tile over a 128 GiB BAR1 would be 65 536 too. So `T` grows only when 1024 tiles of 2 MiB cannot
//! cover the window: `T = min(window, max(2 MiB, next_pow2(ceil(window / 1024))))`. Two adjacent
//! tiles never merge into one mapping (their file offsets restart), so covering a whole window
//! costs `ceil(window / T)` mappings, at most 1024.
//!
//! ## ⊘ What tiling does not bound
//!
//! - The host page tables and KVM's SPTEs for a fully touched window (about `window / 256` at 4 KiB
//!   pages, charged to the same cgroup). A placed store view costs the same, so that term comes
//!   from the BAR's size, not from scratch.
//! - The mappings themselves: kernel slab, about 212 B each, at most two per live placement on top
//!   of the tiling (see the correction above). Bounded by `vm.max_map_count` per process and by
//!   `window / page` per window, not by `T`.
//! - Guest RAM, which is pinned whole. The launcher's memory cgroup has to cover all of these
//!   (`V3_SWEEP_AND_INSTALL.md` §2.5).

use crate::bounds::{self, HostOffset};
use crate::error::RawError;
use crate::geometry;
use crate::host_fd_unsafe::SharedRam;
use crate::mapping_unsafe::Backing;
use crate::window_unsafe::{GuestWindow, WindowAdvice};
use std::ffi::CStr;

/// The smallest tile a window larger than it gets: 2 MiB. Also the shmem huge-page size on x86,
/// so a host with shmem THP forced on allocates the same bound, only sooner.
pub const SCRATCH_TILE_MIN: u64 = 2 * 1024 * 1024;

/// The most tiles a whole window is covered by. Bounds the mappings the scratch adds per window,
/// far below the kernel's default `vm.max_map_count` (65 530).
pub const SCRATCH_TILES_MAX: u64 = 1024;

/// ★ The scratch tile length for a window of `window` bytes:
/// `min(window, max(2 MiB, next_pow2(ceil(window / 1024))))`. 0 for an empty window.
///
/// A window smaller than 2 MiB is its own tile, so every placement inside it is ONE piece. PRAMIN
/// (1 MiB) relies on that: its trap re-points it with ONE `mmap` (owner ruling 2026-09-25).
#[must_use]
pub const fn scratch_tile_len(window: u64) -> u64 {
    let per_tile = window.div_ceil(SCRATCH_TILES_MAX);
    let pow2 = match per_tile.checked_next_power_of_two() {
        Some(p) => p,
        None => window,
    };
    let t = if pow2 > SCRATCH_TILE_MIN {
        pow2
    } else {
        SCRATCH_TILE_MIN
    };
    if t < window { t } else { window }
}

/// One `mmap` of a tiled cover: window `[at, at + len)` shows tile bytes
/// `[file_off, file_off + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePiece {
    /// Window offset of the piece.
    pub at: u64,
    /// Its length; never runs past the tile's end (`file_off + len <= tile`).
    pub len: u64,
    /// The tile offset it shows: `at % tile`.
    pub file_off: u64,
}

/// The pieces that cover window `[at, at + len)` with a tile of `tile` bytes, in order. See
/// [`tile_pieces`].
#[derive(Debug, Clone)]
pub struct TilePieces {
    next: u64,
    end: u64,
    tile: u64,
}

impl Iterator for TilePieces {
    type Item = TilePiece;

    fn next(&mut self) -> Option<TilePiece> {
        if self.tile == 0 || self.next >= self.end {
            return None;
        }
        let file_off = self.next % self.tile;
        let len = (self.tile - file_off).min(self.end - self.next);
        let piece = TilePiece {
            at: self.next,
            len,
            file_off,
        };
        self.next += len;
        Some(piece)
    }
}

/// ★ Split window `[at, at + len)` into the pieces a `tile`-byte scratch covers it with: piece `i`
/// shows tile offset `at_i % tile` and stops at the tile's end, so no piece maps past the end of
/// the file. That matters: shmem refuses a page index at or past the file's size, which is
/// `SIGBUS` for a host access and `EFAULT` (guest death) for a guest one.
///
/// The count is `ceil((at % tile + len) / tile)`. A range inside one tile is ONE piece.
///
/// Pure arithmetic: it yields nothing for `tile == 0`, and an end past `u64::MAX` is clamped.
/// [`ScratchTile::cover`] validates the range against the window before using it.
#[must_use]
pub fn tile_pieces(at: u64, len: u64, tile: u64) -> TilePieces {
    TilePieces {
        next: at,
        end: at.saturating_add(len),
        tile,
    }
}

/// ★★★ The VMA flags every window mapping placed OFF the vCPU carries (2026-10-03):
/// `VM_DONTDUMP`, `VM_HUGEPAGE` and `VM_DONTCOPY`.
///
/// **Why.** QEMU advises a window once, when it registers it as a `ram_device` region: QEMU 10.2.4
/// `ram_block_add` applies `MADV_DONTDUMP` (only with `dump-guest-core=off`), `MADV_HUGEPAGE` and
/// `MADV_DONTFORK` (unless qtest) to the whole range (`system/physmem.c:2294-2304`, `:1858-1871`;
/// a `RAM_PREALLOC` block is never re-advised, `:2684-2685`). A `MAP_FIXED` placement made after
/// that comes back without those flags, and Linux merges neighbouring mappings only when their
/// flags match (7.1 `mm/vma.c:84-96`). So without this, a sink never merged back into the tiling
/// and every place-then-sink cycle left its split boundaries behind (see the module docs).
///
/// **A superset, set FIRST.** The initial cover carries all three before QEMU registers the window
/// (`kf_qemu::mem::window_with_scratch`), so QEMU's own `madvise` finds them set and changes
/// nothing, whatever `dump-guest-core` says. Every later off-vCPU sink carries the same three, so
/// every scratch mapping in the window agrees and a sink merges back. ⚠ This holds while QEMU's
/// set stays inside this one: read in 10.2.4 only.
///
/// **Why each flag is right for a window, not only harmless.**
/// - `DONTDUMP`: with `dump-guest-core=off`, a guest-RAM page placed into a window (a sysmem leaf)
///   would otherwise land in QEMU's core dump; with `=on` (the default) that page is still dumped
///   through guest RAM's own mapping, so nothing is lost.
/// - `DONTFORK`: what QEMU asks of every RAM block; a child of `fork` gets no copy of a window.
/// - `HUGEPAGE`: needed only so that QEMU's advice is a no-op. On the scratch tile it can make one
///   touch allocate a 2 MiB huge page (under `shmem_enabled=advise`); the tile's size still bounds
///   the total.
///
/// Device views are not advised: their mappings are `VM_IO | VM_PFNMAP` and never merge with
/// anything (`VM_SPECIAL`), and the driver already marks them `VM_DONTDUMP`.
pub const WINDOW_ADVICE: [WindowAdvice; 3] = [
    WindowAdvice::DontDump,
    WindowAdvice::HugePage,
    WindowAdvice::DontFork,
];

/// ★ Apply [`WINDOW_ADVICE`] to window `[at, at + len)`: one `madvise` per flag.
///
/// `MADV_HUGEPAGE` refused with `EINVAL` (a kernel built without transparent huge pages) is not an
/// error: QEMU's identical call is refused the same way, so no mapping in the window carries the
/// flag and they still all agree.
///
/// # Errors
/// As [`GuestWindow::advise`]: the first other refusal.
///
/// # Panics
/// If called with any ranked lock held (R1, §4.5).
pub fn advise_window(window: &GuestWindow, at: HostOffset, len: u64) -> Result<(), RawError> {
    for a in WINDOW_ADVICE {
        match window.advise(at, len, a) {
            Ok(()) => {}
            Err(RawError::Syscall {
                errno: Some(libc::EINVAL),
                ..
            }) if a == WindowAdvice::HugePage => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// ★ One window's scratch: a sealed memfd of [`scratch_tile_len`] bytes, mapped again and again
/// over every unmapped part of the window. See the module docs for the bound and the semantics.
#[derive(Debug)]
pub struct ScratchTile {
    ram: SharedRam,
    tile: u64,
}

impl ScratchTile {
    /// Create the scratch tile for `window`, under the memfd creation name `name` (what
    /// `/proc/<pid>/fd` shows as `memfd:<name>`). Covers nothing yet: call [`ScratchTile::cover`].
    ///
    /// # Errors
    /// [`RawError::Misaligned`] if the tile is not a whole number of the window's pages (it always
    /// is for a page-aligned window, since the tile is the window or a power of two of at least
    /// 2 MiB), or the memfd's own refusal ([`SharedRam::create_named`]).
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn for_window(name: &CStr, window: &GuestWindow) -> Result<Self, RawError> {
        let tile = scratch_tile_len(window.len_bytes());
        geometry::require_aligned(tile, window.page_size(), "scratch tile length")?;
        let ram = SharedRam::create_named(name, tile)?;
        Ok(ScratchTile { ram, tile })
    }

    /// ★ Show scratch over window `[at, at + len)`: one `MAP_FIXED` placement per tile piece
    /// ([`tile_pieces`]), so `ceil((at % T + len) / T)` of them, at most `ceil(len / T) + 1`.
    /// Returns the number of `mmap` calls made. It sets no VMA flag: off the vCPU, use
    /// [`ScratchTile::cover_advised`].
    ///
    /// Every argument is checked BEFORE the first `mmap` (zero length, overflow, the window's
    /// bound, page alignment), so a refusal by argument places nothing. Only an `mmap` the kernel
    /// refuses can stop the cover part-way. The pieces before it then show scratch and the pieces
    /// after it still show what they showed before.
    ///
    /// ⊘ **Corrected 2026-10-03: the REFUSED piece itself may be left as a HOLE**, not as what it
    /// showed before. Linux 7.1 clears the old mapping's page tables before it allocates the new
    /// mapping (`mm/vma.c:2476`, in `__mmap_setup`), and a refusal after that point (an allocation,
    /// or the driver's `mmap`) cannot put the old mapping back: it leaves *"a gap where the
    /// MAP_FIXED mapping failed"* (`mm/vma.c:2368-2388`). A guest touching a hole dies (`EFAULT`).
    /// So a refused piece is retried ONCE (into a gap the retry needs no split, so it needs less
    /// memory); if the retry is refused too, the error is returned and that piece may be a hole.
    /// The retry is the only way one piece costs two `mmap`s, and it happens only after a refusal.
    ///
    /// A caller must therefore never assume a refused cover left the old backing visible, and
    /// releases a view only after a successful cover: then the view is reachable nowhere.
    ///
    /// # Errors
    /// [`RawError::ZeroLength`], [`RawError::LengthOverflow`], [`RawError::OutOfRange`],
    /// [`RawError::Misaligned`], [`RawError::TooLargeForHost`], [`RawError::Syscall`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn cover(&self, window: &GuestWindow, at: HostOffset, len: u64) -> Result<usize, RawError> {
        bounds::checked_span(window.len_bytes(), at, len, "scratch cover")?;
        geometry::require_aligned(at.get(), window.page_size(), "scratch cover offset")?;
        geometry::require_aligned(len, window.page_size(), "scratch cover length")?;
        let mut mmaps = 0;
        for p in tile_pieces(at.get(), len, self.tile) {
            // ⊘ Re-checked here, not trusted from the arithmetic: a piece past the tile's end
            // maps a page index shmem refuses, which kills the guest on its first touch.
            if p.file_off.checked_add(p.len).is_none_or(|e| e > self.tile) {
                return Err(RawError::OutOfRange {
                    offset: p.file_off,
                    len: p.len,
                    object_len: self.tile,
                });
            }
            let backing = Backing::SharedFile {
                fd: self.ram.as_backing_fd(),
                offset: p.file_off,
            };
            mmaps += 1;
            if window.place(HostOffset::new(p.at), p.len, backing).is_err() {
                // ⊘ The refused piece may now be a hole (see the docs): one retry.
                mmaps += 1;
                window.place(HostOffset::new(p.at), p.len, backing)?;
            }
        }
        Ok(mmaps)
    }

    /// ★★ [`ScratchTile::cover`], then [`WINDOW_ADVICE`] over the same range ([`advise_window`]):
    /// the initial cover of every window (`kf_qemu::mem::window_with_scratch`) and every sink made
    /// OFF the vCPU. Returns the `mmap` calls made; the advice adds one `madvise` per flag.
    ///
    /// ⊘ Not optional there: without the advice a sink never merges back into the tiling QEMU
    /// advised, and every place-then-sink cycle leaves its split boundaries for the VM's life
    /// (module docs). An advice refusal is returned, so a caller does not count the sink as done:
    /// the range already shows scratch, and a later sink re-covers and re-advises it.
    ///
    /// PRAMIN's trap calls [`ScratchTile::cover`] instead: no `madvise` on a vCPU (owner ruling
    /// 2026-09-25, ONE `mmap` per window move), and it needs none, because every move re-places
    /// all 16 slots, so that window never holds more than 16 mappings.
    ///
    /// # Errors
    /// As [`ScratchTile::cover`] and [`advise_window`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn cover_advised(
        &self,
        window: &GuestWindow,
        at: HostOffset,
        len: u64,
    ) -> Result<usize, RawError> {
        let mmaps = self.cover(window, at, len)?;
        advise_window(window, at, len)?;
        Ok(mmaps)
    }

    /// The tile's length in bytes: the most host RAM this window's scratch can ever hold.
    #[must_use]
    pub fn tile_len(&self) -> u64 {
        self.tile
    }

    /// Host memory the tile has actually allocated ([`SharedRam::allocated_bytes`]).
    ///
    /// # Errors
    /// As [`SharedRam::allocated_bytes`].
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn allocated_bytes(&self) -> Result<u64, RawError> {
        self.ram.allocated_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostPageSize;

    const MIB: u64 = 1024 * 1024;

    fn page() -> HostPageSize {
        HostPageSize::query()
    }

    /// A memfd creation name unique to one test, so tests running in parallel threads of this
    /// binary never count each other's mappings.
    fn named(tag: &str) -> std::ffi::CString {
        std::ffi::CString::new(format!("kf-scratch-test-{tag}-{}", std::process::id()))
            .expect("no NUL in a test name")
    }

    /// Read one byte of every page, the way a guest probing an unmapped BAR does. The sum goes
    /// through `black_box` so the reads cannot be dropped as unused.
    fn read_every_page(w: &GuestWindow) {
        let p = w.page_size().bytes();
        let mut sum = 0u64;
        let mut b = [0u8; 1];
        let mut at = 0;
        while at < w.len_bytes() {
            w.read_into(HostOffset::new(at), &mut b).expect("read");
            sum += u64::from(b[0]);
            at += p;
        }
        std::hint::black_box(sum);
    }

    fn write_every_page(w: &GuestWindow, v: u8) {
        let p = w.page_size().bytes();
        let mut at = 0;
        while at < w.len_bytes() {
            w.write_from(HostOffset::new(at), &[v]).expect("write");
            at += p;
        }
    }

    /// Mappings of this process whose file is the memfd `name`, from `/proc/self/maps`.
    fn mappings_of(name: &std::ffi::CStr) -> usize {
        let want = format!("/memfd:{} (deleted)", name.to_str().expect("utf-8"));
        std::fs::read_to_string("/proc/self/maps")
            .expect("/proc/self/maps")
            .lines()
            .filter(|l| l.ends_with(&want))
            .count()
    }

    /// ★★★ THE KNOWN-POSITIVE. Today's shape until 2026-10-03: ONE memfd of the whole window,
    /// placed 1:1, and nothing but READS. If this ever reads 0 the instrument (`st_blocks`) is
    /// blind, and the bound test below would pass vacuously.
    #[test]
    fn the_old_whole_window_scratch_allocates_the_whole_window_on_reads_alone() {
        let p = page();
        let len = 64 * MIB;
        let w = GuestWindow::create(len, p).expect("a 64 MiB window");
        let old = SharedRam::create_named(&named("old"), len).expect("a whole-window memfd");
        w.place(
            HostOffset::ZERO,
            len,
            Backing::SharedFile {
                fd: old.as_backing_fd(),
                offset: 0,
            },
        )
        .expect("placed 1:1 over the whole window");
        assert_eq!(old.allocated_bytes(), Ok(0), "a fresh memfd holds nothing");

        read_every_page(&w);

        let got = old.allocated_bytes().expect("fstat");
        assert_eq!(
            got, len,
            "a READ of every page of a shared-file window allocates every page (shmem has no \
             zero page). If this is 0 the instrument is blind, not the hole closed."
        );
        assert!(
            got > scratch_tile_len(len),
            "the old shape exceeds the tiled bound: {got} > {}",
            scratch_tile_len(len)
        );
    }

    /// ★★★ THE BOUND. A guest that reads, then writes, every page of a fully unmapped window makes
    /// the host allocate at most one tile, whatever the window's size.
    #[test]
    fn a_tiled_scratch_holds_a_guest_that_touches_every_page_to_one_tile() {
        let p = page();
        for (tag, len) in [("b64", 64 * MIB), ("b256", 256 * MIB)] {
            // The bound is the POLICY's number, never the object's own claim about itself: an
            // implementation that made its tile the whole window must fail here.
            let bound = scratch_tile_len(len);
            assert_eq!(bound, 2 * MIB, "{tag}: a BAR this size gets a 2 MiB tile");
            let w = GuestWindow::create(len, p).expect("window");
            let s = ScratchTile::for_window(&named(tag), &w).expect("tile");
            let mmaps = s.cover(&w, HostOffset::ZERO, len).expect("cover");

            read_every_page(&w);
            let after_reads = s.allocated_bytes().expect("fstat");
            assert!(
                after_reads <= bound,
                "{tag}: reads of every page allocated {after_reads} > bound {bound}"
            );
            assert!(after_reads > 0, "{tag}: the reads did reach the tile");

            write_every_page(&w, 0x5A);
            let after_writes = s.allocated_bytes().expect("fstat");
            assert!(
                after_writes <= bound,
                "{tag}: writes of every page allocated {after_writes} > bound {bound}"
            );
            assert_eq!(after_writes, bound, "{tag}: and they fill exactly one tile");
            assert_eq!(s.tile_len(), bound);
            assert_eq!(mmaps as u64, len / bound, "{tag}: one mmap per tile");
        }
    }

    /// Window offset `o` shows tile byte `o % T`: a write lands at every `T` stride of the same
    /// window and nowhere else in it.
    ///
    /// ⊘ `T` is the POLICY's number ([`scratch_tile_len`]), never the object's own claim
    /// (`s.tile_len()`): a scratch that made its tile the whole window would report `T = len`,
    /// check a single stride, and pass (it did, in the 2026-10-03 review's bite run).
    #[test]
    fn scratch_aliases_every_tile_length_and_nowhere_else() {
        let p = page();
        let len = 8 * MIB;
        let t = scratch_tile_len(len);
        assert!(t < len, "an 8 MiB window is more than one tile ({t:#x})");
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&named("alias"), &w).expect("tile");
        s.cover(&w, HostOffset::ZERO, len).expect("cover");
        let x = p.bytes() + 8;
        w.write_from(HostOffset::new(x), &[0xAB]).expect("write");
        let mut b = [0u8; 1];
        let mut strides = 0u64;
        for k in 0..len / t {
            w.read_into(HostOffset::new(x + k * t), &mut b)
                .expect("read");
            assert_eq!(b[0], 0xAB, "tile {k} shows the same byte");
            strides += 1;
        }
        assert!(
            strides > 1,
            "only {strides} stride checked: nothing was shown to alias"
        );
        w.read_into(HostOffset::new(x + 1), &mut b).expect("read");
        assert_eq!(b[0], 0, "the neighbouring byte is untouched");
    }

    /// The flags QEMU 10.2.4's `ram_block_add` gives a window when it registers it
    /// (`system/physmem.c:2294-2304`): `MADV_DONTDUMP` only with `dump-guest-core=off`, then
    /// `MADV_HUGEPAGE`, then `MADV_DONTFORK`. Applied the way QEMU does, AFTER our cover. Returns
    /// whether the kernel has transparent huge pages (`MADV_HUGEPAGE` not refused with `EINVAL`).
    fn qemu_registers(w: &GuestWindow, dump_guest_core: bool) -> bool {
        let all = w.len_bytes();
        if !dump_guest_core {
            w.advise(HostOffset::ZERO, all, WindowAdvice::DontDump)
                .expect("MADV_DONTDUMP");
        }
        let thp = match w.advise(HostOffset::ZERO, all, WindowAdvice::HugePage) {
            Ok(()) => true,
            Err(RawError::Syscall {
                errno: Some(libc::EINVAL),
                ..
            }) => false,
            Err(e) => panic!("MADV_HUGEPAGE: {e:?}"),
        };
        w.advise(HostOffset::ZERO, all, WindowAdvice::DontFork)
            .expect("MADV_DONTFORK");
        thp
    }

    /// The `VmFlags` of every mapping of memfd `name`, from `/proc/self/smaps`.
    fn vmflags_of(name: &std::ffi::CStr) -> Vec<String> {
        let want = format!("/memfd:{} (deleted)", name.to_str().expect("utf-8"));
        let smaps = std::fs::read_to_string("/proc/self/smaps").expect("/proc/self/smaps");
        let mut ours = false;
        let mut out = Vec::new();
        for l in smaps.lines() {
            // A mapping's header line starts with `start-end`; every field line with `Key:`.
            let header = l
                .split_whitespace()
                .next()
                .is_some_and(|f| !f.ends_with(':'));
            if header {
                ours = l.ends_with(&want);
            } else if let Some(flags) = l.strip_prefix("VmFlags:")
                && ours
            {
                out.push(flags.trim().to_string());
            }
        }
        out
    }

    /// Window pages for place-then-sink cycle `k`: distinct for every `k`, 13 pages apart so no two
    /// stand-ins touch, and some of them straddle a tile boundary.
    fn cycle_at(k: u64, pg: u64) -> u64 {
        (13 * k + 5) * pg
    }

    /// ★★★ THE MAPPING BOUND, as production runs it (2026-10-03): the window is covered with the
    /// advice ([`ScratchTile::cover_advised`], what `window_with_scratch` does), THEN QEMU
    /// registers it and advises it, and then the guest drives many place-then-sink cycles at
    /// distinct offsets. After every sink the scratch mappings are back to `ceil(W / T)`, for both
    /// `dump-guest-core` settings, and every one of them carries the window's flags.
    ///
    /// ⊘ Against the shape before this fix (a plain [`ScratchTile::cover`] for the cover and the
    /// sinks) it fails on the first cycle and ends at `canonical + 2 × cycles`: that is
    /// `an_unadvised_sink_beside_qemus_advice_never_merges_back`, its known-positive.
    #[test]
    fn a_sink_restores_the_canonical_tiling_after_qemu_has_advised_the_window() {
        let p = page();
        let pg = p.bytes();
        let len = 64 * MIB;
        let t = scratch_tile_len(len);
        let canonical = usize::try_from(len / t).expect("small");
        assert!(canonical > 1, "the window must span several tiles");
        for dump_guest_core in [true, false] {
            let name = named(if dump_guest_core {
                "sink-dump"
            } else {
                "sink-nodump"
            });
            let w = GuestWindow::create(len, p).expect("window");
            let s = ScratchTile::for_window(&name, &w).expect("tile");
            s.cover_advised(&w, HostOffset::ZERO, len)
                .expect("the initial cover");
            let thp = qemu_registers(&w, dump_guest_core);
            assert_eq!(
                mappings_of(&name),
                canonical,
                "QEMU's advice split nothing: the cover already carried it"
            );

            let stand_in = SharedRam::create_named(&named("view"), 8 * pg).expect("a stand-in");
            let cycles = ((len / pg).saturating_sub(16) / 13).min(600);
            assert!(cycles >= 100, "enough cycles to see growth ({cycles})");
            let mut worst = (0usize, 0u64);
            let mut worst_live = 0usize;
            let mut crossed = 0;
            for k in 0..cycles {
                let at = cycle_at(k, pg);
                w.place(
                    HostOffset::new(at),
                    8 * pg,
                    Backing::SharedFile {
                        fd: stand_in.as_backing_fd(),
                        offset: 0,
                    },
                )
                .expect("place a view");
                // A live placement splits at most one tile into two.
                worst_live = worst_live.max(mappings_of(&name));
                let mmaps = s
                    .cover_advised(&w, HostOffset::new(at), 8 * pg)
                    .expect("sink");
                let want = (at % t + 8 * pg).div_ceil(t);
                assert_eq!(mmaps as u64, want, "one mmap per tile the sink touches");
                if want == 2 {
                    crossed += 1;
                }
                let after = mappings_of(&name);
                if after > worst.0 {
                    worst = (after, k);
                }
            }
            assert!(crossed > 0, "some cycle straddled a tile boundary");
            let end = mappings_of(&name);
            assert_eq!(
                (worst.0, end),
                (canonical, canonical),
                "dump-guest-core={dump_guest_core}: after {cycles} place-then-sink cycles the \
                 scratch has {end} mappings (worst {} after cycle {}), canonical {canonical}: a \
                 sink did not merge back",
                worst.0,
                worst.1
            );
            assert!(
                worst_live <= canonical + 1,
                "a live placement split more than one tile: {worst_live}"
            );
            let flags = vmflags_of(&name);
            assert_eq!(flags.len(), canonical, "one VmFlags line per mapping");
            for f in &flags {
                let has = |x: &str| f.split_whitespace().any(|w| w == x);
                assert!(
                    has("dd") && has("dc"),
                    "a scratch mapping without the advice: {f}"
                );
                assert!(
                    !thp || has("hg"),
                    "a scratch mapping without VM_HUGEPAGE: {f}"
                );
            }

            read_every_page(&w);
            write_every_page(&w, 0xA5);
            assert_eq!(
                s.allocated_bytes(),
                Ok(t),
                "the bound still holds after sinks"
            );
        }
    }

    /// ★ THE KNOWN-POSITIVE for the mapping bound: the shape before 2026-10-03. A sink with no
    /// advice, beside a tiling QEMU advised, never merges back, and every cycle at a new offset
    /// leaves two more mappings. If this ever reads `canonical`, the count is blind (or the kernel
    /// stopped comparing flags) and the test above passes vacuously.
    ///
    /// PRAMIN still sinks this way (on the vCPU, no `madvise`); it stays bounded only because every
    /// move re-places its 16 slots whole.
    #[test]
    fn an_unadvised_sink_beside_qemus_advice_never_merges_back() {
        let p = page();
        let pg = p.bytes();
        let len = 64 * MIB;
        let t = scratch_tile_len(len);
        let canonical = usize::try_from(len / t).expect("small");
        let name = named("unadvised");
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&name, &w).expect("tile");
        s.cover(&w, HostOffset::ZERO, len).expect("cover");
        qemu_registers(&w, true);
        let stand_in = SharedRam::create_named(&named("view2"), 8 * pg).expect("a stand-in");
        let cycles = 64u64;
        for k in 0..cycles {
            let at = cycle_at(k, pg);
            w.place(
                HostOffset::new(at),
                8 * pg,
                Backing::SharedFile {
                    fd: stand_in.as_backing_fd(),
                    offset: 0,
                },
            )
            .expect("place a view");
            s.cover(&w, HostOffset::new(at), 8 * pg).expect("sink");
        }
        let got = mappings_of(&name);
        assert!(
            got >= canonical + usize::try_from(cycles).expect("small"),
            "{got} mappings after {cycles} unadvised sinks (canonical {canonical}): the instrument \
             no longer sees the growth"
        );
    }

    /// ★ PRAMIN's invariant (owner ruling 2026-09-25): every re-point of a 1 MiB window is ONE
    /// `mmap`, so its tile must be the whole window and any run inside it ONE piece.
    #[test]
    fn a_pramin_sized_window_is_its_own_tile_and_every_sink_is_one_mmap() {
        let p = page();
        let len = MIB;
        assert_eq!(scratch_tile_len(len), len);
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&named("pramin"), &w).expect("tile");
        assert_eq!(s.cover(&w, HostOffset::ZERO, len), Ok(1));
        let g = 64 * 1024;
        for first in 0..16 {
            for n in 1..=(16 - first) {
                assert_eq!(
                    s.cover(&w, HostOffset::new(first * g), n * g),
                    Ok(1),
                    "slots {first}+{n}"
                );
            }
        }
    }

    /// Arguments are refused before any `mmap`: nothing past the window, nothing misaligned,
    /// nothing empty.
    #[test]
    fn a_cover_is_refused_by_argument_before_it_maps_anything() {
        let p = page();
        let len = 4 * MIB;
        let name = named("refuse");
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&name, &w).expect("tile");
        assert!(matches!(
            s.cover(&w, HostOffset::new(2 * MIB), 4 * MIB),
            Err(RawError::OutOfRange { .. })
        ));
        assert!(matches!(
            s.cover(&w, HostOffset::new(1), p.bytes()),
            Err(RawError::Misaligned { .. })
        ));
        assert!(matches!(
            s.cover(&w, HostOffset::ZERO, p.bytes() + 1),
            Err(RawError::Misaligned { .. })
        ));
        assert!(matches!(
            s.cover(&w, HostOffset::ZERO, 0),
            Err(RawError::ZeroLength { .. })
        ));
        assert!(matches!(
            s.cover(&w, HostOffset::new(u64::MAX - 1), 4),
            Err(RawError::LengthOverflow { .. })
        ));
        assert_eq!(mappings_of(&name), 0, "no refusal placed anything");
    }

    #[test]
    fn the_tile_length_follows_the_table() {
        let gib = 1024 * MIB;
        for (window, tile) in [
            (0, 0),
            (64 * 1024, 64 * 1024),
            (MIB, MIB),
            (3 * MIB, 2 * MIB),
            (32 * MIB, 2 * MIB),
            (128 * MIB, 2 * MIB),
            (256 * MIB, 2 * MIB),
            (2 * gib, 2 * MIB),
            (2 * gib + 4096, 4 * MIB),
            (16 * gib, 16 * MIB),
            (128 * gib, 128 * MIB),
        ] {
            assert_eq!(scratch_tile_len(window), tile, "window {window:#x}");
            if window > 0 {
                assert!(window.div_ceil(tile) <= SCRATCH_TILES_MAX);
            }
        }
    }

    /// ★ The piece arithmetic, swept: the pieces tile `[at, at + len)` exactly, in order, never
    /// cross the tile's end, show `at % tile`, and number `ceil((at % tile + len) / tile)`.
    #[test]
    fn tile_pieces_cover_the_range_exactly_and_never_cross_the_tile_end() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for tile in [4096, 64 * 1024, MIB, 2 * MIB, 16 * MIB] {
            for _ in 0..2000 {
                let at = (next() % (64 * MIB / 4096)) * 4096;
                let len = (next() % (16 * MIB / 4096) + 1) * 4096;
                let pieces: Vec<TilePiece> = tile_pieces(at, len, tile).collect();
                let want = (at % tile + len).div_ceil(tile);
                assert_eq!(
                    pieces.len() as u64,
                    want,
                    "count at {at:#x}+{len:#x} T {tile:#x}"
                );
                let mut cur = at;
                for pc in &pieces {
                    assert_eq!(pc.at, cur, "contiguous");
                    assert!(pc.len > 0);
                    assert_eq!(pc.file_off, pc.at % tile);
                    assert!(pc.file_off + pc.len <= tile, "never past the tile's end");
                    cur += pc.len;
                }
                assert_eq!(cur, at + len, "covers the range exactly");
            }
        }
        assert_eq!(tile_pieces(0, 4096, 0).count(), 0, "no tile, no pieces");
        assert_eq!(tile_pieces(4096, 0, MIB).count(), 0, "no length, no pieces");
    }
}
