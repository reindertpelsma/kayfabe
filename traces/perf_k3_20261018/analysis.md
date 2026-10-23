# K3: cost attribution

STATUS: ANSWERED WITH LIMITS, 2026-10-23. This perf investigation does not change
candidate6d's PARTIAL/HOLD correctness verdict. No runtime optimization was made.

MEASURED 2026-10-23, runs4620/4621/4622: CATIA and Creo completed once per boot
in each of candidate, VFIO and optional-instrument arms. No new host Xid, guest153
or4101, reboot, budget refusal or nonzero viewperf exit was observed. The post-profile
[host check after profiling](health-after-profiles.json) has NVIDIA bound, fresh cuInit0, GDM active,
ubuntu logged in and two monitors. These are separate diagnostic profiling arms.

| Hypothesis / falsifier stated in plan.json | Result and evidence | Conclusion |
| --- | --- | --- |
| H1: GPU starvation; falsifier busy within5 points despite a performance gap | MEASURED 2026-10-23, telemetry-4620.json / telemetry-4621.json: CATIA74.93% vs85.78%; Creo59.34% vs58.57%. The stamped score comparison is linked below. | INFERRED: supports starvation in CATIA; stated falsifier met in Creo. Global busy includes walker/display GPU work, so this does not isolate rendering-engine busy. |
| H2: vCPU trap/exit dominates; falsifier low active exit rate/time | MEASURED 2026-10-23, kvm-4620-catia.json / kvm-4620-creo.json: non-HLT51.7k/s /42.3k/s,0.290/0.198 core-equivalents. VFIO in kvm-4621-catia.json / kvm-4621-creo.json:26.7k/s /36.8k/s,0.047/0.070. Clean5s instrument captures give0.310/0.192. | INFERRED: exit cost survives as a contributor; dominance of frame critical path is unproved. HLT idle waits excluded; summed vCPU elapsed overlaps. |
| H3: USER dispatch queueing dominates; falsifier paired p99 under100us and no load growth, adequate coverage | MEASURED 2026-10-23, submit-4622.json: USER paired p50 upper3.071/10.239us; p99 upper1572.863/1835.007us for CATIA/Creo. Paired133250/316364; native calls5985789/14762800; pairing2.23%/2.14%. | INFERRED: falsifier not met, but dispatch dominance is unresolved. Empty kicks, guest production and gates can intervene; continuation/fanout calls are unpaired. Long first-kick tails are not proof of scheduler queueing. No inline stage2 justified; see kick-accounting.json for nearly complete kick coverage despite native fanout. |
| H4: display/present caps; falsifier verified no-present workload retains gap | CODE READ at commit70f7fb591: the synthetic60Hz test broker still carries presents. The official SPECviewperf15 guide's documented CLI options supply no verified no-present mode. | NOT EXERCISED. A headless broker is not a no-present arm. |
| H5: relay is a userspace CPU concentration; falsifier samples elsewhere/idle | MEASURED 2026-10-23, cpu-4622.json:947 userspace samples in one30s Creo window; workers52.8%, display29.87%; mapped load_u32 self13.83%. No worker was consistently saturated in telemetry-4622.json. | INFERRED: relay processing is substantial, but not a proven single saturated CPU or the whole gap. Userspace samples exclude kernel and guest execution. |

MEASURED 2026-10-23, [costs-4622.json](costs-4622.json): interior workload windows
contain3.60m/6.03m BAR0 writes in185.59/305.96s. Interrupt-tree writes account for
at least92.48%/90.82%; USER doorbells1165/1538 per second. These offset counts do
not assign every KVM NPF exit to a particular register. IRQ suppression is not a
permitted shortcut: owner ruling X requires every armed edge to be delivered.

MEASURED 2026-10-23, submit-4622.json: authored Translated paired p50 upper
131.071/81.919us, p99 upper3145.727/10485.759us, n36699/54264. Windows Passthrough
was zero; the separate [Linux positive control](linux-profile.json) exercised one
real hardware-forwarded, paired Passthrough call, exact observed maximum0.7us,
bucket upper0.767us, no failure or new Xid. That n=1 validates coverage, not a
performance distribution. Interval histogram maxima are occupied-bucket upper
bounds, not subtracted cumulative maxima. Failed/discarded samples were zero.

MEASURED 2026-10-23, telemetry-4620.json / telemetry-4621.json: GPU clocks remained
high; CATIA mean2842/2812MHz and power105.45/123.81W, Creo2819/2566MHz and
82.61/94.49W. Candidate clocks were not lower. Driver sampling, display carriers
and9216/12282MiB capacities differ. GPU polls200ms and repeated subtests within
one boot are correlated samples, not independent replications.

MEASURED 2026-10-23, [kick-accounting.json](kick-accounting.json): paired first-kick
consumptions plus coalesced kicks differ from BAR0 doorbell counts by only+2/-5
in the two interior windows. Snapshot reads race ongoing calls. INFERRED: native
fanout explains the small paired-native-call fraction; it does not mean most
guest kicks were missed. This does not isolate worker dispatch from gate/guest
waits inside a valid kick-to-native interval.

## One concrete next optimization candidate

CODE READ at commit70f7fb591, crates/kf-qemu/src/mem.rs::RowsLog::changed_since:
each lookup linearly visits old history entries before finding the first later
intersecting epoch, under the log mutex; history is bounded at4096 entries.
MEASURED 2026-10-23, run4622 CPU leaf reports: this function accounts for5.64% of
213 CATIA userspace samples and1.06% of947 longer Creo samples. Raw reports remain
in the run4622 host evidence directory; leaf resolution succeeded, but worker
stack unwinding often yielded no frames.

INFERRED candidate: use an epoch lower-bound search to skip history older than
the requested epoch, then preserve the same forward intersection search and
original timestamp result. Preserve Unknown for truncated history, the same
mutex/snapshot boundary, and all range/epoch semantics. This introduces no
batching delay, no polling, no vCPU work, no changed admission and no new timer.
It is a proposed follow-up, not an implemented or hardware-validated improvement.

INFERRED expected gain: if the local scan cost halves, approximately2.8% of CATIA
and0.5% of Creo sampled userspace CPU is saved; even complete removal is bounded
by5.64%/1.06% in these windows. No end-to-end FPS gain is established, and this
candidate cannot credibly explain or close the whole observed gap. FALSIFIER
BEFORE follow-up: a longer CATIA profile fails to reproduce the site, or the
semantics-equivalent lookup does not reduce its CPU cost beyond run spread.
Tests must first fail for truncated history, earliest overlapping commit,
wrapped storage and concurrent snapshots; mutants must lose at least two tests.

## Evidence and limits

MEASURED 2026-10-23, [stamped profile comparison](profile-comparison-nc.md):
SPEC composites and ratios are confined to that stamped table and CSV. They
are not headline results. No private Steel Nomad/Geekbench scores are included.

MEASURED 2026-10-23, controls/software-controls.json and tests-k3.json:19 focused
API tests passed; the broader two-crate regression passed1097 executions with4
existing ignores, including two duplicate histogram executions. Five mutants
had7/2/2/2/2 distinct assertion killers. Expected reds were missing-API compile
failures, not proof of a pre-existing runtime bug. Clippy added0; fmt passed.

MEASURED 2026-10-23, run4620: the original perf-kvm wrapper aborted while closing
its byte-limited recording; usable prefixes are labelled partial. Plain five-
second entry/exit recordings in runs4621/4622 completed with exit0 and no lost/
overwritten exit records. All analysis distinguishes idle HLT from active exits.
CPU component additivity failed in pidstat, so directly reported CPU totals are
used; user+system+guest are not added into an invented host-overhead figure.

CODE READ scope: no closed binary was reverse engineered. CPU samples use public
symbols and our own source; unknown closed-driver addresses are uninterpreted.
No genuine no-present comparison, per-engine rendering utilization or direct
worker-pickup timestamp was obtained. Those limits prevent a single-cause claim.

MEASURED 2026-10-23, ci-instrument-final.json: exact instrument commit70f7fb591
passed hosted stable, aarch64, broker and firmware; slow was skipped by policy.

MEASURED 2026-10-23, [instrument-sanity-nc.md](instrument-sanity-nc.md):
the diagnostic instrument-on/off comparison uses one boot per head; it is a
sanity comparison, not an estimate of instrumentation overhead or repeat spread.

CODE READ controller ruling2026-10-23: the RowsLog suggestion remains research
only. No implementation is authorized before the longer CATIA falsifier.

## Mutation evidence

MEASURED 2026-10-23, controls/software-controls.json: each mutation terminated
with test exit101. Distinct failed assertions per mutant are listed below;
these counts are not additional independent hardware trials.

| Mutation | Distinct assertion killers |
| --- | ---: |
| Drop pending stamp | 7 |
| Reuse consumed stamp | 2 |
| Suppress observer completion | 2 |
| Reverse observer result | 2 |
| Zero histogram export | 2 |

## Review hardware and final disposition

MEASURED 2026-10-23, ce714-ci-final.json / ce714-build.json: exact candidate
714ec29d3 has all five CI jobs green, eight focused native tests green, and
successful exact-head native/QEMU builds. The earlier changed-crate suite
ran1083 passing executions with4 existing ignores at48431b8e0; the source diff
and its limited equivalence are recorded, not described as an exact-head rerun.

MEASURED 2026-10-23, ce714-native.json: native dedicated engine0xb executes the
supplied no-SET_OBJECT oracle with host-derived ce_seed0xc7b5, seeded_launches1,
real engine semaphore completion and released resources. Zero NoCeObject,
ForeignClass or new Xid. The native ring does not traverse VMM birth identity.

MEASURED 2026-10-23, ce714-linux-guest-ram.json / ce714-linux-ce-client.json /
ce714-energy-four.json: two Linux copy arms and four fresh Energy boots pass,
with host-derived VMM ce_seed0xc7b5 and zero class refusals/new Xid. All VMM
seeded_launches remain0, including each of24 Windows KERNEL copy lifetimes.
The pre-run Energy positive-witness prediction meets its stated falsifier in
these four trials. Neither lifecycle completion nor an available birth seed
establishes missing-bind decoding. The VMM proof remains NOT EXERCISED.
The UVM-only probe is excluded with its original strict copy assertion failure
retained; it is not a third copy pass or a demonstrated product failure.

MEASURED 2026-10-23, ce714-energy-4630.json through ce714-energy-4633.json:
stock-broker logs include an advertised XR24 dma-buf layout import refusal and
subsequent F_SHM fallback; the connection continued with zero rejected commits
at detach. These existing carrier diagnostics prevent a clean dma-buf claim.
Whole-session presents include workload idle intervals; broker low-percentile
new-content FPS cannot be reconstructed from stock aggregate counters.

MEASURED 2026-10-23, master10-cursor-comparison.json: exact mastere9c6d39e5
and candidate01a have identical guest/reference pixel maps and identical host
pixel maps after alignment; visible shape counts254/281 match. The original
RGB max errors48/52 reproduce on master. Hook and oracle hashes remain as
pinned in master-cursor-plan.json; original tolerance2 is unchanged. A host-only
control has all RGBA differences0. INFERRED: the candidate-regression explanation
for this specific discrepancy is falsified under the matched setup; component
localization and a correction remain outside this run. Native hover/grab/input
observations and the original RGB failures remain separate results.

CODE READ independent review origin/claude/review-cand6d-20261023,
docs/audits/2026-10-23-candidate6d-review.md: M3 asks for resolution of a
candidate-only lockstep test failure. This lane did not diagnose or clear it.
Full native gates, Linux30 and all application smokes were not repeated at714.
Candidate correctness therefore stays PARTIAL/HOLD; no merge action was taken.

MEASURED 2026-10-23, k3-host-return.json / process-audit-final.json /
local-cleanup-k3.json / final-xid-k3.json: no own runtime/build/input workers
remain; dedicated dev and host Cargo targets are absent. NVIDIA is bound,
fresh cuInit0, GDM active, Ubuntu logged in, autologin configured, two monitors;
exact journal Xid history unchanged since before run4630. Runtime firmware
and evidence are retained. MEASURED host-yield.json: fastguest lock released,
quiet flag absent and host yielded06:49:51UTC. No GPU or VM work is authorized
after yield until the controller returns it.
