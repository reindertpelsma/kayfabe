# Die coverage, Turing, on the integration revision (2026-11-04)

STATUS: RESEARCH, 2026-11-04. Lane 3 (owner, 2026-11-04), family Turing. The pre-registration below was written and
pushed BEFORE any run on the boxes; results are appended under "RESULTS". MEASURED = a line in a committed file in
this directory, named with its file; INFERRED = reasoning.

## Revision under test

`0c515b3b3` (`origin/claude/integration-20261104`: `next` + the Windows gate + Turing/Blackwell/Ampere fixes + caps
rows + the walk wake-up fix, PR #241). The boxes build from `claude/die-coverage-turing-pin-20261104`, a branch that
points at exactly `0c515b3b3`, so `merge_check.sh` and `build_kf3.sh` record `HEAD=0c515b3b3...` whatever the
integration branch does later. Evidence is committed on `claude/die-coverage-turing-20261104`.

Per die: `windows_lane.sh` (host driver, kf3 build, Windows install; `HOST_DRIVER=580.159.04`, the host driver of
the earlier TU106 29/30 run, `traces/turing_boot_20261103/boxA/`, so the revision is the one variable that changed
there), then `merge_check.sh` with `KF_MERGE_WINDOWS=1` (v3 gates, kf3 build, raw client and fast guest rebuilt,
`fast_suite.sh` 30 arms at 180 s, birth census, Windows gate). If the fast suite misses an arm, `merge_check.sh`
stops before its Windows stage, and the gate is then run by hand from the same checkout
(`bash scripts/bench/box/windows_gate.sh <tag>`), as in `traces/turing_boot_20261103/` tbA2.

## Hypotheses and falsifiers (pre-registered)

- **H-walkwake-TU106**: with the walk wake-up fix, the TU106 thin guest is 30/30 at 180 s. The earlier TU106 run
  (`traces/turing_boot_20261103/boxA/tbA1_suite.out`, rev `08dae8a3`, host 580.159.04) was 29/30: `ce-client-guest-ram
  verdict=TIMEOUT secs=185`, and the arm alone took 310 s (client 282 s), against 203 s before `next`'s walk-latency
  regression. PREDICTION: `FAST_SUITE_PASS=30 ... ARMS=30`, and `ce-client-guest-ram` well under 180 s (INFERRED from
  the AD104 measurement in `traces/walk_wake_20261103/`: 62.9 s -> 21.3 s there; no Turing number exists).
  FALSIFIER: `ce-client-guest-ram` (or any arm) TIMEOUT/FAIL/CRASH again. On the falsifier: per-arm timings and the
  GPU nonstall interrupt counts as in `traces/walk_wake_20261103/` (`vat[walk_avg_us]`, `gpu_avg_us`, the kf3 log's
  pending-walk-wait line), and the arm alone at 600 s.
- **H-gate-TU106**: `WINDOWS_GATE_PASS ... resets=0` on TU106 at this revision (n=3 PASS at `e6dad00f`/`c71c07cb`/
  `0aaba25c`, the same derived page). FALSIFIER: `WINDOWS_GATE_FAIL`, any reset counter > 0, or
  `caps page derived ... (u unmeasured)` with u > 0.
- **H-TU116-row**: TU116 now has a T3 `0x00c` row (`crates/kf-disp/data/caps_measured.tsv`, arch 0x160 impl 0x8,
  measured 2026-11-03 on a GTX 1660 SUPER; the 2026-11-01 TU116 FAIL ran the authored page, not the derived one).
  PREDICTION: the Linux bar is green (9/9 + gate10, 30/30, census OK), kf3 logs `caps page derived — ... 1 T3 measured
  (0 unmeasured)` for the TU116, and `WINDOWS_GATE_PASS ... resets=0`. FALSIFIER: any Linux bar line missing or
  different; `unmeasured` > 0; `WINDOWS_GATE_FAIL`; or a window never scanned after the modeset with RM unloads before
  the stop (the 2026-11-01 storm shape). On a FAIL, the first evidence (serial log, kf3 log, Xid, event IDs) is kept,
  one re-run on a fresh box at most, and at most 3 negative boots on the question.
- A `WINDOWS_GATE_RETRY` is not a verdict on the guest: re-run on a fresh box and fix the script.

## Offers seen (2026-11-04T00:05Z, `vms_enabled=true num_gpus=1`)

Turing offers with VMs: RTX 2060S (Sweden, 0.135 USD/h, 12 vCPU, 31 GB), GTX 1660 Ti (UK, 0.069 USD/h, 8 vCPU,
31 GB), RTX 2060 (Quebec CA, 0.534 USD/h, 96 vCPU). None for TU104 (RTX 2070 SUPER/2080/2080 SUPER) or TU102
(RTX 2080 Ti, Titan RTX) or TU117 (GTX 1650). Without the VM filter Vast listed 10 RTX 2080 Ti, 3 Titan RTX,
2 RTX 2070S, 6 RTX 2060, 4 RTX 2060S, 3 RTX 2070, 4 GTX 1660 S, 4 GTX 1660 Ti, 1 GTX 1660, 1 Tesla T4: plain
containers, which cannot run a kf3 guest.

## Boxes (own ids only)

| box | id | GPU | offer | where | created |
|---|---|---|---|---|---|
| A | 59031420 | RTX 2060 SUPER (TU106 expected) | 58972867 | Sweden SE, 0.135 USD/h | 2026-11-04T00:09:38Z |
| B | 59031423 | GTX 1660 Ti (TU116 expected) | 50291703 | UK, 0.069 USD/h | 2026-11-04T00:09:41Z |

## RESULTS
