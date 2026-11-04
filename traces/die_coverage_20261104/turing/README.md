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

### Verdict (2026-11-04, rev `0c515b3b3`, host driver 580.159.04 open on both boxes)

| die | SKU, PCI id | box (own id) | host driver as found -> run | v3 gates | bare metal | thin guest 30 arms @180 s | census | Windows gate |
|---|---|---|---|---|---|---|---|---|
| TU106 | RTX 2060 SUPER, `10de:1f06` | 59031420, Sweden, 11 vCPU, 24 GB, 146 GB root | 575.51.03 -> 580.159.04 | 9/9, gate10=PASS | 30/30 | **30/30** | OK | **PASS** (`tu106a1`) |
| TU116 | GTX 1660 Ti, `10de:2182` | 59031423, UK, 7 vCPU, 24 GB, 146 GB root | 575.51.03 -> 580.159.04 | 9/9, gate10=PASS | 30/30 | **30/30** | OK | **PASS** (`tu116b1`) |
| TU104, TU102, TU117 | - | - | - | not run: no VM offer (see *Offers seen* and *Scans*) | | | | |

- **H-walkwake-TU106: not falsified.** MEASURED (`tu106_box59031420/tu106a1.log`, run `tu106a1`, 2026-11-04, rev
  `0c515b3b3`): `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30`; `ce-client-guest-ram
  verdict=PASS secs=88 client_ms=71929` (`tu106a1_suite.out`), against `TIMEOUT secs=185` in tbA1 (rev `08dae8a3`,
  another TU106 machine, same host driver). Other walk-heavy arms moved the same way: `concurrency` 28 s / client
  11,680 ms (tbA1: 82 s / 54,879 ms), `concurrent-fuzz` 20 s / 3,969 ms (tbA1: 50 s / 22,450 ms). Client time summed
  over the 30 arms: 159,885 ms (tbA1: 245,971 ms over 29 arms, the timed-out arm excluded). The kf3 log of every
  arm carries `pending-walk wait 50us..1ms (epoll_pwait2) (timer slack 5us)`; per-walk latency `vat[walk_avg_us]`
  is 149-254 us on the arms with walks (`tu106a1_armstats.txt`: `ce-client-guest-ram` 254 us, GPU 191 us;
  `concurrency` 158 us, GPU 136 us). GPU nonstall: `nsi=[CE2:823/0raised]` on `concurrency`, `CE2:4/0raised` on
  `ce-client-guest-ram` (the counts the walk-wake fix must not inflate; no per-walk nonstall burst). INFERRED: the
  earlier TU106 miss was `next`'s 1 ms pending-walk poll, as on AD104; tbA1 ran on another machine, so the
  machine is a second variable (n=1 per side).
- **H-gate-TU106: not falsified (n=4 with the three runs of 2026-11-03).** MEASURED
  (`tu106_box59031420/tu106a1_windows.log`): `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 2060 SUPER"
  idle_s=300 hostref=green/green secs=1282 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0
  known_live_dumps_193=3 stale_lke_resubmits=0 strict=0`. kf3 (`tu106a1_windows/gate_tu106a1_b2_qemu.log.gz`): `caps
  page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)`, window 0 scanned at +176.2 s;
  `gsp_signals.txt`: `running_to_suspending=1`, `host_xid_in_boots=0`; `tdr_counts.txt`: every reset counter 0,
  `livekernelevent193=3` (reported, not gated).
- **H-TU116-row: not falsified. First Windows gate PASS on TU116.** MEASURED (`tu116_box59031423/tu116b1.log`, run
  `tu116b1`, 2026-11-04, rev `0c515b3b3`): `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, `KF3_RC=0`, `BARE_SUITE_PASS=30`,
  `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK arms=30 suite_births=165 gate_births=12`, and
  (`tu116b1_windows.log`) `WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce GTX 1660 Ti" idle_s=300
  hostref=green/green secs=1926 ... code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=2`. kf3: `caps page
  derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)`, window 0 scanned at +250.4 s,
  `running_to_suspending=1` (the stop only), `host_xid_in_boots=0`, every reset counter 0. The storm shape of
  2026-11-01 (no window PUT after the modeset, an RM unload every ~25 s) is absent. INFERRED: the TU116 T3 `0x00c` row
  (measured on a GTX 1660 SUPER, `10de:21c4`) serves this GTX 1660 Ti (`10de:2182`) because rows are keyed by
  arch/impl (0x160/0x8), not by PCI id; the log's `1 T3 measured (0 unmeasured)` shows the row was used.
- Host Xids: 10 per box, all from the two Linux suites (bare metal + thin guest), 2 x the 5 admitted shapes
  (`tu116_box59031423/host_xid_lines.txt`: Xid 31 CE `FAULT_PTE` at `0x60_/0x62_/0x62_/0x64_00010000` and one
  `FAULT_PDE VIRT_WRITE` at `0xa_11000000` per suite); the Windows gate's host reference read `xid_delta=0` before
  and after on both boxes.
- Lane timings (`*/winlane.log`): TU106 1822 s (install 1257 s), TU116 2725 s (install 1566 s); merge check incl.
  Windows: TU106 00:54:58Z -> 01:32:05Z, TU116 01:04:32Z -> 01:56:55Z.

### Findings

- The slowest arm on TU116 (7 vCPU box) is `ce-client-guest-ram` at 125 s of the 180 s budget (client 104,199 ms);
  on TU106 (11 vCPU) 88 s. Margin 55 s. MEASURED, n=1 each; not a failure. INFERRED: the remaining cost is per-box
  (nested KVM exits), walk latency is in the fixed range on both (TU116 173-256 us, `tu116b1_armstats.txt`).
- Unmeasured on this lane: TU104, TU102, TU117 (no Vast VM offer during the lane); no PRECOMP or GSS-size question
  arises for Turing in these runs (kf3 logged no `NO MEASURED` line in either gate boot).
