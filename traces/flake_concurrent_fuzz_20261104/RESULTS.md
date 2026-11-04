# Flake hunt results: `--concurrent-fuzz` and `--bus-info-sweep` in the thin fast guest

**STATUS: ANSWERED, 2026-11-04.** No failure reproduced in this run. The pre-registration is in
`PREREG.md` (with two amendments), the per-iteration data are in `box_22775/` and `box_30524/`,
and the offline replay is `control_sim.py` / `check_sim.py`.

## Boxes and revision (measured)

| lease | machine | GPU | rev under test | role |
|---|---|---|---|---|
| 59051876 | 22775 (Quebec, the CI failures' machine) | RTX 3060 (GA106), host 580.159.04 | `d6066c8a6` (origin/next, contains #241 and #248) | phase A, suite × 3, BI, bare-metal seed replay |
| 59055125 | 30524 (UK) | RTX 3060 (GA106), host 580.159.04 | `d6066c8a6` | suite × 2, A2, BI2 |

Both were provisioned with `provision_full.sh` (`KAYFABE_BRANCH=next`, `READY`, `KF3_RC=0`). The
kf3 binary is `kf3-bins/d6066c8a/`.

## Results (measured)

| box (machine) | phase | what | iterations | non-PASS | REFUSED walk / CUDA_ERROR lines | host Xid in the arm's window |
|---|---|---|---|---|---|---|
| 22775 | A | `--concurrent-fuzz` alone, fresh QEMU each | 40 | **0** | 0 | 0 |
| 22775 | S | whole 30-arm thin suite, suite order | 3 runs (90 arm boots) | **0** (30/30 ×3) | 0 arms | 5 per run, all Xid 31 `MMU Fault: ENGINE CE`, 4+1 in two boots (the suite's intentional-fault arms) |
| 22775 | BI | `--bus-info-sweep` alone | 8 (a 9th was cut off by the stop and is discarded) | **0** | 0 | 0 |
| 30524 | S2, S3 | whole 30-arm thin suite | 2 runs (60 arm boots) | **0** (30/30 ×2) | 0 arms | 5 per run, the same Xid 31 pattern |
| 30524 | A2 | `--concurrent-fuzz` alone | 12 | **0** | 0 | 0 |
| 30524 | BI2 | `--bus-info-sweep` alone | 12 | **0** | 0 | 0 |

**Per arm, all boxes:** `--concurrent-fuzz` ran **57** times (22775: 43, 30524: 14) with 0
failures. `--bus-info-sweep` ran **25** times (22775: 11, 30524: 14) with 0
failures. The whole suite ran 5 times, 150 arm boots, with 0 failures.

**Upper bounds (rule of three, 95 %, valid only for this revision, d6066c8a6):**

| arm | where | n | bound |
|---|---|---|---|
| concurrent-fuzz | 22775 | 43 | p < 7.0 % |
| concurrent-fuzz | both machines | 57 | p < 5.3 % |
| bus-info-sweep | 22775 | 11 | p < 27 % |
| bus-info-sweep | both machines | 25 | p < 12 % |
| any arm, whole suite | 22775 | 3 runs | p(suite red) < 100 %, not informative; the CI observation was 2 reds in 3 runs |

The fuzz client's own wall time in phase A was 6020–11160 ms (median 8930). The CI red's 8620 ms
lies inside that range, which supports the inference that the red run went to completion and was
graded rather than wedged.

**The replay was checked against every measured control.** `check_sim.py` compared the
control outcome `control_sim.py` predicted from each iteration's printed seed with the measured
control line: **52/52 agree** (A, A2), and the 5 suite seeds agree too. None of the 57 seeds was a
predicted no-engine seed, as expected for p≈0.8 % (P(none in 57) ≈ 64 %).

**The bare-metal replay of predicted seeds (measured, 22775, 05:15 UTC, `box_22775/BM/`).**
`kayfabe-rm-ladder --gpu 0 --concurrent-fuzz --seed S --fuzz-threads 3 --fuzz-iters 64
--fuzz-clients 2 --probe-launch-dma` ran on the host, with no kayfabe involved:

| seed | replay predicted | measured | control census measured (= predicted for the red seeds) |
|---|---|---|---|
| 9187369680131679237 | no engine op | **rc=1 `FUZZ_REASON=CONTROL_FAILED`** | alloc=3 map_a=1 map_b=1 free=3 recycle=1 idle=55 |
| 2676526539383654871 | no engine op | **rc=1 `CONTROL_FAILED`** | alloc=4 map_a=1 map_b=1 free=2 recycle=1 idle=55 |
| 4165894845459366051 | no engine op | **rc=1 `CONTROL_FAILED`** | alloc=8 map_a=2 free=5 idle=49 |
| 2946041849437773955 | no engine op | **rc=1 `CONTROL_FAILED`** | alloc=6 map_a=2 map_b=2 probe=3 free=4 recycle=2 idle=45 |
| 5827511587167918413 | engine reached | rc=0 CLEAN | |
| 2978884326927896233 | engine reached | rc=0 CLEAN | |
| 4600381545837863385 | engine reached | rc=0 CLEAN | |

7/7 as predicted, and the four red censuses match the replay op for op. The exit code is `rc=1`,
the same as in the CI red.

## Hypotheses

- **H1 (an arm went unmeasured, `ARM_UNMEASURED`): not observed.** 0 of 57 fuzz runs. Every
  arm had overlap > 0 and engine ops > 0 in every run, so it is neither confirmed nor falsified as
  the CI red's cause.
- **H1b (the seed-drawn control has no engine op): confirmed on hardware as a mechanism.**
  On bare metal, 4/4 predicted-red seeds went red (`rc=1 CONTROL_FAILED`, the censuses exactly as
  predicted) and 3/3 predicted-green seeds passed. The replay also reproduces the 2026-09-26 measured census exactly. It agrees with all
  57 measured controls in this run (none was a no-engine seed). It predicts a red on **0.79 %** of
  seeds, on any machine and on bare metal. ⇒ **This is a known, standing instrument defect that by
  itself turns the arm red about once in 126 runs.** Whether *the* 41763934006 red was this one
  **cannot be measured**: the seed and FUZZ_REASON are not in the CI log, and the run has no
  artifacts. Inferred as the most likely cause, because client_ms is normal, rc=1 is the graded
  exit (not the watchdog's 3 and not a panic's 101), and nothing else reproduced. Proposed fix:
  branch `claude/flake-concurrent-fuzz-fix-20261104`, commit b22cda760, **UNTESTED on hardware**.
  The control steps to the first seed derived from `--seed` whose replay reaches an engine op,
  and prints that seed. It has unit tests, and the measured red seed is recognised. **Better
  still, the CI payload should keep the arm's serial log (or at least its `FUZZ_*`/`W385
  control` lines and the seed) for every red arm.** Without them neither red can be attributed.
- **H2 (a real invariant violation in kayfabe): not observed.** 0 of 57. Upper bound above.
- **H3 (a host Xid in the fuzz window): falsified for every run here.** The fuzz and bus-info
  windows had 0 host Xids in all 72 single-arm boots (52 fuzz, 20 bus-info). In the suites, the only Xids are
  the 5 intentional Xid 31 CE MMU faults per run, in two boots. CI's xid-watch also reported
  CLEAN for the stages it printed. (Its merge_bar line was trimmed.)
- **H4 (#241 walk wake-up is involved).** For the observed concurrent-fuzz red it is
  **falsified by ancestry**: rev 942ca5431 lacks 4f420b373. ⚠ **That cuts both ways, and it is
  the main limit of this hunt.** Both CI reds ran product code *without* #241: 942ca5431 and
  e7d6a3fa have identical `crates/`, `qemu/` and `cuda/` (`git diff --stat 942ca5431 e7d6a3fa`
  is empty for them). Both equal d6066c8a6 minus exactly #241's walk-wake files
  (`kf-cuda/src/{completion,walk}.rs`, `kf-linux-raw/src/{epoll,timerslack}_unsafe.rs`,
  `kf-qemu/src/device.rs`). **Every run here tested d6066c8a6, which contains #241.** So this hunt
  did not run the failing product code. A walk-timing change could plausibly shrink a race window
  behind the CUDA 700. That is an inference, and it is not tested.
- **H-machine vs H-kayfabe: unresolved.** Neither machine failed (22775: 0 of 43 fuzz, 11
  bus-info and 90 suite arm boots; 30524: 0 of 14 fuzz, 14 bus-info and 60 suite arm boots). H-machine's
  falsifier (30524 fails) did not fire. H-kayfabe's falsifier ("neither fails") fired *for
  d6066c8a6 only*. The CI reds were on the pre-#241 code. The `--bus-info-sweep` red's kf3 line
  (`REFUSED walk: cuEventQuery refused: 700 (CUDA_ERROR_ILLEGAL_ADDRESS) [walked 1:
  0xc1e000070000000a=s3@0x1efa8c000 …]`, CI run 41768641016, t=17.099 s, on the first walk of a
  freshly declared VAS root) is a GPU-side illegal address inside the walk path. **It is never a
  harness failure**, and the walker's bounds checks (`cuda/walk/kf_walk.cu` I2) should make it
  impossible for any guest table content. It stays an open kayfabe defect, unexplained, with a
  measured CI rate of 1 in the 3 runs on 22775 at e7d6a3fa/942ca5431 and 0 in this run's 25 bus-info
  boots at d6066c8a6.

## Next step (proposed)

1. Run the same loops on **942ca5431 (= e7d6a3fa product code)** on 22775: `--bus-info-sweep` ×
   40 and the suite × 3. A red there and none on d6066c8a6 would point at the code without #241
   (the walk poll timing). It needs a fresh build of that revision (about 20 minutes on this box).
2. Make the CI payload keep the full serial and kf3 logs of every red arm (gzip, a few hundred KB).
   Neither CI red can be attributed today.
3. Review and then hardware-test the H1b fix branch.

## Cost and teardown

Lease 59051876 (22775, $0.166/h, 03:41–05:16 UTC, about $0.26) and lease 59055125 (30524,
$0.140/h, 04:16–05:11 UTC, about $0.13): **about $0.39 in all.** Both were destroyed by exact id,
and `vastai show instances` then listed neither.

## Traps hit in this run

- Copying a new `cfuzz_loop.sh` over the running one (scp) made bash read the old offsets of the
  new file. Phase A's 40 rows were all written, but its `CFZ_EXIT` line was not (`syntax error:
  unexpected end of file` in `chain.nohup`). Never overwrite a script that is still running.
- The CI rev 942ca5431 lacks PR #241 (git ancestry), so "the integration tree plus the Windows
  harness" did not hold for `crates/` in that run.
