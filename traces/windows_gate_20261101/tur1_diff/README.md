# TU106 gate failure vs the tur1 branch: a differential experiment (2026-11-01)

STATUS: LIVE, 2026-11-01 (RESEARCH). Written BEFORE the hardware runs (the hypotheses and falsifiers
below are the pre-registration; results are appended below the line "RESULTS").

## Question (owner): "I thought we got Turing working in other branches, or didn't we?"

Candidate A = `origin/claude/windows-gate-20261101` at `f9883ecc` (the gate head, PR #238).
Candidate B = `claude/tur1-gate-candidate-20261101` = A + a real merge of `origin/claude/tur1-20261027`
(`0b44315e`), merge commit `1581edb4`.

## Step 1 (CODE READ and a GPU-free build, 2026-11-01): what of tur1 is really missing from A

`git cherry` and a reading of the merge conflicts show that the Turing FIXES of tur1 are already in A under
other SHAs (they landed as PRs #161, #162, #164, #168 against `next`):

| tur1 commit | what | in A as |
|---|---|---|
| `67339dc0a`, `285525f90` | Xid guard accepts the Turing `HUBCLIENT_HSCE` spelling | `e6039dc33` (#161) |
| `f958afe20`, `8eaf158c2` | the Windows USER ring admits GPFIFO class 0xc46f | `cf58769c0`, `d0dadb55b` (#162) |
| `a7162f2a9`, `267d93a74` | Turing window semaphores are 32-bit, one context DMA; `display=on` realizes | `c7e085b3d`, `12f9c679a` (#164) |
| `340c6b5a1`, `e98475ef2` | a worker that finds its slot locked gives the ring back (`HostIdx`, `owe_ring`) | present in A (`hostidx.rs`, `reowe`, `hostidx_model.rs`; `git cherry` marks both as already upstream) |
| `29acd5188`, `a1cd0e628` (+ tests) | `WINVM_KF3_DISPLAY=off`, `windows_lane.sh`, `WINVM_INSTALL_CACHE` | present in A (conflicts resolved to A's newer copies) |

Missing from A (the whole net code delta of B over A, +207 lines in kf-chan and kf-qemu, no unsafe):

| tur1 commit | what |
|---|---|
| `33d7a8f0a`, `385c9285f` | the completion probe dump says whether the host finished and no pump saw it (`wake_verdict`) |
| `d0a209e27`, `1e1052171` | the probe names the cause of a lost wake from wake and pump timestamps (`lost_wake_cause`, `Completions::note_wake/last_wake`, `TranslatedChannel::probe_last_pump`) |
| `b439ae820` | the probe dump prints the channel's token word |
| `49b4844c0` | CI trigger only |

All of it is DIAGNOSTIC and active only with `KF3_COMPLETION_PROBE=<ms>`, which the gate's flag file does not set.
The remaining tur1 delta is evidence under `traces/tur1_20261027/` (six files not in A).
`cargo test -p kf-chan -p kf-qemu -p kf-disp` on B: all `test result: ok`.

Consequence, stated before any run: B and A execute the same product code in the gate unless the probe flag is
added, so a PASS/FAIL difference between A and B on the same box is not expected from code. The tur1 README
itself (`traces/tur1_20261027/README.md`, leases 57882297, 57908255) records the same reset storm on TU106 and on
GA106 with these fixes in, and says "the Windows guest on TU106 is NOT yet stable".

## Hypotheses (pre-registered)

- H1: the TU106 gate failure is caused by Turing code on tur1 that A lacks, so the gate passes (or gets
  materially further: CUDA probes pass) with tur1 merged. FALSIFIER: the gate FAILS the same way (CUDA 999 /
  RM unload storm) on B on a fresh Turing box. Prior (INFERRED from step 1): H1 is unlikely.
- H0: the failure is the box (or something else): if A PASSES on the fresh box, the earlier failure was
  box-specific.
- Reading rule: A FAIL and B FAIL with the same signature falsifies H1; A PASS falsifies "Turing fails the gate"
  (box- or die-specific); A FAIL and B PASS supports H1 (then the probe flag, not the merge, is the confound to
  test).

## Deviations from the task statement (stated, not hidden)

- No US Vast offer with `vms_enabled=true` and a Turing die existed on 2026-11-01. Of the 49 KVM-capable
  single-GPU offers listed, the only Turing ones were a GTX 1660 SUPER (Quebec CA) and a GTX 1660 Ti (UK); no
  RTX 20xx, TU102, TU104 or TU106 KVM offer was listed in about 15 searches over 11 minutes. The lease is a GTX 1660 SUPER
  (TU116, Turing, 6 GiB, 5 vCPU), not a TU106: it tests Turing generally, not the same die.

## RESULTS (MEASURED 2026-11-01, Vast lease 58705804, GTX 1660 SUPER = TU116, Quebec CA)

Box: 5 vCPU, 49 GB RAM, 146 GB root disk, host driver 595.91.07 open (`a1/hostref_before.txt`), 0.094 USD/h
with disk, created 20:01Z, destroyed 21:43Z (about 1 h 42 min; about 0.16 USD, INFERRED from the rate: the
account credit is shared with other sessions). Lane: `a1/winlane.log` `WINLANE_TIMING total=2225s ...
install_windows=1368s`. The Windows install was done once (on A's checkout) and reused; each gate run used its own
fresh overlay and a fresh kf3 build of its revision.

| run | revision | flags | verdict | host reference | guest evidence |
|---|---|---|---|---|---|
| a1 | A `f9883ecc` | the sample list (19) | `WINDOWS_GATE_FAIL diag=KAYFABE stage=smi_probes why="graded probes failed: d3d11"` after 945 s (`a1_windows.log`) | green/green, `cup8=PASS`, Xid 0 (`a1/hostref_*`) | Code 0 and `nvidia-smi` worked; `cup8` PASS (7 s); then `kf_dxprobe d3d11` died with the guest: `GUEST_PANICKED`, bugcheck 0x113 (arg2 0x2b), at 21:02:40Z, 3 min 39 s after the enable (`a1/gate_a1_b2_qmp_events.jsonl`); 7 RM unloads (`a1/gsp_signals.txt`) |
| b1 | B `1581edb4` | the sample list plus `KF3_COMPLETION_PROBE=1000` | `WINDOWS_GATE_FAIL diag=KAYFABE stage=smi_probes why="graded probes failed: cup8"` after 1120 s (`b1_windows.log`) | green/green, `cup8=PASS`, Xid 0 (`b1/hostref_*`) | Code 0 and `nvidia-smi` worked; `cup8` `FAIL cuInit(0) -> unknown error (999)`, `cup3` `cuCtxCreate` 999, `d3d11` and `d3d12` OK (`b1/probe_*.txt`); 20 RM unloads; at the failure `nvlddmkm153=36 livekernelevent_other=71 livekernelevent193=13` (`b1/state_at_failure_tdr.txt`) |
| control | `1581edb4` | bare metal, same box | `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, births 12 of 12 USER (`v3_gates_tu116.log`) | | |

a1 afterwards. The gate's own state collection found a dead guest (`ssh` refused), and the supervisor of
`win_vm.sh run` had already started a fresh QEMU (boot 3), which also panicked at 21:06:18Z (seen in the box's
`gate_a1_run.log`; that file was not copied before the box was destroyed). The ssh attempts of my post-hoc
script into that guest timed out (banner exchange). Reading the stopped overlay offline (read-only NTFS mount, text only,
no dump copied) gave: `Windows/LiveKernelReports/WATCHDOG-20261101-2059.dmp` (1.39 GB, 20:59:34Z, the
LiveKernelEvent 193 dump, 33 s after the enable at 20:59:01Z), one archived LiveKernelEvent report of code 117
with FILETIME event time 21:01:49Z (`a1/ReportArchive_item1.wer`; code 117 is the video-TDR class), and
`MEMORY.DMP` plus one minidump from the bugcheck (`a1/offline_listing.txt`). The `ReportQueue` directory was
unreadable offline (NTFS I/O error), so the code-193 Report.wer text of A is NOT collected. Timeline A (guest clock
= UTC): enable 20:59:01, 193 dump 20:59:34 (+33 s), code-117 report 21:01:49 (+168 s), first probe 21:01:55,
bugcheck 21:02:40 (+219 s).

Timeline B (`b1_windows.log`, `GATE_TIMELINE`): enable 21:31:56, first 193 dump 21:32:33 (+37 s, 1.38 GB, 145 s
before the first probe), first `nvlddmkm 153` 21:32:59 (+63 s), first probe 21:34:58 (+182 s), `cup8` fails.
`b1/state_at_failure_wer.txt` holds the Report.wer text of the LiveKernelEvent reports (code 117).

New probe output exercised on B (`b1/gate_b1_b2_qemu.log.gz`, MEASURED): 1284 `PROBE-DUMP` lines, with the new
`verdict:` and `token word:` lines; 0 `HOST-FENCE-OVERDUE`; the dumps are `GUEST-SILENT-AFTER-COMPLETION` with
`verdict: no fence in flight` and token word `Idle`; the ring-return counter `contended` reached 1612 and 4679
on different samples. INFERRED: on this die the storm is not the "lost completion" class that tur1's probe was
built to name (no overdue fence is printed), and the ring-return fix is exercised heavily (it is in A as well).

## Verdict on the hypotheses

- H1 (tur1 code that A lacks fixes the Turing gate failure): FALSIFIED on this box. B, which is A plus everything
  tur1 has, fails the gate with the TU106 signature (`cup8` `cuInit` 999, `cup3` `cuCtxCreate` 999, RM unload
  storm, `nvlddmkm` 153) with the host reference green before and after and the V3 gates 9/9. A fails too. Step
  1 explains why: the Turing fixes are in A already; B adds only the probe.
- H0 (the earlier TU106 failure was box-specific, A would pass on a fresh box): NOT SUPPORTED. A fails on a fresh
  Turing box (a different die, TU116). Not decidable for TU106 itself: no TU106 KVM box was on offer. Caveat
  (INFERRED): a1 (guest bugcheck 0x113 during d3d11, `cup8` passed) and b1 (CUDA 999, d3d11 passed) differ in the
  failure point, as the two boots of tur1's lease 57882297 and t1/t2 on TU106 did: one class (reset storm after
  the enable), not one deterministic failure point.
- Confound stated: b1 ran with `KF3_COMPLETION_PROBE=1000` and a1 without it. The probe's code is merged in B but
  off by default; a1 failed without it, so the probe is not needed to fail; it was not shown to be needed to pass.
- The WATCHDOG (code 193) falsifier of `docs/design/V3_WINDOWS_GATE.md` ("a 193 dump precedes a reset, a TDR or a
  probe failure") IS triggered on both Turing runs here: the 193 dump came 33 s and 37 s after the enable, before
  the first reset-class event (A: code 117 at +168 s; B: `nvlddmkm` 153 at +63 s). On the AD102 it preceded
  passing probes only. n=2 against n=2: this does not show that 193 causes the storm, only that it is not
  harmless on every die.
- Negative boots spent: a1 boot 2, a1 boot 3 (the supervisor's restart, which also panicked) and b1 boot 2:
  three, so the rule stopped the lease.

## Gate-script findings from a1 (OPERATOR class, not a guest result; no script was changed here)

- After the guest panicked, `win_vm.sh run` started a fresh QEMU (boot 3) while the gate was concluding; the gate
  exited (rc=1) with that QEMU still alive under the supervisor (pid 77628 was running 5 minutes after
  `WINDOWS_GATE_EXIT` and had to be stopped with `win_vm.sh stop`). A following gate run would have contended for the
  GPU.
- `state_at_failure_wer.txt` and `state_at_failure_tdr.txt` were not collected for a1 because the guest was dead
  when `conclude` ran, so a failure by guest panic has no WER text in the evidence.
