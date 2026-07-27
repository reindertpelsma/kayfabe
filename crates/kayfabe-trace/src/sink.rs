//! Where events go, and what stamps them.
//!
//! Three types, with one job each:
//!
//! - [`TraceSink`] — the port. Adapters implement it (a host log, a raw file); tests
//!   implement it with [`TraceLog`], a plain `Vec`. The core never knows which.
//! - [`Recorder`] — owns the counter and the sink **together**, so a record cannot be
//!   written without being sequenced and counted. It is the only implementor of
//!   [`Journal`]; adapters never implement that.
//! - [`Trace`] — the *argument* the emitting code holds: `Option<&mut dyn Journal>`,
//!   where `None` is the disabled state. This is the shape that makes tracing cheap
//!   (see [`Trace::emit`]).
//!
//! ## ★ Cheap when disabled, by construction rather than by assertion
//!
//! Tracing sits on the doorbell and completion-drain hot paths. [`Trace::emit`] takes a
//! **closure**, and calls it only if a recorder is attached, so on the disabled path:
//!
//! - the [`crate::TraceEvent`] is never constructed — no `Vec<u8>` allocation for a
//!   guest-RAM payload, no field copies;
//! - there is no virtual call. [`Trace`] is `Option<&mut dyn Journal>`, a fat pointer
//!   whose `None` is a null data pointer, so the guard is one predictable branch on a
//!   value already in a register — the vtable is never loaded.
//!
//! `tests/tests/trace_replay.rs` proves the first half *structurally* (the closure's
//! side effect is observable, and the disabled path leaves it at zero, with the enabled
//! path as the non-vacuity arm) rather than by timing, and reports a measured
//! nanoseconds-per-emit figure separately.
//!
//! ## ★ Ordering: what the counter guarantees, and what it does not
//!
//! One [`Recorder`] owns one `u64`, bumped inside `&mut self`. So:
//!
//! **It guarantees** a total order over the records written *into that recorder*: dense
//! (no gaps), strictly increasing, and — because it is stamped in the same exclusive
//! borrow that writes to the sink — the stamp order is the sink order. Two threads that
//! share one recorder through the adapter's exclusion are therefore totally ordered
//! against each other. `mode2_gsp_port_plan.md` §6.1 relies on exactly this and no more:
//! the device it records is single-threaded, so one counter *is* the total order of the
//! device.
//!
//! **It does not guarantee:**
//!
//! 1. **Anything across two recorders.** Two recorders both start at zero; merging their
//!    streams by [`crate::Seq`] produces a fabricated interleaving.
//!    `two_recorders_do_not_share_an_order` pins this so nobody assumes otherwise.
//! 2. **That record order is operation order.** The stamp orders *emissions*. If a plane
//!    does its work and emits afterwards, two operations can complete in one order and
//!    be recorded in the other unless the emit is inside the same exclusion as the work.
//!    Emitting under the lock that already serialises the plane is what makes the two
//!    coincide; that is the adapter's obligation, not this crate's.
//! 3. **That it is a clock.** [`crate::TraceEvent::Clock`] carries time; the sequence
//!    number carries order. They are separate for the reason §6.1 gives — a replay must
//!    be deterministic without reading a real clock.
//! 4. **Anything about operations that emit nothing.** An un-instrumented plane is
//!    invisible, which is what [`crate::Counters::silent_kinds`] exists to expose.
//!
//! ## ★ What an adapter may do inside `record`
//!
//! A sink's `record` runs wherever the emitting plane runs — which, under the L1 shell,
//! can be inside a ranked lock. Rule R1 (no blocking under a lock) therefore binds the
//! *sink*: appending to memory is fine, and a sink that performs a blocking write must
//! buffer and hand off, never block in `record`. This is why the file sink lives in the
//! adapter and not here: this crate has no way to do the wrong thing.

use crate::budget::Counters;
use crate::event::{Record, Seq, TraceEvent};

/// Where trace events go. Implemented by adapters, and by tests as a plain `Vec`.
///
/// Object-safe: the emitting code holds `&mut dyn` and never knows the concrete sink.
/// Deliberately **not** `Send + Sync` — a sink's synchronisation belongs to whoever owns
/// it (`kayfabe-core`'s crate docs: ports the core *stores* carry the bound, ports passed
/// as arguments do not).
pub trait TraceSink {
    /// Record one sequenced event.
    ///
    /// Must not block: see the module docs. A sink that cannot keep up should drop and
    /// count its own drops, which [`crate::Counters`] will then disagree with — the
    /// disagreement being the point.
    fn record(&mut self, rec: Record);
}

/// The sink that discards everything.
///
/// Useful as a "counters only" mode: attach it to a [`Recorder`] and the per-plane counts
/// are still maintained while the stream costs nothing to store. Note this is **not** the
/// disabled path — the event is still constructed. For "off", use [`Trace::off`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoTrace;

impl TraceSink for NoTrace {
    #[inline]
    fn record(&mut self, _rec: Record) {}
}

/// The in-memory reference sink: the whole stream, in order.
///
/// This is what a differential replays against, and what tests assert on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceLog {
    records: Vec<Record>,
}

impl TraceLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> TraceLog {
        TraceLog {
            records: Vec::new(),
        }
    }

    /// The records, in the order they were recorded.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// How many records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True if nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Just the events, dropping the sequence numbers — the *decoded projection* a
    /// differential compares (`mode2_gsp_port_plan.md` §6.3). Position carries the order,
    /// so two streams recorded by two different recorders are still comparable here even
    /// though their [`Seq`] spaces are not.
    #[must_use]
    pub fn projection(&self) -> Vec<TraceEvent> {
        self.records.iter().map(|r| r.ev.clone()).collect()
    }

    /// Every record whose event satisfies `pred`.
    pub fn filter(&self, pred: impl Fn(&TraceEvent) -> bool) -> Vec<&Record> {
        self.records.iter().filter(|r| pred(&r.ev)).collect()
    }

    /// Drop everything recorded so far, keeping the allocation.
    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Take the records out.
    #[must_use]
    pub fn take(&mut self) -> Vec<Record> {
        core::mem::take(&mut self.records)
    }
}

impl TraceSink for TraceLog {
    fn record(&mut self, rec: Record) {
        self.records.push(rec);
    }
}

/// Fan one stream out to two sinks — e.g. the full log plus a live adapter.
///
/// Both see the same [`Record`] with the same [`Seq`], because sequencing happens in the
/// [`Recorder`] above them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tee<A, B> {
    /// The first sink.
    pub a: A,
    /// The second sink.
    pub b: B,
}

impl<A: TraceSink, B: TraceSink> TraceSink for Tee<A, B> {
    fn record(&mut self, rec: Record) {
        self.a.record(rec.clone());
        self.b.record(rec);
    }
}

/// The sequencing half of tracing.
///
/// Implemented **only** by [`Recorder`]. It exists so [`Trace`] can be a thin
/// `Option<&mut dyn _>` without exposing the counter and the sink as two separate
/// borrows — which is also what makes it impossible to write to the sink without
/// stamping and counting.
pub trait Journal {
    /// Stamp, count, and hand the event to the sink.
    fn write(&mut self, ev: TraceEvent);

    /// The sequence number the next record will carry.
    fn next_seq(&self) -> Seq;
}

/// Owns the counter and the sink together.
///
/// The adapter (or a test) owns one of these and hands out [`Trace`] borrows. See the
/// module docs for the ordering contract — it is a property of *this* object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recorder<S> {
    seq: u64,
    counters: Counters,
    sink: S,
}

impl<S: TraceSink> Recorder<S> {
    /// A recorder over `sink`, starting at sequence zero.
    pub fn new(sink: S) -> Recorder<S> {
        Recorder {
            seq: 0,
            counters: Counters::new(),
            sink,
        }
    }

    /// Borrow this recorder as an *enabled* trace argument.
    pub fn trace(&mut self) -> Trace<'_> {
        Trace(Some(self))
    }

    /// The perf-budget counters — what was **offered**, per plane.
    #[must_use]
    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    /// The sink.
    #[must_use]
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// The sink, mutably.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume the recorder, yielding the sink.
    #[must_use]
    pub fn into_sink(self) -> S {
        self.sink
    }
}

impl<S: TraceSink> Journal for Recorder<S> {
    fn write(&mut self, ev: TraceEvent) {
        self.counters.bump(ev.kind());
        let seq = Seq(self.seq);
        self.seq += 1;
        self.sink.record(Record { seq, ev });
    }

    fn next_seq(&self) -> Seq {
        Seq(self.seq)
    }
}

/// The trace argument emitting code holds. `None` inside is the disabled state.
///
/// Passed as `&mut Trace<'_>` — a *thin* pointer to a two-word value, so the disabled
/// check is a null test on a register, with no vtable load. See the module docs.
pub struct Trace<'r>(Option<&'r mut dyn Journal>);

impl core::fmt::Debug for Trace<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.0 {
            Some(j) => write!(f, "Trace(on, next={})", j.next_seq()),
            None => f.write_str("Trace(off)"),
        }
    }
}

impl Trace<'_> {
    /// The disabled trace. `const`, so a call site can name it without a binding.
    #[must_use]
    pub const fn off() -> Trace<'static> {
        Trace(None)
    }

    /// True if a recorder is attached.
    ///
    /// Call sites that would have to do real work *just* to build an event (walking a
    /// table, copying guest bytes) should gate on this; everything else should use
    /// [`Trace::emit`], whose closure already gives the same laziness.
    #[inline]
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    /// ★ Emit an event, constructing it **only** if a recorder is attached.
    ///
    /// This is the hot-path form and the one to use by default: on the disabled path the
    /// closure is never called, so nothing is allocated, nothing is copied, and nothing
    /// is formatted.
    #[inline]
    pub fn emit(&mut self, f: impl FnOnce() -> TraceEvent) {
        if let Some(j) = self.0.as_deref_mut() {
            j.write(f());
        }
    }

    /// Emit an already-constructed event.
    ///
    /// The event is built by the caller whether or not tracing is on, so this is for
    /// *cold* paths and for events already in hand. On a hot path, prefer
    /// [`Trace::emit`].
    #[inline]
    pub fn emit_now(&mut self, ev: TraceEvent) {
        if let Some(j) = self.0.as_deref_mut() {
            j.write(ev);
        }
    }

    /// The sequence number the next emitted record will carry, or `None` when disabled.
    ///
    /// For a caller that wants to correlate a returned value with its record.
    #[must_use]
    pub fn next_seq(&self) -> Option<Seq> {
        self.0.as_deref().map(|j| j.next_seq())
    }
}

impl Default for Trace<'_> {
    /// The disabled trace, at any lifetime.
    ///
    /// Constructed directly rather than via [`Trace::off`]: `Trace<'r>` is **invariant**
    /// in `'r` (it holds `&'r mut (dyn Journal + 'r)`), so a `Trace<'static>` does not
    /// coerce to a shorter one. That invariance is also why there is no `reborrow` — pass
    /// `&mut Trace<'_>` down a call chain and the compiler's implicit reborrow does the
    /// job, which is what every emitting signature should take anyway.
    fn default() -> Self {
        Trace(None)
    }
}
