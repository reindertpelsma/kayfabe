# V3 headless-graphics test set — nvkvm-pv's headless graphics workloads on kayfabe v3

**STATUS: LIVE, 2026-09-26 (branch `v3-gfxset`; the head to merge is `d06833f0` = master `59cc98a9` + this
branch).** Measured on one RTX 3070 (GA104, vast 52775275, destroyed after the run; every result is in
`traces/v3_gfxset/`). **All 38 items pass at `d06833f0` (`gs3`, 38/38)**: nvkvm-pv's 25 headless rows in 23
items plus 15 extra. 31 items' outputs are byte-identical to bare metal on the same box (content digests),
the 2 nondeterministic Cycles renders lie inside bare metal's own measured spread, 5 items with no
deterministic output pass nvkvm-pv's own criterion, and no item raised a host or guest Xid. Master
`dd3aed08` passed 33/37; the failures were two kayfabe defects — the optical-flow engine was never
advertised, and ctxsw preemption was refused on copy channels — and fixing the first exposed a third (the
SW engine row order); all three are fixed (§4). The merge-ready bar holds at `d06833f0` (§5). The display
phase is §7.

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
- **A nondeterministic image is graded by distance, not skipped.** Cycles path tracing (GPU atomics) gives a
  different PNG on each bare-metal run. `imgcmp.sh` + `imgnoise.py` compare every pair of images (PSNR
  over R, G, B; the number of values that differ; the largest difference) over **every bare-metal run
  available** — the suite's two plus the extra runs of `GSET_NOISE_DIRS` (14 on this box, §3.5):
  floor = the farthest bare-metal pair, guest = the median distance of the guest image to each
  bare-metal image, MATCH iff guest ≥ floor. An image that cannot be graded this way fails
  (`FAIL(ABSENT)`), never passes silently.
  ⊘ **CORRECTED 2026-09-26 (`gs2`):** the first rule was *"within 3 dB of PSNR(bare run 1, bare run 2)"* —
  one sample of the spread plus an assumed margin; it failed a guest image that bare metal's own
  measured spread contains (§3.5).
- **An item bare metal cannot pass twice is not graded** (`NOTRUN(host-flaky)`), and an item whose tool is
  missing is `NOTRUN`, never PASS. Neither occurred in the runs below.
- **A baseline can be reused** (`GSET_HOST_FROM=<run>`): same box, same image, same host driver — the bare
  metal side does not run kayfabe, so a later kf3 revision is graded against the earlier run's bare metal,
  and only items the baseline lacks are measured on bare metal.
- **The live oracle is one switch away** (`GSET_NVDIFF=1`): every `/dev/nvidia*` ioctl on both sides, with
  the parameter buffer before and after the call (`src/nvdiff/`, the recorder of
  `tests/mode2/nvdiff/` in the predecessor tree). It found the ffmpeg defect of §4.3 in one run.
- Performance figures (`GSET_VAL`) are recorded and ratioed (`perf.py`) but **never graded**.

---

## 3. Results

### 3.1 The bench and the runs

- **Box:** vast.ai `52775275`, RTX 3070 (GA104, `0x2484`, 8 GiB, VBIOS 94.04.3A.40.28), Xeon W-2133
  (5 vCPUs), 24 GiB; host driver 580.159.04 (open kernel module), host kernel 6.8.0-59.
  ⚠ **The rented box is itself a KVM VM** (QEMU `i440FX`, nested virtualization on) with the GPU passed
  through. "Bare metal" below is that VM's own NVIDIA driver on the GPU, with no kayfabe in the path; the
  kf3 guest is a **nested (L2) guest** — true of every vast box (`docs/STATUS_DETAIL.md`: a VM exit costs
  roughly 10–40× more). No output digest can see that; every CPU-bound or exit-bound performance ratio is
  biased against the guest by it (§3.4).
- **Guest:** the bench fat guest (Ubuntu 24.04, kernel 6.8.0-139, stock 580.159.04 kernel driver and
  userspace), 12 GiB, 4 vCPUs, kf3 `fb-mb=6144` (the card has 8 GiB), `nvidia-drm modeset=1` (displayless).
- **Runs** (strictly serial on this box; evidence in `traces/v3_gfxset/<run>/`, including every item's
  guest log, guest dmesg, host dmesg and kf3 log slice):

| run | kf3 revision | what | result |
|---|---|---|---|
| `gs1` | master `dd3aed08` (built as `ce20926e`: the same crates; only `scripts/bench/gfxset/` differs) | bare metal ×2, then the guest; 37 items | bare metal 37/37 twice; guest **33/37** |
| `diag1` | `9bc91512` (+ OFA) | 4 items, host + guest, nvdiff recorded | 3/4 — ffmpeg's Vulkan device still fails |
| `diag2` | `c9acfbab` (+ preemption) | the 8 Vulkan items | 8/8 |
| `diag3` | `88dc8778` (+ SW last) | 6 items | 6/6 |
| `gs2` | `b15c7cc8` = the branch merged with master `5d2b0a33` (+ Hopper's OFA fault id) | all 38 items; bare-metal baseline reused from `gs1`, `vk_ofa` measured on bare metal ×2 | 37/38 under the first image rule (Cycles CUDA, §3.5); **38/38** regraded against the 14-run spread |
| `nd/h1`–`h12` | — (bare metal only) | the two Cycles items 12 more times on bare metal: the measured spread of §3.5 | 24/24 PASS, no Xid |
| **`gs3`** | **`d06833f0`** = the branch merged with master `59cc98a9` (the head to merge) | all 38 items, as `gs2`; images graded against the 14-run spread | **38/38** |
| `gs3c1`–`c3` | `d06833f0` | the two Cycles items alone, three fresh boots: guest samples of the render noise | 2/2 in each boot: all 6 renders within the spread |

### 3.2 The table

| # | item | nvkvm-pv row: its result | bare metal (×2) | guest @ master `dd3aed08` (`gs1`) | guest @ `v3-gfxset` `d06833f0` (`gs3`) | failure point (`gs1`) | fixed? |
|---|---|---|---|---|---|---|---|
| 1 | `pv_validate_vk` | H1+H4: PASS, every tested platform | PASS | PASS (2/2 digests) | **PASS** (2/2 digests) | — | — |
| 2 | `pv_validate_gl` | H2: PASS, every tested platform | PASS | PASS (2/2 digests) | **PASS** (2/2 digests) | — | — |
| 3 | `pv_vk_rt_ext` | H3: PASS RTX 4070 (SKIPPED before 2026-09-04) | PASS | FAIL (0/1 digests) | **PASS** (1/1 digests) | kf3: OFA engine never advertised → guest RM drops `NVC7FA_VIDEO_OFA` → UMD withholds `VK_NV_optical_flow` (§4.1) | yes, `bc1a3c3e` |
| 4 | `pv_vk_create_device` | H22: H100 fixed | PASS | PASS (criterion only) | **PASS** (criterion only) | — | — |
| 5 | `vk_info` | H6: PASS RTX 3060 | PASS | FAIL(DIFF) (4/6 digests) | **PASS** (6/6 digests) | same: extension list 250 vs 251, no 6th queue family (optical flow) (§4.1) | yes, `bc1a3c3e` |
| 6 | `vkpeak` | H7: 1.00x | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 7 | `egl_offscreen` | H8: 1.00x | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 8 | `gl_micro` | H13: CHECK ok; fill 1.000x | PASS | PASS (criterion only) | **PASS** (criterion only) | — | — |
| 9 | `pv_fbo_formats` | H20: 5/5 incomplete → 0/5 | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 10 | `pv_egl_dmabuf_export` | H16: EGL_BAD_MATCH → PASS | PASS | PASS (criterion only) | **PASS** (criterion only) | — | — |
| 11 | `pv_dmabuf_import` | H17: import=OK | PASS | PASS (4/4 digests) | **PASS** (4/4 digests) | — | — |
| 12 | `pv_xiso_sharing` | H18: PASS, 65536 B, 4 quadrants | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 13 | `pv_signal_restart_export` | H19: 16/200 → 300/300 | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 14 | `pv_gbm_egl_import` | H21: host = guest | PASS | PASS (2/2 digests) | **PASS** (2/2 digests) | — | — |
| 15 | `pv_gbmprobe_gbmshot` | H23: pixels exact | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 16 | `weston_headless` | H14: CONFIRMED RTX 3060 | PASS | PASS (2/2 digests) | **PASS** (2/2 digests) | — | — |
| 17 | `weston_client_diff` | H15: FAIL → fixed 2026-08-19 | PASS | PASS (criterion only) | **PASS** (criterion only) | — | — |
| 18 | `sway_screencap` | H24: ~60 fps (predecessor only) | PASS | PASS (2/2 digests) | **PASS** (2/2 digests) | — | — |
| 19 | `glmark2` | H12: 0.73x-0.89x off-screen, not graded | PASS | PASS (3/3 digests) | **PASS** (3/3 digests) | — | — |
| 20 | `pv_nvenc_nvdec_fps` | H9+H11: hang → fixed; NVDEC 13.3x vs 33.3x | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 21 | `video_nvenc_nvdec` | H10+H11: 720p 0.96x; streams ffprobe-verified | PASS | PASS (13/13 digests) | **PASS** (13/13 digests) | — | — |
| 22 | `geekbench_vulkan` | H5: 94-97 % of bare metal, RTX 4070 | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 23 | `blender_opendata` | H25: GPU render 99.95 % | PASS | PASS (criterion only) | **PASS** (criterion only) | — | — |
| 24 | `vk_compute` | EXTRA (not an nvkvm-pv row) | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 25 | `vk_render` | EXTRA (not an nvkvm-pv row) | PASS | PASS (3/3 digests) | **PASS** (3/3 digests) | — | — |
| 26 | `egl_render` | EXTRA (not an nvkvm-pv row) | PASS | PASS (3/3 digests) | **PASS** (3/3 digests) | — | — |
| 27 | `gl_info` | EXTRA (not an nvkvm-pv row) | PASS | PASS (3/3 digests) | **PASS** (3/3 digests) | — | — |
| 28 | `glx_vgl` | EXTRA (not an nvkvm-pv row) | PASS | PASS (3/3 digests) | **PASS** (3/3 digests) | — | — |
| 29 | `ff_cuda` | EXTRA (not an nvkvm-pv row) | PASS | PASS (6/6 digests) | **PASS** (6/6 digests) | — | — |
| 30 | `ff_vulkan` | EXTRA (not an nvkvm-pv row) | PASS | FAIL (0/9 digests) | **PASS** (9/9 digests) | kf3: `GR_SET_CTXSW_PREEMPTION_MODE` on a bare copy channel answered 0x56 → `vkCreateDevice` VK_ERROR_INITIALIZATION_FAILED (§4.3) | yes, `c9acfbab` |
| 31 | `ff_opencl` | EXTRA (not an nvkvm-pv row) | PASS | PASS (8/8 digests) | **PASS** (8/8 digests) | — | — |
| 32 | `ff_placebo` | EXTRA (not an nvkvm-pv row) | PASS | FAIL (0/2 digests) | **PASS** (2/2 digests) | same (§4.3) | yes, `c9acfbab` |
| 33 | `blender_cycles_cuda` | EXTRA (not an nvkvm-pv row) | PASS | PASS (image within bare metal's spread: median PSNR to the 14 bare renders 98.4 dB ≥ their farthest pair's 94.8 dB) | **PASS** (image within bare metal's spread: median PSNR to the 14 bare renders 98.4 dB ≥ their farthest pair's 94.8 dB) | — | — |
| 34 | `blender_cycles_optix` | EXTRA (not an nvkvm-pv row) | PASS | PASS (image within bare metal's spread: median PSNR to the 14 bare renders 99.5 dB ≥ their farthest pair's 96.5 dB) | **PASS** (image within bare metal's spread: median PSNR to the 14 bare renders 99.5 dB ≥ their farthest pair's 96.5 dB) | — | — |
| 35 | `blender_eevee` | EXTRA (not an nvkvm-pv row) | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 36 | `blender_workbench` | EXTRA (not an nvkvm-pv row) | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 37 | `blender_eevee_vulkan` | EXTRA (not an nvkvm-pv row) | PASS | PASS (1/1 digests) | **PASS** (1/1 digests) | — | — |
| 38 | `vk_ofa` | EXTRA (not an nvkvm-pv row) | PASS | — (added with the OFA fix) | **PASS** (2/2 digests) | (not in `gs1`: the guest had no OFA engine) | — |

"digests" = content digests compared byte for byte with bare metal (frames, rendered images, flow fields,
bitstreams, readbacks, validation outputs, capability lists). "criterion only" = the output is not
deterministic enough to hash, and the item is graded by nvkvm-pv's own criterion (quoted in
`itemdefs.sh`): e.g. `pv_vk_create_device`'s `rc=0 OK`, `weston_client_diff`'s A≠B≠C pixel test,
`blender_opendata`'s completed scenes. ⚠ `geekbench_vulkan` cannot use nvkvm-pv's criterion (composite vs
bare metal): the free Geekbench 7 build prints no score and uploads the result instead, so the item is
graded on all 11 workloads running (their list digested), the upload succeeding and no error line; the
two result pages are linked from the logs (`browser.geekbench.com/v7/gpu/219655` bare metal, `…/219856`
guest in `gs1`). `vk_ofa` is new with the OFA fix (§4.1): a frame and its
(5,3)-pixel shifted copy through `vkCmdOpticalFlowExecuteNV`; the median flow must recover the shift and
the whole flow field must equal bare metal's.

### 3.3 What a PASS rests on

- **Host Xid:** none, in any item of any run (host dmesg slice per item). **Guest Xid:** none.
- **kf3:** no RC (robust-channel recovery) on any host twin in any item. The refusal ledger that master
  gained between `gs1` and `gs2` (one line per control, the first time per boot that the guest RM reads a
  non-OK status from the faked GSP) names **nine controls, all already classified in
  `V3_REFUSAL_AUDIT.md`**: `0x2080012f` (bare metal refuses it too), `0x20800a9a` (← `PERF_BOOST`),
  `0x20800ab8` (P2P caps), `0x20800a9e` / `0x20800a9c` / `0x20800a1e` (UVM fault/access-counter buffer
  teardown), `0x20808165`, `0x2080a0d1` (GSS-legacy perf samples) and `0x20810107` (BINAPI). None is new
  with this branch. The trap plane's `refused=6` counter is already at 6 in the boot's first status line
  (driver load) and never moves during the items.
- **Wedge probe** (a checked Vulkan compute job) after every non-PASS item: the boot still rendered every
  time, and the items after each failure in the same boot passed.
- `kf3_unserviced` (guest-RM control RPCs the emulated GSP leaves unanswered, `unserviced.rs`) is recorded
  per item in `guest.res`; at the user-ioctl level (nvdiff, host vs guest) they surface only as the
  divergences listed next.
- **Divergences from bare metal that change no output** — all pre-existing and already documented. The
  complete set of status divergences over the 8 Vulkan items of `diag2`, host vs guest, is
  `traces/v3_gfxset/diag2/nvdiff_status_by_item.txt`, and it is this list:
  - `NV2080_CTRL_CMD_PERF_BOOST` (`0x2080200a`), `NV2081_BINAPI` `0x20810107`/`0x20810108` and the
    GSS-legacy perf sample `0x2080a0d1` answer 0x56 (`V3_REFUSAL_AUDIT.md` rows `0x20810107`,
    `0x2080a0d1`; `V3_HEADLESS_GRAPHICS.md` §6.3);
  - a `GF100_DISP_SW` (`0x9072`) allocation answers 0x1f and an `nvidia-modeset` ioctl fails: the guest's
    display side is displayless (§7);
  - guest dmesg `kgmmuClientShadowFaultBufferUnregister_IMPL: Unregistering non-replayable fault buffer
    failed (status=0x56)` when a CUDA or NVENC process exits — UVM's shadow fault-buffer teardown. UVM is
    outside this task (recorded, not chased).

### 3.4 Performance — recorded, not graded

| figure | bare metal | guest | guest ÷ bare metal (>1 = guest better) |
|---|---|---|---|
| vkpeak, all 14 figures (GFLOP/s, GIOP/s) | fp32 15061.6 | fp32 15044.9 | 0.99–1.00 |
| egl_offscreen (Mtri/s) | 2.6 | 2.6 | 1.00 |
| glmark2 off-screen score | 41082 | 10720 | 0.26 |
| glmark2 windowed (headless weston) score | 2378 | 937 | 0.39 |
| GL fill rate (Gpix/s) | 162.684 | 153.975 | 0.95 |
| glFinish round trip (µs; lower is better) | 9.07 | 62.08 | 0.15 |
| GL draw calls (k/s) | 21379.8 | 4485.9 | 0.21 |
| texture upload (GB/s) | 9.484 | 8.788 | 0.93 |
| glReadPixels (GB/s) | 8.874 | 6.103 | 0.69 |
| CPU writes into a mapped GL buffer (GB/s) | 8.393 | 9.983 | 1.19 |
| wlr-screencopy capture (fps) | 62 | 61.2 | 0.99 |
| NVENC H.264 1080p (fps) | 198 | 142 | 0.72 |
| NVDEC h264_cuvid (× real time) | 17.4 | 10.1 | 0.58 |
| Blender Open Data monster (samples/min) | 1069.89 | 972.053 | 0.91 |
| Blender Open Data junkshop (samples/min) | 615.914 | 251.899 | 0.41 |
| Blender Open Data classroom (samples/min) | 511.695 | 473.969 | 0.93 |

From `gs3`; `traces/v3_gfxset/<run>/perf.md` has every figure of every run. The GPU-bound figures
(vkpeak, egl_offscreen, fill rate) agree across `gs1`, `gs2` and `gs3` within 1 %; the per-sync and
transfer figures vary by up to 40 % between runs on this nested box (GL draw calls 0.21–0.36×, NVENC
0.63–0.73×, texture upload 0.93–1.01×), and Blender Open Data by up to 20 %.

- **GPU-bound work runs at bare-metal speed:** all 14 vkpeak figures 0.99–1.00× (nvkvm-pv: 1.00×),
  egl_offscreen 1.00×, GL fill rate 0.95×, texture upload 0.93×, wlr-screencopy capture 0.99×.
- **Anything that synchronises with the GPU per call pays the VM round trip:** a `glFinish` costs 62 µs
  against 9 µs, GL draw-call throughput is 0.2–0.4×, glmark2 0.26× off-screen and 0.39× windowed (short
  frames, one sync each), NVENC 0.72× and NVDEC 0.58×. Each sync is a doorbell the VMM traps and a
  completion interrupt it injects, and this guest is nested (§3.1). Not investigated here — it is not a
  correctness question and nothing in this set grades it; the per-submit trap cost is the subject of
  roadmap item 4 (the guest doorbell module).
- **One figure is above 1, and it was re-checked** (the MC_SERVICE_INTERRUPTS lesson: a refusal the guest
  reads as "wait over" makes a wait end early): CPU writes into a mapped GL buffer, 1.19–1.23×. The timed
  loop maps a freshly orphaned buffer (`GL_MAP_INVALIDATE_BUFFER_BIT`), copies 8 MB with the CPU and
  unmaps, 40 times, with one `glFinish` at the end and no GPU work queued: there is no GPU wait in it to
  end early. The figure is CPU memcpy speed into that mapping; the plausible cause is the mapping's
  memory type in the guest (not measured).
- ⚠ nvkvm-pv's glmark2 off-screen figure was 0.73–0.89× (a different box and a Mode-1 path); this set's
  0.26× is on a nested guest. Neither is graded; neither is quoted as a product number
  (`nvkvm-pv docs/reference/quoting-numbers.md`).

### 3.5 The nondeterministic images: Cycles CUDA and OptiX

The one output in the set that is not deterministic on bare metal. The scene is fixed (seed 1, 64
samples, no denoiser, no adaptive sampling, 640×360), yet **no two bare-metal renders are identical**
(presumably the order in which the GPU's atomics accumulate samples). So the question for these two items
is not "equal?" but
"drawn from the same distribution as bare metal's renders?", and it needs the bare-metal distribution
measured, not assumed (`traces/v3_gfxset/cycles_noise/`: the pairwise table, each image's spread, and
every image as its differences from one reference).

| | Cycles CUDA | Cycles OptiX |
|---|---|---|
| bare metal, 14 renders (`gs1` ×2 + `nd/h1`–`h12`): values that differ between any two (of 691 200) | 3–15 | 2–10 |
| largest difference, any pair | 1 LSB | 1 LSB |
| bare metal: each render's spread (mean differing values to the other 13) | 6.0–11.7 (mean 7.6) | 4.5–8.0 (mean 5.7) |
| guest, 6 renders (`gs1`, `gs2`, `gs3`, `gs3c1`–`c3`): spread to the 14 bare renders | 5.9–11.9 (mean 7.8) | 5.0–7.7 (mean 5.8) |
| guest: largest difference to any bare render | 1 LSB | 1 LSB |

- The guest's renders have the same spread as bare metal's own, and no difference anywhere exceeds 1 LSB
  in 8 bits. A kayfabe fault — a lost write, a wrong page, a stale buffer — would move far more values,
  by far more.
- **`gs2` (`b15c7cc8`) graded Cycles CUDA `FAIL(DIFF)` under the first rule** (PSNR 96.0 dB against a
  floor of 99.3 − 3 dB taken from ONE bare-metal pair). Its render is the guest's farthest (spread 11.9,
  9–17 differing values), and bare metal's own `nd/h9` sits at the same place (11.7, 9–15). The rule, not
  the render, was wrong: it measured the spread from one sample. It now uses every bare-metal render
  available (§2); regraded, `gs1` stays 33/37 (its Cycles images MATCH either way) and `gs2` is 38/38
  (`gs1/verdict_v1.md`, `gs2/verdict_v1.md` keep the first grading).
- ⚠ With 14 bare renders, a guest render that is merely another sample still has roughly a 1-in-15
  chance of being the farthest of all; a `DIFF` at the tail, with every difference 1 LSB, is re-run
  before it is read as a defect. A render outside the spread by more than that — or several guest renders
  outside it that agree with each other — would be a real difference.

---

## 4. Failures: root causes and fixes

Master failed 4 items of 37, for **two** defects; the first fix exposed a **third** (no item failed on it,
the guest's own kernel log did). None was a host Xid, a PTE kind or a UVM path. Two were kf3's statement
of the device (which engines exist, and in what order RM must see them); one was a control kf3 served for
GR channels only.

### 4.1 The optical-flow engine (OFA) was never advertised — `pv_vk_rt_ext`, `vk_info`

- **Symptom (`gs1`):** nvkvm-pv's H3 "RDR2 check" (`vkCreateDevice` with 7 RT/NVX extensions) exited 2:
  `FATAL: requested extension VK_NV_optical_flow not in advertised set`. `vk_info`: features, properties
  and formats equal to bare metal; the extension list 250 vs 251 and the queue families differ (no sixth
  family, `VK_QUEUE_OPTICAL_FLOW_BIT_NV`).
- **Failure point:** kf3's `hostquery::classify_engine` had no arm for `NV2080_ENGINE_TYPE_OFA0`
  (`0x33`), so the host's OFA (listed by the host's `GET_ENGINES_V2`) was dropped: no FIFO row, no
  `engineCaps` bit, no constructed falcon. The guest RM then removes `NVC7FA_VIDEO_OFA` from its class
  list, and the Vulkan driver withholds the extension and the queue family.
- **Fix (`bc1a3c3e`):** OFA is stated exactly as NVENC/NVDEC are (`V3_VIDEO_ENGINES.md`), every fact derived:
  - presence and count from the host's `GET_ENGINES_V2`, the falcon (`registerBase`, ctx) from the host's
    `GET_CONSTRUCTED_FALCON_INFO` — both unprivileged;
  - class sets **generated** (`tools/derive_classes.sh`: `NV*_VIDEO_OFA` → `optical_flow`): Ampere
    `C6FA`/`C7FA`, Ada `C9FA`, Hopper `B8FA`, Blackwell `CDFA`/`CEFA`/`CFFA`/`D1FA`/`D2FA`, Turing none;
  - family constants from ogkm 580.159.04 — `ENG_OFA(i)` = `OBJOFA 0xdd7bab` `<< 8 | i`,
    `MC_ENGINE_IDX_OFA0` 81, `RM_ENGINE_TYPE_OFA0` `0x3e`, `NV2080_ENGINE_TYPE_OFA0/1` `0x33`/`0x3e`,
    notifiers 153 / `180 + n − 1`, fault id `NV_PFAULT_MMU_ENG_ID_OFA0` per family — and nouveau's
    `DEV_TYPE` `0x16`, each pinned by a test to the real GA106's captured FIFO row (`ctl_20801112` row 9);
  - host verb **authored**: the 12-byte `NV_OFA_ALLOCATION_PARAMETERS` with the twin's own instance;
  - completion = the host OFA twin's **non-stall event**, raised on the guest vector it was served.
  An OFA instance a family's headers cannot state (GB100's `OFA1`: no fault id in any header) is **not
  advertised, by name**, instead of refusing the whole engine table.
- **Evidence (`diag1`):** `pv_vk_rt_ext` PASS, `vk_info` 6/6 digests, and `vk_ofa` PASS with its flow
  field equal to bare metal's. The kf3 log shows where the work ran: the guest's OFA channel born as a
  passthrough twin (`engine=0x33`) and its `0xc7fa` (`NVC7FA_VIDEO_OFA`) object allocated on that host twin.

### 4.2 RM's SW pseudo-engine must be the LAST FIFO row — found by reading the guest's dmesg after 4.1

- **Symptom (`diag1`, `diag2`):** every item passed, but the guest kernel printed
  `kfifoEngineInfoXlate_GM107: Asked for host-specific type(0x3) for non-host engine type(0xf)`
  **104** times in `diag1`'s boot and **343** in `diag2`'s (313 of them inside its 8 items) — **0** in
  `gs1` and on bare metal.
- **Failure point:** `kfifoGetNumEngines_GM107` returns `engineInfoListSize − 1` — *"we don't count the SW
  engine entry at the end of the list"* (`ogkm-580: kernel_fifo_gm107.c:838-840`) — so every `i <
  numEngines` walk skips the last row. kf3 served rows in the host's `GET_ENGINES_V2` order, which lists
  SW (`0x22`) before OFA0 (`0x33`): OFA became the uncounted row and SW an engine RM walked. Before 4.1,
  SW was last only because no advertised engine type sorted after `0x22` — correct by accident.
- **Fix (`88dc8778`):** `hostquery::software_last` — SW is always the last row, for every host (the real
  GA106 table ends with SOFTWARE). **Evidence (`diag3`):** 0 prints; 6/6.

### 4.3 `GR_SET_CTXSW_PREEMPTION_MODE` on a bare copy channel — `ff_vulkan`, `ff_placebo`

- **Symptom (`gs1`):** every ffmpeg Vulkan and libplacebo filter failed at device creation —
  `Device creation failure: VK_ERROR_INITIALIZATION_FAILED`, ffmpeg exit 187. The other Vulkan items
  (`vkpeak`, `vk_compute`, `vk_render`, `geekbench_vulkan`, EEVEE on Vulkan) passed.
- **Failure point (`diag1`, nvdiff host vs guest of one ffmpeg Vulkan init):** for each transfer queue
  the Vulkan driver allocates a **bare** `AMPERE_CHANNEL_GPFIFO_A` (parent = the device; RM wraps it in an
  implicit TSG, `ogkm-580: kernel_channel.c:352-381`) and sets CILP on it with
  `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`0x20801210`, `hChannel` = that channel). Bare metal:
  `NV_OK`. kf3: `0x56` — the statement's twin filter kept **GR** twins only, a copy channel has none, so the
  answer was "not ours". ⚠ `V3_REFUSAL_AUDIT.md` had already listed this exact refusal and classed it
  *B (low)* — correctly for `vulkaninfo` and `vkpeak`, the workloads it ran, which survive it; ffmpeg's
  device creation does not (the audit row now carries the correction). The same call on TSG targets (GR)
  matched bare metal. The driver then freed the channel, retried (the retry's implicit TSG allocation
  failed `0x1f` in the guest RM — never seen once the refusal was gone, not chased further), and gave up.
  4 of 4 calls on bare copy channels diverged.
- **Fix (`c9acfbab`, `kf-qemu/src/chan.rs`):** a target with no GR twin but with twins gets the same
  **authored** verb on those twins' own host channel groups (one call per host group); the **host's status
  is the answer** — never a forged OK. A target with no twin at all is still not ours; GR targets are
  served as before (one call per host group). The verb runs off the GSP lock, as before.
- **Evidence (`diag2`):** `ff_vulkan` 9/9 and `ff_placebo` 2/2 digests equal to bare metal, plus the
  Vulkan regression set 8/8; nvdiff after the fix: 0 divergences on `0x20801210` and on channel
  allocations; the guest's `nvAssertOkFailedNoLog … KEPLER_CHANNEL_GROUP_A` lines 9 → 0.

### 4.4 Merging master: Hopper's OFA

Master `5d2b0a33` stopped refusing Hopper's engine table: it states Hopper's fault ids from UVM's copy of
the chip's `dev_fault.h`. The same header states `NV_PFAULT_MMU_ENG_ID_OFA0 = 53`
(`kernel-open/nvidia-uvm/hwref/hopper/gh100/dev_fault.h:81`), so `ofa_fault_id` now states it too
(`b15c7cc8`) instead of dropping a GH100 host's OFA, and `authored::hwref_check` holds OFA0..3 to every
die group's header **in both directions** — an id the header states and kf3 does not would be an engine
silently not advertised. Hardware-unverified.

---

## 5. Merge readiness

| check | `286387f3` (the fixes on master `dd3aed08`) | `b15c7cc8` (+ master `5d2b0a33`) | **`d06833f0`** (+ master `59cc98a9`) |
|---|---|---|---|
| crate tests (`cargo test`, every `kf-*` crate) | 1489 passed, 0 failed | 1585 passed, 0 failed | **1596 passed, 0 failed** |
| v3 gates | 9/9 | 9/9 | **9/9** |
| `KF_DEVICE=kf3 fast_suite 180` (thin guest) | 30/30 | 30/30 | **30/30** |
| CUDA ladder (fat guest) | cup3 = 43, cup8 BAD=0 MAXERR=0 | cup3 = 43, cup8 BAD=0 MAXERR=0 | **cup3 = 43, cup8 BAD=0 MAXERR=0** |
| this set | `diag1`–`diag3` | `gs2`: 37/38 under the first image rule (Cycles CUDA, §3.5); **38/38** regraded against the 14-run spread | **`gs3`: **38/38**** |

All on the RTX 3070 box above, `KF3_FB_MB=6144`, strictly serial; logs in `traces/v3_gfxset/bar/`.
`d06833f0` is the branch head to merge (the commits after it change only documents and traces).

---

## 6. Constraints and family coverage

- **No v3 constraint changed.** Host verbs are authored (`NV_OFA_ALLOCATION_PARAMETERS` with the twin's
  instance; `SET_CTXSW_PREEMPTION_MODE` on the twin's own host group), completions are host events (the
  OFA twin's non-stall event; the host's own status for the preemption call), nothing new blocks a vCPU or
  runs under the GSP lock, no VMM address is added or exposed, and no `unsafe` was added.
- **Derived, not hand-kept:** the OFA class sets are generated from ogkm's class lists; its per-family
  constants come from ogkm 580.159.04 headers and are held to them by tests (`kf-rm/tests/ofa_engine.rs`,
  `authored::hwref_check`); the per-die facts (does the host have an OFA, its falcon) come from
  unprivileged host controls.
- **Families:** Ampere GA10x is measured (this document). Ada (OFA0, fault id 10), Hopper (53, from the
  UVM hwref copy) and Blackwell (48; `OFA1` not advertised, by name) are derived and unit-tested, not run.
  Turing has no OFA; a Turing host that listed one would lose only that engine.

---

## 7. The display phase (next, not this task)

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

## 8. Reproduce

```
# on a box provisioned by provision_box / provision_host_driver / provision_bench_tree / build_kf3:
bash scripts/bench/provision_guest_gfx.sh host && bash scripts/bench/provision_guest_gfx.sh
# the static ffmpeg of V3_VIDEO_ENGINES §5 at /workspace/video/ff/bin/ffmpeg, then:
bash scripts/bench/gfxset/provision.sh          # the set into the guest image (~30 min)
bash scripts/bench/gfxset/suite.sh <run> [item...]
# a later kf3 revision against an earlier run's bare metal (same box, same image):
GSET_HOST_FROM=<run> bash scripts/bench/gfxset/suite.sh <run2> [item...]
# the ioctl-level host-vs-guest diff of an item:
GSET_NVDIFF=1 bash scripts/bench/gfxset/suite.sh <run> <item>
python3 scripts/bench/gfxset/src/nvdiff/nvdiff.py diff <res>/<item>.host_nvdiff.jsonl <res>/<item>.guest_nvdiff.jsonl
# the Cycles spread (§3.5): more bare-metal renders, pooled into the image grade, then the pairwise table
for k in $(seq 1 12); do bash scripts/bench/gfxset/host.sh /workspace/gfxset/results/nd/h$k blender_cycles_cuda blender_cycles_optix; done
GSET_NOISE_DIRS="$(ls -d /workspace/gfxset/results/nd/h*)" GSET_HOST_FROM=<run> bash scripts/bench/gfxset/suite.sh <run2>
CYCLES_SPARSE=<prefix> bash scripts/bench/gfxset/cycles_noise.sh "<bare-metal run dirs>" "<guest run dirs>"
```

Timing on the RTX 3070 box: bare metal ×2 for 37 items ≈ 41 min, the guest's 37 items in one boot ≈ 33 min,
one isolation re-run ≈ 1–2 min. `suite.sh` writes `verdict.md`, `imgcmp.txt` and `GSET_SUITE_VERDICT=`;
`perf.py <res>` and `triage.py <res>` are the performance table and the per-item evidence.
