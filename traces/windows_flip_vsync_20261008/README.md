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

What the KMD reads around a VSync and a flip on real hardware `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]`: the ISR reads only
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

## 3. Run 93 (fresh overlay, identity IOMMU, run92's flags + `KF3_DISPLAY_LOADV` + `KF3_DISPLAY_WRITE_TRACE`, binary `kf3-bins/68673e6e`)

Files: [command](run93-command.json), [marker](run93-marker.txt), [harness](run93-harness.log), [summary](run93-summary.txt)
(`flipsum.py` over the log), trace `run93-qemu.log.gz`. `[measured, run93 at 68673e6e, RTX 4070, 2026-10-08]`:

- **H-loadv FALSIFIED.** The guest read `EVT_STAT_HEAD_TIMING(0)` = `0x7` at its VSync ISR, as on hardware
  (`VSYNC h0 frame=1 evt=0x7 en=0x2`), and still wrote **no display PUT** between the modeset (last modeset core PUT at
  265784.568386) and the TDR: `disp[... puts=39 ... updates=5]` at the stall marker, the same count as runs 88/92. No window
  latch before the teardown. Stall marker `0x00730108` ~15 s after launch; 9 Passthrough twins freed; bugcheck stop.
- LAST_DATA timeline (host uptime s): on 265784.569251 -> VSync raised 265784.584445 (15.2 ms, the next frame edge) ->
  W1C, W1C, **off** 265784.584656; on 265784.629798 -> VSync 265784.634607 (4.8 ms) -> off 265784.634813; on
  265787.309982 -> **off 265787.314265 with no VSync in between** (4.3 ms). So the guest also disables LAST_DATA without
  any VSync having arrived: the disable is not caused by an early VSync.
- Flips that reached kf3's display: **0**; flips completed by a VSync: 0 (no window latch at all). dxgkrnl's own flip count
  needs the guest ETW, which this run did not collect. The guest TDRs (stall marker, then the teardown: 9 frees).
- The kf3 sequence parallels the hardware one step for step up to the first VSync after the modeset, RPC by RPC:
  hardware `0x00730282` (0x56), **`0x00730280` GET_HDCP_STATE = OK**, `0x007302a4`, LAST_DATA on, `0x20808159`, VSync,
  `0x90f10106`, then **alloc `0x007e` + window 0 programmed (12.596)**; kf3: `0x00730282` (0x56), **`0x00730280` REFUSED
  0x56**, LAST_DATA on, `0x007302a4`, `0x20808159`, VSync, LAST_DATA off, `0x90f10106` — and no alloc `0x007e`, no window
  programming. The same refusal precedes the modeset on both sides' order (hardware 12.494635 OK). The hardware's reply is
  NV_OK with flags 0 (nothing capable, nothing encrypting; request and reply decoded from `boot{1,2,3}-gsp.jsonl.gz`).

## 4. Run 94 setup: H-hdcp (falsifier stated before the run)

- **H-hdcp** (inferred): the KMD gates the first programming of the primary surface on `NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_STATE`
  succeeding; refused, it never programs a window, so no flip is ever latched or retired and dxgkrnl's present never
  completes. Variable: `KF3_DISPLAY_HDCP_STATE=1` (answer NV_OK, flags 0 — the hardware's answer; id and 12-byte layout
  hand-typed from ogkm-595.84, the 580 headers lack it; default off). `KF3_DISPLAY_LOADV` is OFF again (one variable).
  *Prediction:* after the modeset, the guest writes window PUTs and window updates latch. *Falsifier:* the display PUT count
  stays at the modeset's value through the TDR (no window programming), as in run 93.

## 5. Run 94 (binary `kf3-bins/2c77140b`): INVALID for H-hdcp — the variable never reached the guest

`[measured, run94 at 2c77140b, RTX 4070, 2026-10-08]` files `run94-*`. The experiment flag was read (its log line is
there) but `0x00730280` stayed `UNSERVICED` (3 of 3 `result=none`): the display link's claim set is taken from the
model when the link is built, and only the model's flag was set. Nothing about H-hdcp was tested; the boot repeats run
93 without LOADV (`puts=39` at the stall marker, stall ~15 s after launch, 9 Passthrough frees, bugcheck stop). Fixed in
the next commit (`DisplayPolicy::answering_hdcp_state` adds the claim; a link-level test answers it through
`respond()`). The `xid=57` at the end is dmesg's ring rotating (the newest Xid line is 22:14:45, before this session).
Run 95 repeats run 94's setup with the fix; the falsifier of §4 stands as written.

## 6. Run 95 (binary `kf3-bins/5f0e3b37`, `KF3_DISPLAY_HDCP_STATE` + write trace): H-hdcp FALSIFIED

`[measured, run95 at 5f0e3b37, RTX 4070, 2026-10-08]` files `run95-*`. `0x00730280` answered `result=0x0` 4 of 4 times
(flags 0, the hardware's answer); the guest still wrote **no display PUT** after the modeset (`puts=39` at the stall
marker, ~15 s after launch), no window latch, the same three short LAST_DATA enables (15.4 ms with a VSync at the frame
edge, 4.7 ms with one, 4.2 ms without one), stall marker, 9 Passthrough frees, bugcheck stop. The refusal of
GET_HDCP_STATE is not what keeps the KMD from programming the primary surface.

## 7. Run 96 setup: H-corelatch (the last boot of the time-box; falsifier stated before the run)

The one remaining measured difference in the same stretch: `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]`
Windows' modeset core PUTs come one per frame on hardware (12.520125, 12.535982, 12.552647 s — each core update completes at
a frame edge and the KMD waits for it), while under kf3 the same PUT sequence completes within 3 ms (`[measured, run93,
2026-10-08]` 265784.565581-265784.568386): kf3 latches every update group that includes the core at once.
- **H-corelatch** (inferred): the KMD's state machine expects the modeset's core updates to complete at frame edges (it
  enables LAST_DATA to see the next one); when they have all completed before its first VSync, it takes the path that
  disables LAST_DATA and never programs the primary surface. Variable: `KF3_DISPLAY_CORE_AT_VBLANK=1` (a core update on
  an active head latches and notifies at that head's next vblank; the first modeset, with no active head, still at once;
  default off; engine test `a_core_update_on_an_active_head_waits_for_the_vblank_only_under_the_experiment`). LOADV and
  HDCP OFF. *Falsifier:* the display PUT count stays at the modeset's value through the TDR (no window programming).
