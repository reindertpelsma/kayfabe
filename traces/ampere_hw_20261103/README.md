# Ampere: the Linux bar and the first Windows gate on a GA10x die (2026-11-03)

STATUS: LIVE, 2026-11-03. Pre-registration first (written and pushed BEFORE any measurement or run on the box);
results are appended under "RESULTS". MEASURED = a line in a committed log, named with its file; INFERRED =
reasoning; COMPUTED = a GPU-free computation from the committed code.

## What is booted

Branch `claude/ampere-hw-20261103` = `origin/claude/turing-boot-20261103` (PR #243: `next` with the Blackwell merge
#239, the Windows gate and its fixes) plus whatever this lane adds (listed under RESULTS with its revision). Flags:
`scripts/bench/windows/kf3_flags.sample.env` unchanged; no `KF3_DIAG_*` variable. Host driver: provisioning's
default, 580.159.04.

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at the base): there is NO tier-3 row for any Ampere die
(architecture 0x170). At the base revision kf3 therefore serves an Ampere die the clean T1 page and logs
`NO MEASURED caps word 0x00c ... a Windows guest's display may not come up` (`Derived::unmeasured` non-empty). The
Turing storm (traces/turing_boot_20261103/) and the GB203 bwhw1 run show what that page costs on Windows.

## Hypotheses and falsifiers (pre-registered)

- **H-ga-00c-measurable**: the Ampere die's caps word `0x00c` reads back non-zero under the stock host driver with
  `tools/read_display_caps_page.py` as root (read-only BAR0, PROT_READ). No value is predicted (every measured die
  so far has 0x100 or 0x102). FALSIFIER: the tool exits non-zero, or the page is empty/all-ones, or a new host Xid
  appears around the read. The value goes into `caps_measured.tsv` only for driver branch 580 (the host it was read
  on, 580.159.04, and the guest's 580.88), with a failing-first GPU-free test that the derived page for this die has
  `unmeasured` empty.
- **H-linux-ampere**: on the exact revision with that row, the Linux bar holds on this die:
  `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`, `BARE_SUITE_PASS=30 ... ARMS=30`,
  `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK`. FALSIFIER: any line missing or different. Known exception
  pre-stated (another lane's walk-latency regression in `next`): if the ONLY miss is `ce-client-guest-ram`
  `verdict=TIMEOUT`, that arm is re-run alone at a 600 s budget and recorded; the 180 s bar is then NOT met on this
  revision and is reported as such.
- **H-windows-ampere**: with the derived page (`caps page derived — N words: ... (0 unmeasured)`), the Windows LTSC
  gate passes on this die: `WINDOWS_GATE_PASS ... resets=0`, a window PUT after the modeset (`the console shows head
  0 ... [window 0`), exactly one `Running -> Suspending` (the graceful stop), no `USER_RING_SCAN` rejection in the
  kf3 log (the Ampere `0xC56F` user rings are admitted), host Xid delta 0. Target n>=2 (fresh overlays).
  - FALSIFIER: `WINDOWS_GATE_FAIL`, any reset counter > 0 since the enable, more than one `Running -> Suspending`,
    an `unmeasured` count > 0, no window PUT after the modeset, or a `USER_RING_SCAN` rejection.
  - On a FAIL: state at failure (the gate's own `state_at_failure_*`), then the first arm is the display path (as
    on GB203: did the core channel get an `UPDATE`?), then the user-ring path. Max 3 negative boots per wall.
  - A `WINDOWS_GATE_RETRY` is not a verdict on the guest: fix the script or re-run on a fresh box.

## Box (own id only)

| box | id | GPU | offer | where | created |
|---|---|---|---|---|---|
| A | 58969637 | RTX 3090 (GA102), 32 vCPU, ~250 GB RAM, 160 GB disk requested | 46019435 | United Kingdom | 2026-11-03T13:58Z |

## RESULTS

(pending)
