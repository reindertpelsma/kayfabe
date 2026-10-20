# Night N1 preregistration

STATUS: RESEARCH, 2026-10-20. Measurement and diagnosis only.

INFERRED hypothesis H1: two scripted GPU clients reproduce the owner's per-app
presentation silences. FALSIFIER, recorded before the run: either 12-minute A run
has no >=4s app silence, or their counts differ by more than a factor of three.
A/A agreement qualifies this load only, not every mixed workload. Terminal gaps
are labeled censored; watchdog/ETW evidence is needed to classify GPU versus
measurement/CPU stalls. Missing expected presents invalidates a run.

INFERRED H2: the defect was introduced after candidate-4. FALSIFIER: any verified
candidate-4 >=4s app silence under the same qualified load. B follows A/A first.

INFERRED H3: async disable/re-enable contributes. FALSIFIER: the otherwise matched
async-preempt-removed arm freezes. CODE READ: the historical ablation says this
flag is unnecessary for boot/idle Edge but explicitly ambiguous under load. This
permits the requested experiment; removal is not a fidelity fix.

INFERRED H4: an unfulfilled GPU semaphore acquire explains some stalls.
FALSIFIER: a valid stalled segment has no pending acquire, its awaited value is
already satisfied, or the entire private ring is consumed. G collects diagnostics
when an immutable binary and its report are available; do not infer semaphore
values from GP_GET alone.

CODE READ: D is excluded by the owner's cabling constraint. F equals the current
1920x1080 desktop as inspected before the first boot, so there is no distinct
1080p comparison unless later measurement finds a different baseline resolution.
C is available as a prebuilt candidate-5 binary; all comparisons require fresh
boots and at least two runs. A zero-freeze arm needs two further repeats and a
longer load before any absence claim. No product behavior will be changed.

## Coordinator-authorized continuation, 2026-10-20 03:31 CEST

CODE READ: authorization supersedes the earlier stop on collector-format errors:
repair those test-first and continue; host safety conditions or new host Xids
stop experiments and restore the original owner session. Existing deadlines hold.

INFERRED H-fast: the Vulkan pair repeatedly reaches DEVICE_LOST_TDR within 90 s.
FALSIFIER before the next complete result: a qualified fresh A run reaches 90 s
without nvlddmkm Event 153 or device-loss output. Report censored times and all
runs, not just failures. A/A still runs twelve minutes per boot. Kernel-Boot
Event 153 is not nvlddmkm Event 153. Silence rates from a device-loss run are
retained separately and never counted as the independent-freeze class.

INFERRED H-pair: two Heavens, Heaven+Blender viewport animation, or Heaven+Edge
WebGL can produce >=4s per-PID silences without Event 153/device loss. FALSIFIER
for a particular trial: the pair produces Event 153/device loss, does not actually
present on both clients, or has no qualifying silence during its stated window.
A no-event window is not an absence or stability claim. Arrange non-Vulkan pair
windows so both remain partly visible; otherwise normal occlusion could imitate
silence. Do not change guest resolution, product behavior or timers.

CODE READ planned budget: twelve-minute A/A for the Vulkan pair; fresh three-
minute fast trials on B/G/E (>=2 each); twelve-minute A/A for a candidate
independent pair, then ten-minute B/G/E independent-pair trials if qualified and
time/disk permit. Try Blender/Edge separately with explicitly stated durations.
Interleave arms where available. G follows B as soon as its binary/report exists.
The original C binary exists, but coordinator-prioritized B/G/E take precedence.

CODE READ, 04:12 CEST, before G/C runs: G source 98a7bbfb8 branches at f8f9d1a5f,
not 0d7ba743; it contains no TSG mirror module. It cannot isolate adding diagnostics
to A. Add the existing f8f9d1a5 C control, preserve exact product revisions/flags,
and report G's actual lineage. FALSIFIER for diagnostic-only causal attribution:
A and G differ in product source outside diagnostics (already found); do not
attribute any A/G outcome difference solely to KF3_DIAG_USER_STALL. No product
rebase or modification is performed by N1.

## Coordinator H-VA priority, before H measurements

H-VA: a refresh cleared without covering a changed page leaves stale GPU page-table state and can misdirect a semaphore release. Falsifier: the H independent-freeze rate is within the measured control spread. Compare C f8f9d1a5 with H 5c1a864d2, same profile without mirror flags, fresh boots, at least two runs each, interleaved. Keep DEVICE_LOST_TDR separate: a fast device-loss response does not establish improvement in independent freezes. Record superseded/overflow/ALL_PDB lines and report zero matches explicitly. Build the immutable H binary from the named source with two CPU jobs into a new private output; never replace a binary. C/H supersedes B/E in run priority. Pair screening proceeds while H builds.

Before the first two-Heaven capture: H-VA's independent-freeze falsifier requires a valid control workload that actually produces independent freezes. If neither C control window produces that class, report the comparison as unqualified for that hypothesis, with observed zero-event rates and finite durations; do not claim absence or a fix. The fast TDR comparison remains reportable separately.

04:41 CEST, before C3: add C3-fast-matched so the primary fast comparison is H1/C2/H2/C3, all without a mirror flag and after the coordinator's compression completed. The exploratory C1 window carried ignored mirror=0 and overlapped compression; retain it separately rather than counting it in the exact-profile primary C/H pair. This is a control replacement before observing C3, not an event-based exclusion.

04:45 CEST, before any two-Heaven measurement: use the first C two-Heaven window for discovery. If load is valid and the independent-freeze class occurs, complete the planned H/C/H windows on that pair. Otherwise screen A with Blender and Edge (eight minutes each) before choosing the first valid independent-positive pair for C/H. Both fast-class C/H comparisons remain primary and unchanged. If no screened pair reproduces independently, report that limitation and finite zero-event observations; use remaining time for the requested arms rather than claiming a cure. Pause the queue between discovery phases while retaining the exclusive lease.
