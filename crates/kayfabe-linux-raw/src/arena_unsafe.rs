//! ★★★★★ **w393 — THE FRAMEBUFFER PAGE ARENA**: one sealed `memfd`, mapped once, handed out
//! as exclusively-owned 4 KiB pages that a KVM memslot can ALSO name.
//!
//! # Why this exists — the demand-driven BAR mirror needs pages a memslot can point at
//!
//! `docs/design/bar1_passthrough_device_local_host_visible.md` §2.3 ends with *"install on
//! PTE publication or on first-touch"*. First-touch means: the guest's access to an
//! uncovered BAR1/BAR2 page traps **once**, we resolve the page through the guest's own
//! page table, and we install a memslot over **the memory the store already serves that
//! frame from** — so every later access to the frame is exit-free and lands in the very
//! bytes the walker, the BAR0 window and the join all read.
//!
//! A `Box<[u8; 4096]>` on the heap cannot be that memory: a memslot names a host virtual
//! range, and a heap page has no descriptor, no stable file offset, and no second mapping.
//! So the store's pages have to come from something with a descriptor. This is that
//! something: a `memfd` of [`SharedPageArena::LEN`] bytes, sealed against resize, mapped
//! `MAP_SHARED` **once** for the store's own reads and writes. A page handed out of it is
//! `(descriptor, index × 4096)`, and a second `MAP_SHARED` mapping of that offset — placed
//! inside a [`crate::GuestWindow`] and installed as a memslot — is the **same physical
//! pages**. One memory, two names, which is the whole invariant of the BAR mirror.
//!
//! ⊘ The `memfd` is **sparse**: `ftruncate` allocates nothing, `MAP_SHARED` of a writable
//! shmem mapping is not overcommit-accounted, and a page costs host memory only when it is
//! first written — so the 1 GiB length is an address-space reservation, not a residency
//! claim. Residency is bounded where it always was, by the store's own cap.
//!
//! # Soundness — why [`ArenaPage`] may be `Send` and the arena `Sync`
//!
//! `MappedRegion` is deliberately **not** `Sync`: two threads writing the same bytes through
//! `&self` would be a data race. The arena shares one `MappedRegion` between every page
//! handle, so it needs a different argument, and it has one: **every page index is owned by
//! exactly one live [`ArenaPage`] at a time.** [`SharedPageArena::alloc`] hands an index out
//! under a mutex and never hands it out again until the handle's `Drop` returns it, so two
//! live handles address **disjoint** byte ranges by construction, and `ArenaPage::write_from`
//! takes `&mut self`. A `memcpy` into one 4 KiB range and a concurrent `memcpy` into a
//! different one do not race. The only shared state — the free list and the bump cursor —
//! is behind a `Mutex`.
//!
//! ⚠ What this argument does **not** cover, stated so nobody extends it by accident: a
//! *second* mapping of the same page (the memslot the guest writes through) is outside the
//! abstract machine, exactly as a GPU writing a `VolatileRegion` is. The bytes a guest
//! stores through its memslot and the bytes the store reads through this mapping are the
//! same physical page and may be torn against each other; the store's callers already hold
//! that hazard for every joined leaf, and the mirror does not add a new kind of it.

use crate::bounds::HostOffset;
use crate::error::RawError;
use crate::host_fd_unsafe::SharedRam;
use crate::cache::CachePolicy;
use crate::mapping_unsafe::{Backing, HostProt, MappedRegion};
use crate::page_size::HostPageSize;
use kayfabe_util::lockwitness;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::{Arc, Mutex};

/// The page size this arena deals in — the framebuffer store's `FB_PAGE`, restated here
/// because this crate must not name that crate. Checked against the host page size at
/// creation: an arena whose page is not a whole number of host pages cannot be memslotted.
pub const ARENA_PAGE: u64 = 4096;

/// The `memfd`'s creation name, for [`crate::MemfdCensus`] to tell it apart from guest RAM.
const ARENA_NAME: &std::ffi::CStr = c"kayfabe-fb-arena";

/// ★ The arena refused to hand out a page: every index is live. Named, never silent — the
/// caller falls back to a heap page and counts it, so the boot's census can say how often
/// the arena was the binding limit.
/// A framebuffer address that is not a whole page — refused rather than rounded, because
/// rounding would put two distinct addresses on one page and give one of them the other's
/// bytes, silently.
pub const ARENA_MISALIGNED: &str = "that framebuffer address is not page-aligned";

pub const ARENA_EXHAUSTED: &str = "the framebuffer page arena is exhausted: every 4 KiB index \
     of its fixed extent is owned by a live page";

#[derive(Debug)]
struct ArenaInner {
    /// The sealed descriptor. Kept for the whole life of the arena: a page's export names
    /// it, and a memslot placement `mmap`s it again.
    file: SharedRam,
    /// The store's own single mapping of the whole extent.
    map: MappedRegion,
    /// Total pages this arena can hand out.
    pages: u64,
    /// The allocator: a bump cursor plus a LIFO free list, both behind one lock.
    free: Mutex<ArenaFree>,
}

#[derive(Debug)]
struct ArenaFree {
    /// The next never-issued index.
    next: u64,
    /// Indices whose page handle was dropped and that may be issued again.
    // ⊘⊘ **REMOVED w587.** This was the bump allocator's free list, left behind by w569 when
    // the framebuffer address became the file offset and recycling stopped existing. Nothing
    // POPPED from it after w569 — it only grew — and w585's read path made that fatal: every
    // trapped read of a non-resident frame took a transient page handle whose `Drop` pushed
    // one `u64` here, under this mutex, **inside an MMIO exit**. An unbounded `Vec` push on
    // the vCPU's read path, in a device whose governing rule is *no blocking work in any MMIO
    // trap*. ⇒ Deleted, and `read_at` below no longer allocates a handle at all.
    /// Cumulative reuses — the non-vacuity witness for the free list.
    recycled: u64,
    /// How many pages are live right now.
    live: u64,
    /// The most pages ever live at once.
    peak: u64,
}

// SAFETY: see the module docs. `map` is reached only through `ArenaPage` handles, each of
// which owns a distinct index for its whole life (issued once under `free`, returned only by
// its own `Drop`), so concurrent accesses through different handles address disjoint byte
// ranges of one mapping. `file` is an `OwnedFd` plus a length and is never mutated. `free`
// is a `Mutex`. `pages` is immutable.
unsafe impl Sync for ArenaInner {}
// SAFETY: every field is `Send` (`MappedRegion` and `SharedRam` are, `Mutex<ArenaFree>` is).
unsafe impl Send for ArenaInner {}

/// ★★★★★ **The arena** — see the module docs. Cheap to clone; every clone is the same
/// `memfd`, the same mapping and the same allocator.
#[derive(Debug, Clone)]
pub struct SharedPageArena {
    inner: Arc<ArenaInner>,
}

impl SharedPageArena {
    /// The arena's fixed extent: 1 GiB, matching the framebuffer store's residency ceiling
    /// so the arena is never the tighter of the two limits on a healthy boot.
    /// ⊘⊘ **w578 — THE EXTENT IS THE FRAMEBUFFER'S, NOT A RESIDENCY CEILING.**
    ///
    /// This was 1 GiB, *"matching the framebuffer store's residency ceiling"*. That made sense
    /// while a page's file offset was its ALLOCATION ORDER — the arena only ever had to hold as
    /// many pages as were resident at once. Since w569 the offset IS the framebuffer address,
    /// so the extent has to span every address the guest can name, and the two numbers stopped
    /// being the same thing.
    ///
    /// ★ It costs nothing. A `memfd` is sparse: a page exists when it is first touched, so a
    /// 16 GiB extent with 300 MiB resident occupies 300 MiB. Residency is still bounded by the
    /// STORE's own ceiling, which is where that limit belongs.
    ///
    /// ⚠ Kept as a default for callers that do not know the chip; `create_for` takes the real
    /// framebuffer length.
    pub const LEN: u64 = 16 << 30;

    /// Create the `memfd`, seal it, and map it once.
    ///
    /// # Errors
    /// [`RawError`] from `memfd_create`, `ftruncate`, sealing or `mmap`; and
    /// [`RawError::Misaligned`] if [`ARENA_PAGE`] is not a whole number of host pages, which
    /// is the one geometry under which a page here could not be memslotted.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    /// The arena sized for a specific framebuffer. See [`SharedPageArena::LEN`] for why the
    /// extent is the framebuffer's length and why a larger one is free.
    ///
    /// # Errors
    /// As [`SharedPageArena::create`].
    pub fn create_for(fb_len: u64, page: HostPageSize) -> Result<Self, RawError> {
        Self::create_sized(fb_len.max(ARENA_PAGE), page)
    }

    pub fn create(page: HostPageSize) -> Result<Self, RawError> {
        Self::create_sized(Self::LEN, page)
    }

    fn create_sized(len: u64, page: HostPageSize) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("memfd_create + mmap (the framebuffer page arena)");
        crate::geometry::require_aligned(ARENA_PAGE, page, "arena page")?;
        let file = SharedRam::create_named(ARENA_NAME, len)?;
        let map = MappedRegion::map(
            Backing::SharedFile {
                fd: file.as_backing_fd(),
                offset: 0,
            },
            len,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )?;
        Ok(SharedPageArena {
            inner: Arc::new(ArenaInner {
                file,
                map,
                pages: len / ARENA_PAGE,
                free: Mutex::new(ArenaFree {
                    next: 0,
                    recycled: 0,
                    live: 0,
                    peak: 0,
                }),
            }),
        })
    }

    /// Hand out one page, or refuse by name when every index is live.
    ///
    /// ⊘ **No syscall, no ranked lock, no blocking**: a bump or a `Vec::pop` under a plain
    /// mutex. That is what lets the framebuffer store call this from under the register
    /// plane's own lock, which nothing blocking may sit beneath.
    ///
    /// The page's bytes are whatever the `memfd` holds at that offset: zero for a
    /// never-issued index (sparse), and **whatever the previous owner left** for a recycled
    /// one. A caller that needs zeros writes them; this does not, because the common caller
    /// (the store) overwrites the whole page from its own copy anyway.
    ///
    /// # Errors
    /// [`ARENA_EXHAUSTED`].
    /// ★★★★★ **w569 — THE PAGE BACKING FRAMEBUFFER ADDRESS `frame`, at fd offset `frame`.**
    ///
    /// # ⊘ This REPLACED an allocator, it did not gain a parameter
    ///
    /// `alloc()` used to hand out the next free index — a bump cursor plus a recycling free
    /// list — so a page's file offset recorded WHEN it was created and said nothing about
    /// WHERE in the framebuffer it lives. A contiguous run of framebuffer addresses was a
    /// scattered set of file offsets, and the 1 MiB `PRAMIN` window could not be placed with
    /// one `mmap` (`the_bar0_read_surface.md` §3b).
    ///
    /// ★ Address-indexing deletes the allocator: the address IS the offset. No cursor, no
    /// free list, no recycling — and no way for two framebuffer addresses to collide on one
    /// page, which the old scheme prevented only by never reusing an index while it was live.
    ///
    /// ⊘ Idempotent by construction: asking twice for one `frame` yields the same file
    /// offset, so the caller cannot mint a second home for one framebuffer byte.
    ///
    /// # Errors
    /// [`ARENA_EXHAUSTED`] when `frame` lies past the arena's extent — which now means *"past
    /// the framebuffer"* rather than *"we ran out of pages"*, and is a refusal about an
    /// ADDRESS instead of about a supply.
    pub fn alloc_at(&self, frame: u64) -> Result<ArenaPage, &'static str> {
        if !frame.is_multiple_of(ARENA_PAGE) {
            return Err(ARENA_MISALIGNED);
        }
        let index = frame / ARENA_PAGE;
        if index >= self.inner.pages {
            return Err(ARENA_EXHAUSTED);
        }
        let mut f = self.inner.free.lock().unwrap_or_else(|e| e.into_inner());
        f.live += 1;
        f.peak = f.peak.max(f.live);
        f.recycled += 1; // ★ w587: allocations, under its old field name — see `census`.
        f.next = f.next.max(index + 1); // ★ w587: the highest index named, i.e. the span used.
        Ok(ArenaPage {
            arena: Arc::clone(&self.inner),
            index,
        })
    }

    /// ★★★★★ **Read from a framebuffer address WITHOUT taking a page handle (w587).**
    ///
    /// ⊘⊘ w585 needed this and did not have it, so it took a transient [`ArenaPage`] per read
    /// and dropped it — which meant a mutex acquisition and, until this commit, an unbounded
    /// `Vec` push **inside an MMIO exit**, on the vCPU. The read itself needs neither: the
    /// mapping is already there and the bound is arithmetic.
    ///
    /// `addr` is a framebuffer BYTE ADDRESS; the page it names is at the same file offset.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] when the access leaves the named page — refused, never wrapped
    /// into a neighbour, exactly as [`ArenaPage::read_into`] refuses it.
    pub fn read_at(&self, addr: u64, off: u64, dst: &mut [u8]) -> Result<(), RawError> {
        let len = dst.len() as u64;
        let end = off.checked_add(len).ok_or(RawError::LengthOverflow {
            offset: off,
            len,
        })?;
        let index = addr / ARENA_PAGE;
        if end > ARENA_PAGE || !addr.is_multiple_of(ARENA_PAGE) || index >= self.inner.pages {
            return Err(RawError::OutOfRange {
                offset: off,
                len,
                object_len: ARENA_PAGE,
            });
        }
        self.inner
            .map
            .read_into(HostOffset::new(index * ARENA_PAGE + off), dst)
    }

    /// The descriptor a memslot placement maps. Borrowed from the arena, which outlives
    /// every page and every placement (the placement's VMA holds the file regardless).
    #[must_use]
    pub fn as_backing_fd(&self) -> BorrowedFd<'_> {
        self.inner.file.as_backing_fd()
    }

    /// ★★★★★ **Return every page to zero, and release its memory (w585).**
    ///
    /// ⊘ Needed only because the store now treats this file as the **single source of truth
    /// for residency**: after an arena is installed, a framebuffer address that no store page
    /// covers is not "unwritten and therefore zero" — it is *whatever the file holds*, because
    /// a memory slot over this arena lets the guest write it with no trap at all.
    ///
    /// ★ That makes a device reset a **content-leak question**. `SparseFb::device_reset`
    /// clears its page map precisely so one guest's framebuffer bytes cannot be read by the
    /// next; under the new model clearing the map no longer clears the bytes, so the reset
    /// must reach the file. ⚠ It could not do that by writing: the arena is framebuffer-sized
    /// (12 GiB on a GA106), and a `memset` of it on the reset path is not a reset, it is a
    /// hang.
    ///
    /// `FALLOC_FL_PUNCH_HOLE` is the primitive that says exactly this: drop the backing pages,
    /// keep the length, subsequent reads see zero. It is O(extents), not O(bytes).
    ///
    /// ⚠ Existing `MAP_SHARED` mappings — ours, and any memslot the VMM has installed — stay
    /// valid and start reading zero. That is the wanted semantics for a device reset and the
    /// reason this is safe to call with placements live.
    ///
    /// # Errors
    /// [`RawError`] from `fallocate`.
    pub fn punch_all(&self) -> Result<(), RawError> {
        let len = self.inner.pages * ARENA_PAGE;
        // SAFETY: `fd` is this arena's own `memfd`, borrowed for the call; the range is the
        // file's whole declared length; `fallocate` writes no memory of ours.
        let rc = unsafe {
            libc::fallocate(
                self.inner.file.as_backing_fd().as_raw_fd(),
                libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
                0,
                len as libc::off_t,
            )
        };
        if rc != 0 {
            return Err(crate::error::last_syscall_error(
                "fallocate(PUNCH_HOLE) on the page arena",
            ));
        }
        Ok(())
    }

    /// `(live, peak, allocations, distinct indices)` — the allocator's census.
    ///
    /// ⊘⊘ **w587 — TWO OF THESE FIELDS COULD ONLY EVER PRINT ZERO, and I read one of them as
    /// evidence.** They are the bump allocator's `recycled` and `next`, left behind when w569
    /// replaced the allocator with address indexing: `alloc_at` bumps neither, so
    /// `recycled=0 issued=0` was printed in every census ever taken, for a reason that has
    /// nothing to do with the arena's behaviour.
    ///
    /// ⚠ `[measured w587]` I read `arena[... recycled=0 issued=0]` off a boot as *"the arena
    /// issued zero pages"*, and reasoned from it that my own changes were not active — which
    /// would have exonerated the wrong commits. **A field that is structurally zero is not a
    /// measurement, and it is indistinguishable from one that is zero because nothing
    /// happened.** ⇒ `allocations` now counts `alloc_at` calls and `distinct` counts the
    /// indices they named, both of which are facts about this allocator rather than the one
    /// it replaced.
    #[must_use]
    pub fn census(&self) -> (u64, u64, u64, u64) {
        let f = self.inner.free.lock().unwrap_or_else(|e| e.into_inner());
        (f.live, f.peak, f.recycled, f.next)
    }
}

/// One exclusively-owned 4 KiB page of a [`SharedPageArena`].
///
/// `Send` (the handle may move between threads), **not** `Sync` (two threads must not share
/// one handle — the store owns each behind its own lock, which is enough).
#[derive(Debug)]
pub struct ArenaPage {
    arena: Arc<ArenaInner>,
    index: u64,
}

impl ArenaPage {
    /// The byte offset **into the arena's `memfd`** where this page begins — the second half
    /// of the name a memslot placement needs (the first is
    /// [`SharedPageArena::as_backing_fd`]).
    #[must_use]
    pub fn file_offset(&self) -> u64 {
        self.index * ARENA_PAGE
    }

    /// The page's index, for a census line.
    #[must_use]
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Fill `dst` from byte `off` of this page.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] (or its span-arithmetic siblings) when `off + dst.len()`
    /// leaves the page — refused, never wrapped into a neighbour that another handle owns.
    pub fn read_into(&self, off: u64, dst: &mut [u8]) -> Result<(), RawError> {
        self.span(off, dst.len() as u64)?;
        self.arena
            .map
            .read_into(HostOffset::new(self.file_offset() + off), dst)
    }

    /// Write `src` at byte `off` of this page. `&mut self`, so the exclusive-ownership
    /// argument in the module docs is stated by the signature and not only by the allocator.
    ///
    /// # Errors
    /// As [`ArenaPage::read_into`].
    pub fn write_from(&mut self, off: u64, src: &[u8]) -> Result<(), RawError> {
        self.span(off, src.len() as u64)?;
        self.arena
            .map
            .write_from(HostOffset::new(self.file_offset() + off), src)
    }

    /// The page-local bound, checked **before** the arena-wide one so a caller's mistake is
    /// refused as "outside this page" and can never become "inside a neighbour".
    fn span(&self, off: u64, len: u64) -> Result<(), RawError> {
        match off.checked_add(len) {
            Some(end) if end <= ARENA_PAGE => Ok(()),
            Some(_) => Err(RawError::OutOfRange {
                offset: off,
                len,
                object_len: ARENA_PAGE,
            }),
            None => Err(RawError::LengthOverflow { offset: off, len }),
        }
    }
}

impl Drop for ArenaPage {
    fn drop(&mut self) {
        // ★ Only the live count moves. ⊘ There is no free list to return the index to: under
        // address indexing the index IS the framebuffer address, so it is not a resource that
        // can be recycled — it is a name. See the removed field in `ArenaFree`.
        let mut f = self.arena.free.lock().unwrap_or_else(|e| e.into_inner());
        f.live = f.live.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_disjoint_and_addressed_by_their_framebuffer_offset() {
        let arena = SharedPageArena::create(HostPageSize::query()).expect("arena");
        // ★★★ w569 — the test's own subject changed with the allocator. It used to pin LIFO
        // RECYCLING of a free list; there is no free list now, because the framebuffer address
        // IS the file offset. What matters instead is that the offset EQUALS the address, and
        // that two addresses never share a page.
        let mut a = arena.alloc_at(0).expect("a");
        let mut b = arena.alloc_at(ARENA_PAGE).expect("b");
        assert_eq!(a.file_offset(), 0, "the address is the offset");
        assert_eq!(b.file_offset(), ARENA_PAGE, "the address is the offset");
        assert_ne!(a.file_offset(), b.file_offset());
        a.write_from(0, &[1, 2, 3, 4]).unwrap();
        b.write_from(0, &[9, 9, 9, 9]).unwrap();
        let mut buf = [0u8; 4];
        a.read_into(0, &mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3, 4]);
        // Out of the page, never into the neighbour.
        assert!(a.read_into(ARENA_PAGE - 2, &mut buf).is_err());
        // ★★★★★ **IDEMPOTENT, which the old allocator could not be.** Asking twice for one
        // framebuffer address must give the same file offset — otherwise one framebuffer byte
        // would have two homes, and nothing would keep them equal.
        drop(a);
        let again = arena.alloc_at(0).expect("again");
        assert_eq!(again.file_offset(), 0, "the same address is the same offset");
        // ⊘ And the bytes survived, because it is the same page of the same file.
        let mut buf = [0u8; 4];
        again.read_into(0, &mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3, 4], "re-asking returned the SAME page, not a fresh one");
        // ⊘ Refusals are about an ADDRESS now, not about a supply.
        assert!(arena.alloc_at(1).is_err(), "a misaligned address is refused, never rounded");
        assert!(
            arena.alloc_at(SharedPageArena::LEN).is_err(),
            "an address past the arena's extent is refused"
        );
    }

    #[test]
    fn a_second_mapping_of_the_same_offset_is_the_same_memory() {
        let arena = SharedPageArena::create(HostPageSize::query()).expect("arena");
        let mut p = arena.alloc_at(3 * ARENA_PAGE).expect("p");
        p.write_from(16, &[0xAB; 8]).unwrap();
        let second = MappedRegion::map(
            Backing::SharedFile {
                fd: arena.as_backing_fd(),
                offset: p.file_offset(),
            },
            ARENA_PAGE,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            HostPageSize::query(),
        )
        .expect("second mapping");
        let mut buf = [0u8; 8];
        second.read_into(HostOffset::new(16), &mut buf).unwrap();
        assert_eq!(buf, [0xAB; 8]);
        second.write_from(HostOffset::new(32), &[0xCD; 4]).unwrap();
        let mut back = [0u8; 4];
        p.read_into(32, &mut back).unwrap();
        assert_eq!(back, [0xCD; 4], "one memory, two names");
    }
}
