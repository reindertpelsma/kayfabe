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
use std::os::fd::BorrowedFd;
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
    list: Vec<u64>,
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
    pub const LEN: u64 = 1 << 30;

    /// Create the `memfd`, seal it, and map it once.
    ///
    /// # Errors
    /// [`RawError`] from `memfd_create`, `ftruncate`, sealing or `mmap`; and
    /// [`RawError::Misaligned`] if [`ARENA_PAGE`] is not a whole number of host pages, which
    /// is the one geometry under which a page here could not be memslotted.
    ///
    /// # Panics
    /// If called with any ranked lock held (R1, §4.5).
    pub fn create(page: HostPageSize) -> Result<Self, RawError> {
        lockwitness::assert_lock_free("memfd_create + mmap (the framebuffer page arena)");
        crate::geometry::require_aligned(ARENA_PAGE, page, "arena page")?;
        let file = SharedRam::create_named(ARENA_NAME, Self::LEN)?;
        let map = MappedRegion::map(
            Backing::SharedFile {
                fd: file.as_backing_fd(),
                offset: 0,
            },
            Self::LEN,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )?;
        Ok(SharedPageArena {
            inner: Arc::new(ArenaInner {
                file,
                map,
                pages: Self::LEN / ARENA_PAGE,
                free: Mutex::new(ArenaFree {
                    next: 0,
                    list: Vec::new(),
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
    pub fn alloc(&self) -> Result<ArenaPage, &'static str> {
        let mut f = self.inner.free.lock().unwrap_or_else(|e| e.into_inner());
        let index = if let Some(i) = f.list.pop() {
            f.recycled += 1;
            i
        } else if f.next < self.inner.pages {
            let i = f.next;
            f.next += 1;
            i
        } else {
            return Err(ARENA_EXHAUSTED);
        };
        f.live += 1;
        f.peak = f.peak.max(f.live);
        Ok(ArenaPage {
            arena: Arc::clone(&self.inner),
            index,
        })
    }

    /// The descriptor a memslot placement maps. Borrowed from the arena, which outlives
    /// every page and every placement (the placement's VMA holds the file regardless).
    #[must_use]
    pub fn as_backing_fd(&self) -> BorrowedFd<'_> {
        self.inner.file.as_backing_fd()
    }

    /// `(live, peak, recycled, issued)` — the allocator's census.
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
        // ★ The index goes back to the free list AFTER this handle can no longer be used to
        // address it — which is now, by the ownership the allocator hands out. No syscall,
        // no ranked lock: a `Vec::push` under the same mutex `alloc` takes.
        let mut f = self.arena.free.lock().unwrap_or_else(|e| e.into_inner());
        f.list.push(self.index);
        f.live = f.live.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_disjoint_and_recycled() {
        let arena = SharedPageArena::create(HostPageSize::query()).expect("arena");
        let mut a = arena.alloc().expect("a");
        let mut b = arena.alloc().expect("b");
        assert_ne!(a.file_offset(), b.file_offset());
        a.write_from(0, &[1, 2, 3, 4]).unwrap();
        b.write_from(0, &[9, 9, 9, 9]).unwrap();
        let mut buf = [0u8; 4];
        a.read_into(0, &mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3, 4]);
        // Out of the page, never into the neighbour.
        assert!(a.read_into(ARENA_PAGE - 2, &mut buf).is_err());
        let ia = a.index();
        drop(a);
        let c = arena.alloc().expect("c");
        assert_eq!(c.index(), ia, "LIFO recycling of the freed index");
        let (live, peak, recycled, issued) = arena.census();
        assert_eq!((live, peak, recycled, issued), (2, 2, 1, 2));
    }

    #[test]
    fn a_second_mapping_of_the_same_offset_is_the_same_memory() {
        let arena = SharedPageArena::create(HostPageSize::query()).expect("arena");
        let mut p = arena.alloc().expect("p");
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
