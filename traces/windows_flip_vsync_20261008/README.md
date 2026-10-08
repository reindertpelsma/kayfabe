# Windows flip / vsync (H-flip) — record

**STATUS: LIVE, 2026-10-08.** Branch `claude/windows-flip-vsync-20261008` = `claude/windows-pass-20261008` `c39681eb` + the
Passthrough completion-interrupt commits of `claude/passthrough-interrupt-20261008` (the same four code commits run92 used,
taken from the host checkout `916355df`, plus the §1.1 doc commit `cdacd21c`). Host: RTX 4070, 595.91.07 host driver,
Windows 11 guest with 580.88, the trusted bench host.

## 1. Before any boot: what kf3 already does at a frame edge (reading, no run)

- `[read: crates/kf-qemu/src/display.rs step 4/8 at c39681eb]` every armed head's paced tick sets LAST_DATA|VBLANK in
  `EVT_STAT_HEAD_TIMING(h)` whether or not it is enabled; the display vector is raised after the tick's effects exactly
  when `EVT & RM_INTR_EN` is non-zero; a disable is honoured at the next edge; an enable raises at once only when an
  enabled bit is ALREADY pending (level semantics). So "raise at each frame edge while enabled, honour disable" is what kf3
  already does. The new tests `the_head_timing_interrupt_is_every_frame_edge_while_enabled_and_never_at_the_enable` pins it
  with Windows' measured order (W1C, then enable: no interrupt at the enable).
- `[measured, run88 at 256e510f, 2026-10-08; traces/windows_code43_walls_20261007/run88-*]` the three enable -> VSync
  latencies are 13-28 ms, <= 43 ms and 0.5-0.6 ms (enable lines bracketed by `maplog t=`; VSyncs from run88-etw), each
  VSync on kf3's 60 Hz grid. Real hardware has 8 of 113 enables with a first interrupt < 1 ms (min 0.58 ms;
  `traces/vfio_dvi_reference_20261008/analysis/etwirq/boot3-enable-latency.txt`). *Inferred:* the 0.6 ms VSync is a
  frame edge, not an early one-shot.
- `[measured, run88 at 256e510f, 2026-10-08]` **the display PUT count is 39 from the modeset (status line at
  254844.389) to the teardown (254878)**: Windows wrote NO display-channel PUT for flips 2/4/6 (no window method, no
  window UPDATE ever executed; `disp[... puts=39 ... updates=5]`). On real hardware `[measured, VFIO DVI reference boot3,
  2026-10-08]` every flip writes window 0's PUT 3x, WINIM 0's PUT and window 0's PUT again ~1 ms after its queue
  completion (QE), and a flip queued while the previous one is still in hardware is programmed only right after the VSync
  that retires the previous one (flip 389: QE 19.661099, PUTs 19.671023, after VI 19.669821 that retired flip 387).
  Right after the modeset, the KMD enables LAST_DATA (12.570799), waits for the first VSync (12.586) and then programs
  window 0 (12.596-12.619). Under kf3 the same enable got its VSync and the guest disabled LAST_DATA and never programmed
  a window.
- So H-flip as framed (kf3 does not raise a VSync per frame while enabled) is contradicted by the code and by run88's
  timing; the measured first divergence is earlier: **the KMD never programs a flip into kf3's display** — so no flip can
  be retired by a VSync, whatever kf3 raises.

What the KMD reads around a VSync and a flip on real hardware `[measured, boot3]`: the ISR reads only
`RM_INTR_DISPATCH` (0x611ec0), `RM_INTR_STAT_HEAD_TIMING(0)` (0x611c00 = 0x2), `EVT_STAT_HEAD_TIMING(0)` (0x611800 = **0x7**)
and write-1-clears **0x2** only (2017 times; it reads 0x5 after the clear and never clears bit 0); each flip reads window
0's ARMED `SET_PARAMS` (0x690a2c = 0xcf) and `SET_COMPOSITION_CONTROL` (0x690aec), and the core's
`NVC67D_GET_RG_SCAN_LINE(0)` (0x680220, a live scanline). kf3 publishes EVT_STAT = **0x6** (no bit 0), mirrors no window
ARMED state, and keeps no live scanline. Bit 0 is unnamed in ogkm's published headers; nouveau names bits 0-1 "LAST_DATA,
LOADV" (`gv100_disp_intr_head_timing`).

## 2. Hypotheses and falsifiers (stated before run 93)

- **H-flip** (task, inferred): the guest TDRs because its flips are not completed by a VSync. *Falsifier:* a boot where
  every flip that reaches kf3's display is latched and followed by a raised LAST_DATA interrupt while LAST_DATA is enabled,
  and the guest still bugchecks 0x116 (`0x00730108` marker) in the same window.
- **H-loadv** (this record, inferred): the KMD decides whether its last display programming has been LOADED from bit 0
  (LOADV) of `EVT_STAT_HEAD_TIMING`, which kf3 never sets; so after the modeset it holds every window programming and
  flip, waiting for a load that never shows. Variable: `KF3_DISPLAY_LOADV=1` (every frame edge also sets bit 0, and the
  event registers are republished at every edge; default off). *Prediction:* after the modeset the guest writes window
  PUTs (`puts` rises past the modeset count; window UPDATEs latch). *Falsifier:* the display PUT count stays at the
  modeset's value through the TDR, as in runs 88/92.
- Diagnostic in every run (no behaviour change): `KF3_DISPLAY_WRITE_TRACE=1` — every guest display write, every raised
  head-timing interrupt and every window latch, with the `maplog` clock.
