//! Bounded unique missing coverage. Overflow is an explicit conservative upper bound.
use std::collections::BTreeMap;
const INTERVALS: usize = 256;
#[derive(Debug, Default)]
pub(super) struct Deficit {
    spans: BTreeMap<u64, u64>,
    pub(super) bytes: u64,
    pub(super) capped: bool,
    next_warning_ns: u64,
}
impl Deficit {
    pub(super) fn mark(&mut self, lo: u64, hi: u64, missing: bool, ceiling: u64) {
        if lo >= hi {
            return;
        }
        if self.capped {
            if !missing && lo <= super::super::channel::TWIN_VA_FLOOR && hi >= ceiling {
                let next_warning_ns = self.next_warning_ns;
                *self = Self {
                    next_warning_ns,
                    ..Self::default()
                };
            }
            return;
        }
        let start = self.spans.range(..=lo).next_back().map_or(lo, |(&s, _)| s);
        let keys: Vec<_> = self
            .spans
            .range(start..=hi)
            .filter(|&(&s, &e)| if missing { lo <= e } else { lo < e && s < hi })
            .map(|(&s, &e)| (s, e))
            .collect();
        let (mut a, mut b) = (lo, hi);
        for (s, e) in keys {
            self.spans.remove(&s);
            self.bytes -= e - s;
            if missing {
                a = a.min(s);
                b = b.max(e);
            } else {
                if s < lo {
                    self.insert(s, lo);
                }
                if hi < e {
                    self.insert(hi, e);
                }
            }
        }
        if missing {
            self.insert(a, b);
        }
        if self.spans.len() > INTERVALS {
            self.spans.clear();
            self.capped = true;
            self.bytes = ceiling.saturating_sub(super::super::channel::TWIN_VA_FLOOR);
        }
    }
    fn insert(&mut self, lo: u64, hi: u64) {
        self.spans.insert(lo, hi);
        self.bytes = self.bytes.saturating_add(hi - lo);
    }
    pub(super) fn warning(&mut self, now_ns: u64, ceiling: u64) -> Option<String> {
        if self.bytes < 1 << 20 || now_ns < self.next_warning_ns {
            return None;
        }
        self.next_warning_ns = now_ns.saturating_add(30_000_000_000);
        let ranges: Vec<_> = if self.capped {
            vec![(super::super::channel::TWIN_VA_FLOOR, ceiling)]
        } else {
            self.spans.iter().take(4).map(|(&a, &b)| (a, b)).collect()
        };
        Some(format!(
            "current_unhonoured_bytes={} capped={} ranges={ranges:x?}",
            self.bytes, self.capped
        ))
    }
}

pub(crate) fn adjust_gauge(counter: &std::sync::atomic::AtomicU64, before: u64, after: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    let _ = counter.try_update(Relaxed, Relaxed, |n| {
        Some(n.saturating_sub(before).saturating_add(after))
    });
}
