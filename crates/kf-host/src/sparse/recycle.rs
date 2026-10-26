//! Recycle only empty, intent-free normal geometry. Unchanged sparse VAs never lose coverage.
use super::*;
impl Reservations {
    fn recyclable(b: &Block) -> bool {
        b.handle != 0 && !b.sparse && !b.live.any() && b.wanted.iter().all(|&v| v == 0)
    }
    fn pair(&mut self, key: u64) {
        self.mergeable.remove(&key);
        let Some(b) = self.blocks.get(&key) else {
            return;
        };
        let Some((_, next)) = self
            .blocks
            .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
            .next()
        else {
            return;
        };
        if b.hi == next.lo && Self::recyclable(b) && Self::recyclable(next) {
            self.mergeable.insert(key);
        }
    }
    pub(super) fn reindex(&mut self, lo: u64, hi: u64) {
        let first = self.key(lo).unwrap_or(lo);
        let mut keys: Vec<_> = self.blocks.range(first..hi).map(|(&k, _)| k).collect();
        if let Some((&k, _)) = self.blocks.range(..first).next_back() {
            keys.push(k);
        }
        for key in keys {
            self.pair(key);
        }
    }
    // At most4 pairs (12 RM verbs without repair), even when the guest filled every slot.
    // Both objects are empty/normal; faults stay faults while the placement lock excludes
    // unrelated allocations. An RM failure restores both owners or poisons placement.
    pub(super) fn recycle(&mut self, ops: &dyn Ops, spare: usize) -> Result<(), RmError> {
        for _ in 0..4 {
            if self.blocks.len().saturating_add(spare) <= self.limit {
                break;
            }
            let Some(key) = self.mergeable.pop_first() else {
                break;
            };
            let Some(a) = self.blocks.get(&key).cloned() else {
                continue;
            };
            let Some(b) = self
                .blocks
                .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                .next()
                .map(|(_, b)| b.clone())
            else {
                continue;
            };
            if a.hi != b.lo || !Self::recyclable(&a) || !Self::recyclable(&b) {
                continue;
            }
            let originals = [a, b];
            let mut released = 0;
            let mut error = None;
            for old in &originals {
                match ops.release(old.handle) {
                    Ok(()) => {
                        released += 1;
                        self.stats.freed = self.stats.freed.saturating_add(1);
                    }
                    Err(e) => {
                        self.poisoned = true;
                        error = Some(e);
                        break;
                    }
                }
            }
            let lo = originals[0].lo;
            let hi = originals[1].hi;
            let result = if let Some(e) = error {
                Err(e)
            } else {
                ops.reserve(lo, hi - lo, false)
            };
            match result {
                Ok(handle) => {
                    self.stats.made = self.stats.made.saturating_add(1);
                    for old in &originals {
                        self.blocks.remove(&old.lo);
                        self.retry_dirty.remove(&old.lo);
                        self.mergeable.remove(&old.lo);
                    }
                    self.blocks.insert(
                        lo,
                        Block {
                            lo,
                            hi,
                            handle,
                            sparse: false,
                            live: Live::default(),
                            wanted: [0; 8],
                        },
                    );
                    self.cache.touch(lo, hi);
                    self.reindex(lo, hi);
                }
                Err(e) => {
                    self.stats.refused = self.stats.refused.saturating_add(1);
                    for mut old in originals.into_iter().take(released) {
                        old.handle = match ops.reserve(old.lo, old.hi - old.lo, false) {
                            Ok(h) => {
                                self.stats.made = self.stats.made.saturating_add(1);
                                h
                            }
                            Err(_) => {
                                self.poisoned = true;
                                0
                            }
                        };
                        self.blocks.insert(old.lo, old);
                    }
                    self.cache.touch(lo, hi);
                    self.reindex(lo, hi);
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}
