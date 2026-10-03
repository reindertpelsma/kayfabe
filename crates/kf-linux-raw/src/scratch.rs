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
//! - Guest RAM, which is pinned whole. The launcher's memory cgroup has to cover both
//!   (`V3_SWEEP_AND_INSTALL.md` §2.6).

use crate::bounds::{self, HostOffset};
use crate::error::RawError;
use crate::geometry;
use crate::host_fd_unsafe::SharedRam;
use crate::mapping_unsafe::Backing;
use crate::window_unsafe::GuestWindow;
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
    /// ([`tile_pieces`]). Returns the number of `mmap` calls made.
    ///
    /// Every argument is checked BEFORE the first `mmap` (zero length, overflow, the window's
    /// bound, page alignment), so a refusal by argument places nothing. Only an `mmap` the kernel
    /// refuses can stop the cover part-way; the error then says so, and the pieces before it show
    /// scratch while the rest still show what they showed before. A caller that releases a view
    /// only after a successful cover therefore never exposes a released view.
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
            window.place(
                HostOffset::new(p.at),
                p.len,
                Backing::SharedFile {
                    fd: self.ram.as_backing_fd(),
                    offset: p.file_off,
                },
            )?;
            mmaps += 1;
        }
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
    #[test]
    fn scratch_aliases_every_tile_length_and_nowhere_else() {
        let p = page();
        let len = 8 * MIB;
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&named("alias"), &w).expect("tile");
        s.cover(&w, HostOffset::ZERO, len).expect("cover");
        let t = s.tile_len();
        let x = p.bytes() + 8;
        w.write_from(HostOffset::new(x), &[0xAB]).expect("write");
        let mut b = [0u8; 1];
        for k in 0..len / t {
            w.read_into(HostOffset::new(x + k * t), &mut b)
                .expect("read");
            assert_eq!(b[0], 0xAB, "tile {k} shows the same byte");
        }
        w.read_into(HostOffset::new(x + 1), &mut b).expect("read");
        assert_eq!(b[0], 0, "the neighbouring byte is untouched");
    }

    /// ★★ A sink (cover) after a placement restores the canonical tiling: the bound still holds,
    /// and the scratch mappings merge back to `ceil(W / T)`, for a placement inside one tile and
    /// for one that crosses a tile boundary.
    #[test]
    fn a_sink_after_a_placement_restores_the_canonical_tiling() {
        let p = page();
        let pg = p.bytes();
        let len = 64 * MIB;
        let name = named("sink");
        let w = GuestWindow::create(len, p).expect("window");
        let s = ScratchTile::for_window(&name, &w).expect("tile");
        let t = s.tile_len();
        s.cover(&w, HostOffset::ZERO, len).expect("cover");
        let canonical = usize::try_from(len / t).expect("small");
        assert_eq!(mappings_of(&name), canonical, "one mapping per tile");

        let stand_in = SharedRam::create_named(&named("view"), 8 * pg).expect("stand-in view");
        // Inside tile 1, then across the tile 1 / tile 2 boundary.
        for (at, crosses) in [(t + 4 * pg, false), (2 * t - 4 * pg, true)] {
            w.place(
                HostOffset::new(at),
                8 * pg,
                Backing::SharedFile {
                    fd: stand_in.as_backing_fd(),
                    offset: 0,
                },
            )
            .expect("place a view");
            let split = if crosses { canonical } else { canonical + 1 };
            assert_eq!(
                mappings_of(&name),
                split,
                "the placement splits at most one tile"
            );

            let mmaps = s.cover(&w, HostOffset::new(at), 8 * pg).expect("sink");
            assert_eq!(
                mmaps,
                if crosses { 2 } else { 1 },
                "one mmap per tile touched"
            );
            assert_eq!(
                mappings_of(&name),
                canonical,
                "the sink merges back into the canonical tiling"
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
