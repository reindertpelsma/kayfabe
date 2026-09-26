# V3 headless-graphics test set — nvkvm-pv's headless graphics workloads on kayfabe v3

**STATUS: IN PROGRESS, 2026-09-26 (branch `v3-gfxset`).** Inventory (§1), harness (§2) and the
display-phase list (§5) are done; the measured table (§3) is being filled on a vast box. This line is
replaced by the measured verdict when the run finishes.

Roadmap item 2 (`docs/STATUS_DETAIL.md` §7): *"the headless-graphics test set from nvkvm-pv"* must pass
on kayfabe v3. Read with `V3_HEADLESS_GRAPHICS.md` (the five-step lane this set extends) and
`V3_VIDEO_ENGINES.md` (NVENC/NVDEC). Legend: **[E]** read in source at the cited place, **[M]** measured,
with the run named.

---

## 1. Inventory — what nvkvm-pv validated, headless vs display

Sources read (2026-09-26): `/workspace/nvkvm-pv` working tree `368d2db` (v0.2.2) and, where later,
`integration/candidate-2026-09-18` `13e1c9a` (**@cand**, contains v0.2.5) and `main` `a1f8ec3`; the
predecessor tree `/workspace/nvidia-gpu-passthrough`; the predecessor's notes, which live in the agent
memory directory, not in its `docs/` (`headless_compositor_unlock.md`, `nvenc_101_root_cause_wc_input.md`,
`nvenc_encode_working.md`, `realapp_matrix_done.md`). Mode 1 staged the **host's** NVIDIA userspace into
the guest, so its guest driver version is always the host's (`nvkvm-pv docs/reference/guest-userspace-libraries.md:3-19`).

**Headless** = no scanout: EGL device/surfaceless/GBM-to-FBO, Vulkan without a swapchain, headless-backend
compositors, capture from them, encode/decode to files. **Display** = a KMS head, page flips, a
DRM-backend compositor, a desktop session, a real monitor. Every row below is **[E]**.

| # | workload | nvkvm-pv program / recipe | nvkvm-pv criterion | nvkvm-pv recorded result | test-set item |
|---|---|---|---|---|---|
| H1 | Vulkan loader, instance, device, is-NVIDIA, compute dispatch | `tests/validate.sh:1618-2153` (embedded `vk_probe.c`) | vendor 0x10DE, not a software device; `data[i] == i*3+7` over 4096 (`:1935-1954, :2108-2119`) | PASS on every tested-platforms row, Turing→Blackwell, 535–610 (`tested-platforms.md:11-21, :75-113`) | `pv_validate_vk` |
| H2 | EGL device platform, GLES2 draw into an FBO, two probe pixels | `tests/validate.sh:2197-2587` (embedded `gl_probe.c`) | renderer names NVIDIA; (16,16) ≈ (255,127,0,255) **and** (48,48) = (0,0,0,255) (`:2533-2546`) | PASS, same rows (0x8CDD on 595/610 fixed 2026-08-17, `BOOT_MATRIX.md:227-236`) | `pv_validate_gl` |
| H3 | vkCreateDevice with 7 RT/NVX extensions ("the RDR2 check") | `tests/repro/vk_device_extensions.c` (@cand) | `RESULT: 0` (exit 0) | PASS RTX 4070/595.84, 2026-09-04 (@cand `validate.sh:2278-2280`); SKIPPED everywhere before (no `libvulkan-dev`) | `pv_vk_rt_ext` |
| H4 | `VK_EXT_external_memory_host` import of 2 MiB | @cand `validate.sh` `vk_import_host_ptr` | `imported … MiB of host memory` | fixed in v0.2.5 (`43c8d75`), 37/37 on RTX 4070 only (@cand `CHANGELOG.md:112-135`) | inside `pv_validate_vk` (its own digest) |
| H5 | Geekbench 7 GPU, Vulkan backend | binary, no script (@cand `CHANGELOG.md:86-96`) | composite vs bare metal; no workload at 0 | 175 398–181 298 = 94–97 % of bare metal, RTX 4070, 2026-09-05 | `geekbench_vulkan` |
| H6 | `vulkaninfo` enumerates the GPU | `tests/perf/graphics_remote.sh:18-25` | `deviceName` matches `nvidia\|rtx\|geforce` | PASS RTX 3060, 580.159.04 (`realapp_matrix.md:67`) and 575.51.03 (`:140`) | `vk_info` |
| H7 | vkpeak | `graphics_remote.sh:27-39` | fp32 figure; guest/host ≥ 0.90 | 9341 vs 9307 GFLOP/s, **1.00x** (`realapp_matrix.md:68`) | `vkpeak` |
| H8 | `egl_offscreen` (EGL device, 1024² FBO, 20k tris × 2000) | `tests/perf/apps/egl_offscreen.c` | `CHECK ok` = no GL error (no pixel assert, `:79-84`) | 1.8 vs 1.8 Mtri/s, **1.00x** (`realapp_matrix.md:69`) | `egl_offscreen` (+ a framebuffer digest) |
| H9 | NVENC H.264 1080p, `-f null` | `graphics_remote.sh:56-59` | an `fps=` figure; ratio ≥ 0.90 | hang (575.51.03, 2026-08-17) → "does not reproduce" (@cand `known-limitations.md:744-782`) | `pv_nvenc_nvdec_fps` |
| H10 | NVENC like-for-like, H.264/HEVC bitstreams | predecessor `PRE_PUBLIC_CHECKLIST.md:9-39` | fps; "ffprobe-verified" streams | 720p 0.96x; 1080p CPU input 6.8x slower (UC GPA window) | `video_nvenc_nvdec` (md5-exact lane) |
| H11 | NVDEC `h264_cuvid` | @cand `graphics_remote.sh:60,71` | a `speed=` figure, all 300 frames | host 33.3x, guest 13.3x, RTX 4070 (@cand `known-limitations.md:828-862`) | `pv_nvenc_nvdec_fps`, `video_nvenc_nvdec` |
| H12 | glmark2 2023.01 on headless weston, off-screen + windowed | `tests/perf/build_glmark2.sh`, `glmark_remote.sh:71-89` | none — metrics only | off-screen 0.73x (kvm-clock) / 0.89x (tsc) (`results/glmark2_2026-08-21/RESULTS.md:24-31`) | `glmark2` (+ `--validate`) |
| H13 | GL micro-probes: decompose, draw rate, finish rate | `tests/perf/apps/gl_{decompose,drawrate,finishrate}.c` | `CHECK ok` (no GL error) | fill 1.000x; readback 0.76x (`RESULTS.md:98-110`) | `gl_micro` |
| H14 | headless weston (GL) + `es2gears_wayland` + `weston-screenshooter` | `tests/perf/run_headless_compositor.sh:33-68` | weston alive, renderer lines, a screenshot (content judged by eye) | CONFIRMED RTX 3060, 575.51.03 (`realapp_matrix.md:171-178`) | `weston_headless` (+ a deterministic composite digest) |
| H15 | client-pixel differential on headless weston | `tests/perf/verify_client_window_differential.sh` | A≠B (client drew) and B≠C (animating), per-channel > 8 | FAIL 2026-08-17 → fixed 2026-08-19 (`known-limitations.md:253-279`) | `weston_client_diff` |
| H16 | GL texture → EGLImage → `eglExportDMABUFImageMESA` | `tests/perf/apps/egl_dmabuf_export_probe.c` | `RESULT: PASS` | EGL_BAD_MATCH → PASS after the fix (`known-limitations.md:221-255`) | `pv_egl_dmabuf_export` |
| H17 | same-process PRIME re-import as an EGLImage | `tests/perf/apps/dmabuf_import_probe.c` | `RESULT import=OK` | OK (`cross-isolate-sharing.md:130-137`) | `pv_dmabuf_import` |
| H18 | two processes share a buffer, bytes compared | `tests/perf/apps/xiso_{import,bytes}_probe.c` | `RESULT import=OK`; `xiso_bytes=PASS checked=65536 quadrants=4` | PASS (`cross-isolate-sharing.md:142-181`) | `pv_xiso_sharing` |
| H19 | dma-buf exports under a fast SIGALRM | `tests/repro/signal_restart_export.c` | exit 0 = every export succeeded | 16/200 → 300/300 after the fix (`known-limitations.md:1470-1477`) | `pv_signal_restart_export` |
| H20 | 5 FBO colour formats | `tests/integration/fbo_formats_probe.c` | `SUMMARY\| 0/5 configurations incomplete` | 5/5 incomplete → 0/5 after the fix (`BOOT_MATRIX.md:733-757`) | `pv_fbo_formats` |
| H21 | GBM bo → dma-buf → EGLImage → FBO (+ NATIVE_PIXMAP sweep) | `tests/repro/gbm_egl_import.c` | host and guest must agree | identical; NATIVE_PIXMAP fails on both (`tests/repro/README.md:45-75`) | `pv_gbm_egl_import` |
| H22 | vkCreateDevice | `tests/repro/vk_create_device.c` | `RESULT: vkCreateDevice rc=0 OK` | H100 fixed (`correctness.md:192-231`) | `pv_vk_create_device` |
| H23 | EGL on the GBM platform on card0, render to FBO, PPM | `tests/perf/apps/gbm{probe,shot}.c` | `RESULT GPU-OK`; an image | "pixels read back exact", `9c827a7` | `pv_gbmprobe_gbmshot` |
| H24 | headless sway + wlr-screencopy into a client dma-buf | `tests/perf/run_screencap.sh` + `apps/wlr_screencap.c` | `RESULT captured=N/N` | ~60 fps @1080p, RTX 3060 580.159.04, predecessor only (`headless_compositor_unlock.md`) | `sway_screencap` (+ grim and a deterministic composite digest) |
| H25 | Blender Open Data 4.5.0, Cycles on CUDA | `docs/reference/blender-opendata.md:115-125` (launcher 3.3.0) | `render_time_no_sync` / total per scene | GPU render 99.95 %, total 93.10 %, RTX 4070 (`c980dd0`) | `blender_opendata` |

### 1.1 Named on the roadmap but **never validated by nvkvm-pv** (checked, not assumed)

Grep of both trees finds no EEVEE, VirtualGL, OBS, Vulkan swapchain/graphics-pipeline probe,
`VK_EXT_headless_surface`, Vulkan Video, NVJPEG, or CUDA-GL interop result. Vulkan rendering ran only as
vkcube/games under SteamOS, which are **display** rows (D22, D24) and were never graded on output. ffmpeg
GPU filters appear only as `hwupload_cuda` feeding NVENC (H10). "OBS-style capture" is, in nvkvm-pv's
terms, H14/H24 (capture from a headless compositor) plus D12 (in-guest NVENC streaming of a head).
⇒ The set runs these as **EXTRA** items, reported separately and never counted as nvkvm-pv parity:
the v3-gfx lane's renders (`vk_compute`, `vk_render`, `egl_render`, `glx_vgl` = Xvfb + VirtualGL),
`gl_info` (GL strings/extensions/limits), ffmpeg CUDA/Vulkan/OpenCL/libplacebo filters, and Blender
Cycles (CUDA, OptiX), EEVEE (OpenGL and Vulkan backends) and Workbench on one deterministic scene.

### 1.2 nvkvm-pv's own failures and exclusions that bear on this set

- Its compositor, capture and dma-buf rows were fixed **for Mode 1's forwarding** (proxy GEM, cross-isolate
  import brokering). In Mode 2 the guest kernel owns `/dev/dri` and does all of it natively, so those
  fixes do not transfer (`V3_HEADLESS_GRAPHICS.md` §5.2); the rows still test real paths here (GEM
  allocs, PRIME import/export, cross-process sharing through the guest's own RM).
- Numbers nvkvm-pv itself says not to quote: "795 fps glmark2", "glmark2 6857 vs 21571", "632 FPS
  surfaceless EGL" (`docs/reference/quoting-numbers.md:25`, `RESULTS.md:3-8`). This set records scores
  but grades none.

---

## 2. How the set is run and graded

Harness: `scripts/bench/gfxset/` (`suite.sh` is the one command).

- **One userspace for both sides.** The bench host is Ubuntu 22.04 and the fat guest 24.04, so a
  render hash or a glmark2 validation compared across them would compare two different userspaces. The
  bare-metal side therefore runs **inside the guest image**: `hostroot.sh` attaches the powered-off
  qcow2 read-only (`qemu-nbd`), puts a tmpfs overlay on it and chroots in as the image's own uid 1000
  with the image's `video`/`render` gids. Same binaries, same libraries, same NVIDIA 580.159.04
  userspace (installed into the image from the same `.run` as the host module). What differs is exactly
  what is being tested: the kernel + GPU path (bare metal vs the kf3 device).
- **Bare metal twice.** A digest that differs between the two bare-metal runs is **nondeterministic**
  and is not graded by equality (the noise floor is measured, not assumed).
- **Per item, in the guest:** one item per ssh call; the item's guest log, guest dmesg slice, **host**
  dmesg slice and kf3 log slice are kept. A host Xid during an item fails it even if the item printed
  success (**FAIL(XID)**, the glxgears-style false pass); a digest that differs from bare metal fails it
  (**FAIL(DIFF)**, a silent wrong answer). A boot that dies or stops rendering (wedge probe: a checked
  Vulkan compute job) ends; the rest continue in a fresh boot; every failure is re-run alone in a fresh
  boot and that run is the verdict of record.
- Each item keeps **nvkvm-pv's own criterion** (quoted in `itemdefs.sh`) as its floor and adds a content
  digest wherever the output is deterministic.

---

## 3. Results

*(filled from the run — see STATUS)*

---

## 4. Failures: root causes and fixes

*(filled from the run)*

---

## 5. The display phase (next, not this task)

Everything below needs a KMS head (NVKMS display object), page flips, a DRM-backend compositor or a
desktop session. In Mode 2 the guest's NVKMS has **no display hardware** (displayless KAPI fallback,
`V3_HEADLESS_GRAPHICS.md` §3), so none of these can run until the display plane exists (`NVA083` on a
610+ vGPU guest driver, or an emulated display engine — `V3_HEADLESS_GRAPHICS.md` §3.3, §5). All rows
**[E]** from nvkvm-pv.

| # | workload | nvkvm-pv program / criterion | nvkvm-pv result |
|---|---|---|---|
| D1 | `modetest` on the virtual head | "Virtual-1 connected, 1920x1080, 23 modes" (`known-limitations.md:61-80`) | PASS, RTX 3050 Laptop |
| D2 | weston on the DRM backend (+ `weston-simple-egl`, `weston-terminal`) | `tests/perf/run_weston_head.sh:20-43`; renderer NVIDIA, flips | hung in `libnvidia-egl-gbm` until **fixed 2026-08-20** (RTX 4070 595.84, 59.9 Hz) |
| D3 | guest desktop in a host QEMU window (GL zero-copy / readback), glmark2, 8 EGL clients | 60 × 1 s samples of exactly 60 frames (`howto/run.md:504-660`) | PASS, RTX 4070 595.84 |
| D4 | desktop + 4 `weston-simple-egl` + Firefox, after the security fixes | 263 s, 16 139 frames, 0 dropped | PASS, RTX 4070 |
| D5 | glmark2-wayland 20 scenes, 150 s run, 5-min desktop soak (sync-fd fix) | all scenes, 0 errors | PASS, RTX 4070 595.84 |
| D6 | X11 apps through Xwayland on weston (glxgears, glmark2, xterm, Firefox) | "check the screenshot, not the frame rate" | fixed 2026-08-20 |
| D7 | Mint 22.3: weston DRM + Xwayland (glxinfo, eglinfo, glxgears, Firefox, Nemo) | NVIDIA renderer, glxgears vsync-locked 60 FPS (`mint-guest-desktop.md:35-64`) | PASS, RTX 4070 |
| D8 | Mint Cinnamon X11 session inside a fullscreen Xwayland | 3 distinct screendumps; `validate.sh` 28/28 alongside | PASS, RTX 4070 |
| D9 | Mint through the display broker to a real 4K monitor | by eye only (`broker-design.md:303-402`) | PASS by eye |
| D10 | real monitor: Mint + Minecraft at max settings | by watching (`tested-platforms.md:339-346`) | 60 fps, 595.84 |
| D11 | real monitor, RTX 3050: Mint, pointer lock, keyboard grab | visual | PASS |
| D12 | 6× T4 headless host: XFCE on the head, **in-guest NVENC H.265 streamed to a browser**, Minecraft | 1080p 58 fps, 26 ms end-to-end (`tested-platforms.md:169-182`) | PASS, 580.178.04 |
| D13 | CentOS Stream 9: Xorg + openbox on the head | none stated | exercised |
| D14 | weston on the head → readback → Xvfb → x11vnc → noVNC | pixel-verified, 12 303 frames 0 failed | PASS, 580.173.02 |
| D15 | Xorg `modesetting` `AccelMethod none` + PRIME offload (XFCE, glxgears) | glxgears ~2460 vs ~490 FPS llvmpipe | PASS, RTX 3070 |
| D16 | 40 short-lived glxgears × 2 rounds (VRAM leak) | net +14 MiB over 80 cycles | PASS |
| D17 | `wcapflip`: capture from headless weston, flip it on card0 | `RESULT presented=N/M` | SHM 30/30; dma-buf 0/60 ("GL: unsupported buffer") |
| D18 | `gbmflip`: legacy KMS flips | 200 flips reach the host | PASS (predecessor) |
| D19 | `wlr_screencap --present`: 4-quadrant image to the host | host PPM has 4 colours in the right places | PASS (predecessor) |
| D20 | SteamOS KWin/Plasma (atomic modesetting) | output enabled, flips with modifier `0x…606014` | SOLVED 2026-08-29 |
| D21 | SteamOS gamescope (OOBE / Game Mode) | connector enabled, host flips > 0 | works on RTX 3050 Laptop; **open** on RTX 4070 (records conflict) |
| D22 | vkcube in SteamOS (gamescope, KWin) | graded only on host leak counters | 59 deferred unmaps per exit |
| D23 | sweep SteamOS stage `display` verdict | GBM files, card0, connector, flips > 0 | **never run on hardware** (fixture only) |
| D24 | SteamOS games (Portal 2, SotTR, RDR2, Just Cause 2) | screenshots only | "launch and play" claims |

**Known display-side walls from nvkvm-pv, useful when that phase starts:** NVIDIA's own X driver (DDX)
cannot run (NVKMS command 33 walks onto the host's connectors, `known-limitations.md:661-687`); Xorg
`modesetting` + glamor fails "Failed to create pixmap" on bare metal too (`mint-guest-desktop.md:122-199`);
Cinnamon's Wayland session crashes without a cursor plane (`:66-120`); host `nvidia-drm modeset=1` is
required for the whole display path (`known-limitations.md:381-419`).

---

## 6. Reproduce

```
# on a box provisioned by provision_box / provision_host_driver / provision_bench_tree / build_kf3:
bash scripts/bench/provision_guest_gfx.sh host && bash scripts/bench/provision_guest_gfx.sh
# the static ffmpeg of V3_VIDEO_ENGINES §5 at /workspace/video/ff/bin/ffmpeg, then:
bash scripts/bench/gfxset/provision.sh          # the set into the guest image (~30 min)
bash scripts/bench/gfxset/suite.sh <run> [item...]
```
