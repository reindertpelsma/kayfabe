# Turing: the derived caps page booted (Windows gate) and the Linux bar on the same die (2026-11-03)

STATUS: RESEARCH, 2026-11-03. The pre-registration below was written and pushed BEFORE any run on the boxes;
results are appended under "RESULTS". MEASURED = a line in a committed log, named with its file; INFERRED =
reasoning; COMPUTED = a GPU-free computation from the committed code (reproducible, named).

## What is booted

Branch `claude/turing-boot-20261103` = `claude/turing-windows-fix-20261101` (gate PR #238 + the Turing caps
fix) + `c4fc0790` (drops the `KF3_DIAG_CAPS_PAGE_FILE` diagnostic, do-not-merge) + `5676fd89` (merge of `next`
after PR #239, Blackwell). In the merge, `kf_disp::capsderive` and the kf-qemu display construction are
`next`'s: the same per-die derivation (the T3 TU106 `0x00c` row is byte-identical), plus the T2 host snapshot
hook and the precomp field rules of the Blackwell branch. No `KF3_DIAG_*` variable is set in any run.

COMPUTED (`derived_vs_d2_page.txt`, the scratch dump of `kf_disp::caps::sdr_page` at `67f4b2ed` and of
`capsderive::derive(TU106, sdr=true, branch 580, HostFacts::default())` at `5676fd89`): the page this head serves
a TU106 is NOT the d2 page that passed on lease 58721898. It has 56 words against d2's 44, and 12 words differ:

- `0x680 + 0x20*h` (`POSTCOMP_HEAD_HDR_CAPA`, 4 heads): bit 9 set (`UNIT_WIDTH` = 16) — from `next`'s authored
  base page (the authored page alone changed from 43 to 47 words). The TU106 native page has `UNIT_WIDTH` = 14
  and `FULL_WIDTH` = 16 there.
- `0x788 + 0x20*w` (`PRECOMP_WIN_PIPE_HDR_CAPC`, 8 windows): `CSC00_PRECISION` = `CSC01_PRECISION` = 14 — the
  T1 "inferred virtual precision" rows of `data/caps_rows.tsv` from `next`. The TU106 native page has these
  bits on the main-layer windows and `0` on the overlay windows.
- The two bisected facts (`0x00c` = 0x100, main-layer `SCLR_PRESENT`) are present, as in d2.

The T2 host facts at run time (`kf_qemu::rmfacts::display_precomp`) are not in this computation; the kf3 log
line `caps page derived — N words: a T1 rule, b T2 host, c T3 measured (u unmeasured)` records them per boot.

## Hypotheses and falsifiers (pre-registered)

- **H-derived-TU106**: the merged head's derived page ends the Turing reset storm on a TU106 under the Windows
  gate (`KF_MERGE_WINDOWS=1 merge_check.sh` path, flags `kf3_flags.sample.env` unchanged).
  - PREDICTION: `WINDOWS_GATE_PASS ... resets=0`; kf3 log `caps page derived — ... 1 T3 measured (0
    unmeasured)`; a window PUT after the modeset (`console shows head 0 ... [window 0`); exactly one
    `Running -> Suspending` (the graceful stop); `nvlddmkm153=0`.
  - FALSIFIER: any reset counter > 0 since the enable, a TDR, a probe failure (`WINDOWS_GATE_FAIL`), more than
    one `Running -> Suspending`, an `unmeasured` count > 0 for TU106, or no window PUT after the modeset.
  - On a FAIL: the d2 page passed and this page differs from it ONLY in the 12 words above (plus whatever T2
    supplies at run time), so the first bisection arm is those words, not the d2 core. Max 3 negative boots.
  - A `WINDOWS_GATE_RETRY` is not a verdict on the guest: re-run (fresh box or a fixed script).
- **H-linux-not-regressed**: the display change does not regress the Linux bar on the same TU106 box:
  `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`, `FAST_SUITE_PASS=30 ... ARMS=30`,
  `BIRTH_CENSUS_OK`. FALSIFIER: any of these lines missing or different on the exact merge-check revision.
- **Control (box B, host driver 595.91.07, the d1/d2 lease's host driver; 5 vCPU)**: `windows_lane.sh` then
  `windows_gate.sh` by hand. Same prediction and falsifier as H-derived-TU106. A PASS on box A (host 580.159.04)
  and box B (595.91.07) is n=2 on two machines and two host drivers; a split result names the host driver or
  the box as the variable, not the page.
- **TU116 0x00c (box C, GTX 1660 SUPER)**: `tools/read_display_caps_page.py` as root on the stock host driver
  and on 580.159.04. Expected (INFERRED from TU106/AD10x/GB20x, every measured die has 0x100 or 0x102): a
  non-zero `DECISIVE 0x00c`. No prediction is made about the value; it is recorded, and a row is added only for
  the driver branch it was read on.

## Boxes (own ids only)

| box | id | GPU | offer | where | created |
|---|---|---|---|---|---|
| A | 58956007 | RTX 2060 SUPER (TU106, 10de:1f06), 15 vCPU, 98 GB, 155 GB disk | 57046638 | Quebec CA | 2026-11-03T11:26Z |
| B | 58957107 | RTX 2060 SUPER (TU106, 10de:1f06), 5 vCPU, 24 GB, 155 GB disk | 58947025 | Quebec CA | 2026-11-03T11:36Z |
| C | 58957109 | GTX 1660 SUPER (TU116) | 58437683 | Austria AT | 2026-11-03T11:36Z |

### Added at 12:25Z, when the arm re-run below was launched (before its result)

- **H-guest-ram-perf**: box A's one fast-suite miss (`--ce-client-guest-ram` TIMEOUT at 185 s of the 180 s budget) is
  the known Turing PERF case of `traces/tur1_20261027/README.md` (lease 57882297, before this change: TIMEOUT at
  180 s, PASS in 203 s at 600 s, 187 s in a 300 s suite), not a regression. FALSIFIER: the arm alone at a 600 s
  budget fails or crashes, or passes in clearly more than ~205 s, or its kf3 counters stop advancing (hung).

## RESULTS

### Box B, Windows gate run tbB1 (MEASURED 2026-11-03, rev `e6dad00f` = `08dae8a3` + docs, host 595.91.07, `boxB/`)

Lane: `boxB/winlane.log` `WINLANE_TIMING total=2031s ... install_windows=1226s`, `WINLANE_EXIT rc=0` (5 vCPU, install
SMP 5, 20 min). Gate `boxB/tbB1_windows.log`, launched 12:21:32Z on the sealed image, flags file unchanged, no
`KF3_DIAG_*` variable:

```
WINDOWS_GATE_PASS rev=e6dad00f gpu="NVIDIA GeForce RTX 2060 SUPER" idle_s=300 hostref=green/green secs=1481 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=3 strict=0 enable_utc=2026-11-03T12:32:35Z
WINDOWS_GATE_EXIT rc=0 2026-11-03T12:46:13+00:00
```

Against the pre-registered falsifier (every item from a committed file):
- kf3 log (`tbB1/gate_tbB1_b2_qemu.log.gz`): `caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0
  unmeasured)` (56 words, as computed in `derived_vs_d2_page.txt`; T2 supplied nothing at run time) and `+184046 ms
  the console shows head 0 1920x1080: [window 0 ...]` (a window PUT after the modeset).
- `tbB1/gsp_signals.txt`: `running_to_suspending=1` (the stop), `unloading_guest_driver=1`, `host_xid_in_boots=0`.
- `tbB1/tdr_counts.txt`: every reset counter 0, `livekernelevent193=3` (one WATCHDOG dump 12:33:04Z, +29 s after the
  enable, before the probes; as on AD102 and d1/d2).
- probes: cup8 3 s, d3d11 2 s, cup3, d3d12 all rc=0 before and after the idle (the t2 failure run needed 56 s for
  cup8 and failed it after the idle).
- No QEMU left after the gate (`pgrep qemu-system` empty; `run/stop-requested` present).
- Not falsified: H-derived-TU106 holds for this run (n=1 on box B).

The new `gate_state.ps1` (`c290ec24`, stale-LKE filter) was parse- and run-checked on this guest during the idle
window (read-only, `-Mode tdr`): it printed `livekernelevent_stale=0 ... total=0` and `GATE_STATE_DONE`. Its stale
branch (an old report re-submitted) is not exercised on a fresh image.

### Box B, Windows gate run tbB2 (MEASURED 2026-11-03, rev `c71c07cb`: the gate fixes `08dae8a3` + `c290ec24` included)

A fresh overlay and a fresh QEMU on the same installed image (`boxB/tbB2_windows.log`, launched 12:56:56Z):

```
WINDOWS_GATE_PASS rev=c71c07cb gpu="NVIDIA GeForce RTX 2060 SUPER" idle_s=300 hostref=green/green secs=1449 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=3 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-03T13:07:22Z
WINDOWS_GATE_EXIT rc=0 2026-11-03T13:21:05+00:00
```

Same signals as tbB1: `caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)`, window 0
scanned +179.8 s, `running_to_suspending=1`, `host_xid_in_boots=0`, `GATE_TDR ... livekernelevent_stale=0 ... total=0`
(the new counter ran in the gate). No QEMU and no `win_vm.sh run` loop left after the gate (`pgrep`, empty). H-derived-
TU106 not falsified (n=2 on box B). Box B destroyed 13:22Z (`vastai destroy instance 58957107 -y`; the listing then
held only 58956007 of this lane).

### Box A, Linux bar (MEASURED 2026-11-03, rev `08dae8a3`, host 580.159.04, `boxA/tbA1.log`, `boxA/tbA1_suite.out`)

- `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, `KF3_RC=0`, `BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0
  ARMS=30`, `FG_RC=0`.
- Thin guest: `FAST_SUITE_PASS=29 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=1 NOTRUN=0 ARMS=30` — the one miss is
  `ce-client-guest-ram verdict=TIMEOUT secs=185`; its kf3 counters kept advancing to the kill (slow, not hung). So
  `merge_check.sh` ended `EXIT rc=1` and its Windows stage did not run; the census was not reached.
- The arm alone at 600 s (`boxA/tbA1gr_suite.out`): `verdict=PASS secs=310 client_ms=281890`.
- **H-guest-ram-perf: FALSIFIED as stated** — 310 s (client 282 s) against the 203 s (client 187 s) of lease 57882297.
  The arm is slower than before, not only slow. INFERRED attribution: not this branch's change — this branch's crate
  code is `next`'s plus one test file (`git diff origin/next -- crates/` = `kf-disp/tests/turing_caps.rs`), and the
  Ada-PC lane measured a walk-latency regression of `next` 70e3fabd8 against 9297ffdd9 on an RTX 4070 (walk 1009 us vs
  ~110 us, suite client time 1.91x; `traces/ada_pc_20261103/` on `claude/ada-pc-regression-20261103`), which a
  separate lane is fixing in kf-cuda/kf-qemu. Not measured here: the same arm on this box at 9297ffdd9. The two
  reference runs also differ in box and host driver (595.91.07 then, 580.159.04 now).
