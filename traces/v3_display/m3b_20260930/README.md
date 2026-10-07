# m3b — the first desktop on the virtual monitor: Xorg + the stock NVIDIA X driver, GL at 60 FPS

**STATUS: MEASURED, 2026-09-30 — partial M3; superseded by the later m3 runs for the grade.**
Exact binary: kf3 `d43dbcdd` (`run_m3b_rev.txt`), vast 53505783, RTX 3060 (GA106), host + guest
580.159.04, `bash dlane.sh m3b DISPLAY_DESKTOP=1 KF3_DISPLAY_TRACE=1` (tests 664 / 0).

What came up (`m3b.log`, `hook/`):
- Xorg 21.1.11 with the stock NVIDIA X driver on the kf3 device: `KFB kayfabe (DFP-0): connected`,
  `Internal TMDS`, `Setting mode "DFP-0:nvidia-auto-select"` 1920x1080 (`hook/Xorg.0.log`); lightdm
  autologin into Cinnamon; X started ONCE (no crash loop).
- `glxinfo -B`: **`OpenGL renderer string: NVIDIA GeForce RTX 3060/PCIe/SSE2`, direct rendering
  Yes** — the guest's GL runs on the real GPU through kayfabe.
- `glxgears`: **59.77 / 59.79 FPS** — vsync-locked to the emulated head's vblank.
- The block-linear scanout kernel (hand-written PTX) passed its bring-up self-test.
- 0 GPU-progress errors, 0 Xid, lane rc=0; the probe stage again pixel-exact.

What was wrong, and is fixed in `4475b9fe`:
1. **The console's block-linear copy had the GOB's bits 5 and 6 exchanged.**
   `console_desk_1.png` (our scanout, host screendump) vs `xserver_root.png` (the X server's own
   root window, `import -window root`): 23 208 pixels differ, all in the dialog, each a 16-byte
   chunk displaced by (±16 bytes, ∓2 rows); remapping the console image with bits 5/6 exchanged
   makes it identical to the X server's in all 2 073 600 pixels. The real in-GOB order on GA106 is
   `x[3:0] y[1:0] x[4] y[2] x[5]`.
2. `(EE) NVIDIA(0): Failed to allocate display software resources` — the NV9072 display-SW object's
   constructor query `INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES` (`0x20800a5d`) was refused.
Open after m3b: `vkcube` aborts in `demo_prepare_buffers` (`Assertion '!err'`, rc 134) after
selecting the RTX 3060; Cinnamon runs in "fallback mode" (its compositor crashed — the screenshot);
`(EE) NVIDIA(GPU-0): Failed to get virtual display support info` (non-fatal).

`SHA256SUMS` = local hashes of the 27 copied files (all matched remote); the two PNGs were made
locally from the copied screendump / `import` output. No executable was copied back.
