//! Fixed-capacity SPSC event transport. Producers never read file-provided cursors.
use crate::publication::{AtomicWords, WordStorage};
use core::sync::atomic::{AtomicU64, Ordering};

/// A 56-byte event payload; publication adds one sequence word on the wire.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Event {
    /// Absolute monotonic nanoseconds.
    pub ts_ns: u64,
    /// Statically enumerated event kind.
    pub tag: u64,
    /// Packed state/site IDs.
    pub site: u64,
    /// Bounded integer argument or obligation ID.
    pub arg: u64,
    /// Duration or counter value.
    pub value: u64,
    /// Packed device/generation IDs.
    pub scope: u64,
    /// Immutable clock calibration identity.
    pub clock: u64,
}
impl Event {
    fn words(self) -> [u64; 7] {
        [
            self.ts_ns, self.tag, self.site, self.arg, self.value, self.scope, self.clock,
        ]
    }
    fn from_words(w: [u64; 7]) -> Self {
        Self {
            ts_ns: w[0],
            tag: w[1],
            site: w[2],
            arg: w[3],
            value: w[4],
            scope: w[5],
            clock: w[6],
        }
    }
}
#[repr(align(64))]
struct Cursor(AtomicU64);
/// Invalid SPSC capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityError;
/// A bounded single-producer, single-consumer ring. Split requires exclusive ownership.
pub struct Ring<const N: usize> {
    head: Cursor,
    tail: Cursor,
    slots: [AtomicWords<7>; N],
}
impl<const N: usize> Ring<N> {
    /// Require a nonzero power-of-two capacity below the cursor's half-range.
    pub fn new() -> Result<Self, CapacityError> {
        if !N.is_power_of_two() || N as u128 >= 1u128 << 63 {
            return Err(CapacityError);
        }
        Ok(Self {
            head: Cursor(AtomicU64::new(0)),
            tail: Cursor(AtomicU64::new(0)),
            slots: core::array::from_fn(|_| AtomicWords::new()),
        })
    }
    /// Create the only two handles. A second split cannot coexist with them.
    pub fn split(&mut self) -> (Producer<'_, N>, Consumer<'_, N>) {
        // Split can follow a previous completed use; preserve its cursor positions.
        let head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Relaxed);
        (
            Producer {
                ring: self,
                next: tail,
                dropped: 0,
            },
            Consumer {
                ring: self,
                next: head,
            },
        )
    }
}
/// Unique producer handle. No Clone; writes require `&mut self`.
pub struct Producer<'a, const N: usize> {
    ring: &'a Ring<N>,
    next: u64,
    dropped: u64,
}
impl<const N: usize> Producer<'_, N> {
    /// Try to append. Full means a counted drop without claiming a cursor or waiting.
    pub fn push(&mut self, event: Event) -> bool {
        let head = self.ring.head.0.load(Ordering::Acquire);
        if self.next.wrapping_sub(head) >= N as u64 {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let slot = (self.next as usize) & (N - 1);
        for (i, v) in event.words().into_iter().enumerate() {
            self.ring.slots[slot].store(i, v, Ordering::Relaxed);
        }
        self.next = self.next.wrapping_add(1);
        self.ring.tail.0.store(self.next, Ordering::Release);
        true
    }
    /// Number of rejected events in this producer's ownership interval.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
/// Unique consumer; it never waits for the producer.
pub struct Consumer<'a, const N: usize> {
    ring: &'a Ring<N>,
    next: u64,
}
impl<const N: usize> Consumer<'_, N> {
    /// Pop one complete event; an empty ring returns immediately.
    pub fn pop(&mut self) -> Option<Event> {
        if self.next == self.ring.tail.0.load(Ordering::Acquire) {
            return None;
        }
        let slot = (self.next as usize) & (N - 1);
        let mut words = [0; 7];
        for (i, w) in words.iter_mut().enumerate() {
            *w = self.ring.slots[slot].load(i, Ordering::Relaxed);
        }
        self.next = self.next.wrapping_add(1);
        self.ring.head.0.store(self.next, Ordering::Release);
        Some(Event::from_words(words))
    }
}
