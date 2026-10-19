//! Bounded, append-only interrupt diagnostic. Default unarmed. Producers make ONE
//! reservation attempt; contention/full capacity drops diagnostic data and never
//! changes delivery. No allocation, lock, callback, syscall, clock or retry loop.
//! Snapshots are sequences of atomic reads, NOT a consistent multiword snapshot.
//! Concurrent overlapping records must not be treated as a total hardware order.
use crate::cpuintr::{CpuIntr, Raise, Reg};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Pending[0..16], leaf enables[16..32], top enable[32]. Unused leaves are zero.
pub const WORDS: usize = 33;
/// One observed operation. A latch may run on any device thread, not just a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Guest interrupt register write.
    Write {
        /// BAR0 register offset.
        off: u64,
        /// Exact written value.
        value: u32,
    },
    /// Device-announced completion or interrupt.
    Latch(u32),
}
struct Slot {
    ready: AtomicBool,
    off: AtomicU64,
    value: AtomicU32,
    before: [AtomicU32; WORDS],
    after: [AtomicU32; WORDS],
    raise: AtomicU32,
    end: AtomicUsize,
}
impl Slot {
    fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            off: AtomicU64::new(0),
            value: AtomicU32::new(0),
            before: core::array::from_fn(|_| AtomicU32::new(0)),
            after: core::array::from_fn(|_| AtomicU32::new(0)),
            raise: AtomicU32::new(0),
            end: AtomicUsize::new(0),
        }
    }
}
/// A reserved record. Private fields prevent guest-authored record identities.
pub struct Ticket {
    index: usize,
    event: Event,
    before: [u32; WORDS],
    leaves: usize,
}
/// A committed diagnostic sample, copied by the consumer without locks.
#[derive(Debug)]
pub struct Record {
    /// Original operation.
    pub event: Event,
    /// Atomic observations before apply (not a multiword transaction).
    pub before: [u32; WORDS],
    /// Atomic observations after apply (not a multiword transaction).
    pub after: [u32; WORDS],
    /// Actual apply/latch result, rather than inferred subscription arming.
    pub raise: Raise,
    /// Completion reservation order, to expose overlapping operations.
    pub end: usize,
}
/// Fixed-capacity diagnostic, allocated at realize by its owner, never on apply.
pub struct Audit<const N: usize> {
    armed: AtomicBool,
    next: AtomicUsize,
    ends: AtomicUsize,
    full: AtomicU64,
    contended: AtomicU64,
    slots: [Slot; N],
}
fn sample(tree: &CpuIntr, leaves: usize) -> [u32; WORDS] {
    core::array::from_fn(|i| match i {
        0..16 if i < leaves => tree.read(Reg::Leaf(i)),
        16..32 if i - 16 < leaves => tree.read(Reg::LeafEnSet(i - 16)),
        32 => tree.read(Reg::TopEnSet),
        _ => 0,
    })
}
impl<const N: usize> Default for Audit<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> Audit<N> {
    /// Unarmed, empty, fixed storage.
    #[must_use]
    pub fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            next: AtomicUsize::new(0),
            ends: AtomicUsize::new(0),
            full: AtomicU64::new(0),
            contended: AtomicU64::new(0),
            slots: core::array::from_fn(|_| Slot::new()),
        }
    }
    /// One-shot arm. No reset/reuse: old tickets cannot overwrite a new capture.
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }
    /// Stop new reservations; existing tickets can finish. Consumer never waits.
    pub fn stop(&self) {
        self.armed.store(false, Ordering::Release);
    }
    /// One bounded attempt. A contending producer drops only its diagnostic sample.
    #[must_use]
    pub fn begin(&self, tree: &CpuIntr, leaves: usize, event: Event) -> Option<Ticket> {
        if !self.armed.load(Ordering::Acquire) {
            return None;
        }
        let index = self.next.load(Ordering::Relaxed);
        if index >= N {
            self.full.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        if self
            .next
            .compare_exchange(index, index + 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            self.contended.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(Ticket {
            index,
            event,
            before: sample(tree, leaves.min(16)),
            leaves: leaves.min(16),
        })
    }
    /// Commit after apply. Caller uses the SAME Audit that issued the ticket.
    pub fn finish(&self, ticket: Ticket, tree: &CpuIntr, raise: Raise) {
        let slot = &self.slots[ticket.index];
        let (off, value) = match ticket.event {
            Event::Write { off, value } => (off, value),
            Event::Latch(v) => (u64::MAX, v),
        };
        slot.off.store(off, Ordering::Relaxed);
        slot.value.store(value, Ordering::Relaxed);
        for (dst, v) in slot.before.iter().zip(ticket.before) {
            dst.store(v, Ordering::Relaxed);
        }
        for (dst, v) in slot.after.iter().zip(sample(tree, ticket.leaves)) {
            dst.store(v, Ordering::Relaxed);
        }
        slot.raise.store(
            match raise {
                Raise::None => 0,
                Raise::Message => 1,
                Raise::OutOfRange => 2,
            },
            Ordering::Relaxed,
        );
        slot.end
            .store(self.ends.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        slot.ready.store(true, Ordering::Release);
    }
    /// Consumer: None for absent/in-flight records. Never waits for a producer.
    #[must_use]
    pub fn read(&self, index: usize) -> Option<Record> {
        let slot = self.slots.get(index)?;
        if !slot.ready.load(Ordering::Acquire) {
            return None;
        }
        let off = slot.off.load(Ordering::Relaxed);
        let value = slot.value.load(Ordering::Relaxed);
        Some(Record {
            event: if off == u64::MAX {
                Event::Latch(value)
            } else {
                Event::Write { off, value }
            },
            before: core::array::from_fn(|i| slot.before[i].load(Ordering::Relaxed)),
            after: core::array::from_fn(|i| slot.after[i].load(Ordering::Relaxed)),
            raise: match slot.raise.load(Ordering::Relaxed) {
                1 => Raise::Message,
                2 => Raise::OutOfRange,
                _ => Raise::None,
            },
            end: slot.end.load(Ordering::Relaxed),
        })
    }
    /// Reserved records, including in-flight reservations, always <= N.
    #[must_use]
    pub fn used(&self) -> usize {
        self.next.load(Ordering::Acquire)
    }
    /// Samples dropped after filling capacity.
    #[must_use]
    pub fn full(&self) -> u64 {
        self.full.load(Ordering::Relaxed)
    }
    /// Samples dropped after one contended reservation attempt.
    #[must_use]
    pub fn contended(&self) -> u64 {
        self.contended.load(Ordering::Relaxed)
    }
}
