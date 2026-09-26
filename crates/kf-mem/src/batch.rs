//! ★★★ **The handles behind our batched placements** — `V3_BATCHED_MAP.md` §5.
//!
//! A batched map (`kf_host::HostRm::map_scattered`) places N VA-contiguous guest-RAM runs through
//! ONE host object of our own. The walker still commits — and later unmaps — those runs one by one
//! (or in ranges), so the object must outlive every piece of it and be freed exactly when its last
//! piece is gone: freeing it earlier would make RM unmap every piece still in use
//! (`ogkm-580 rs_client.c:1342-1395`), never freeing it pins guest pages for the VM's life.
//!
//! This is a ledger of OUR OWN host handles (the kind `V3_BUILD.md`'s 2026-09-25 amendment
//! sanctions), never a copy of the guest's tables: nothing resolves through it and nothing about
//! a translation is read from it. It answers one question — *"which of our objects are now wholly
//! unmapped?"* — and it answers it from what WE unmapped, page by page, so an unmap repeated or
//! overlapping two objects cannot double-count.
//!
//! ⊘ Pure data, no host calls: the caller frees what [`BatchBook::unmapped`] returns, outside any
//! lock (THE_CONSTRAINTS "no blocking under a lock others block on").

use crate::ledger::{Desired, HostVas, MapTarget};
use std::collections::BTreeMap;

/// The granule the book tracks liveness in — the smallest GMMU page of every family, and the unit
/// every guest-RAM row is whole multiples of (`crate::apply` refuses a sub-page row).
pub const BATCH_PAGE: u64 = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    handle: u32,
    len: u64,
    /// One bit per [`BATCH_PAGE`] still mapped.
    live: Vec<u64>,
    live_pages: u64,
}

/// Our batch objects in one host VA space, by `(VA their mapping starts at, handle)` — two can
/// start at one VA (a later batch placed over an earlier one's dead pages while other pages of the
/// earlier one are still live).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchBook {
    by_va: BTreeMap<(u64, u32), Entry>,
    /// The longest entry ever recorded — bounds the backward scan in [`BatchBook::unmapped`].
    max_len: u64,
}

/// Why [`BatchBook::insert`] refused to record a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookRefusal {
    /// Empty, not whole [`BATCH_PAGE`]s, or overflowing.
    Shape,
    /// That handle is already tracked at that VA.
    Occupied,
}

impl BatchBook {
    /// Record a batch object `handle` mapped at `[va, va+len)`, every page live.
    ///
    /// # Errors
    /// [`BookRefusal`] — the caller must then free `handle` itself (it cannot be tracked).
    pub fn insert(&mut self, va: u64, len: u64, handle: u32) -> Result<(), BookRefusal> {
        if len == 0 || !(va | len).is_multiple_of(BATCH_PAGE) || va.checked_add(len).is_none() {
            return Err(BookRefusal::Shape);
        }
        if self.by_va.contains_key(&(va, handle)) {
            return Err(BookRefusal::Occupied);
        }
        let pages = len / BATCH_PAGE;
        let words = usize::try_from(pages.div_ceil(64)).map_err(|_| BookRefusal::Shape)?;
        let mut live = vec![u64::MAX; words];
        if !pages.is_multiple_of(64)
            && let Some(last) = live.last_mut()
        {
            *last = (1u64 << (pages % 64)) - 1;
        }
        self.by_va.insert((va, handle), Entry { handle, len, live, live_pages: pages });
        self.max_len = self.max_len.max(len);
        Ok(())
    }

    /// `[va, va+len)` is no longer mapped by us: clear those pages in every batch they touch, and
    /// return (and forget) the handles with no live page left — the caller frees them.
    #[must_use]
    pub fn unmapped(&mut self, va: u64, len: u64) -> Vec<u32> {
        let end = va.saturating_add(len);
        let floor = va.saturating_sub(self.max_len);
        let mut emptied = Vec::new();
        for (&(start, h), e) in self.by_va.range_mut((floor, 0)..(end, 0)) {
            let e_end = start + e.len;
            if e_end <= va {
                continue;
            }
            let lo = (va.max(start) - start) / BATCH_PAGE;
            let hi = (end.min(e_end) - start).div_ceil(BATCH_PAGE);
            for p in lo..hi {
                let (w, b) = ((p / 64) as usize, p % 64);
                if e.live[w] & (1 << b) != 0 {
                    e.live[w] &= !(1 << b);
                    e.live_pages -= 1;
                }
            }
            if e.live_pages == 0 {
                emptied.push((start, h));
            }
        }
        emptied.into_iter().filter_map(|s| self.by_va.remove(&s)).map(|e| e.handle).collect()
    }

    /// Whether a batch of ours has a live page at `va` — the caller must then unmap by RANGE (a
    /// whole-mapping unmap keyed by `va` would take the rest of that batch with it).
    #[must_use]
    pub fn covers(&self, va: u64) -> bool {
        let floor = va.saturating_sub(self.max_len);
        self.by_va.range((floor, 0)..=(va, u32::MAX)).any(|(&(start, _), e)| {
            let p = (va - start) / BATCH_PAGE;
            va < start + e.len && e.live[(p / 64) as usize] & (1 << (p % 64)) != 0
        })
    }

    /// Forget every batch and return every handle (the space is being retired).
    #[must_use]
    pub fn drain(&mut self) -> Vec<u32> {
        self.max_len = 0;
        core::mem::take(&mut self.by_va).into_values().map(|e| e.handle).collect()
    }

    /// Batches tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_va.len()
    }

    /// Whether none is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_va.is_empty()
    }
}

/// ★★★ **A host VA space that places batches and keeps their objects' books** — the one
/// implementation production (`kf_qemu::mem::GpuMirror`) and the hardware gate share.
///
/// Every verb here is one WE author on OUR host space (§9); the book's lock is never held across a
/// host call.
#[derive(Debug)]
pub struct BatchedVas<'rm> {
    /// The host space.
    pub vas: HostVas<'rm>,
    /// Our batch objects in it.
    pub book: std::sync::Mutex<BatchBook>,
    /// Batch objects freed (a counter for the instruments).
    pub frees: std::sync::atomic::AtomicU64,
}

impl<'rm> BatchedVas<'rm> {
    /// Batches over `vas`, none yet.
    #[must_use]
    pub fn new(vas: HostVas<'rm>) -> Self {
        BatchedVas { vas, book: std::sync::Mutex::new(BatchBook::default()), frees: std::sync::atomic::AtomicU64::new(0) }
    }

    /// ★ Place VA-contiguous guest-RAM `rows` as ONE batch stitched from `ram_fd` and book its
    /// object. `Ok` ⇔ every row is our mapping; `Err` ⇔ none is (an object the book cannot
    /// account for is freed — which unmaps it — before the refusal returns).
    ///
    /// # Errors
    /// The host's refusal or the book's, by name.
    pub fn place(&self, ram_fd: std::os::fd::BorrowedFd<'_>, rows: &[Desired], defer: bool) -> Result<(), String> {
        let (va, len) = match (rows.first(), rows.last()) {
            (Some(a), Some(b)) => (a.va, (b.va + b.len).checked_sub(a.va).ok_or("batch rows out of order")?),
            _ => return Err("empty batch".into()),
        };
        let handle = self.vas.map_scattered(ram_fd, rows, defer)?;
        let booked = self.book.lock().map_err(|_| "batch book poisoned".to_string()).and_then(|mut b| b.insert(va, len, handle).map_err(|e| format!("{e:?}")));
        if let Err(e) = booked {
            self.free(vec![handle]);
            return Err(format!("batch {va:#x}+{len:#x}: its object could not be booked ({e}) — freed, nothing placed"));
        }
        Ok(())
    }

    /// ★ Unmap every mapping of ours in `[va, va+len)` (ONE host call), then free the batch
    /// objects that left wholly unmapped.
    ///
    /// # Errors
    /// The host's refusal (the book is untouched: what is gone is unknown until the per-run retry).
    pub fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        self.vas.unmap_range(va, len, defer)?;
        self.retired(va, len);
        Ok(())
    }

    /// ★ Unmap ONE committed run at `va` (`len` when known). A run inside a live batch is a PIECE
    /// of a larger host mapping, so it goes by range — the whole-mapping verb keyed by `va` would
    /// take the rest of the batch with it (or find nothing); anything else keeps that verb.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unmap_run(&self, va: u64, len: Option<u64>, defer: bool) -> Result<(), String> {
        let in_batch = self.book.lock().is_ok_and(|b| b.covers(va));
        match (len, in_batch) {
            (Some(len), true) => self.unmap_range(va, len, defer),
            (Some(len), false) => {
                self.vas.unmap(va, defer)?;
                self.retired(va, len);
                Ok(())
            }
            (None, true) => Err(format!("unmap {va:#x}: a piece of a batch with no known length — refused (a whole-mapping unmap would take the batch)")),
            (None, false) => self.vas.unmap(va, defer),
        }
    }

    /// Free every batch object left (the space is being retired). Returns how many.
    pub fn drain(&self) -> usize {
        let rest = self.book.lock().map(|mut b| b.drain()).unwrap_or_default();
        let n = rest.len();
        self.free(rest);
        n
    }

    fn retired(&self, va: u64, len: u64) {
        let emptied = self.book.lock().map(|mut b| b.unmapped(va, len)).unwrap_or_default();
        self.free(emptied);
    }

    fn free(&self, handles: Vec<u32>) {
        for h in handles {
            match self.vas.rm.free(h) {
                Ok(()) => {
                    self.frees.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                // ⊘ Not fatal to the unmap that emptied it (its mappings ARE gone); the object and
                // its pinned pages live until the host client closes — named.
                Err(e) => eprintln!("kf-mem: batch object {h:#x} free refused: {e:?} — its pages stay pinned until the host client closes"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: u64 = BATCH_PAGE;

    #[test]
    fn a_batch_is_freed_exactly_when_its_last_page_goes() {
        let mut b = BatchBook::default();
        b.insert(0x10_0000, 4 * P, 7).unwrap();
        assert!(b.unmapped(0x10_0000 + P, P).is_empty(), "one page gone, three live");
        assert!(b.unmapped(0x10_0000 + P, P).is_empty(), "a repeated unmap does not double-count");
        assert!(b.unmapped(0x10_0000, P).is_empty());
        assert!(b.unmapped(0x10_0000 + 3 * P, P).is_empty());
        assert_eq!(b.unmapped(0x10_0000 + 2 * P, P), vec![7]);
        assert!(b.is_empty());
    }

    #[test]
    fn one_range_can_empty_several_batches_and_spare_its_neighbours() {
        let mut b = BatchBook::default();
        b.insert(0x1000_0000, 2 * P, 1).unwrap();
        b.insert(0x1000_0000 + 2 * P, 3 * P, 2).unwrap();
        b.insert(0x1000_0000 + 5 * P, 2 * P, 3).unwrap();
        let mut got = b.unmapped(0x1000_0000, 5 * P);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
        assert_eq!(b.len(), 1, "the neighbour past the range is untouched");
        // A range that only grazes it (ends at its start) leaves it live.
        assert!(b.unmapped(0x1000_0000 + 4 * P, P).is_empty());
        assert_eq!(b.unmapped(0x1000_0000 + 5 * P, 64 * P), vec![3]);
    }

    #[test]
    fn a_later_batch_over_an_earlier_ones_dead_pages_keeps_both_accounts() {
        let mut b = BatchBook::default();
        b.insert(0x20_0000, 4 * P, 1).unwrap();
        assert!(b.unmapped(0x20_0000 + P, 2 * P).is_empty());
        // Pages 1-2 of batch 1 are dead; batch 2 is placed over them.
        b.insert(0x20_0000 + P, 2 * P, 2).unwrap();
        assert_eq!(b.unmapped(0x20_0000 + P, 2 * P), vec![2], "batch 1's pages there were already dead");
        let mut got = b.unmapped(0x20_0000, 4 * P);
        got.sort_unstable();
        assert_eq!(got, vec![1]);
    }

    #[test]
    fn page_counts_that_are_not_a_multiple_of_64_track_exactly() {
        let mut b = BatchBook::default();
        b.insert(0, 65 * P, 9).unwrap();
        assert!(b.unmapped(0, 64 * P).is_empty(), "page 64 is still live");
        assert_eq!(b.unmapped(64 * P, P), vec![9]);
        b.insert(0, 3 * P, 4).unwrap();
        assert_eq!(b.drain(), vec![4]);
    }

    #[test]
    fn shapes_it_cannot_account_for_are_refused() {
        let mut b = BatchBook::default();
        assert_eq!(b.insert(0x1000, 0, 1), Err(BookRefusal::Shape));
        assert_eq!(b.insert(0x1001, P, 1), Err(BookRefusal::Shape));
        assert_eq!(b.insert(u64::MAX - P + 1, 2 * P, 1), Err(BookRefusal::Shape));
        b.insert(0x1000, P, 1).unwrap();
        assert_eq!(b.insert(0x1000, P, 1), Err(BookRefusal::Occupied));
        b.insert(0x1000, P, 2).unwrap();
        assert_eq!(b.len(), 2, "two handles may start at one VA");
    }

    #[test]
    fn covers_answers_for_live_pages_only() {
        let mut b = BatchBook::default();
        b.insert(0x40_0000, 3 * P, 1).unwrap();
        assert!(b.covers(0x40_0000) && b.covers(0x40_0000 + 2 * P + 5));
        assert!(!b.covers(0x40_0000 + 3 * P) && !b.covers(0x40_0000 - 1));
        assert!(b.unmapped(0x40_0000 + P, P).is_empty());
        assert!(!b.covers(0x40_0000 + P), "a dead page is not covered");
    }
}
