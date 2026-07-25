//! # kayfabe-trace — structured tracing (SKELETON this milestone)
//!
//! Lesson L6: instrument the *process*, not just driver internals — every plane
//! exposes structured trace events, and the conformance harness diffs observable
//! end-states against goldens (never green-log heuristics). This crate will own:
//!
//! - the `TraceEvent` vocabulary (one variant per plane: rmgraph apply, routing
//!   decision, address bind/miss, doorbell dispatch, completion post/drain/poll,
//!   isolate verb) — the **replay format** the differential harness consumes;
//! - a [`TraceSink`] trait so production adapters (QEMU log, file via the adapter —
//!   never the core) and tests (in-memory vec) plug in without the core knowing;
//! - the perf budget counters (R5 mitigation: every port step has before/after
//!   numbers on the same bench).
//!
//! Kept OS-free: the sink is a trait; the core only *emits*.

/// Where trace events go. Implemented by adapters (and by tests as a plain `Vec`).
pub trait TraceSink {
    /// Record one pre-rendered event line. (Typed events replace this with the port.)
    fn emit(&mut self, event: &str);
}
