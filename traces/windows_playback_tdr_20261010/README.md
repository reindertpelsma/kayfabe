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
