# Die coverage, Ampere family, on the integration revision (2026-11-04)

STATUS: LIVE, 2026-11-04. Lane 3 (owner, 2026-11-04), family Ampere. Pre-registration first (written and pushed
BEFORE any gate result was read); results are appended under RESULTS. MEASURED = a line in a committed file in this
directory, named with its file, run tag, revision and date; INFERRED = reasoning; COMPUTED = GPU-free, from the
committed tree.

## What is booted

Revision under test: `origin/claude/integration-20261104` at `0c515b3b3` (next + the Windows gate + the
Turing/Blackwell/Ampere fixes + caps rows + the walk wake-up fix). `merge_check.sh` is passed that branch and logs the
`HEAD=` it fetched; a run whose `HEAD=` is not `0c515b3b3...` is not a result for this revision. Flags:
`scripts/bench/windows/kf3_flags.sample.env` unchanged; no `KF3_DIAG_*` variable. Host driver: provisioning's
default (580.159.04 at this revision).

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at `0c515b3b3`): T3 caps word `0x00c` rows exist for GA102
(0x170 impl 0x2), GA104 (impl 0x4) and GA106 (impl 0x6), all `0x00000100`, all scoped to driver branch 580; the
GA104/GA106 rows come from the 2026-11-03 caps sweep (plain container leases, no guest was booted on them). GA103 and
GA107 have no row: kf3 serves them the clean page and a realize notice.

## Dies and priority

GA104 [W], GA106 [W if budget], GA102 Windows re-run only if budget remains, GA103/GA107 only if offered cheaply.
Note: the RTX 3080 named in the lane prompt is a GA102 board (10de:2206/2216), not GA104; GA104 boards are the RTX
3060 Ti, 3070, 3070 Ti, RTX A4000 (and some RTX 3060 8 GB). The die is taken from the box's PCI id, never from the
offer's name.

## Hypotheses and falsifiers (pre-registered)

- **H-linux-<die>** (GA104, GA106): on `0c515b3b3` the Linux bar holds on the die:
  `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`, `BARE_SUITE_PASS=30 ... ARMS=30`,
  `FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30`, `BIRTH_CENSUS_OK`.
  FALSIFIER: any of those lines missing or different, or `HEAD=` not `0c515b3b3`.
- **H-windows-<die>** (GA104, GA106): with the die's caps row served (`caps page derived — N words: ... 1 T3
  measured (0 unmeasured)` in the kf3 log), the Windows gate passes: `WINDOWS_GATE_PASS ... resets=0`, a window PUT
  after the modeset (`the console shows head 0 ... [window 0`), host Xid delta 0.
  FALSIFIER: `WINDOWS_GATE_FAIL`; any reset counter > 0 since the enable; an `unmeasured` count > 0 or a
  `NO MEASURED caps word 0x00c` notice for the die; no window PUT after the modeset.
  A `WINDOWS_GATE_RETRY` is not a verdict on the guest: re-run once on a fresh box and fix the script.
  On a FAIL: the gate's own `state_at_failure_*`, then the display path first (did the core channel get an
  `UPDATE`?), then the user-ring path; at most 3 negative boots on one question.
- Not claimed by any PASS here: other host drivers, other machines of the same die, n>1 unless run twice.

## Boxes (own ids only)

| id | offer | GPU (offer name) | where | created (UTC) | price (USD/h, all-in at rent) |
|---|---|---|---|---|---|
| 59031379 | 49032228 | RTX 3060 Ti (GA104 expected) | United Kingdom | 2026-11-04T00:09:07Z | 1.22 |
| 59031382 | 59022147 | RTX 3060 (GA106 expected) | Quebec, CA | 2026-11-04T00:09:10Z | 0.166 |

Offer search 2026-11-04 ~00:05Z (`vms_enabled=true`, single GPU): no RTX 3070, 3070 Ti or A4000 KVM offer existed;
the one GA104 board offered was the RTX 3060 Ti above (1.19 USD/h base). No board name that can only be GA103 or
GA107 (RTX 3050, RTX A2000) had a KVM offer; whether an offered RTX 3060 / 3060 Ti is a GA103/GA104 variant is only
known from its PCI id on the box.

## RESULTS

(appended after the runs)
