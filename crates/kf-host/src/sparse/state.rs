//! Fixed-budget occupancy and invalidation metadata. Never an address authority.
use std::collections::{BTreeMap, VecDeque};
const RANGES: usize = 512;
const NEGATIVES: usize = 256;

#[derive(Clone, Debug, Default)]
pub(super) struct Live {
    spans: Vec<(u64, u64)>,
    uncertain: bool,
}
impl Live {
    pub(super) fn any(&self) -> bool {
        self.uncertain || !self.spans.is_empty()
    }
    // Overflow conservatively pins this block until a whole-block unmap. Never lose
    // occupancy just because tracking filled. At most512 intervals per block.
    pub(super) fn mark(&mut self, lo: u64, hi: u64, start: u64, end: u64, set: bool) -> bool {
        let (mut a, mut b) = (start.max(lo), end.min(hi));
        if a >= b {
            return false;
        }
        if !set && a == lo && b == hi {
            self.spans.clear();
            self.uncertain = false;
            return false;
        }
        if self.uncertain {
            return false;
        }
        let mut next = Vec::with_capacity(self.spans.len().saturating_add(1).min(RANGES + 1));
        for &(s, e) in &self.spans {
            if set {
                if e < a || b < s {
                    next.push((s, e));
                } else {
                    a = a.min(s);
                    b = b.max(e);
                }
            } else if e <= a || b <= s {
                next.push((s, e));
            } else {
                if s < a {
                    next.push((s, a));
                }
                if b < e {
                    next.push((b, e));
                }
            }
        }
        if set {
            next.push((a, b));
        }
        if next.len() > RANGES {
            self.spans.clear();
            self.uncertain = true;
            return true;
        }
        next.sort_unstable();
        self.spans = next;
        false
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Negative {
    pub(super) lo: u64,
    pub(super) hi: u64,
    epoch: u64,
    blocks: usize,
    pub(super) overflow: bool,
    pub(super) retry: bool,
    /// Retry tick from which a decision taken on a host RM refusal is asked again (0 = none).
    due: u64,
}
#[derive(Debug, Default)]
pub(super) struct Cache {
    pub(super) entries: BTreeMap<(u64, u64), Negative>,
    changes: VecDeque<(u64, u64, u64)>,
    epoch: u64,
}
impl Cache {
    pub(super) fn touch(&mut self, lo: u64, hi: u64) {
        if self.epoch == u64::MAX {
            self.entries.clear();
            self.changes.clear();
            self.epoch = 0;
        }
        self.epoch += 1;
        if self.changes.len() == NEGATIVES {
            self.changes.pop_front();
        }
        self.changes.push_back((self.epoch, lo, hi));
    }
    pub(super) fn valid(&self, n: Negative, blocks: usize, tick: u64) -> bool {
        n.blocks == blocks
            && (n.due == 0 || tick < n.due)
            && self.changes.front().is_none_or(|c| n.epoch >= c.0 - 1)
            && !self
                .changes
                .iter()
                .any(|&(epoch, lo, hi)| epoch > n.epoch && lo < n.hi && n.lo < hi)
    }
    pub(super) fn get(&self, key: (u64, u64), blocks: usize, tick: u64) -> Option<Negative> {
        self.entries
            .get(&key)
            .copied()
            .filter(|&n| self.valid(n, blocks, tick))
    }
    pub(super) fn insert(
        &mut self,
        key: (u64, u64),
        lo: u64,
        hi: u64,
        blocks: usize,
        overflow: bool,
        retry: bool,
        due: u64,
    ) -> bool {
        let evicted = self.entries.len() >= NEGATIVES && !self.entries.contains_key(&key);
        if evicted {
            self.entries.pop_first();
        }
        self.entries.insert(
            key,
            Negative {
                lo,
                hi,
                epoch: self.epoch,
                blocks,
                overflow,
                retry,
                due,
            },
        );
        evicted
    }
    pub(super) fn accept(&mut self, key: (u64, u64)) {
        if let Some(n) = self.entries.get_mut(&key) {
            n.retry = true;
        }
    }
    /// Force the next admission of `key` to re-run, keeping the remembered intent.
    pub(super) fn expire(&mut self, key: (u64, u64)) {
        if let Some(n) = self.entries.get_mut(&key) {
            n.blocks = usize::MAX;
        }
    }
    pub(super) fn capacity_changed(&mut self) {
        for n in self.entries.values_mut().filter(|n| n.overflow) {
            // Invalidate the cached decision, not the accepted pending intent.
            n.blocks = usize::MAX;
        }
    }
    pub(super) fn retire(&mut self, lo: u64, hi: u64) {
        let overlap: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|&(s, len)| s < hi && lo < s.saturating_add(len))
            .collect();
        for key @ (start, len) in overlap {
            let Some(n) = self.entries.remove(&key) else {
                continue;
            };
            let end = start.saturating_add(len);
            if start < lo {
                self.insert(
                    (start, lo - start),
                    n.lo,
                    n.hi,
                    n.blocks,
                    n.overflow,
                    n.retry,
                    n.due,
                );
            }
            if hi < end {
                self.insert(
                    (hi, end - hi),
                    n.lo,
                    n.hi,
                    n.blocks,
                    n.overflow,
                    n.retry,
                    n.due,
                );
            }
        }
        self.touch(lo, hi);
    }
}
