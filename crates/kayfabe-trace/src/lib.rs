//! # kayfabe-trace — the structured trace vocabulary and the replay format
//!
//! Lesson L6: instrument the *process*, not just driver internals — every plane emits
//! structured trace events, and the conformance harness diffs observable end-states
//! against goldens (never green-log heuristics). This crate owns:
//!
//! - the [`TraceEvent`] vocabulary — one variant per thing a **differential** can
//!   compare, across the device wire plane (MMIO, guest RAM, IRQ, clock) and the core
//!   decision planes (rmgraph apply, routing, address bind/unbind/resolve, doorbell
//!   dispatch, completion post/drain/poll, isolate verb, control);
//! - [`TraceSink`], so production adapters (a host log, a raw file — in the *adapter*,
//!   never the core) and tests (an in-memory [`TraceLog`]) plug in without the core
//!   knowing which;
//! - the perf-budget [`Counters`], which double as the **non-vacuity** instrument;
//! - the two stream properties a replay differential rests on: [`check_dense_order`] and
//!   [`diff`].
//!
//! Kept OS-free: the sink is a trait; the core only *emits*. This crate names no host
//! descriptor, no syscall and no hypervisor API, and CI greps for all three.
//!
//! ## The format is specified elsewhere; this crate carries it
//!
//! `docs/design/mode2_gsp_port_plan.md` §6 specifies the replay stream the faked-GSP
//! oracle needs: **one interleaved stream, totally ordered by a single counter**, carrying
//! MMIO reads *with the value served*, MMIO writes, guest-RAM reads *with the bytes
//! returned*, our writes, IRQ raises and clock. §6.3 specifies that the differential is
//! over a **decoded projection**, never raw bytes. Both are implemented here rather than
//! reinvented; the wire-plane variants of [`TraceEvent`] are that enum, refined only where
//! a closed type removes a way to be wrong (`size: u8` becomes [`Width`]).
//!
//! ## ★ Why this crate sits below `kayfabe-core`
//!
//! Every plane must be able to emit, so every plane must be able to *depend* on this
//! crate — which means it has to sit below `kayfabe-core` and `kayfabe-fwd` in the
//! lattice. It therefore depends only on the leaf identity/port crates
//! (`kayfabe-arch`, `kayfabe-completion`, `kayfabe-mmu`, `kayfabe-isolate`) and cannot
//! name `RmEvent`, `ProcId`, `FwdFault` or `RmGraphError`. Where a payload type is above
//! it, the bridge is a **trait the owning crate implements with an exhaustive `match`**
//! ([`AsRmVerb`], [`Faulted`]) — so a new variant upstream fails the build until it is
//! named here, rather than silently vanishing from the trace.
//!
//! ## Reading order
//!
//! - [`event`] — the vocabulary, and the two bridging traits.
//! - [`sink`] — the sink port, the recorder, and **the ordering contract**: what the
//!   single counter guarantees and, in four numbered points, what it does not.
//! - [`budget`] — the counters, and why they live in the recorder rather than the sink
//!   (a sink that silently records nothing is the failure mode a tracing crate invites).
//! - [`replay`] — order checking and the projection differential.
//!
//! ## Example
//!
//! ```
//! use kayfabe_arch::ids::{Gpa, GpuId, GpuVa, Pdb};
//! use kayfabe_mmu::AddressFault;
//! use kayfabe_trace::{EventKind, Recorder, Resolved, Seq, TraceEvent, TraceLog, check_dense_order};
//!
//! let mut rec = Recorder::new(TraceLog::new());
//! {
//!     let mut tr = rec.trace();
//!     // The hot-path form: the event is built only because a recorder is attached.
//!     tr.emit(|| TraceEvent::GuestRead { gpa: Gpa(0x1000), bytes: vec![1, 2, 3, 4] });
//!     tr.emit(|| TraceEvent::AddressResolve {
//!         gpu: GpuId(0),
//!         pdb: Pdb(0xdead_0000),
//!         va: GpuVa(0x7f00_0000),
//!         outcome: Resolved::Fault(AddressFault::Miss {
//!             pdb: Pdb(0xdead_0000),
//!             va: GpuVa(0x7f00_0000),
//!         }),
//!     });
//! }
//! assert_eq!(check_dense_order(rec.sink().records(), Seq(0)), Ok(()));
//! assert_eq!(rec.counters().of(EventKind::AddressResolve), 1);
//!
//! // Disabled: the closure never runs, so nothing is allocated.
//! let mut off = kayfabe_trace::Trace::off();
//! let mut built = 0;
//! off.emit(|| {
//!     built += 1;
//!     TraceEvent::Clock { ns: 0 }
//! });
//! assert_eq!(built, 0);
//! ```

pub mod budget;
pub mod event;
pub mod replay;
pub mod sink;

pub use budget::Counters;
pub use event::{
    AsRmVerb, Bar, CompletionOp, Dispatched, EventKind, FaultTag, Faulted, IrqSpec, Outcome,
    ProcRef, Record, Resolved, RmVerb, RouteKey, Routed, Seq, TraceEvent, VerbOutcome, VerbTag,
    Width,
};
pub use replay::{Divergence, OrderingError, check_dense_order, diff, diff_records};
pub use sink::{Journal, NoTrace, Recorder, Tee, Trace, TraceLog, TraceSink};

// The concurrency contract, compile-time-asserted (decision #17). A trace record crosses
// threads by construction — a per-thread recorder's stream gets merged, and a shared
// recorder is reached through the adapter's exclusion — so the vocabulary itself must be
// `Send + Sync` or neither is expressible. `Trace<'_>` is deliberately absent: it holds
// `&mut dyn Journal`, and its synchronisation is the adapter's, exactly as
// `kayfabe-core`'s crate docs require of a port passed as an argument.
kayfabe_util::assert_send_sync!(
    Seq,
    Record,
    TraceEvent,
    EventKind,
    ProcRef,
    FaultTag,
    RmVerb,
    RouteKey,
    Routed,
    Resolved,
    Dispatched,
    CompletionOp,
    VerbTag,
    VerbOutcome,
    Outcome,
    IrqSpec,
    Width,
    Bar,
    Counters,
    TraceLog,
    NoTrace,
    Divergence,
    OrderingError,
);
