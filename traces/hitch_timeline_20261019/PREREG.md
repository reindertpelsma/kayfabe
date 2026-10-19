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

SUPERSEDED budget: the original 180-minute plan covered only the six fresh-boot
Heaven comparisons. Coordinator additions 2/3 replace that budget with about
240 minutes and add Bedrock, a three-hour uninterrupted soak, and a VRAM pair.
The expanded program does not fit 240 minutes: even if one of the six original
boots doubles as the soak boot, the workload-only lower bound is
180 + 5 x (600 + 260)/60 = 251 minutes 40 seconds, before Bedrock, the VRAM pair,
boot/setup, collection, analysis or restoration. Do not silently reduce n=3 per
arm or shorten the soak. The coordinator must choose a longer total window or
split the fresh-boot comparison and soak/VRAM study into separate windows.
Recommended proposal: separate approximately three-hour fresh-boot and four-hour
soak/VRAM windows, each with its own explicit `go` and restoration reserve.
This is a proposal, not an authorization or claim that all work fits four hours.
Restore the coordinator-supplied launcher and flags exactly; do not edit the
owner's launcher. Never write the owner's disk.

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

## Additions 2/3: Bedrock, aging, paging and per-stall evidence

MEASURED (coordinator observations, not new H1 runs): owner reports progressed
from approximately one-second to approximately ten-second Bedrock stalls on
run 3015, source `0d7ba7438`, mirror OFF, at roughly two hours of uptime. P1's
`mirror-b5-sn-r1/r2` new-content max gaps were 1066.717/1083.384 ms. These are
separate captures; they do not measure an uptime slope or establish a mirror
effect. Do not pool them with a repeated workload in one boot.

Before any new capture, preregister these falsifiers:

- **INFERRED aging/session hypothesis:** repeated identical active walks in the
  same boot show a rising stall rate/duration with VM uptime. FALSIFIER: no
  reproducible increase across 10-minute uptime bins after conditioning on
  workload and measured host/guest memory state. One soak can show temporal
  association; it cannot separate uptime from accumulated workload or establish
  a general aging effect without replication. No relaunch/reset may reset the
  VM-start anchor. Record device/driver generations separately.
- **INFERRED guest VidMm overcommit hypothesis:** ETW-confirmed active-game
  stalls coincide with residency-budget pressure, eviction/migration events,
  CE2/CE3 activity and invalidate bursts; the 10240-MiB arm reduces those
  stalls compared with 8192 MiB at matched workload/uptime. FALSIFIER: adequate
  guest residency headroom and no paging/eviction evidence during the stall,
  or no replicated improvement in the larger-VRAM arm under comparable state.
  A post-exit VRAM reading does not test this hypothesis. CE activity alone
  does not prove paging. Larger guest VRAM may increase host pressure: record
  it rather than silently attributing any change to guest overcommit.
- **INFERRED S1 invalidate tail hypothesis:** individual arrival-to-clear and
  apply intervals delay guest progress during an ETW-confirmed stall. FALSIFIER:
  all relevant requests complete promptly before the first stalled stage,
  with event coverage complete. Increased invalidate count alone is not a tail
  latency measurement. Keep pending/censored requests and failed/superseded
  clears visible. Do not read the lifetime vat max as a current-window sample.
- **INFERRED slow-act hypothesis:** a slow act and its held guest reply precede
  the first stalled stage of the same active process. FALSIFIER: no temporally
  and causally linked act; a boot-time 0.9-second act cannot explain a gameplay
  stall two hours later merely because its lifetime maximum remains printed.

Workloads: (1) the Heaven free-camera path, (2) Heaven F9 benchmark, and (3)
Bedrock world load plus a deterministic walk in a disposable world copy.
After `go`, discover the installed Bedrock package and executable through a
read-only guest package query; record actual app/version/process/API and world
identity. Do not assume RenderDragon uses D3D11 or D3D12 from the product name.
Use ETW/PresentMon provider evidence and record the actual rendering API.
Separate world-load, walking, intentional idle/menu and app-exit phases.
Do not modify or reuse the owner's live world. No guest query is run in Part A.

Soak: one boot remains up at least three hours; repeat the fixed active walk
every 15 minutes, keep host sampling enabled, and record phase markers and
input delivery times. Register the app/world reset policy so accumulated chunk
cache state is measured, not silently discarded. A single boot is n=1 for age;
repeated walk windows are dependent observations, not independent boots.

VRAM arms: 8192 versus 10240 MiB (`WIN_FB_MB`) on throwaway overlays with the
same source, mirror setting, RAM, CPU, renderer, world, distance/settings and
input path. Alternate boot order and compare matched uptime bins; target n>=3
boots per size for a causal comparison. If the allotted hardware window allows
only one pair, report a pilot, not removal of the overcommit hypothesis.
Record guest dedicated/shared usage AND VidMm budget/resident/evict/page-in
events, host GPU-memory pressure and host reclaim. Unsupported counters are
UNKNOWN, not zero. Do not query the host GPU during Part A.

Every recorder record must carry host monotonic ns, VM-start monotonic ns or
the matching session ID, derived signed time since VM start, device generation,
thread ID, request/packet/act correlation IDs, and explicit loss counters.
The fixed-capacity recorder must drain periodically off the producer threads
for at least three hours, and flush at exit/on request. Bounded ring memory
does not by itself bound output size: preregister a disk budget/rotation policy
that preserves every event or reports lost/truncated ranges. No vCPU allocation,
lock, file IO or flush. Feature-off builds eliminate instrumentation entirely.

For every confirmed new-content gap, retain guest ETW Present/vsync and active
workload context. Report exact observed invalidate count plus per-request
arrival/clear/apply durations (pending durations remain unknown/censored),
CE2/CE3/GR0 completion-edge counts/rates with coverage, and bracketing guest
residency samples without pretending stale values were measured at the stall.
Summarize S1..S7/UNKNOWN in 10-minute uptime bins using each bin's actual observed
exposure. Assign each gap to the bin of its preceding new-content present;
keep its entire duration and cross-bin end timestamp. No gaps are manufactured
from `kms=0`/`copies=0`; those counters cannot distinguish idle from a stall.

Long-act table: every observed act strictly above 20 ms, including API, actual
object class, guest client/object handles, group mode, executor thread/TID,
guest waiter process/thread, duration and measured VM uptime. Unknown identity
or timing stays UNKNOWN. Existing successful-act log lines omit some fields,
and quiet/requeued acts may not print a line; the existing log cannot prove a
complete inventory or isolate individual RM-call duration from closure time.
Never turn an opaque RM client handle into a guest PID by assumption.
