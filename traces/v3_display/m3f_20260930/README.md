# m3f — the X desktop's console is pixel-identical to the X server's own screenshot; weston runs

**STATUS: MEASURED, 2026-09-30 — M3 in progress (see `V3_DISPLAY.md` STATUS for the grade state).**
Exact binary: kf3 `7d79074c` (`run_m3f_rev.txt`), vast 53505783, RTX 3060 (GA106), host + guest
580.159.04, `bash dlane.sh m3f DISPLAY_DESKTOP=1 DISPLAY_WESTON=1 DISPLAY_SESSION_ENV=__GL_SYNC_TO_VBLANK=0`
(tests 664 / 0). `DISPLAY_HOST_XID=0` (the display-SW object refused, `9adb26a8`).

X11 (stage 4, `hook/`):
- Xorg + the stock NVIDIA X driver on the virtual monitor, lightdm autologin, X started once;
  `glxinfo`: NVIDIA GeForce RTX 3060, direct rendering; glxgears 57.1-57.4 FPS.
- ★ **`console_cinnamon.png` (our scanout: the NVIDIA X driver's BLOCK-LINEAR surface, un-swizzled by
  the display plane's PTX kernel, host screendump) and `xserver_root.png` (the X server's own
  root window, `import -window root`) are identical in all 2 073 600 pixels.**
- Cinnamon itself segfaults right after it starts — `cinnamon[2788]: segfault at 0 … error 6 in
  libnvidia-glcore.so.580.159.04` (`hook/crashes.log`) — so the desktop shows Cinnamon's fallback
  dialog; `__GL_SYNC_TO_VBLANK=0` does not change it. X11 Vulkan presentation fails
  (`vkCreateSwapchainKHR`, `hook/vkcube*.log`): the NVIDIA X driver's swapchain needs the
  GF100_DISP_SW object, which is refused because its software methods trap on the HOST GPU
  (m3c: 186 host Xid 32 while it was offered).

Wayland (stage 5): weston 13 on the DRM backend comes up on `DVI-D-1` 1920x1080@60 with the GL
renderer and desktop-shell (`hook/weston.log`, `console_weston.png`); `weston-simple-egl` runs its
10 s. (`vkcube --wsi wayland` is not an option of Ubuntu's vkcube — see m3g.)

`SHA256SUMS` = local hashes of the 39 copied files (all matched remote); the PNGs were made locally
from copied screendumps / `import` output. No executable was copied back.
