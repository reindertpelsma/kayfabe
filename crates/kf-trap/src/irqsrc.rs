//! ★ **Why the device raised a guest interrupt** — source-tagged raise counters
//! (`docs/design/V3_IRQ_SOURCE_TRACE.md`, 2026-10-09).
//!
//! The MSI trace (`vfio_msi_interrupt`) says THAT kayfabe interrupted the guest, on vector 0 every
//! time (RM demultiplexes through the CPU interrupt tree, [`crate::cpuintr`]); it does not say WHY.
//! Every place that latches a tree vector now names itself with an [`IrqSource`], and
//! [`IrqSourceCounts`] keeps, per `(source, vector)`, how many latches SENT a message and how many
//! stayed HELD (the leaf or top enable was clear). Always on: two relaxed `fetch_add`s and one
//! relaxed load, no lock, no allocation, no print — the production line stays quiet; the
//! `irq[...]` segment of the status line renders it ([`IrqSourceCounts::render`]).
//!
//! ## Trace mode
//!
//! With the BAR0 read trace on (`KF3_BAR0_READ_TRACE=1`, perturbing, default off) the counts are
//! ALSO queued in a bounded ring ([`RaiseRing`]); the C device's main loop drains it and writes one
//! `kf3_irq_raise` QEMU trace event per record in front of the `vfio_msi_interrupt` line the same
//! wake writes, so both land in one `trace.log`. The ring has many producers (vCPUs, the worker,
//! the drainer, the display worker) and ONE consumer (the main loop).
//!
//! ## What is NOT counted here
//!
//! A raise the tree itself decides without a latch: a guest write that ENABLES an already-pending
//! vector sends it ([`crate::cpuintr::CpuIntr::write`]); that is tagged [`IrqSource::GuestEnable`]
//! with the vector bucket [`ANY_VECTOR`] (one write can release several vectors, one message).

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::cpuintr::{MAX_LEAF, Raise};

/// Distinct tree vectors a counter row covers (the widest family's `LEAF__SIZE_1` × 32).
pub const MAX_VECTORS: usize = MAX_LEAF * 32;
/// The "no single vector" bucket: a tree write released pending vectors (see the module docs).
pub const ANY_VECTOR: u32 = MAX_VECTORS as u32;
/// Counter-row stride: the vectors plus the [`ANY_VECTOR`] bucket.
const STRIDE: usize = MAX_VECTORS + 1;

/// Why a guest interrupt was raised. `Copy`, one byte of class plus (for engines) one byte of index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqSource {
    /// A call site that did not name itself (none should remain).
    Other,
    /// The display engine's frame edge made an ENABLED head-timing event pending
    /// (`RM_INTR_STAT_HEAD_TIMING`, `kf-qemu::display` `frame_edge`).
    DisplayTiming,
    /// A window or core `AWAKEN` event was pending at the display worker's raise.
    DisplayAwaken,
    /// A window semaphore event (`SEM_WIN`) was pending at the display worker's raise.
    DisplaySem,
    /// The guest's own write to `RM_INTR_EN_HEAD_TIMING` enabled an already-pending event.
    DisplayEnable,
    /// A host engine's notifier edge raised the engine's own non-stall vector (`index` = the
    /// device's engine slot).
    EngineNonstall(u8),
    /// A host `FIFO_EVENT_MTHD` edge relayed to the vector `ptnsi::host_notify_vector` picks.
    FifoRelay,
    /// The Translated GR-tier relay after an engine-retired ring.
    TranslatedGrRelay,
    /// The Translated copy-engine relay (`KF3_TRANSLATED_CE_RELAY`).
    TranslatedCeRelay,
    /// What optional pacing owed, released by the worker tick.
    NsiTick,
    /// `RC_TRIGGERED` posted to the guest's status queue (GSP stall vector).
    Rc,
    /// A hotplug `POST_EVENT` posted (GSP stall vector).
    Hotplug,
    /// A `RUNLIST_PREEMPT_COMPLETE` `POST_EVENT` posted (GSP stall vector).
    PreemptDone,
    /// The guest's own `CPU_INTR_LEAF_TRIGGER` write (the `_osVerifyInterrupts` loopback).
    GuestTrigger,
    /// A guest write that ENABLED an already-pending vector ([`ANY_VECTOR`]).
    GuestEnable,
}

impl IrqSource {
    /// Number of source CLASSES (a counter row each).
    pub const CLASSES: usize = 15;

    /// The class index, `0..CLASSES` (engine index not included).
    #[must_use]
    pub const fn class(self) -> usize {
        match self {
            IrqSource::Other => 0,
            IrqSource::DisplayTiming => 1,
            IrqSource::DisplayAwaken => 2,
            IrqSource::DisplaySem => 3,
            IrqSource::DisplayEnable => 4,
            IrqSource::EngineNonstall(_) => 5,
            IrqSource::FifoRelay => 6,
            IrqSource::TranslatedGrRelay => 7,
            IrqSource::TranslatedCeRelay => 8,
            IrqSource::NsiTick => 9,
            IrqSource::Rc => 10,
            IrqSource::Hotplug => 11,
            IrqSource::PreemptDone => 12,
            IrqSource::GuestTrigger => 13,
            IrqSource::GuestEnable => 14,
        }
    }

    /// The id written in the trace event: `class | engine << 8`.
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            IrqSource::EngineNonstall(e) => self.class() as u32 | (e as u32) << 8,
            _ => self.class() as u32,
        }
    }

    /// The short name of class `c` in the status line (`""` past the last).
    #[must_use]
    pub const fn class_name(c: usize) -> &'static str {
        match c {
            0 => "other",
            1 => "dtim",
            2 => "dawk",
            3 => "dsem",
            4 => "den",
            5 => "eng",
            6 => "fifo",
            7 => "grr",
            8 => "cer",
            9 => "tick",
            10 => "rc",
            11 => "hot",
            12 => "pre",
            13 => "trig",
            14 => "enr",
            _ => "",
        }
    }

    /// The short name of the class of a trace-event source id (`""` for an unknown one).
    #[must_use]
    pub const fn id_name(id: u32) -> &'static str {
        Self::class_name((id & 0xff) as usize)
    }
}

/// What a latch did, as counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A message was sent.
    Sent,
    /// The vector is pending but a leaf or top enable is clear.
    Held,
    /// The vector is outside this family's leaves.
    OutOfRange,
}

impl Outcome {
    /// The counted outcome of a [`Raise`].
    #[must_use]
    pub const fn of(r: Raise) -> Outcome {
        match r {
            Raise::Message => Outcome::Sent,
            Raise::None => Outcome::Held,
            Raise::OutOfRange => Outcome::OutOfRange,
        }
    }

    /// The word the trace event carries: 1 sent, 0 held, 2 out of range.
    #[must_use]
    pub const fn word(self) -> u32 {
        match self {
            Outcome::Sent => 1,
            Outcome::Held => 0,
            Outcome::OutOfRange => 2,
        }
    }
}

/// One queued trace record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaiseRecord {
    /// The tree vector, or [`ANY_VECTOR`].
    pub vector: u32,
    /// [`IrqSource::id`].
    pub source: u32,
    /// [`Outcome::word`].
    pub outcome: u32,
}

/// Slots in the trace ring (a power of two).
const RING: usize = 4096;
/// The written marker (bit 63) of a slot.
const WRITTEN: u64 = 1 << 63;

/// ★ A bounded multi-producer, single-consumer ring of [`RaiseRecord`]s, off until [`Self::enable`].
///
/// A slot is ONE atomic word — `WRITTEN | ticket(32) | outcome(2) | source(16) | vector(10)` — so a
/// reader can never pair half of one record with half of another; a producer that laps the consumer
/// overwrites the oldest record and the consumer counts it dropped.
#[derive(Debug)]
pub struct RaiseRing {
    on: AtomicBool,
    head: AtomicU64,
    tail: AtomicU64,
    dropped: AtomicU64,
    slots: Box<[AtomicU64]>,
}

impl Default for RaiseRing {
    fn default() -> Self {
        RaiseRing {
            on: AtomicBool::new(false),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            slots: (0..RING).map(|_| AtomicU64::new(0)).collect(),
        }
    }
}

impl RaiseRing {
    /// Start queueing (trace mode only; once).
    pub fn enable(&self) {
        self.on.store(true, Ordering::Release);
    }

    /// Whether records are queued.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Queue one record (any thread; a relaxed load and nothing else while the ring is off).
    pub fn push(&self, r: RaiseRecord) {
        if !self.is_on() {
            return;
        }
        let t = self.head.fetch_add(1, Ordering::AcqRel);
        let word = WRITTEN
            | (t & 0xffff_ffff) << 28
            | u64::from(r.outcome & 3) << 26
            | u64::from(r.source & 0xffff) << 10
            | u64::from(r.vector & 0x3ff);
        self.slots[(t as usize) % RING].store(word, Ordering::Release);
    }

    /// The oldest queued record, if one is complete (the ONE consumer: the main loop).
    pub fn pop(&self) -> Option<RaiseRecord> {
        loop {
            let want = self.tail.load(Ordering::Relaxed);
            if want >= self.head.load(Ordering::Acquire) {
                return None;
            }
            let w = self.slots[(want as usize) % RING].load(Ordering::Acquire);
            if w & WRITTEN == 0 {
                return None; // the producer holds the ticket and has not stored yet
            }
            let seen = ((w >> 28) & 0xffff_ffff) as u32;
            let ahead = seen.wrapping_sub(want as u32) as i32;
            if ahead < 0 {
                return None; // the slot still holds an older lap: not written yet
            }
            if ahead > 0 {
                // Lapped: producers are at least a ring ahead. Skip to the oldest record still held.
                let head = self.head.load(Ordering::Acquire);
                let to = head.saturating_sub(RING as u64).max(want + 1);
                self.dropped.fetch_add(to - want, Ordering::Relaxed);
                self.tail.store(to, Ordering::Relaxed);
                continue;
            }
            self.tail.store(want + 1, Ordering::Relaxed);
            return Some(RaiseRecord {
                vector: (w & 0x3ff) as u32,
                source: ((w >> 10) & 0xffff) as u32,
                outcome: ((w >> 26) & 3) as u32,
            });
        }
    }

    /// Records the consumer lost to a lapping producer.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// ★ The counters: `sent` and `held` per `(source class, vector)`, plus out-of-range per class.
#[derive(Debug)]
pub struct IrqSourceCounts {
    sent: Box<[AtomicU64]>,
    held: Box<[AtomicU64]>,
    oor: [AtomicU64; IrqSource::CLASSES],
    /// The trace ring (off unless trace mode enabled it).
    pub ring: RaiseRing,
}

impl Default for IrqSourceCounts {
    fn default() -> Self {
        let rows = || {
            (0..IrqSource::CLASSES * STRIDE)
                .map(|_| AtomicU64::new(0))
                .collect::<Box<[AtomicU64]>>()
        };
        IrqSourceCounts {
            sent: rows(),
            held: rows(),
            oor: core::array::from_fn(|_| AtomicU64::new(0)),
            ring: RaiseRing::default(),
        }
    }
}

impl IrqSourceCounts {
    /// Count one latch of `vector` ([`ANY_VECTOR`] for "no single vector") from `src` that ended in
    /// `raise`; queue the trace record when the ring is on. Lock-free; any thread.
    pub fn note(&self, src: IrqSource, vector: u32, raise: Raise) {
        let outcome = Outcome::of(raise);
        let v = (vector as usize).min(MAX_VECTORS);
        let at = src.class() * STRIDE + v;
        match outcome {
            Outcome::Sent => self.sent[at].fetch_add(1, Ordering::Relaxed),
            Outcome::Held => self.held[at].fetch_add(1, Ordering::Relaxed),
            Outcome::OutOfRange => self.oor[src.class()].fetch_add(1, Ordering::Relaxed),
        };
        self.ring.push(RaiseRecord {
            vector: vector.min(ANY_VECTOR),
            source: src.id(),
            outcome: outcome.word(),
        });
    }

    /// Messages sent for `vector`, summed over every source.
    #[must_use]
    pub fn sent_on(&self, vector: u32) -> u64 {
        let v = (vector as usize).min(MAX_VECTORS);
        (0..IrqSource::CLASSES)
            .map(|c| self.sent[c * STRIDE + v].load(Ordering::Relaxed))
            .sum()
    }

    /// Latches of `vector` that stayed held, summed over every source.
    #[must_use]
    pub fn held_on(&self, vector: u32) -> u64 {
        let v = (vector as usize).min(MAX_VECTORS);
        (0..IrqSource::CLASSES)
            .map(|c| self.held[c * STRIDE + v].load(Ordering::Relaxed))
            .sum()
    }

    /// Messages sent over every vector and source.
    #[must_use]
    pub fn sent_total(&self) -> u64 {
        self.sent.iter().map(|a| a.load(Ordering::Relaxed)).sum()
    }

    /// Latches that stayed held, over every vector and source.
    #[must_use]
    pub fn held_total(&self) -> u64 {
        self.held.iter().map(|a| a.load(Ordering::Relaxed)).sum()
    }

    /// Latches outside the family's leaves, over every source.
    #[must_use]
    pub fn oor_total(&self) -> u64 {
        self.oor.iter().map(|a| a.load(Ordering::Relaxed)).sum()
    }

    /// Messages sent from class `c` over every vector.
    #[must_use]
    pub fn sent_from(&self, c: usize) -> u64 {
        (0..STRIDE)
            .map(|v| self.sent[c * STRIDE + v].load(Ordering::Relaxed))
            .sum()
    }

    /// The compact status-line fragment: for each vector with any traffic,
    /// `v<N>=<sent>[<src>=<n>,…]` and, if some latches stayed held, `/h<held>[<src>=<n>,…]`; the
    /// [`ANY_VECTOR`] bucket prints as `v*`. Empty when nothing was ever latched. Not lock-free
    /// to the cent (it walks the table), but only the drainer's heartbeat calls it.
    #[must_use]
    pub fn render(&self) -> String {
        use core::fmt::Write;
        let part = |rows: &[AtomicU64], v: usize| -> (u64, String) {
            let mut total = 0u64;
            let mut s = String::new();
            for c in 0..IrqSource::CLASSES {
                let n = rows[c * STRIDE + v].load(Ordering::Relaxed);
                if n != 0 {
                    total += n;
                    if !s.is_empty() {
                        s.push(',');
                    }
                    let _ = write!(s, "{}={n}", IrqSource::class_name(c));
                }
            }
            (total, s)
        };
        let mut out = String::new();
        for v in 0..STRIDE {
            let (sent, by) = part(&self.sent, v);
            let (held, hby) = part(&self.held, v);
            if sent == 0 && held == 0 {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            if v == MAX_VECTORS {
                out.push_str("v*");
            } else {
                let _ = write!(out, "v{v}");
            }
            let _ = write!(out, "={sent}[{by}]");
            if held != 0 {
                let _ = write!(out, "/h{held}[{hby}]");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpuintr::{CpuIntr, Reg};

    const UM: u64 = 0x00BB_0000;

    /// Every source, once, through the tree the device uses: counted on the vector it raised, as
    /// sent when the tree sends, as held when an enable is clear.
    fn every_source() -> [IrqSource; IrqSource::CLASSES] {
        [
            IrqSource::Other,
            IrqSource::DisplayTiming,
            IrqSource::DisplayAwaken,
            IrqSource::DisplaySem,
            IrqSource::DisplayEnable,
            IrqSource::EngineNonstall(3),
            IrqSource::FifoRelay,
            IrqSource::TranslatedGrRelay,
            IrqSource::TranslatedCeRelay,
            IrqSource::NsiTick,
            IrqSource::Rc,
            IrqSource::Hotplug,
            IrqSource::PreemptDone,
            IrqSource::GuestTrigger,
            IrqSource::GuestEnable,
        ]
    }

    /// A latch exactly as `Device::latch_and_deliver` does it: tree first, then the count.
    fn latch(t: &CpuIntr, c: &IrqSourceCounts, s: IrqSource, v: u32) -> Raise {
        let r = t.latch(v);
        c.note(s, v, r);
        r
    }

    #[test]
    fn the_classes_are_dense_unique_and_named() {
        let mut seen = [false; IrqSource::CLASSES];
        for s in every_source() {
            assert!(!seen[s.class()], "class {} twice", s.class());
            seen[s.class()] = true;
            assert!(!IrqSource::class_name(s.class()).is_empty());
            assert_eq!(IrqSource::id_name(s.id()), IrqSource::class_name(s.class()));
        }
        assert!(seen.iter().all(|x| *x), "a class with no source");
        assert_eq!(IrqSource::EngineNonstall(7).id(), 5 | 7 << 8);
    }

    #[test]
    fn each_source_is_counted_on_its_vector_and_sums_agree() {
        let t = CpuIntr::new(kf_chip::Family::Ampere, UM).unwrap();
        // vectors 154 and 155 are in leaf 4 (128..159), subtree 2: enable both levels.
        assert_eq!(t.write(Reg::TopEnSet, 1 << 2), Raise::None);
        assert_eq!(t.write(Reg::LeafEnSet(4), u32::MAX), Raise::None);
        let c = IrqSourceCounts::default();
        for (i, s) in every_source().into_iter().enumerate() {
            let v = if i % 2 == 0 { 154 } else { 155 };
            assert_eq!(latch(&t, &c, s, v), Raise::Message);
            assert_eq!(c.sent_from(s.class()), 1, "{s:?}");
        }
        // even indices (0,2,..,14) -> 154: 8 sources; odd -> 155: 7 sources.
        assert_eq!(c.sent_on(154), 8);
        assert_eq!(c.sent_on(155), 7);
        assert_eq!(c.sent_total(), 15);
        assert_eq!(c.sent_on(154) + c.sent_on(155), c.sent_total());
        assert_eq!(c.held_total(), 0);
        assert_eq!(c.oor_total(), 0);
    }

    #[test]
    fn a_masked_latch_is_held_and_an_out_of_range_one_is_counted_apart() {
        let t = CpuIntr::new(kf_chip::Family::Ampere, UM).unwrap();
        let c = IrqSourceCounts::default();
        // nothing enabled: held
        assert_eq!(latch(&t, &c, IrqSource::Rc, 155), Raise::None);
        assert_eq!(c.held_on(155), 1);
        assert_eq!(c.sent_on(155), 0);
        // Ampere has 8 leaves: vector 16*32 is out of range
        assert_eq!(
            latch(&t, &c, IrqSource::Other, 16 * 32),
            Raise::OutOfRange
        );
        assert_eq!(c.oor_total(), 1);
        assert_eq!(c.sent_total() + c.held_total(), 1, "oor is in neither row");
        // the "no single vector" bucket
        c.note(IrqSource::GuestEnable, ANY_VECTOR, Raise::Message);
        assert_eq!(c.sent_on(ANY_VECTOR), 1);
        assert_eq!(c.sent_total(), 1);
    }

    #[test]
    fn the_status_fragment_names_sources_per_vector() {
        let c = IrqSourceCounts::default();
        assert_eq!(c.render(), "");
        for _ in 0..3 {
            c.note(IrqSource::DisplayTiming, 154, Raise::Message);
        }
        c.note(IrqSource::Rc, 155, Raise::Message);
        c.note(IrqSource::Hotplug, 155, Raise::Message);
        c.note(IrqSource::Rc, 155, Raise::None);
        c.note(IrqSource::GuestEnable, ANY_VECTOR, Raise::Message);
        c.note(IrqSource::EngineNonstall(1), 1, Raise::Message);
        assert_eq!(
            c.render(),
            "v1=1[eng=1] v154=3[dtim=3] v155=2[rc=1,hot=1]/h1[rc=1] v*=1[enr=1]"
        );
    }

    #[test]
    fn the_ring_is_off_until_enabled_then_keeps_order_and_ids() {
        let c = IrqSourceCounts::default();
        c.note(IrqSource::Rc, 155, Raise::Message);
        assert!(c.ring.pop().is_none(), "off: nothing queued");
        c.ring.enable();
        c.note(IrqSource::EngineNonstall(2), 1, Raise::Message);
        c.note(IrqSource::DisplayTiming, 154, Raise::None);
        c.note(IrqSource::GuestEnable, ANY_VECTOR, Raise::Message);
        assert_eq!(
            c.ring.pop(),
            Some(RaiseRecord {
                vector: 1,
                source: 5 | 2 << 8,
                outcome: 1
            })
        );
        assert_eq!(
            c.ring.pop(),
            Some(RaiseRecord {
                vector: 154,
                source: 1,
                outcome: 0
            })
        );
        assert_eq!(
            c.ring.pop(),
            Some(RaiseRecord {
                vector: ANY_VECTOR,
                source: 14,
                outcome: 1
            })
        );
        assert_eq!(c.ring.pop(), None);
        assert_eq!(c.ring.dropped(), 0);
    }

    #[test]
    fn a_lapped_ring_drops_the_oldest_and_counts_them() {
        let r = RaiseRing::default();
        r.enable();
        let n = RING as u32 + 10;
        for i in 0..n {
            r.push(RaiseRecord {
                vector: i % 512,
                source: 1,
                outcome: 1,
            });
        }
        let mut got = Vec::new();
        while let Some(x) = r.pop() {
            got.push(x);
        }
        assert_eq!(got.len(), RING, "one ring's worth survives");
        assert_eq!(r.dropped(), 10);
        assert_eq!(got[0].vector, 10 % 512, "the oldest kept is record 10");
        assert_eq!(got[RING - 1].vector, (n - 1) % 512);
    }

    #[test]
    fn producers_on_many_threads_lose_nothing_the_ring_can_hold() {
        let c = std::sync::Arc::new(IrqSourceCounts::default());
        c.ring.enable();
        let handles: Vec<_> = (0..4u32)
            .map(|k| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for _ in 0..500 {
                        c.note(IrqSource::EngineNonstall(k as u8), k, Raise::Message);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut per = [0u32; 4];
        while let Some(r) = c.ring.pop() {
            assert_eq!(r.source >> 8, r.vector, "a record is never torn");
            per[r.vector as usize] += 1;
        }
        assert_eq!(per, [500; 4]);
        assert_eq!(c.sent_total(), 2000);
        for k in 0..4 {
            assert_eq!(c.sent_on(k), 500);
        }
    }
}
