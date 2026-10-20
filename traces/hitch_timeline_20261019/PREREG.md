# Hitch timeline preregistration

STATUS: DESIGN-ONLY, updated 2026-10-20. Lane H1. No hardware experiment is authorized
until the coordinator writes `go` in this session. All hypotheses below are
INFERRED; none is an attribution of the owner's hitches.

## Falsifiers, stated before collection

| Suspect | Decisive observation | Falsifier for a particular gap |
|---|---|---|
| S8 held/forwarded guest reply (FIRST PRIORITY) | A correlated reply wait spans the ETW-verified present-thread wait, with arrival/hold, rows-applied/act-resolution and posted timestamps | Complete correlated traces show every relevant reply posted promptly before the verified guest wait; no overlapping held/forwarded request |
| S1 MMU / host act | Correlated invalidate arrival, walk, plan, apply, clear or RM interval spans the first delayed stage | All relevant operations complete before that stage is due, with complete coverage |
| S2 tick / credit | Copy-ready frame waits for a missing tick/credit, followed by fallback or handoff | Credits and ticks progress throughout the delay and the frame is sent promptly |
| S3 broker / compositor | A sent content sequence waits for commit or presentation feedback | Its compositor present follows promptly; the earlier stage already stalled |
| S4 host CPU / IRQ / frequency | Same-thread scheduling or IRQ evidence precedes the delayed stage | Thread runs promptly throughout that stage; frequency snapshots alone never establish causation |
| S5 GPU | Correlated submitted work has a late real GPU completion, with timely CPU execution | Real completion is timely; no GPU completion coverage means UNKNOWN, not removal |
| S6 guest app / OS | Aligned ETW first stops producing presents, with preceding host stages healthy | Guest continues producing frames during the host-side gap |
| S7 reclaim / compaction / runqueue | A same-thread reclaim, compaction or runqueue interval covers the first delayed stage | Complete thread tracing shows no such interval; global PSI/vmstat alone cannot assign S7 |
| S9 pre-TDR degradation (phenomenon, separate from causal S1–S8) | Aligned, complete active-game Present timeline falls during the ten seconds before a verified TDR | Complete aligned active-game trace shows no decline before the TDR; missing traces cannot remove S9 |

Classification is an evidence-supported inference, never a proof from temporal
coincidence. Missing correlation, uncertain clocks, dropped recorder events,
missing stage coverage or competing explanations produce UNKNOWN. Report every
new-content gap strictly above 80 ms, including UNKNOWN. Also tabulate strictly
above 100/200 ms, and the owner's >33/>50/>100 ms metrics. Preserve event windows.
S7 explicitly includes same-thread runqueue intervals per the coordinator's
addition; S4 remains available for independently established CPU/IRQ causes.
Addition 4 sets investigation priority to S8, then S1, then the remaining
suspects. Priority is not attribution or permission to skip the earliest-stage
and UNKNOWN rules. Summary tables must include S8 as a separate class.

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

CODE READ (coordinator's consolidated instruction): K1 owns the single runtime
profiler, hooks and collectors. H1 must not build another ring. The previous
H1 `hitch-timeline` / `KF3_DIAG_TIMELINE` proposal is superseded. K1's design is
`docs/design/V3_PROFILER_20261019.md` on `codex/kf-prof-20261019`; outputs are
`prof.jsonl` and `prof.mmap`, using host CLOCK_MONOTONIC ns. H1 owns the offline
analyzer and the importer once a concrete JSONL format lands on origin.
Feature-off cost, bounded producer/storage behavior, loss and rotation remain
acceptance requirements for K1, not claims about implemented H1 runtime code.
Tests precede H1 behavior changes in EXPECTED RED commits; every behavior
mutant must fail at least two tests before promotion.

K1-owned collectors must remain inactive in Part A. Part B samples pressure,
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
Summarize S1..S8/UNKNOWN in 10-minute uptime bins using each bin's actual observed
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

## Addition 4: held-reply and forwarded-request timing, priority S8

MEASURED by the coordinator at 15:45 with Bedrock running: guest dedicated
2010/8192 MiB, total committed 2396 MiB, shared 268 MiB; host GPU 9093/12282 MiB;
swap disabled. INFERRED: this weakens guest VRAM overcommit at that snapshot.
It does not measure a stall peak, resident budgets or eviction activity. Sample
guest `\GPU Adapter Memory(*)\Dedicated Usage` and Total Committed every second
during Part B alongside ETW VidMm; retain instance identities, PID/LUID and
sampling age. Do not replace the snapshot with a peak claim.

MEASURED by the coordinator: roughly 3800 HELD-REPLY / POSTED pairs, depth/n=1,
without timestamps. CODE READ: the common HELD-REPLY text is printed for both
map-refresh holds and deferred host acts. RmControl alone does not identify a
mapping operation. Matching pair counts say nothing about individual latency.

Record every RmControl/RmAlloc/Free request, including immediate, refused,
deferred and held replies, with these distinct same-clock phases:

1. Queue-head trap and command arrival/service (different points in the path,
   both host CLOCK_MONOTONIC; never silently rename service as arrival).
2. Hold created, reason `map_refresh`, `deferred_act` or both; unique RPC sequence
   plus VM/device/driver generation, client/object, actual API/class, requested
   row/page counts and the inbox/walk settlement watermark.
3. Real host rows acknowledged/applied and/or host act resolved, observed
   applied row/page counts and status. Preserve both phases when both gate it.
4. Reply successfully written to the guest queue, then publication/IRQ edge.
   A failed/full-queue post is not a posted reply. Retain retries and censor
   waits outstanding at capture end. Posting is not guest consumption.

The analyzer reports the longest arrival-to-post and hold-to-post intervals,
their row-apply/act-resolution splits and overlap with EACH verified stall.
Missing arrival, row counts, application phase or waiter identity stays UNKNOWN;
missing post is right-censored, not zero and not a fabricated completion. Keys
include device/generation, not a reused raw handle or depth=1. Report host-call
service duration separately from guest-visible reply latency.

When feasible capture guest kernel CSwitch/ReadyThread events, present-thread
PID/TID/wait reason and the NVIDIA/dxgkrnl work leading to the RPC. Verify ETW
loss, guest/host clock uncertainty and identity mapping before assigning S8.
No host timestamp overlap or opaque RM client handle alone proves the presenting
guest thread waited for that RPC. A spinning kernel waiter may not generate a
sleep interval; absence of CSwitch is not its falsifier. Without verified guest
linkage, show the host reply interval but keep causal attribution UNKNOWN.

## Consolidated update: K1 handoff, Java and S9

MEASURED by coordinator, not independently reconstructed by H1: Minecraft Java
run 3017 (`0d7ba743`, mirror OFF, iothread disk) reached TDR at 2026-10-19
16:04:30 CEST. Display fps fell from 57 to 45 at :20, 31 at :22, 2.9 at :28,
and zero at :30, before nvlddmkm 153 x3. Disk await was 0.1–0.5 ms, with no
inflight IO during the degradation. Storage is excluded as the trigger for
that observed window; this does not remove all guest/disk causes in other runs.
REFUSED/absence/unsettled counts matched production run 3011; their presence
is not a new regression signal. Preserve the ten-second lead-in rather than
starting the analysis at the reset/event-log messages.

CODE READ: a read-only origin fetch pinned K1 at
`9b5f12f292c7ede0867b2834d7be427eda15d5e1`. Its design is still explicitly
proposed; the source has no JSONL writer or reader. The document specifies
`schema:1` and conceptual interval fields, but no complete canonical JSON
object, event-tag encoding, coverage/loss representation or example. H1 must
not invent those names/semantics. The importer remains pending that contract;
aggregate per-second data cannot become individual presentation timestamps.
See [K1_IMPORT.md](K1_IMPORT.md) for the pinned compatibility requirements.

MEASURED read-only check, 2026-10-20: the named saved `sampler.tsv` has 362 lines
including its header, wall-clock-only labels 02:09:04 through 02:15:04 and no
run/boot or host-monotonic identity. This snapshot cannot independently align
the coordinator's 16:04:20–:30 event. Do not relabel it as run 3017 or infer a
timezone/clock correction. The coordinator's run-3017 measurements above retain
their stated provenance. Saved source: `/data/claude-stall10s-20261019/sampler.tsv`.

INFERRED preregistered S9 analysis: retain ten one-second bins immediately
before each explicitly verified guest TDR, using normalized, aligned individual
guest Presents for that PID. Keep host new-content bins alongside them as a
separate observation. Compare the first two seconds with the last two seconds;
report DECLINE_OBSERVED only when the early count's uncertainty lower bound is
greater than the late count's upper bound. Report NO_DECLINE_OBSERVED only with
a positive early baseline and the converse nondecrease established; otherwise
UNKNOWN. Use twice measured guest/host calibration error for boundary bounds.
Require complete ten-second capture/exposure, no loss, verified TDR/PID and a
recorded active workload covering the entire window. An Event 153 line alone
is not a verified TDR timestamp. Unknown coverage is not measured zero fps.
This is a descriptive S9 phenotype, never a root-cause attribution: retain
S1–S8/UNKNOWN for each gap and attach overlapping pre-TDR windows separately.

Fourth workload: Minecraft Java world load plus the same registered walking
policy in a disposable world copy. After `go` with the coordinator's `-prof`
binary, discover and record actual Java/game/mod/renderer versions, process,
world, settings and input schedule. Separate load/walk/idle/exit phases and
capture TDR/reset events plus the ten-second prehistory. Do not substitute
Bedrock/RenderDragon or assume a rendering API. Add this arm to the fresh-boot
and matched-uptime comparison; never silently reduce the registered boot count.
The 251 min 40 s workload-only lower bound above now excludes both Minecraft
workloads and the VRAM study; it remains a lower bound, not a feasible budget.
Part B remains NOT GO, pending the binary and explicit coordinator release.
