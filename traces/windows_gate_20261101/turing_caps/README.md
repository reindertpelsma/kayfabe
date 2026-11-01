# Turing Windows reset storm: is it the display caps page? (2026-11-01)

STATUS: RESEARCH, 2026-11-01. Written BEFORE the hardware run (the timeline, hypotheses and falsifiers below are the
pre-registration); results are appended below the line "RESULTS". MEASURED = a line in a committed log, named with its
file; INFERRED = reasoning.

## Timeline finding (CPU-only, from the committed kf3 logs)

Logs: AD102 PASS `../g4/gate_g4_b2_qemu.log.gz`; TU116 FAIL `../tur1_diff/a1/gate_a1_b2_qemu.log.gz` (same flags, same
product code), cross-checked on `../tur1_diff/b1/` and `../tu106/t1/`. Clock: kf-mem `t=` seconds; the display's
`+N ms` lines and the 2 s heartbeat's `vblanks=` (60 Hz from the first armed head) agree with it. Anchor: the guest RM's
first `fn70 Bar2` (the RM start), AD102 t=265.04, TU116 t=187.59.

| event (MEASURED) | AD102 g4 (PASS) | TU116 a1 (FAIL) |
|---|---|---|
| display methods/PUTs at the driver-start init (first heartbeat after the RM start) | `methods=5338 puts=35` (+1.7 s) | `methods=8330 puts=31` (+2.3 s) |
| guest arms head 0 (modeset), UPDATEs | +11.5 s, 5 updates, `methods=5628 puts=43` | +32.2 s, 4 updates, `methods=8611 puts=38` |
| first window PUT / scanned window after the modeset | +16.2 s after the arm: `console shows head 0 1920x1080: [window 0 ...]`, updates 5 -> 15, releases 0 -> 1 | **never** in any of 7 lives: `puts` 38 -> 40 only at the teardown, no `console shows head` line, `releases=0` |
| first RM unload (fn 47, `Running -> Suspending`) | only the final one, at the shutdown (+754 s) | +57.0 s (head arm +24.7 s), then every 22.5-25.9 s |

The same per-life pattern (4 modeset UPDATEs at ~8580 methods, no window PUT, an unload 12-25 s later) holds in every life
of TU116 b1 (12+ lives, first unload +57.7 s) and TU106 t1 (12+ lives, first +60.3 s). No refusal by name, RPC status,
interrupt-plane or completion-plane counter separates the dies before the first unload: the refused control/class sets are
equal except one control seen only after the first unload (`0x2080a088`); the RPC call sequences of the three Turing runs
are identical up to interleaving (~5000 calls to fn 47); the NSI arm/live-twin counters are equal; the completion probe
covers Translated rings only and found no overdue fence (b1).

The one realize-time difference: `KF3_DISPLAY_CAPS_PROBE=1` (in the gate's flag file) serves the captured AD104 page on
Ada's caps class `0xC773` only; on Turing it logs `REFUSED — caps class 0xc573 ... the authored page stays`.

INFERRED: this is the class already measured on Ada without the probe — `traces/display_reply_diff_20261008/` runs 97-99
(authored page: `puts=39`, all eight windows initialised, no window PUT after the modeset, TDR; real page alone: window 0
programmed and flipped) and the AD104 bisection on `claude/display-caps-derive-20261014` (run h0 without the page:
"reset loop, 10 Running -> Suspending in 2 min, 8 window channels per life, 8589 methods at the first update"; sufficient:
even-window `SCLR_PRESENT` + unnamed word `0x00c`). GA104 and AD106 without a matching page showed the same loop
(`V3_DISPLAY_CAPS_DERIVATION_20261014.md` section 7). The derived page (`kf_disp::capsderive`, with a TU106 T3 row) exists only
on `claude/display-caps-derive-20261014` and `claude/blackwell-into-next-20261101`, not in `next` or the gate head.

## Hypotheses (pre-registered)

- H-caps-Turing: the Turing storm is caused by kf3's authored caps page: Windows never programs a window after the modeset,
  DWM's present stalls, a TDR follows, the RM unloads and reloads, and the cycle repeats.
  - Arm d1 (sufficiency of the die's own page): the gate, unchanged, with `KF3_DIAG_CAPS_PAGE_FILE=` the box's own page read
    as root with `tools/read_display_caps_page.py`. PREDICTION: a window PUT after the modeset (`console shows head 0 ...
    [window 0`, `puts` > 38 within 30 s of the arm), 0 `Running -> Suspending` before the shutdown, 0 nvlddmkm 153, cup8 and
    d3d11 pass. FALSIFIER: no window PUT after the modeset (`puts` stays at the modeset value) and >= 2 RM unloads in the boot.
  - Arm d2 (the two bisected facts, the shape of the derived page): `KF3_DIAG_CAPS_PAGE_FILE=two_facts_overlay.txt` (ORWORD
    `0x00c`=0x100 + main-layer `SCLR_PRESENT` over the authored page). Same prediction and falsifier. Run only if d1 passes.
- Not tested here: the LiveKernelEvent 193 cause (it appears on AD102 too, near the modeset, and is not decided by this run).

Rules: the gate script and its flag file are unchanged (the diagnostic is one extra `KF3_*` variable in the calling
environment, logged in `KF3_ENV`); max 3 negative boots; the diagnostic is never a product route (a per-die read is the
defect v3 exists to end; the product route is the derived page plus a T3 row, `OWNER_RULINGS.md` §AH).

## RESULTS
