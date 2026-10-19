# Hitch timeline preregistration

STATUS: DESIGN-ONLY, 2026-10-19. Lane H1. No hardware experiment is authorized
until the coordinator writes `go` in this session. All hypotheses below are
INFERRED; none is an attribution of the owner's hitches.

## Falsifiers, stated before collection

| Suspect | Decisive observation | Falsifier for a particular gap |
|---|---|---|
| S1 MMU / host act | Correlated invalidate arrival, walk, plan, apply, clear or RM interval spans the first delayed stage | All relevant operations complete before that stage is due, with complete coverage |
| S2 tick / credit | Copy-ready frame waits for a missing tick/credit, followed by fallback or handoff | Credits and ticks progress throughout the delay and the frame is sent promptly |
| S3 broker / compositor | A sent content sequence waits for commit or presentation feedback | Its compositor present follows promptly; the earlier stage already stalled |
| S4 host CPU / IRQ / frequency | Same-thread scheduling or IRQ evidence precedes the delayed stage | Thread runs promptly throughout that stage; frequency snapshots alone never establish causation |
| S5 GPU | Correlated submitted work has a late real GPU completion, with timely CPU execution | Real completion is timely; no GPU completion coverage means UNKNOWN, not removal |
| S6 guest app / OS | Aligned ETW first stops producing presents, with preceding host stages healthy | Guest continues producing frames during the host-side gap |
| S7 reclaim / compaction / runqueue | A same-thread reclaim, compaction or runqueue interval covers the first delayed stage | Complete thread tracing shows no such interval; global PSI/vmstat alone cannot assign S7 |

Classification is an evidence-supported inference, never a proof from temporal
coincidence. Missing correlation, uncertain clocks, dropped recorder events,
missing stage coverage or competing explanations produce UNKNOWN. Report every
new-content gap strictly above 80 ms, including UNKNOWN. Also tabulate strictly
above 100/200 ms, and the owner's >33/>50/>100 ms metrics. Preserve event windows.
S7 explicitly includes same-thread runqueue intervals per the coordinator's
addition; S4 remains available for independently established CPU/IRQ causes.

## Arms and stopping rule

Six independent boots, alternating A/B/B/A/A/B: A = production `0452613b5`;
B = `0d7ba7438`, TSG mirror ON. Each uses a fresh throwaway overlay, identical
Heaven D3D11 Ultra 1920x1080 fullscreen/vsync settings, 600 seconds of a scripted
free-camera walk plus one 260-second F9 benchmark. Reset the app and camera
before each workload. Record the actual input schedule, load time, revision,
binary hashes, diagnostic flags, host pressure, ETW loss and timeline loss.
No tuning or suspect-driven changes between boots. Native Heaven and compositor
control are separate follow-up controls, not silently substituted arms.

Budget requested: exactly 180 minutes from `go`, including graceful stop,
six boots, collectors, analysis and owner-session restoration. Workload time is
86 minutes (6 x 860 seconds). Reserve 54 minutes for boot/setup and 40 minutes
for shutdown, collection, analysis and restoration. Abort a new boot if it would
consume the restoration reserve. Restore the coordinator-supplied launcher and
flags exactly; do not edit the owner's launcher. Never write the owner's disk.

The instrumented source must be reported separately from the historical arm:
candidate-5 cannot silently stand in for production A. Instrumentation patches
need to apply to both exact arm sources, with matching feature-off controls.
No claims that diagnostics are neutral until the control comparison passes.

## Recorder and analysis gates

INFERRED design: a compile-time `hitch-timeline` feature plus default-off
`KF3_DIAG_TIMELINE=1`; one host CLOCK_MONOTONIC nanosecond domain; fixed-capacity
atomic ring; bounded nonblocking producers; explicit drops; off-thread text
drain at exit and on request. No strings, allocation, file IO, mutex, environment
lookup or lazy initialization in a vCPU producer. Diagnostic arguments are not
evaluated in feature-off builds. Tests precede implementation in EXPECTED RED
commits. Each behavior mutation must fail at least two tests before promotion.

Prepared collectors must remain inactive in Part A. Part B samples pressure,
vmstat, schedstat and supported task delay accounting every 100 ms; captures
same-clock per-thread reclaim/compaction/runqueue intervals; records IRQ and
frequency/C-state evidence and broker compositor timestamps. Unsupported
taskstats/tracepoints are reported as missing coverage, never zero delay.

Clock calibration must include error bounds and before/after samples. CODE READ:
the earlier hitch plotting code uses guest FILETIME, a timezone offset and
rate matching; the P1 harness uses matched fence observations and QPC calibration.
Neither is itself evidence of an invariant-TSC offset. Do not assume QPC equals
TSC or use visual/rate alignment as evidence for 80 ms stage ordering. A measured
guest-QPC to host-monotonic calibration is required; otherwise cross-clock
classification remains UNKNOWN.
