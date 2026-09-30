# m3i — the Mint desktop (Cinnamon) on the virtual monitor, GPU-composited, with vkcube in it

**STATUS: MEASURED, 2026-09-30 — the M3 desktop grade (see `V3_DISPLAY.md` STATUS).**
Exact binary: kf3 `60891aef` (`run_m3i_rev.txt`), vast 53505783, RTX 3060 (GA106), host + guest
580.159.04, `bash dlane.sh m3i DISPLAY_CINNAMON_WAYLAND=1` (tests 666 / 0). `DISPLAY_HOST_XID=0`,
0 GPU-progress errors, 0 flip-event timeouts, 0 DRM WARNs, probe stage pixel-exact, lane rc=0.

- lightdm autologin into Cinnamon's **Wayland** session (`cinnamon-wayland`): Cinnamon's own
  compositor (muffin) drives the virtual display through nvidia-drm KMS and renders with the NVIDIA
  EGL/GBM stack on the RTX 3060 through kayfabe; the session came up (`cinnamon` process, socket
  `wayland-0`) and **no component crashed** (`DISPLAY_CINNAMON_WAYLAND_CRASHES 0`).
- ★ `console_cinnamon_wayland.png` — Cinnamon's panel (menu, file manager, tray, clock), the pointer;
  ★ `console_cinnamon_wayland_vkcube.png` — **`vkcube-wayland` (Vulkan, "NVIDIA GeForce RTX 3060")
  rendering in a Cinnamon window**, both as the HOST screendumped the kf3 console. (It ran until
  the 12 s timeout: `RC=124`.)
- Cosmetic, not display: no wallpaper — `cinnamon-settings-daemon-background` and other csd
  plugins failed to register before cinnamon-session's timeout (`hook/cw_errors.log`).
- Why Wayland: on X11 the same Cinnamon segfaults in `libnvidia-glcore` (m3e/m3f/m3h), and X11
  Vulkan presentation fails — both need the GF100_DISP_SW object the NVIDIA X driver / GLX use, which
  kayfabe refuses because its software methods trap on the HOST GPU (m3c: 186 host Xid 32).

`SHA256SUMS` = local hashes of the 21 copied files (all matched remote); the PNGs were made locally
from copied screendumps. No executable was copied back.
