//! Atomic bounded journals: writers (including a vCPU) never wait, allocate or lock.
use std::sync::atomic::{
    AtomicU64,
    Ordering::{AcqRel, Acquire, Relaxed, Release},
};
pub(super) const SLOTS: usize = 256;
#[derive(Debug, Clone, Copy)]
pub(super) struct Event {
    pub seq: u64,
    pub words: [u64; 7],
}
struct Slot {
    version: AtomicU64,
    words: [AtomicU64; 7],
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            version: AtomicU64::new(0),
            words: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
pub(super) struct Journal {
    pub next: AtomicU64,
    pub lost: AtomicU64,
    slots: [Slot; SLOTS],
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(0),
            lost: AtomicU64::new(0),
            slots: std::array::from_fn(|_| Slot::default()),
        }
    }
}
impl Journal {
    pub fn push(&self, words: [u64; 7]) {
        let seq = self.next.fetch_add(1, Relaxed) + 1;
        let slot = &self.slots[seq as usize % SLOTS];
        let before = slot.version.load(Acquire);
        if before % 2 != 0
            || before / 2 >= seq
            || slot
                .version
                .compare_exchange(before, seq * 2 - 1, AcqRel, Relaxed)
                .is_err()
        {
            self.lost.fetch_add(1, Relaxed);
            return;
        }
        for (a, v) in slot.words.iter().zip(words) {
            a.store(v, Relaxed);
        }
        slot.version.store(seq * 2, Release);
    }
    pub fn read(&self) -> Vec<Event> {
        let mut out = Vec::with_capacity(SLOTS);
        for slot in &self.slots {
            let before = slot.version.load(Acquire);
            if before == 0 || before % 2 != 0 {
                continue;
            }
            let words = std::array::from_fn(|i| slot.words[i].load(Relaxed));
            if slot.version.load(Acquire) == before {
                out.push(Event {
                    seq: before / 2,
                    words,
                });
            }
        }
        out.sort_by_key(|e| e.seq);
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ens11_atomic_journal_sequence_exhaustion_is_terminal() {
        let j = Journal::default();
        j.next.store(u64::MAX, Relaxed);
        j.push([7; 7]);
        assert_eq!(j.next.load(Relaxed), u64::MAX);
        assert_eq!(j.lost.load(Relaxed), 1);
        assert!(j.read().is_empty());
    }
    #[test]
    fn ens11_atomic_journal_version_encoding_never_wraps_into_an_old_sequence() {
        let j = Journal::default();
        j.next.store(u64::MAX / 2, Relaxed);
        j.push([8; 7]);
        assert_eq!(j.lost.load(Relaxed), 1);
        assert!(j.read().is_empty());
    }
    #[test]
    fn ens11_atomic_irq_journal_retains_exact_last_capacity_without_allocation_by_writers() {
        let j = Journal::default();
        for n in 0..1000 {
            j.push([n; 7]);
        }
        let r = j.read();
        assert_eq!(r.len(), SLOTS);
        assert_eq!(r[0].words, [744; 7]);
        assert_eq!(r.last().unwrap().words, [999; 7]);
    }
    #[test]
    fn ens11_atomic_irq_journal_never_exposes_torn_concurrent_records() {
        let j = std::sync::Arc::new(Journal::default());
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let j = j.clone();
                scope.spawn(move || {
                    for n in 0..5000 {
                        j.push([n; 7]);
                    }
                });
            }
            for _ in 0..100 {
                for e in j.read() {
                    assert!(e.words.iter().all(|v| *v == e.words[0]));
                }
            }
        });
        assert!(j.read().len() <= SLOTS);
    }
}
