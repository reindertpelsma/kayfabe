**STATUS: RESEARCH, 2026-10-12 04:15UTC.**

Correction 2026-10-12: this is an unconnected model, not the live worker's
progress rule. The diagnostic worker now reports source GP_GET from the first
actual native descriptor-fetch receipt, independently of later staged storage
retirement and software action completion. See ../user-worker/README.md. The
model's final-only source progress below must not be read as current runtime.

Ordered source progress for the Windows user relay; no runtime caller yet.
Based on pushed9f7cdd81f; the commit containing this file is the tested checkpoint.

The model exposes one Channel per native allocation, one active immutable source
plan, internally allocated increasing plan IDs, and reserved-before-issued work.
Intermediate HOST receipts free command cells, never final guest source progress.
Software actions wait for prior receipts and block later work until actual success.
A source result is taken once, after its generation is revalidated, before another
plan can start. Header-only/unfinished parser snapshots cannot enter; fully
inspected no-op snapshots have a separate explicit path.

Independent source review found and corrected two prototype blockers before any
runtime wiring: independently constructed plans could bypass a pending software
action; an empty plan could retire a header-only unfinished source. Low-level plan
and receipt APIs are now private. Review then found no remaining source blocker.

MEASURED: full `cargo test -p kf-chan -p kf-qemu -p kf-rm` passed on this tree,
including coordinator, low-level wrap/replay/software/backpressure/host-plan
exhaustion and clean pre-I/O refusal checks. Filtered Cargo summary is retained.
Earlier failing-first model compile stopped at the absent user_progress module;
that is a model introduction check, not evidence of a hardware defect.

External gates remain mandatory: unique fresh native allocation/zero cursors,
fresh coherent ordered native USERD reads, complete parser/planner snapshots,
source pins/VA epoch/lifetime, actual channel-owned software success, and source
generation validation before writing guest USERD. An uncertain publication I/O
result stops/quiesces the channel; no timeout supplies a receipt. HOST consumption
never proves engine operands retired. No Windows migration or GravityMark PASS,
and no current-revision Linux hardware regression claim.
