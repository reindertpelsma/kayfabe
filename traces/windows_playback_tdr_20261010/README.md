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
