# V3 SPECviewperf gate: the `specviewperf` stage of the Vast release tier

**STATUS: DESIGN-ONLY, 2026-11-07** for everything that runs on a box, the Worker and the tier plan: this is a draft PR against
`next`, **disabled by default** (nothing sends the stage until the owner enables it for the large tier), with GPU-free tests and mutants
only; **no box, no rental and no Windows guest has run any of it**, and the stage's PowerShell has never run on Windows. The GPU-free code
(`lane.py`, `tarfilter.py`, the payload grading, the report rows) is real and tested. Parent documents:
[`V3_WINDOWS_GATE.md`](V3_WINDOWS_GATE.md) (the gate this stage follows, same guest and same verdict philosophy),
[`V3_CI_TIERS_AND_IGNORE_LADDER.md`](V3_CI_TIERS_AND_IGNORE_LADDER.md) (the tiers), [`../DOWNLOADS.md`](../DOWNLOADS.md)
(handling rules of the private copy, and the section "SPECviewperf in the Vast gate") and
[`../SPEC_RESULTS.md`](../SPEC_RESULTS.md) (what may be published). Measurements it rests on:
`traces/specviewperf_native_20261107/README.md` (the native reference, branch of PR #289; **not on `next` yet**, so not linked) and [`traces/specviewperf_vast_20261105/`](../../traces/specviewperf_vast_20261105/README.md) (kayfabe, PR #265).

Marking: **MEASURED** = a line in a file of those traces (revision, box and die in them), **CODE READ** = read in this repository,
**INFERRED** = reasoned from those numbers, not seen, **DESIGN** = intent, **OPEN** = an owner decision. An unmarked sentence is a
claim of fact about the code in this PR.

## 1. What the stage is for

The release tier (the large tier) is meant to run the nine SPECviewperf 15.0.0 viewsets through the kayfabe Windows guest on a real
GPU, once per box of a die, and to fail the release when a viewset fails. A failure under kayfabe is kayfabe's: the native arm
passes every viewset on the same die. **MEASURED** (RTX 4090, Windows 11 LTSC, driver 580.88, one iteration, FullHD): all nine
viewsets finish `Completed` natively with SPEC's own validation passing and zero event-log resets, and catia-07 also passes on a
3060 and a 5070. **MEASURED** under kayfabe on 2026-11-05 (rev `6a0683af`, the same die): medical-04, solidworks-08, maya-07 and
energy-04 completed with 0 resets; catia-07 failed 2 of 6 subtests with 20 resets (the guest survived); enscape-01 was refused by SPEC's
validation (`raytracing`, `sufficientVideoMemory`); blender-01, creo-04 and unreal_engine-01 were not run for lack of lease time.
So the stage must treat catia-07 and enscape-01 as failures, and **nothing is excluded or weakened**: there is no skip list in this
PR, and `test_vast_spec_lane.py` replays the real kayfabe catia-07 failure and requires FAIL.

## 2. What is built

| Piece | File | State |
|---|---|---|
| The lane table (one source: lanes, native walls, budgets, tree sizes, time constants) | `scripts/bench/specviewperf/lanes.json` | read by the stage and by the payload |
| The decisions as pure functions (grade a viewset, decide the stage, time guard, scrub) | `scripts/bench/specviewperf/lane.py` | GPU-free tests |
| The tar filter (one lane's viewsets only, refuses links and `..`) | `scripts/bench/specviewperf/tarfilter.py` | GPU-free tests |
| The orchestration (fetch, copy, boots, polling, teardown) | `scripts/bench/box/specviewperf_stage.sh` | tests against fakes; no box |
| The guest side (prep, verify, launch as an interactive task, status, collect, kill) | `scripts/bench/windows/spec/spec_guest.ps1`, `spec_run.ps1` | **never run on Windows** |
| The wrapper that lets only status lines through | `scripts/ci/vast_stage_specviewperf.sh` (existed as plumbing since 2026-11-05) | tests |
| The payload side: `KF_SPEC`, rows, time check, disk size | `scripts/ci/vast_payload.py`, `vast_report.py` | tests and mutants |
| The report rows | `scripts/ci/vast_run_report.py` | tests and mutants |
| The sha256 pin (PROPOSED, owner confirms) | `scripts/bench/box/specviewperf.pin` | |
| The Worker's option (`spec` of `tierEnvironment`, never passed) | `ci/vast-broker/src/tier.js` | node tests |

Nothing was deployed and nothing in the Worker changes behaviour: no caller passes `spec`, `releaseStages` stays empty.

## 3. Inputs

DESIGN, as the owner's handling rules require (`docs/DOWNLOADS.md`): the only input is the Worker's `/bench-data` route, through
`fetch_bench_data.sh` with the run token. The stage asks for `specviewperf-15.0.0.tar.zst` (42,002,015,727 B, **MEASURED**) and for
nothing else: no spec.org, no vendor, never the installer, no licence PDF per run. Its sha256 is pinned in the repo
(`specviewperf.pin`); the Worker holds no pin. A missing pin line, an object the Worker does not know (404) or a mismatch is RETRY
class **MEDIA** and the helper deletes the bytes. A transport failure of the download is **PROVIDER**; running out of the stage's
time while downloading is **TIME**. The Worker mints at most 4 presigned URLs per object per run, each valid at most 20 minutes
(CODE READ, `benchdata.js`); the helper takes at most 3 and resumes. **MEASURED** download speed 48 to 52 MB/s, so 14 to 16 minutes
for the archive, and the hash of the whole file 1.5 to 5 minutes.

The archive is downloaded in the background while the stage does its host reference and boots the copy guest. It is then verified
(inside the helper), streamed through `zstd -d | tarfilter.py | ssh ... tar -xf -` into the guest, and deleted right after the copy.
Only the lane's viewsets are copied (**MEASURED** tree sizes: lane A 49.9 GB, lane B 43.9 GB of the 94.2 GB), because the guest
disk takes about 40 to 45 MB/s (**MEASURED** 2026-11-05): the whole tree would take 35 to 40 minutes. The guest copy is compared with
the filter's manifest (files and bytes per viewset directory), so a truncated copy is **TRANSPORT**, not a result. The guest is
booted **without** kf3 (standard VGA) for the copy, as on 2026-11-05: 94 GB through the kf3 guest would only add risk, and the copy
is not what is under test.

## 4. The lanes

**MEASURED** native walls (seconds, one CLI run each, RTX 4090; each includes the CLI's own file verification, which took 5 to 15
minutes of guest disk reads on slow hosts): medical 318, energy 323, maya 400, catia 803, solidworks 989, unreal 1236, creo 1243,
blender 1876, enscape 2042; their sum is 9,230 s (154 minutes). The native agent measured about 200 minutes for everything on a fast
host (45 minutes from rental to the first CLI run, **INFERRED** sum of parts) and **INFERRED** 275 to 305 minutes on the slower hosts
seen (3060, 5070), over the 300-minute box limit. Hence two lanes (DESIGN, as the native report proposed):

| Lane | Viewsets in run order (ascending native wall) | Native sum | Tree |
|---|---|---:|---:|
| `lanea` | energy-04, creo-04, blender-01, enscape-01 | 5,484 s (91 min) | 49.9 GB |
| `laneb` | medical-04, maya-07, catia-07, solidworks-08, unreal_engine-01 | 3,746 s (62 min) | 43.9 GB |
| `both` | lane A then lane B | 9,230 s | 93.8 GB |

The cheapest viewset runs first so that a lane that runs out of time has already reported the most rows. `both` exists for a manual
run on a longer lease: with the payload's whole-lane time check (section 7) it never starts inside a 300-minute box (it needs 247
minutes left; **INFERRED** about 225 are left after the Windows gate).

## 5. The stage, step by step

1. **Preflight** (RETRY, never a verdict on the guest): root, `/dev/kvm`, the gate's `WINDOWS_GATE_PASS` for this tag and its overlay,
   the pin, at least 24 GB of RAM (the guest gets 16 GB: SPEC's own check wants about 15 GB, **MEASURED** 2026-11-05, and fails every
   workload at 8 GB), free disk (130 GB lane A, 125 GB lane B, 190 GB both, **INFERRED** from archive plus tree plus Windows; class
   PROVIDER, the disk request can be ignored by the rental).
2. **Host reference before** (the same-box host GPU reference of the gate: `nvidia-smi` and a CUDA matmul on the Linux host). Red:
   RETRY class HOSTREF and no guest starts.
3. **Boot A** (no kf3): the gate's overlay is sealed read-only and a fresh child overlay is created over it, so the gate's evidence
   stays as it was and the tree lands in a file the teardown deletes. Defender path exclusions, write access for the run user and the
   shipped `vcredist_x64.exe` (energy-04 fails `LoadLibrary Windows Error 126` without it) are set; these are the four traps of
   2026-11-05 that were harness, not kayfabe.
4. **Copy** (above), verify, delete the archive, stop the guest.
5. **Boot B** (kf3, the gate's flags, driver and per-VM identity), the signed-in desktop, then the **settle wait**: no CLI run
   sooner than 300 s after the boot (`settle_s`). **MEASURED** on two native boxes: the first CLI run within about 1 to 2 minutes of a
   reboot failed with `Unable to gather system details`; a rerun a few minutes later worked. The stage waits, and if a run still ends
   with exactly that message and no viewset started, it waits 120 s and runs that viewset **once more**; a second miss is NOT_RUN
   (RETRY class OPERATOR), never a pass and never a failure of the guest.
6. **Each viewset** runs once (`--iterations 1 --resolution FullHD`), started detached as a one-shot interactive scheduled task of the
   console user and polled every 20 s with short ssh calls (a hung or reset-stalled run cannot hold an ssh session). The reset
   counters come from the gate's own `gate_state.ps1 -Mode tdr` with the instant the run started, in the guest. A guest that died or
   stopped answering during a viewset is that viewset's failure. After a viewset that reset, hung (its CLI was killed at its budget) or
   took the guest down, the next viewset gets a fresh boot and settle wait (kf3 has no reset path, and a guest after a TDR is not
   evidence for the next viewset; on 2026-11-05 every viewset after catia-07 also ran in a fresh kf3 boot), at most 3 kf3 boots in all.
7. **Host reference after** (after the guest stopped, as in the gate), the host Xid delta, the verdict, the teardown.

## 6. Verdict rules

The rule is the apps stage's and the gate's: **the gate is the guest passing** (owner, 2026-11-01). `lane.py` holds them as pure
functions; each has at least two tests and a mutant killed by at least two of them (`spec_stage_mutants.py`).

| Outcome of a viewset | Row | Stage verdict |
|---|---|---|
| `Completed`, every subtest `Completed`, CLI exit 0, reset counters `total=0` | PASS | needed for every viewset of the lane |
| a subtest `Failed` or never `Completed`, or the listing names a subtest that did not run | FAIL `SUBTEST` / `NOTDONE` | FAIL |
| any reset (Display 4101, nvlddmkm 153, Kernel-Power 41, 6008, BugCheck 1001, LiveKernelEvent other than 193), even under a `Completed` viewset | FAIL `RESET` | FAIL |
| refused by SPEC's own validation (`Requirement:`, memory requirement, no compatible resolution) | FAIL `REFUSED` / `DISPLAY` | FAIL, never an exclusion |
| the CLI did not end inside the viewset's own budget (`budget_s`) | FAIL `TIMEOUT` | FAIL (a hang is kayfabe's) |
| the guest died or stopped answering during the run | FAIL `GUEST` | FAIL |
| a kf3 boot that fails minutes after the gate booted the same image on kf3 | FAIL `GUEST` for the first viewset not run | FAIL |
| the CLI exit code is not 0 under an otherwise `Completed` viewset | FAIL `RC` | FAIL |
| cut by the clock (the time left minus the tail reserve was below its own budget and it did not end) | NOT_RUN `TIME` | RETRY class TIME |
| not started: not enough time for its native wall with the margin plus the tail reserve | NOT_RUN `TIME` | RETRY class TIME |
| `Unable to gather system details` twice | NOT_RUN `GATHER` | RETRY class OPERATOR |
| the guest script answers `SPEC_LAUNCH fail` (our script could not start the task) | NOT_RUN `LAUNCH` | RETRY class OPERATOR |
| the launch call gets no answer at all | FAIL `GUEST` | FAIL |
| reset counters or the END record unreadable | NOT_RUN `NO_TDR` | RETRY class TRANSPORT |

`decide()` then adds the same-box reference exactly as the gate does: a FAIL with the host reference red before or after is RETRY
class HOSTREF; kf3 refusing a BAR1 size before any guest answered ssh is RETRY class BAR1_EXCEPTION (the documented Vast exception,
never a new excuse); a PASS with the reference red after is RETRY HOSTREF; a PASS with a new host Xid over the run is FAIL. A failure
beats a time shortfall: when one viewset FAILed and another could not start, the stage is FAIL. Setup failures are RETRY with a class:
MEDIA, PROVIDER, TRANSPORT (ssh, scp, the copy, a guest that does not hold the tree that was streamed), TIME, HOSTREF, OPERATOR, never
a pass. Exit codes: PASS 0, FAIL 1, RETRY 3, usage 2.

Why a viewset past its own budget is FAIL and one cut by the clock is RETRY: `budget_s` is `max(1200 s, 3 x native)` rounded
(INFERRED, never measured under kayfabe; the 2026-11-05 kayfabe runs of solidworks, maya, energy and medical took 0.8 to 1.1 times
the native wall). A viewset that outlives three times its native wall with the whole budget available is hanging; one that was handed
less than its budget because the box limit was near has not been shown to hang. A guest TDR that the guest recovers from is still a
FAIL (the reset counters), as in the gate.

Trust: the guest is hostile in this repository's model, and here, as in the Windows gate, the verdict reads the guest's own words (a
guest can print a forged `Completed`). The stage does not widen that: it executes nothing the guest sends, parses with fixed
patterns, and names only validated viewset and subtest names in its output.

## 7. Time

Per-viewset budgets are in `lanes.json` (`budget_s`). Before every viewset the stage asks `lane.py timeout <viewset> <seconds left>`:
`skip` when the time left minus the **tail reserve** (600 s: the graceful stop of the guest is allowed 420 s, then the verdict) is
below `1.25 x native_s`; otherwise `cut` (the time left minus the reserve, when that is below the budget) or `full` (the budget).
The check is made twice per viewset, before a boot that may not be worth starting and again after it, because a boot and the settle
wait take minutes and the viewset's budget is computed from the time that is left then. The stage's own deadline is the payload's soft
deadline minus 120 s. The payload does not start the stage at all when the **whole
lane** cannot plausibly finish: it needs `setup_s` (2,700 s, INFERRED: download 14 to 16 min, copy 18 to 21 min, boots) plus
`1.25 x` the native sum plus the tail: lane A 169 minutes, lane B 133, both 247. A shortfall is `specviewperf_retry_time` and no
download. INFERRED stage times on a fast host: lane A about 146 minutes, lane B about 114; the stage starts after the Windows gate
(**MEASURED** 53 minutes from the rental to a gate PASS on 2026-11-05 without the Linux bar; the payload's Windows stage assumes up to
75), so lane A is close to the box limit on a slow host. The first real run decides; the constants are one file.

## 8. What leaves the box

Policy (owner; `docs/SPEC_RESULTS.md`, `docs/DOWNLOADS.md`): **scores are not published**. The stage reads status lines only: the
CLI's log carries `Started` / `Completed` / `Failed` per subtest and no score (**MEASURED**, the native `cli.log` files in the trace);
the vendor result directory (`C:\spec\results\<viewset>`, where the scores are) is never opened by any script here and dies with the
guest. So there is no private artifact of scores: they are dropped, which is what the rule allows when the evidence path would carry
them. (A private copy of scores would be a separate owner decision and a separate lane; nothing here prepares one.)

- The streamed log (the payload log, GitHub's public log of the run): the wrapper `vast_stage_specviewperf.sh` keeps the lane's whole
  output in a 0600 file in a 0700 directory and prints its own start marker (`SPEC_STAGE_START lane=<lane|none>`, the lane only when it is one of the three) and lets through, as
  the lines arrive (so a run killed by the box limit has already reported what finished), only lines matching fixed anchored
  patterns: the list line, `SPEC_ROW name=<viewset> result=PASS|FAIL|NOT_RUN
  subtests=<done>/<total> reason=<WORD> failed=<subtest names>`, `<viewset> --- FAILED` (the payload's FINAL SUMMARY lists these),
  the counts line and `SPEC_COPY_DELETED`; the verdict line and the exit line (`SPECVIEWPERF_STAGE_EXIT rc=<n>`, always last) at the end, the class from a fixed list. A line with an extra field
  (`score=`, `fps=`) matches nothing. The stage prints its notes too; they go to the private file only.
- The evidence directory `<prov>/<tag>_spec/` (collected by the evidence bundle, which is private R2): the stage log, the host reference,
  and per viewset a **status-only extract** of the CLI log (`lane.py scrub` keeps only the lines of the status vocabulary), the END
  line and the reset counters. The guest's raw output stays in the wrapper's private directory (`KF_SPEC_PRIVATE`).
- The run report (`vast_run_report.py`, PR #282): one test row per viewset and one FAIL row per failed subtest under the suite
  `spec_viewsets`, no timing, in the Windows subrun; the stage's suite rows are renamed `spec_viewsets*`. **The Worker's
  `runreport.js` drops every string that names the benchmark (`FORBIDDEN = /specviewperf|viewperf/`), so a row named `specviewperf` would
  never arrive; the neutral name is how the owner's request for pass/fail in the report is met without touching that guard.** Whether
  the stage may be named in the report is the owner's call (OPEN, section 11); this PR does not change the Worker's filter.
- A SPECviewperf lane that could not finish (RETRY rows) is a RETRY of the Windows subrun in the report, not a failure of the guest.

## 9. The payload and the Worker (DISABLED BY DEFAULT)

`KF_SPEC` (`lanea`, `laneb`, `both`) is validated by the reporter and by the payload like `KF_WINAPPS` in PR #286, and popped before
any stage. It goes with the existing stage slot: the Worker sends `KF_STAGES=specviewperf` (release tier only, `releaseStages`) and
`KF_SPEC`; either alone, a lane that is not one of the three, the stage without the Windows stage and a stage outside the large
tier are refused before the first stage. Absent: today's run, byte for byte (the existing Windows and release tests are unchanged
except the three that pinned the old plumbing). With it, the payload sets `WINVM_DISK_GB=220` for the Windows gate (the install is a
sparse 220 GB disk; win_vm.sh's default 80 GB cannot hold a lane's tree next to Windows; an explicit value wins) and appends the stage
after `windows_gate` and `windows_gate_2`.

**Tier mapping (DESIGN, OPEN for the owner): small and middle never run it; large runs both lanes, on the two boxes of a die.** The
release planner (`ci/vast-broker/src/release.js`) already makes two legs on two machines when a die is offered twice. Proposed: the
first leg of a die sends `KF_SPEC=lanea`, the second `KF_SPEC=laneb`; no ad hoc `both`. A die offered on one machine has only the
single leg that runs the Windows gate twice; it cannot carry both lanes inside 300 minutes, so it needs a second rental of the same
machine for lane B (OPEN: owner decides whether that extra rental is paid, or the die is certified on one lane). Not implemented:
`tier.js` only gains an option that no caller passes (`tierEnvironment(..., {spec})`, tested), the planner is untouched, nothing is
deployed.

**OPEN, a hole to close before enabling:** `releaseVerdict` passes an enabled extra stage "on at least one PASS" per cell. With two
lanes a PASS of lane A alone would certify the stage. The payload therefore also writes `specviewperf_lanea` / `specviewperf_laneb` pass
rows next to `specviewperf`; the Worker must require both lanes on a die before it counts the stage as passed (a Worker change, not in
this PR). Likewise the Worker's rental must ask for the disk (300 GB requested, 220 GB floor, **MEASURED** by the native run; the
Worker rents 120 GB today, `ci/vast-broker/src/vast.js` `disk: 120`, `defaults.json` `diskGb`).

## 10. Tests and mutants

GPU-free, all in `scripts/ci/` (run by the `Vast box scripts` workflow, which discovers `test_vast*.py`):

- `test_vast_spec_lane.py` (101 tests): lanes, time guard, grading with replayed real evidence (the 2026-11-05 kayfabe medical, solidworks,
  maya, energy runs PASS; the kayfabe catia-07 run FAIL with the failed subtests named; the 2026-11-07 native catia-07 run PASS), the
  stage decision, the scrub, the tar filter, the command line.
- `test_vast_spec_stage.py` (123 tests): the REAL stage script against fakes (`spec_stage_fakes.py`: a fake `win_vm.sh` over a scenario directory,
  a fake fetch helper, a clock that moves only when the stage sleeps): happy path for each lane, inputs and media, preflight, copy
  (the filtered stream is captured and inspected), teardown (the archive and the overlay are gone after a pass, a failure, a setup
  failure and a TERM), every verdict row, the gather retry, the settle wait, the time guard, scores, the wrapper and a TERM.
- `test_vast_payload_spec.py` (72 tests): the grading contract, `KF_SPEC` requests and refusals, the lane table, the whole-lane time
  check, the stage through `main()` (order, command, disk size, retry rows).
- `test_vast_run_report.py`: the new `SpecStatusRows` class. `test_vast_payload_tiers.py` and `test_vast_payload_v7.py` were adapted
  (the lane is required).
- Mutants, each killed by at least two tests: `payload_mutants.py` (the 19 `spec-*` cases), `spec_stage_mutants.py` (74: the lane,
  filter, stage and wrapper rules), `run_report_mutants.py` (8 `spec-*` cases), plus the node tests of `tier.js`
  (`ci/vast-broker/test/tiers-release.test.js`).

CI (`.github/workflows/vast-box-scripts.yml`): the job `gpu-free` runs these tests with the rest of `test_vast*.py`; the stage mutants
run in their own job `spec-stage` (a run starts the real stage script a few thousand times). MEASURED on GitHub's runner for PR #291
(2026-11-07, `ubuntu-latest`, the 26.04 image): the baseline of 224 tests plus all 74 stage mutants took 4 min 54 s with 4 jobs.

## 11. What a first real run needs, and open questions

A first real run needs, in this order:

1. **Owner:** confirm the pin (`specviewperf.pin`, from the recorded `CHECKSUMS.txt` values) and that the private R2 objects exist
   and the Worker is deployed with the R2 secrets (`ci/vast-broker/DEPLOY.md`; the `/bench-data` route is code until then).
2. **A box:** a Vast RTX 4090 offer with `vms_enabled`, the KVM image and a **300 GB disk request** (check `df -h /` after boot: the
   request can be ignored), at least 34 GB of RAM, and a manual start with `KF_TIER=large KF_STAGES=specviewperf KF_SPEC=laneb`
   (lane B first: the smaller one), run as an admin start through the Worker or by setting those variables on a box by hand.
3. **A run token** with a live run for `/bench-data` (the stage cannot fetch without one).
4. Expect the first run to fix things the tests cannot see: the PowerShell of `spec_guest.ps1` and `spec_run.ps1`, the exact
   `tar` through the ssh default shell (`cmd /c "tar -xf - -C C:/"` was used by hand on 2026-11-05), the Defender exclusion, the task
   registration, the status poll cost during a loaded GPU, the real settle behaviour. Mark everything the first run measures here.

Open questions (OPEN): (a) which tier setting turns the stage on and who sends `KF_SPEC` per leg (section 9); (b) a die with one
machine; (c) the Worker's lane-aware release verdict; (d) the disk request; (e) whether `specviewperf` may appear by name in the run
report (the Worker filter); (f) the budgets and `setup_s` are INFERRED, the first run replaces them; (g) enscape-01 under
kayfabe was refused by SPEC validation (`raytracing`, `sufficientVideoMemory`, with `fb-mb` 8192 on a 24 GB card): the stage will fail
it, correctly, until kayfabe passes it natively-like; the native run passed it on the same die.
