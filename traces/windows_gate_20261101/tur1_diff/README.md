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
