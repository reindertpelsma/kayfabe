//! Pure, bounded SERIAL replay experiment, default unused. No device/KVM wiring.
//!
//! This is an ordering oracle, not a safe coalesced-MMIO implementation. A single
//! owner supplies a total event order. Host latches are barriers too: allowing a
//! latch to overtake recorded writes invalidates the delivery proof. Real workers
//! and vCPUs cannot establish this order by waiting for one another. Untrapped
//! enable readbacks are stale until drain, so the experiment cannot replace the
//! synchronous device path. No timer or progress guarantee is implied.
//!
//! The vCPU-side capability set is fixed memory plus CpuIntr atomics. There is no
//! allocator, callback, OS handle, lock, retry loop or worker acknowledgement.
//! Draining visits at most CAPACITY entries and returns owed messages as data;
//! delivery and publishing a shadow are outside this module.

use crate::cpuintr::{CpuIntr, Raise, Reg};

/// Fixed upper bound, independent of guest input.
pub const CAPACITY: usize = 64;

/// A validated register write (the caller owns decode and register bounds).
#[derive(Clone, Copy, Debug)]
pub struct Write {
    /// Decoded register.
    pub reg: Reg,
    /// Original value, never merged across SET/CLEAR aliases.
    pub value: u32,
}

/// Messages a caller owes, returned as data, never delivered inline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owed {
    /// Number of Message results (one physical MSI vector in the current device).
    pub messages: u32,
    /// Invalid latch/trigger vectors, preserved separately.
    pub out_of_range: u32,
}

impl Owed {
    fn add(&mut self, other: Self) {
        self.messages += other.messages;
        self.out_of_range += other.out_of_range;
    }
    fn raise(raise: Raise) -> Self {
        Self {
            messages: u32::from(raise == Raise::Message),
            out_of_range: u32::from(raise == Raise::OutOfRange),
        }
    }
}

/// Only masks are candidates for recording. W1C/TRIGGER/TOP are barriers.
#[must_use]
pub const fn recordable(reg: Reg) -> bool {
    matches!(
        reg,
        Reg::LeafEnSet(_) | Reg::LeafEnClear(_) | Reg::TopEnSet | Reg::TopEnClear
    )
}

/// Pure FIFO replay; registers must already be validated. Refuse oversized input
/// before applying anything, so a caller cannot turn this into an unbounded drain.
#[must_use]
pub fn replay(tree: &CpuIntr, writes: &[Write]) -> Option<Owed> {
    if writes.len() > CAPACITY {
        return None;
    }
    let mut owed = Owed::default();
    for write in writes {
        owed.add(Owed::raise(tree.write(write.reg, write.value)));
    }
    Some(owed)
}

/// Single-owner serial simulator. Not shared between vCPU and worker threads.
pub struct Replay<'a> {
    tree: &'a CpuIntr,
    pending: [Option<Write>; CAPACITY],
    len: usize,
}

impl<'a> Replay<'a> {
    /// Stack/fixed-storage initialization; no allocation.
    #[must_use]
    pub const fn new(tree: &'a CpuIntr) -> Self {
        Self {
            tree,
            pending: [None; CAPACITY],
            len: 0,
        }
    }
    /// Record a mask, or drain then synchronously apply an ordering barrier.
    #[must_use]
    pub fn write(&mut self, write: Write) -> Owed {
        if recordable(write.reg) {
            let owed = if self.len == CAPACITY {
                self.drain()
            } else {
                Owed::default()
            };
            self.pending[self.len] = Some(write);
            self.len += 1;
            owed
        } else {
            let mut owed = self.drain();
            owed.add(Owed::raise(self.tree.write(write.reg, write.value)));
            owed
        }
    }
    /// In this serial model, latches cannot overtake writes. A real worker drain
    /// requires a separate concurrency/readback/liveness design before wiring.
    #[must_use]
    pub fn latch(&mut self, vector: u32) -> Owed {
        let mut owed = self.drain();
        owed.add(Owed::raise(self.tree.latch(vector)));
        owed
    }
    /// Reset is quiescent in CpuIntr; preserve owed results before clearing state.
    #[must_use]
    pub fn reset(&mut self) -> Owed {
        let owed = self.drain();
        self.tree.reset();
        owed
    }
    /// At most CAPACITY atomic applies. No kernel/QEMU ring, BQL or callback.
    #[must_use]
    pub fn drain(&mut self) -> Owed {
        let mut owed = Owed::default();
        for slot in &mut self.pending[..self.len] {
            if let Some(write) = slot.take() {
                owed.add(Owed::raise(self.tree.write(write.reg, write.value)));
            }
        }
        self.len = 0;
        owed
    }
}
