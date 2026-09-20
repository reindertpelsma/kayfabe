//! The privileged ring — §5.4. One bounded lock-free MPSC, one dedicated drainer.
//!
//! ## Why a ring at all, and why ONE (not per-vCPU)
//!
//! §5.4: *"a guest thread on one vCPU writes `PDB_LO`/`PDB_HI` under an RM lock and releases it;
//! another thread on another vCPU takes the lock and writes `TRIGGER`. On hardware the first trap
//! returned before the lock released, and PCIe writes to one function are ordered."* ⇒ **Per-vCPU
//! rings preserve only per-vCPU order and fire the trigger against a stale base.** The global
//! order is the whole product here, so there is exactly one ring per GPU.
//!
//! ## ⊘⊘ The claim is CONDITIONAL, and this is the paragraph the shape comes from
//!
//! §5.4: *"read the cursor, test fullness, take the slot by CAS; on full, poison and return
//! **having claimed nothing**. A `fetch_add` claim that then waits for the consumer is the vCPU
//! waiting on a worker in lock-free costume; a claim-then-bail strands the consumer at that slot
//! forever."*
//!
//! ⇒ Both of the obvious implementations are wrong, in opposite ways:
//! - `fetch_add` then wait ⇒ violates §48/§41 — the vCPU blocks on a worker.
//! - `fetch_add` then bail on full ⇒ leaves a **hole** the drainer stops at forever.
//!
//! The only correct claim is CAS-on-a-tested-cursor, which never reserves a slot it will not fill.
//!
//! ## ⊘ Full ⇒ POISON, never wait
//!
//! §5.4: *"a sticky error state, further writes dropped, a guest-visible fault. Dropping one write
//! silently leaves state a later, differently-privileged guest process inherits."* ⚠ That last
//! clause is the security half: a silent drop is an inherited-state bug across §4's inner boundary,
//! not merely a lost write.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// §5.4: *"Capacity: 4096 entries per GPU, 64 KiB, prefaulted. The bound is derived, not guessed."*
/// Realistic maximum in flight is **under 64**; this is a ~60× margin.
pub const CAPACITY: usize = 4096;

/// §5.4: *"sustained occupancy above 25 % is a defect to investigate, and the high-water mark
/// prints at teardown."*
pub const OCCUPANCY_ALARM: usize = CAPACITY / 4;

/// One privileged register write, as the trap classified it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegWrite {
    pub bar: u8,
    pub offset: u32,
    pub value: u64,
    pub width: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// Queued at this sequence. The drainer will apply it in exactly this order.
    Queued(u64),
    /// ⊘ The ring was full. The device is now POISONED and this write was NOT queued.
    /// §5.4: never wait. The caller must raise a guest-visible fault, not retry.
    Poisoned,
    /// Already poisoned; further writes are dropped **by name**, never silently.
    Dropped,
}

pub struct PrivRing {
    slots: Box<[AtomicU64]>,     // packed RegWrite, one per slot
    meta: Box<[AtomicU64]>,      // offset|bar|width, one per slot
    ready: Box<[AtomicBool]>,    // slot is fully written and safe to apply
    head: AtomicU64,             // next to apply (drainer only)
    tail: AtomicU64,             // next to claim (producers CAS this)
    poisoned: AtomicBool,
    high_water: AtomicU32,
    /// §5.4: *"`applied_seq` is a clean monotonic number other paths fence against."*
    applied_seq: AtomicU64,
}

impl Default for PrivRing {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivRing {
    pub fn new() -> PrivRing {
        PrivRing {
            slots: (0..CAPACITY).map(|_| AtomicU64::new(0)).collect::<Vec<_>>().into_boxed_slice(),
            meta: (0..CAPACITY).map(|_| AtomicU64::new(0)).collect::<Vec<_>>().into_boxed_slice(),
            ready: (0..CAPACITY).map(|_| AtomicBool::new(false)).collect::<Vec<_>>().into_boxed_slice(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            poisoned: AtomicBool::new(false),
            high_water: AtomicU32::new(0),
            applied_seq: AtomicU64::new(0),
        }
    }

    /// ★ THE vCPU PATH. Never blocks, never allocates, never waits on the drainer.
    pub fn push(&self, w: RegWrite) -> Push {
        if self.poisoned.load(Ordering::Acquire) {
            // ⊘ By name. §5.4: a SILENT drop "leaves state a later, differently-privileged guest
            // process inherits" — so the drop is reported, counted and visible.
            return Push::Dropped;
        }
        loop {
            let tail = self.tail.load(Ordering::Acquire);
            let head = self.head.load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= CAPACITY as u64 {
                // ⊘ FULL ⇒ poison, having claimed NOTHING.
                self.poisoned.store(true, Ordering::Release);
                return Push::Poisoned;
            }
            // ⊘⊘ CAS the cursor, never fetch_add: this reserves a slot only when we will fill it.
            if self
                .tail
                .compare_exchange_weak(tail, tail + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let i = (tail as usize) % CAPACITY;
                self.slots[i].store(w.value, Ordering::Relaxed);
                self.meta[i].store(
                    (w.offset as u64) | ((w.bar as u64) << 32) | ((w.width as u64) << 40),
                    Ordering::Relaxed,
                );
                // Release: the slot contents happen-before the drainer observes `ready`.
                self.ready[i].store(true, Ordering::Release);
                let occ = (tail + 1 - head) as u32;
                self.high_water.fetch_max(occ, Ordering::AcqRel);
                return Push::Queued(tail);
            }
        }
    }

    /// Drainer: peek the head **without** consuming. §5.4: *"peek → apply → commit, so a failed
    /// apply retries at the head"*. ⊘ Consuming before applying would drop a write whose apply
    /// failed — the silent-drop bug again, one layer down.
    pub fn peek(&self) -> Option<(u64, RegWrite)> {
        let head = self.head.load(Ordering::Acquire);
        if head >= self.tail.load(Ordering::Acquire) {
            return None;
        }
        let i = (head as usize) % CAPACITY;
        if !self.ready[i].load(Ordering::Acquire) {
            // A producer CAS'd the cursor but has not finished writing the slot. Not an error:
            // the drainer simply has nothing applicable yet.
            return None;
        }
        let m = self.meta[i].load(Ordering::Relaxed);
        Some((
            head,
            RegWrite {
                offset: (m & 0xffff_ffff) as u32,
                bar: ((m >> 32) & 0xff) as u8,
                width: ((m >> 40) & 0xff) as u8,
                value: self.slots[i].load(Ordering::Relaxed),
            },
        ))
    }

    /// Drainer: the apply succeeded; advance. Only now does `applied_seq` move.
    pub fn commit(&self) {
        let head = self.head.load(Ordering::Acquire);
        let i = (head as usize) % CAPACITY;
        self.ready[i].store(false, Ordering::Release);
        self.head.store(head + 1, Ordering::Release);
        self.applied_seq.store(head + 1, Ordering::Release);
    }

    /// §5.4 / §5.6: the monotonic number other paths fence against.
    #[inline]
    pub fn applied_seq(&self) -> u64 {
        self.applied_seq.load(Ordering::Acquire)
    }
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }
    #[inline]
    pub fn occupancy(&self) -> usize {
        (self.tail.load(Ordering::Acquire) - self.head.load(Ordering::Acquire)) as usize
    }
    /// §5.4: printed at teardown; >25 % sustained is a defect to investigate.
    #[inline]
    pub fn high_water(&self) -> u32 {
        self.high_water.load(Ordering::Acquire)
    }
}
