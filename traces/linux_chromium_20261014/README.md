# Chromium / Electron GPU load on the Linux guest (kf3 c287af4f) — 2026-10-14

STATUS: RESEARCH, 2026-10-14 (complete for this question; H1 not falsified)

## Question

The owner's Windows 11 guest is stable with Edge but three TDR cycles followed the start of the Minecraft Launcher
(Chromium/Electron class app): guest LiveKernelEvent 141 + nvlddmkm 14/153, three unplanned GSP unloads on the kayfabe
side, the first right after a refused RmControl 0x906f0102 (RESET_CHANNEL). Does the same class of GPU use reset the
GPU on the Linux guest (open module 580.159.04, which can be built from source and debugged)?

## Hypotheses and falsifiers (written before any graded run)

- **H1** "Chromium / Electron GPU use on the Linux guest triggers no unplanned GSP unload within >= 10 minutes of mixed
  load per configuration."  FALSIFIED by one unplanned unload (`GSP phase Running -> Suspending` in the kf3 log with
  no S3/PM request from the guest), one guest Xid 119 / GSP timeout / host `NVRM: Xid` delta, or a hang of the guest GPU
  (page fps counters stop, guest ssh `nvidia-smi` hangs). Supporting-only evidence: zero of those in every configuration.
- **H2** (only if H1 is falsified) "the failing feature is X" — gets a one-variable A/B (X on vs X off, everything else
  identical), falsified if the reset also happens with X off or does not happen with X on.

Measured and inferred are kept apart below: "measured" = a counter or a log line in this directory; "inferred" = my reading.

## Method

- `scripts/bench/display/chromium_lane.sh <rev> <tag> <guest-script>` boots a throw-away overlay of the desktop image
  (Ubuntu 24.04, Cinnamon on Xorg, NVIDIA 580.159.04 open module, GSP on) on kf3 c287af4f (same harness as
  `desktop_smoke.sh` / `desktop_s3.sh`), beside the owner's Win11 VM, under `flock /tmp/claude-linux-guest.lock`.
- Guest software: Google Chrome 155.0.8059.79 (deb) and Electron 44.7.2 (zip + a hello-world BrowserWindow app).
- Load: `scripts/bench/display/chromium/cdp_load.py` drives the browser over CDP: one tab (`mix.html`) with a WebGL2
  scene (3000 instanced lit cubes, 4x MSAA FBO, blur post pass, per-frame texImage2D from a canvas, readPixels sync),
  a canvas-2D scene (gradients, shadowBlur, filter blur), 220 CSS-animated layers + backdrop-filter, a scrolling
  3000-card DOM and a looping 1080p60 video (H.264, then VP9 at 20 %, AV1 at 30 %); fullscreen video at 40 %;
  24 extra tabs/windows at 50 %; `kill -9` of the GPU process at 70 %; a WebGPU tab at 80 %.
- Grades per run: guest dmesg (Xid, NVRM, Flip event timeout, nvAssert, GSP timeout), host kf3 log
  (`GSP phase Running -> Suspending`, USER_RING_REFUSED, USER_RING_SCAN, `display: STALL`, `REFUSED fn76`, `BLACK:`),
  host `NVRM: Xid` delta from `journalctl -k`.
- Baseline of a desktop boot without any browser (measured, setup run): guest dmesg has 24 NVRM lines / 12 nvAssert
  lines at boot (NV_ERR_NOT_SUPPORTED 0x56 from golden-image / gpushareddata paths), kf3 has about 30-39 `REFUSED fn76`
  lines (GSP_RM_CONTROL, status 0x56), 0 unplanned unloads, 0 stalls.

## Result in one paragraph (measured)

**H1 was not falsified.** Across 11 graded runs of Google Chrome 155 and Electron 44 on the Linux desktop guest (X11
and Wayland, ANGLE-GL and ANGLE-Vulkan, YouTube, 40 launch/kill cycles) about 2 hours of GPU load in total,
there was no unplanned GSP unload (`GSP phase Running -> Suspending` count 0 in every run; only the boot sequence
Cold -> Running), no guest Xid, no `Flip event timeout`, no GSP timeout, no `nvAssert` beyond the boot baseline (except the BAR1 ones of the Wayland runs, finding 1), no
host `NVRM: Xid` delta, USER_RING_REFUSED = 0, USER_RING_SCAN = 0, `BLACK:` = 0. The Linux guest does **not** reproduce
the Windows TDR with Chromium-class GPU use. Four things did show up and are worth the owner's attention (below):
a BAR1 VA exhaustion on the Wayland path, a display STALL diagnostic that fires in every browser run, the unavailability of
NVDEC/VA-API in this guest (so the Windows hardware-decode path has no Linux counterpart here), and that a
compute exception on Linux is handled by the host RM and forwarded as RC_TRIGGERED without any RESET_CHANNEL fn76.

## Runs (kf3 c287af4f, GSP on, Ubuntu 24.04 guest, open module 580.159.04; evidence in the directory named in the first column)

Columns: unl = unplanned GSP unloads; ring = USER_RING_REFUSED / USER_RING_SCAN; gXid = guest Xid lines; hXid = host Xid delta;
flip = guest `Flip event timeout`; STALL = distinct `display: STALL Window` episodes (the log caps at 16 per VM); fn76 = `REFUSED fn76` lines
(boot baseline of a no-browser desktop: 30-39, all status 0x56); load = minutes of the mixed load of the method section.

| run dir | config | load | unl | ring | gXid | hXid | flip | STALL | fn76 | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| cd1 | Chrome, X11, default flags (ANGLE-GL) | 11.3 min | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | clean |
| cv1 | Chrome, X11, `--use-angle=vulkan` + WebGPU flags | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 32 | clean; WebGL2 unavailable on this stack (see 4); WebGPU 7918 frames |
| ch1 | Chrome, "hardware decode" flags + `nvidia-vaapi-driver` | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | clean; decode stayed software (see 3) |
| ed2 | Electron 44.7.2 hello-world (BrowserWindow, `--no-sandbox`), X11 | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 32 | clean |
| yt1 | Chrome, X11, YouTube watch page (AV1/VP9 software decode) + the same churn/kill/WebGPU phases | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | clean (the YouTube tab was not sampled for fps; decoder events only) |
| cyc1 | Chrome start/load/stop x 40 (every 3rd ended with SIGKILL of the whole browser) | 40 x ~56 s | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 31 | clean |
| wl1 | Chrome `--ozone-platform=wayland` in a nested weston 13 (x11 backend) | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | no reset; **36 BAR1 VA-alloc failures** in the churn phase (see 1) |
| wl2 | same as wl1 (repeat) | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | no reset; 12 BAR1 failures (reproduced) |
| wl3 | same as wl1 with `bar1-size=512M` (one variable) | 11.3 | 0 | 0/0 | 0 | 0 | 0 | 16 (cap) | 33 | no reset; **0** BAR1 failures |
| mf1 | CUDA store to an unmapped address (SM exception), 3 contexts | seconds | 0 | 0/0 | 0 | **3** (Xid 13 by the host RM) | 0 | 1 | 37 | RC forwarded (8 `RC host twin` lines), no unload, guest recovered (see 5) |
| pv1 | VA-API / NVDEC probe | seconds | 0 | 0/0 | 0 | 0 | 0 | 0 | 38 | CUDA/VA-API unusable here (see 3) |
| (dbg1, ed1) | harness debugging: a 3.5-minute Chrome run (clean) and an Electron run that died at once in the harness (Electron has no `Target.createTarget`) | - | 0 | 0/0 | 0 | 0 | 0 | - | - | not counted |

Each 11.3-minute run: WebGL2 scene + canvas-2D + 220 CSS animated layers with backdrop-filter + a scrolling 3000-card DOM + a looping
1080p60 video (H.264, then VP9 at 2.4 min, then AV1 at 3.6 min) + fullscreen video (53 s) + 24 extra tabs/windows (about 100 s) + `kill -9` of the GPU
process (Chrome restarted it: `--gpu-recent-crash-count=1` afterwards, no tab crash; under Wayland the page stopped painting for a while, finding 6) + a WebGPU window. Page fps
medians were 60 in every X11 run (minimum 49 during the churn).
The kill of the GPU process is a user-space crash only: the kernel/RM saw nothing (no Xid), as expected.

## Findings worth the owner's attention

1. **BAR1 VA exhaustion on the Wayland path (measured; the cause is inferred).** wl1: 36 guest lines `dmaAllocMapping_GM107: can't alloc VA space for mapping`
   + `NV_ERR_NO_MEMORY ... kern_bus_gm107.c:3186` (reusemappingdb) at guest uptime 392-428 s, i.e. in the tab/window churn phase; wl2: 12 of them; wl3 with a
   512 MiB BAR1: 0. No X11 run had any (all stayed at the 24-line boot baseline). Both this guest and the owner's Win11 VM
   (`boundary-kayfabe-2990`: `fb-mb=4096,bar1-size=134217728,bar2-size=33554432`) have a **128 MiB BAR1**. The guest survived (no Xid, no reset), but a driver
   that does not survive a failed BAR1 mapping could turn this into an error that ends in a TDR; this is an inference and was NOT shown on Windows. It is cheap to test:
   start the Win11 VM with `bar1-size=536870912` (or 1 GiB) and start the Minecraft Launcher; if the TDRs vanish, BAR1 is the lead, if not, it is excluded. Note
   that a real 4070 Ti SUPER presents a 256 MiB-16 GiB BAR1.
2. **`display: STALL` fires in every browser run, 16 episodes (the cap) within the first 3-6 minutes** (`kf3-events.txt` has the wall times), against 0-2 per run in the earlier
   glxgears-only desktop runs and 0 in the no-browser/pv1 runs. The guest never saw a `Flip event timeout`. All reports are `Window 6 (chn 7) update 0x1 parked 1000-1013 ms: ready, waiting for a vblank ...`
   with vblanks flowing (head 3, 60 Hz). Reading of `crates/kf-qemu/src/display.rs` (lines ~2840-2860): the stall key is `(chn, update)` and `parked_since` is kept while any entry with the same
   key is parked, so a continuous stream of 60 Hz flips (same update number 0x1 every time) can be reported as one long stall even when every flip completes (inferred, not proven: the per-flip
   latency was not measured). Because the log stops at 16, a real long stall later in a run would have been hidden. The Windows runs 400/401 stall is the same message class.
   Suggestion (not done, no fixing here): add a flip sequence to the key or log the park duration at release.
3. **No hardware video decode in the Linux guest, so the Windows NVDEC/D3D11VA path has no Linux counterpart in this experiment.** Chrome reports "Video Decode: Hardware accelerated" in chrome://gpu but the actual decoders in
   every run were `FFmpegVideoDecoder` / `VpxVideoDecoder` / `Dav1dVideoDecoder` (`kIsPlatformVideoDecoder=false`, `MEDIA` lines in events.log). With the Chrome VA-API flags and `nvidia-vaapi-driver`,
   the `direct` backend fails at `nv_alloc_object NV01_ROOT_CLIENT failed` (pv1) and the EGL backend at `CUDA ERROR 'no CUDA-capable device is detected' (100)` for the vainfo
   process, although a CUDA context could be created in mf1 (and failed afterwards because of the fault). I did not chase this further (separate question: NVDEC under kf3 on Linux).
4. **ANGLE-Vulkan cannot import dmabuf EGLImages** here (`DmaBufImageSiblingVkLinux.cpp: Internal Vulkan error (-8)`, in cv1's browser.log), so WebGL2 was unavailable in that config (the page reported `no webgl2` from
   the first second; canvas-2D/CSS/video/WebGPU ran 11 minutes without trouble). Not compared against bare metal on this host (host driver is 595.91.07, the guest 580.159.04).
5. **A compute exception is not a reset on Linux (mf1).** A CUDA kernel storing to 0xdead00000000 produced a host `NVRM: Xid 13` (Graphics SM Warp Exception, Out Of Range Address, pid = our QEMU), kf3 logged
   `RC host twin ... wrote its notifier: except_type=0xd (Xid 13) - forwarding RC_TRIGGERED` (8 lines), the CUDA caller got error 719, the guest had no Xid of its own, no `REFUSED fn76` for 0x906f0102 and no GSP unload,
   and nvidia-smi/glxgears worked afterwards. In ogkm, NV906F_CTRL_CMD_RESET_CHANNEL is sent from `kernel_rc_watchdog_callback.c` (watchdog timeout), `kern_gmmu_gv100.c` (MMU fault with the
   channel reset reason MMU_FLT) and `kernel_channel.c`; none of the three paths fired in any Linux run. Windows' first refused control 0x906f0102 therefore came from a path the Linux runs never reach (a stuck channel
   past a watchdog, or an MMU fault in a guest-owned channel), which is consistent with the Windows incident being a guest-side timeout/fault, not generic Chromium load.
6. Page rAF stalls after the GPU-process kill under Wayland: in wl2 the main page painted 0 fps for 30 s then recovered, in wl3 it stayed at 0 (video playback and the kernel/RM stayed healthy; WebGPU window got 2 frames).
   wl1 recovered at once. This looks like Chrome/weston frame-callback behaviour after a GPU process restart in a nested compositor; it is not a GPU hang (no Xid, vblanks continue). Not investigated.

## What remains Windows-only (not exercised by any Linux run here)

DirectComposition / DXGI flip-model swap chains and MPO overlay planes (the Windows stall class of runs 400/401 is an overlay-plane flip); D3D11 shared textures/keyed mutexes and `NT handle` sharing between
processes; D3D11VA hardware decode through NVDEC (see finding 3: none on Linux here); WDDM paging/`MMU_INVALIDATE` and preemption (`dxgkrnl` scheduler, GPU watchdog TDR timing, `DxgkDdiResetFromTimeout`);
Game Bar / overlay hooks; HDR/advanced-color paths. Linux runs do cover: GL/ANGLE, Vulkan, WebGPU, channel create/destroy churn, GPU process crash, many contexts, fullscreen flips through Muffin, CPU-visible BAR1 mappings (Wayland).

## Recommendation

Linux does **not** reproduce the Windows TDR with Chromium/Electron GPU use (H1 stands; no H2 was needed), so Linux/ogkm is not a better debug environment *for this symptom* until a trigger is found. Cheap next steps, in order of cost:

1. Windows A/B: `bar1-size=536870912` on the Win11 VM, then start the Minecraft Launcher (finding 1). One boot.
2. Make the Windows incident observable the same way: the kf3 log for the three unloads should be searched for `REFUSED fn76 ... 0x906f0102` plus the preceding GSP events and the `display: STALL` lines (the stall cap of 16 is a visibility risk; raising it or logging at release costs nothing).
3. If a Linux reproduction is still wanted: force the two RESET_CHANNEL callers on Linux (watchdog: a channel whose GPFIFO is stalled past the RC watchdog timeout; MMU fault in a graphics channel instead of an SM exception) and compare the fn76 trail with the Windows one.

## Reproduction

```
# on the host (one guest at a time): flock /tmp/claude-linux-guest.lock
scripts/bench/display/chromium_lane.sh c287af4f <tag> <guest-script> [timeout]    # boots a throw-away overlay, pushes PUSH_DIR into ~/kf, runs the script, grades
# guest scripts (scripts/bench/display/chromium/): run_config.sh (CFG_KIND chrome|electron, CFG_NAME, CFG_MIN, CFG_FLAGS, CFG_ENV, CFG_EXTRA),
#   run_wayland.sh, run_cycle.sh, run_fault.sh, probe_vaapi.sh, setup.sh / setup2.sh (packages, once, into the persistent base overlay)
# pages: mix.html, webgpu.html; driver: cdp_load.py (CDP over a websocket; python3-websocket)
```
Per-run files: `guest.log` (script output), `guest-out/<name>/{events.log,metrics.jsonl,gpuinfo.json,gpu_report.txt}`, `dmesg.txt`, `host-grade.txt`, `kf3-events.txt`
(wall-time stamped interesting kf3 lines), `kf3-filtered.txt`, `guest-grade.txt`, `host-nvrm-tail.txt`. Not kept: the qcow2 overlays and the full qemu.log (on the host in `/var/lib/kf-windows-20261005/claude-chromium-<tag>/`).
Caveats: one kf3 revision (c287af4f), one GPU (RTX 4070), one browser version; the load is synthetic (a local page), not the Minecraft Launcher; Chrome was driven over CDP, which itself keeps the page active; the Electron
sandbox was off (`--no-sandbox`); video decode was software in all runs.
