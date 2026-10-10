# Windows playback TDR, 2026-10-10 (branch `claude/playback-tdr-20261010`, from combined line 3aea0a79)

STATUS: LIVE, 2026-10-10. Progress log; newest HANDOFF at the top when I stop.

## Progress
* 1. Code finding [measured, code]: in `display.rs` `ScanState::start`, when `dp.sdr_color` is set (Windows SDR colour pipeline) any entry in
  `planned.refused` returned `Err("colour frame has refused windows")`; the caller set `self.failed = true`; the display loop's step 6
  does `if scan.failed { queue.clear(); engine.halt_scanout(); }` and `halt_scanout` halts every guest display channel for the VM's life.
  The non-colour path always left the refused window out and carried on.
* Fix: `Fault::Console` vs `Fault::Gpu` (`ScanState::fault`). Console-only refusals (window with no console format; colour program the
  console cannot build) leave the window out / skip the copy, complete the flips behind the copy (`finish`), never set `failed`, and are
  counted (`console_windows_left_out`, `console_copies_skipped` in the device status line, first 16 refusals in the log). GPU failures keep
  `failed` (no forged completion for GPU work). YUV composition for the console was not added: it needs a new CUDA kernel + PTX
  (planar storage), not simple. Test `a_console_only_refusal_completes_the_flip_and_never_halts_the_display`.
  Note: the test exercises the new `fault` policy; the old inline code had no testable seam (it needs the GPU), so "fails on old code" is by construction (old behaviour = `Fault::Gpu` for the refused case, asserted in the test as the stopping branch).

## Progress 2 (YUV console composition; run 400 = f34d8937 with only the isolation fix)
* [measured, run 400] 0 TDR through boot/sign-in/Edge/Shorts-load (first ~6 min of the hold also 0 at the time of writing); 16x
  `scanout REFUSED window 4 ... FORMAT 0x38` and the owner sees the video area BLACK on the console while the rest of the page is fine.
  So the isolation fix works (flips continue), and the black area is the deterministic console-copy gap, not a race.
* [measured, ogkm clc67e.h] FORMAT 0x38 = `Y8___V8U8_N420` (semi-planar 4:2:0, luma plane then interleaved V,U; NV21). Siblings
  `Y8___U8V8_N422` (0x36), `Y8___U8V8_N444` (0x35). SET_PARAMS.SWAP_UV flips the chroma order. Planes: SET_CONTEXT_DMA_ISO(b),
  SET_OFFSET(b), SET_PLANAR_STORAGE(b), b = 0 luma, 1 chroma.
* [measured, harness] the runners' `PLAYBACK frames differ` uses QMP `screendump device kf0` = kayfabe's CONSOLE copy, which could not contain the
  YUV overlay: the check was blind to the video by construction (explains the false 'identical' verdicts; 385 showed video because
  it simply had no refusal: [inferred] the overlay plane was not in use or composed otherwise there; run 385 has 0 `REFUSED`).
* Implemented (commit f72b0d34, da6be258): `ScanFormats.yuv` (derived from the class names), `Scanout.{iso1,offset1,pitch1,swap_uv}`,
  `scanout::plan_yuv` (each plane bounded against its own context DMA; destination clipped), `cuda/display/kf_yuv.cu` + generated PTX
  (own module; `make_yuv_ptx.sh`), `DisplayGpu::compose_yuv` (bounds re-derived), composed after the RGB layers and before the cursor.
  BT.709 limited range, 8.8 fixed point, nearest-neighbour scale, opaque. NOT done (stated): the window's own CSC00/ILUT are ignored for the
  YUV window; no 10/12-bit or 3-plane formats (still left out by name, console-only); depth ordering (YUV after RGB).
  Tests: `tests/yuv_kernel.rs` (the kernel BODY compiled for the host, compared byte-for-byte with `yuv_reference` over pitch/BL, 420/422/444,
  both orders, scale up/down, frame-edge clip), NV12 known answers, `plan_yuv` bounds incl. hostile planes, `YuvLayer::check`.
* Guest-side check added to the runner (tdr-run17.sh, host-only): `kfplay.ps1` runs in the interactive session (scheduled task /it),
  5 captures of the player column 3 s apart with pixel-diff counts, GPU Engine utilisation of msedge by engine type (VideoDecode = NVDEC),
  full PNGs left in C:\kf. Run 400's first attempt failed (`schtasks /run`: Element not found); run 401 discovers the interactive user.

## Progress 3 (runs 400-402; measured vs inferred)
* [measured, run 401 da6be258] the console now DRAWS the overlay (owner photo + `shorts-playing-2.png`): window 4 armed state logged:
  448x796 block-linear (7 GOBs, bh 1), format 0x38, SWAP_UV=1 (so the effective chroma order is U,V = NV12), out (826,180) 448x795,
  depth 11, ISO(1)=OFFSET(1)=0 (never programmed), PLANAR_STORAGE(1)=7. The picture was luma-correct but magenta/green: [measured] the
  luma tile structure is right and a 2x-stretched copy of the luma image's top half appears in the chroma channels, i.e. the kernel read
  the chroma plane from the luma plane's first byte (offset1 = 0). Fix cc378992: an unprogrammed chroma plane follows the luma surface
  (offset + whole-surface luma size, 256-aligned; 358 400 bytes here). It is NOT a U/V-order or matrix bug; the order logic (format order
  XOR SWAP_UV) is unchanged. [unverified on hardware until an overlay run with cc378992+: run 402 had no overlay.]
* [measured] ETW from the WATCHDOG 0x117 of run 400 (cluster at hold_t 624): the guest's flip queue (event 547, pid 4) declared the TDR
  at 17:51:50.159; the last present (id 10097) handed plane 0 and plane 1 (YCBCR_STUDIO_G22_LEFT_P709, 448x796 at 826,180) at
  17:51:48.050; plane 0 completed (505) at .067, plane 1 NEVER completed; no render/video packet was open (178 without 180: none) and
  no host Xid. So it is the same class as 264-266 (a flip handed and never reported), here on the overlay plane. The 0x117 dump's
  param2 is an nvlddmkm address. Run 401: same shape at hold_t 732 (its ETW window ends earlier).
* [measured] overlay presence correlates with the Edge phase: runs 400/401 (overlay active, console black/magenta before the fixes) had 0 TDR
  through Shorts-load and a TDR cluster 10-12 min into the hold; runs 385 and 402 had 2-3 TDRs in the Edge phase, then NO overlay
  window (0 YUV/REFUSED lines), correct video on the console from DWM composition. [inferred, H-MPO] a TDR makes Windows drop MPO
  for the session; not proven (the registry/ETW MPO state was not read).
* [measured] harness: `PLAYBACK frames differ` was blind by construction (console copy without the overlay). The in-guest check
  (`kfplay.ps1`) reports GPU Engine use by msedge: run 401 `videodecode sum=4.74 max=1.25` = NVDEC ran for Shorts (hardware decode IS in use).
  Its crop-diff output was empty in 400/401 (PowerShell alias `diff` shadowed the function; fixed) and 0/15000 in 402 (the clip was paused:
  the Shorts play icon is visible in `shorts-playing-2.png`; [inferred] the scripted PLAY click toggled it).
* Colour bars: `tdrhunt/bars.mp4` (10 stripes, BT.709 limited, exact RGB), staged in the guest, opened in Edge via an /it task: run 402 shows
  the bars correctly from DWM composition (no overlay), so it did not exercise the YUV kernel.
* Added a bounded STALL report (91b2890a): an UPDATE parked > 1 s is named once (stage, group, head, acquire and the semaphore value it
  reads), to see which wait the overlay flip is stuck on in the next overlay run.

## Progress 4 (the overlay flip that never completes: root cause and engine fix; measured vs inferred)
* [measured, ETW of runs 400 and 403] both TDR declarations (flip-queue 547) follow a present whose PLANE 0 completed (505) and whose PLANE 1
  (the video overlay, window 4) never did; run 403: plane 1 had been flipped to NO surface at presents 5212-5214 and was RE-ENABLED
  at present 5215 (run 400: the same last-present shape). No render/video packet was open (178 without 180: none), no Xid.
* [measured, run 403, STALL report 7750480b] chn 5 (window 4) update 0x1000 parked 1016 ms waiting for its interlock group
  `{chn 1, chn 37}`; chn 37 (window-immediate 4) update 0x2 waiting for `{chn 5}`; chn 1 (window 0) NOT parked at an UPDATE; no acquire.
  An earlier line: chn 1 and chn 5 both ready and waiting for head 0's vblank.
* [inferred from those two, consistent with the 21 us between the two 259 events of present 5215] the guest pushes window 0's UPDATE first
  (it names nothing) and window 4's (names window 0 and its immediate channel) microseconds later. The engine parked window 0's update for
  the vblank (Stage::Latch); window 4's update then waited for an UPDATE on window 0 that never comes (the driver waits for plane 1's completion
  before sending more), while window 0 latched alone: plane 0 completed, plane 1 never. nvkms (`nvkms-evo3.c:2829-2905`) confirms the
  hardware rule that an interlocked UPDATE waits for the named channels' updates and latches with them; a parked update is still PENDING until
  its latch.
* Fix 17eb8b2f (`engine.rs` `ready_group`/`group_ready`): a channel parked for its vblank or acquire counts as pending for the closure; an UPDATE
  naming it joins its latch group (with the channels it latches with). Test
  `an_update_naming_a_channel_already_parked_for_the_vblank_joins_its_latch`: FAILS on the old code (only window 0 latches at the vblank,
  window 4 and its immediate channel stay waiting), passes now; the run-293 one-sided tests and the other 118 engine/disp tests pass.
* Also pushed: an always-on ring of recent interlock events and a STALL report with queue/copy/tick context (bounded, loud, read-only).
* Hardware validation of 17eb8b2f: run 408 (queued; 20-minute hold).
* MPO decision (owner question): [measured] nothing in our logs or the open sources names a derived capability the guest reads to turn the overlay
  off: the guest asks nothing we log for the window assignment; the family row's `windows` count and the caps page's `SYS_CAPB_WINDOW_EXISTS`
  bits are the real die's (derived); Windows' MPO decision lives in the closed driver. Therefore no knob is proposed: the product answer
  is the flip-completion fix. `OverlayTestMode=5` stays a guest-side diagnostic (run 404).
