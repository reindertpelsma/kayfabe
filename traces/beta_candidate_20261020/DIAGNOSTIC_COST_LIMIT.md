# Inherited diagnostic cost qualification

STATUS: PARTIAL / NO UNIVERSAL COST CLAIM, 2026-10-20.

INFERRED falsifier for the remaining qualification, before any assembly or
overhead run: any retained diagnostic gate
that can initialize a OnceLock, inspect the environment, allocate or wait on its
first guest-facing invocation is not proven to meet the strict compile-out or
one-relaxed-load bound. Passing functional tests does not establish that bound.

CODE READ: candidate-5 contains cached gates, including
`kf-qemu/src/display.rs::write_trace_on`, the METHOD_TRACE OnceLock in
`DisplayPlane::write`, `debug_cursor_env`, `bar0trace::enabled`, and
`kf-rm/src/census.rs::rpc_trace`. Some gates are read in construction/worker
startup and some call sites remain in guest-command paths. OnceLock's cached
fast path is not an explicit `AtomicBool::load(Relaxed)` contract. This lane
preserves these existing gates and introduces no new diagnostic hot-path gate.
The source alone does not certify the requested universal off-path bound or
prove that all first touches happen before vCPUs run.

CODE READ: the inherited step-meter/event-ring gating tests remain and pass;
all diagnostic/measurement/control environment overrides are optional and
inactive when unset. The compatibility Settings has no KEEP flag settings.
Those functional facts are distinct from universal cost qualification.

INFERRED recommendation: retain the owner-mandated diagnostic/control names,
then review their initialization/call sites and generated off paths separately
before release qualification. Prefer construction-time selection or an explicit
relaxed gate without changing displayed content, pacing, relay or ownership.
Do not delete required diagnostics, rename them, claim zero overhead, or infer
hardware parity from the C6 CPU tests. This remains a Part A qualification gap;
the beta branch is not a supported release or approved master merge.
