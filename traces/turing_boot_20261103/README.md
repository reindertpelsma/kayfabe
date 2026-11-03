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
