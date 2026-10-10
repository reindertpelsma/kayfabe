# Native-NVIDIA Windows overlay (MPO) reference attempt -- kf_overlayprobe on a vast.ai RTX 4090 (2026-10-10)

**STATUS: LIVE (final, box destroyed), 2026-10-10.** Result in one line: **this vast box cannot serve as an MPO reference.** The RTX 4090 has no real display head
(`nvidia-smi`: *Display Attached: No*; the 1024x768 output is a SIMULATED NVIDIA output with no monitor), and on that output the real NVIDIA driver reports
**no overlay and no hardware composition at all** (`CheckOverlaySupport` flags 0x0 for NV12/YUY2/P010, `CheckHardwareCompositionSupport` flags 0x0), on both 580.88 and 595.97.
Consequently no YUV flip-model swap chain can be created, the overlay scenarios never start (exit 5 SETUP), and native Edge + Shorts does not get an overlay. That is a
property of the box (headless), not a statement about what a real head reports. Everything below is labelled *measured* or *inferred*.

## Box and method (measured)
- vast instance 55276586 (the owner's rental; Windows 11 Enterprise LTSC Evaluation build 26100, RTX 4090 24 GB, AMD Ryzen 9 7950X, 4 logical CPUs). Template
  `Windows 11 KVM on Vast.ai` (hash `183b10af7fd788d08aca02e31811d70d`): **it needs `vms_enabled` hosts and the KVM image.** On a host without VM support the instance sits in
  `loading` with `status_msg: machine does not support VMs` (instance 55274607 did); an offer search must carry `vms_enabled=true`. A `create` without `--cancel-unavail`
  can also return `success: False` and leave a stopped instance. Address/port: from `vastai ssh-url 55276586` (not recorded).
- Driver: template 580.88 (WDDM 32.0.15.8088) first, then upgraded with the signed NVIDIA installer to **595.97** (sha256 `979ed00f...3122`, Authenticode Valid, same file as the
  baseline; `-s -n -clean Display.Driver`; after the install the adapter had no output until the next reboot). Autologon of `vast` (throwaway password, controller scratch only)
  gives the interactive session; probes run through `native/run_overlay_it.ps1` (scheduled task `/it`, as the adapter does).
- Output ownership (measured): `Get-PnpDevice -Class Display` = only `NVIDIA GeForce RTX 4090 (OK)`; `-Class Monitor` = **empty**; `GraphicsDrivers\Configuration` holds
  `SIMULATED_10DE_2684_...` (the NVIDIA adapter's simulated output) and `MSNILSIMULATED_1414_008D...` (Basic Display); `Win32_VideoController` 1024x768@60;
  `nvidia-smi -q`: `Display Attached : No`, `Display Active : Enabled`; Edge's `edge://gpu`: `Display[...] bounds=[0,0 1024x768] ... external detected`. It is the NVIDIA GPU's own
  output, with no monitor behind it; no IDD/virtual display driver is installed (inferred: none seen in the PnP display class).
  DXGI enumerates 2 adapters (NVIDIA 0x10DE:0x2684, Basic Render Driver); the NVIDIA one has output 0 = 1024x768, attached.

## Raw evidence (`raw/`, text and JSON only; nothing executable came back)
IDXGIOutput3::CheckOverlaySupport (device = the D3D11 device on the NVIDIA adapter, `DXGI_FORMAT_NV12/YUY2/P010`), IDXGIOutput6::CheckHardwareCompositionSupport, and
`ID3D11Device::CheckFormatSupport`, identical on both drivers (`raw/t3.txt` = **580.88**, taken before the upgrade; `raw/t4.txt` = **595.97**):
```
KFOVL adapter "NVIDIA GeForce RTX 4090" vendor=0x10de device=0x2684 dedicated_mb=24142
KFOVL output left=0 top=0 right=1024 bottom=768 attached=1
KFOVL overlay_support nv12 hr=0x00000000 flags=0x0 direct=0 scaling=0
KFOVL overlay_support yuy2 hr=0x00000000 flags=0x0 direct=0 scaling=0
KFOVL overlay_support p010 hr=0x00000000 flags=0x0 direct=0 scaling=0
KFOVL hwcomp IDXGIOutput6::CheckHardwareCompositionSupport hr=0x00000000 flags=0x0 fullscreen=0 windowed=0 cursor_stretched=0
KFOVL format_support nv12 hr=0x00000000 d3d11_format_support=0xfa82c320 render_target=1 texture2d=1 display=0
KFOVL format_support yuy2 hr=0x00000000 d3d11_format_support=0x3a820320 render_target=0 texture2d=1 display=0
KFOVL format_support p010 hr=0x00000000 d3d11_format_support=0x3a82c320 render_target=1 texture2d=1 display=0
KFOVL RESULT SETUP CheckOverlaySupport reports no DXGI_OVERLAY_SUPPORT_FLAG_DIRECT for the chosen format (use --ignore-support to try anyway) (code=5)
```
(`raw/t0.txt`, 580.88, an earlier build of the probe, has the same three `overlay_support` lines.) **Comparison for kayfabe** (coordinator's data point): the kayfabe guest reports
flags=0x2 (SCALING only), DIRECT=0 for the three formats; this headless native box reports 0x0. So kayfabe is not *below* this native box, but this box is not a head-equipped reference
either, so "do kayfabe's caps differ from real hardware" stays **unanswered**.

## Scenario results (probe as run: 7 scenarios x {default, `--ignore-support`}) -- measured
Every YUV run ends before the scenario starts: default = `RESULT SETUP` (no DIRECT flag), `--ignore-support` = `FAILED CreateSwapChainForComposition nv12 1280x720 flags=0x200 hr=0x887a0001`
(DXGI_ERROR_INVALID_CALL). Also refused with the same hr: yuy2, p010, flags `none`, flags `yuv+fsv`, `--mode hwnd` (`CreateSwapChainForHwnd`). Raw: `raw/y_*.txt`, `raw/t1.txt` (the other variants were run with the same result and not kept).

| scenario | default | `--ignore-support` | overlay modes / transitions / latency |
|---|---|---|---|
| steady, move, resize, fullscreen, occlude, hide, recreate (soak: same path) | SETUP (5) | SETUP (5), swap chain refused | none: the scenario never started (no frames) |

## Machinery validation with the BGRA control format (`--format bgra`) -- measured
`--format bgra` (new) swaps the YUV swap chain for a plain flip-model B8G8R8A8 one so the window, DirectComposition visual, pacing, scenario events, watchdog and JSON can run on a box
without overlays. The expected verdict is `NEVER_OVERLAY` (code 2) by design (no overlay claim; the overlay-required abort is disabled). `GetFrameStatisticsMedia` gives
`CompositionMode` NONE for these (it describes YUV/media swap chains), so the *mode* columns carry no information in this section; the *machinery* columns do.

| scenario (BGRA control) | exit | frames / expected | worst Present1 ms | worst frame gap ms | events applied | notes |
|---|---|---|---|---|---|---|
| steady 8 s | 2 | 240 / 240 | 3.1 | - | - | `raw/t5.*` |
| move 20 s | 2 | 660 / 660 | 1.5 | 35.6 | 200 moves | no STUCK |
| resize | 2 | 465 / 465 | 1.4 | 35.9 | 3 size, maximize, restore | |
| fullscreen | 2 | 705 / 705 | 3.2 | 43.8 | 3x on/off | |
| occlude | 2 | 390 / 390 | 4.7 | 35.3 | occluder open 3 s / close 8 s | 0 occluded presents (`DXGI_STATUS_OCCLUDED` never returned) |
| hide | 2 | 420 / 420 | 2.1 | 36.1 | hide/show, minimise/restore | presents continued while hidden/minimised |
| recreate | 2 | 660 / 660 | 1.8 | 36.7 | 5 ResizeBuffers + 5 swap chain re-creations (6 swap chains total) | 21 mode transitions (none <-> stats_error on a fresh chain) |
| soak 600 s | 2 | 15375 / 18000 | 119.1 | 122.0 | 1340 events (1200 moves, 25+25 sc resize/recreate, 15+15 fullscreen, 5x occlude/hide/minimise) | 26 swap chains, 101 transitions, no STUCK, no device loss; avg Present1 18.0 ms (vsync-paced), 15375 < 18000 is the hidden/minimised/fullscreen windows |
| `--inject-stall 6000` (watchdog self-test) | **4 STUCK** | 90 | - | - | phase `inject-stall`, written by the watchdog after 3047 ms | `raw/c_stuck.*` |
| `--inject-stall 1500` | 2 | 196 | 4.9 | - | | a stall shorter than 3 s is not STUCK |

Machinery verdict: scenarios, pacing, watchdog (exit 4), JSON, exit codes and the interactive-session runner work natively. **Not exercised natively:** anything that needs a YUV
swap chain (NV12/YUY2/P010 fill, copy into the YUV back buffer, `GetFrameStatisticsMedia` modes OVERLAY/COMPOSED, the grace windows around events, the overlay-lost logic). The staging
NV12 Map/CopyResource path is untested on Windows; only the host unit tests cover the fill code.

## Probe changes made (all in `scripts/bench/windows/appmatrix/tools/src/kf_overlayprobe.cpp`; the YUV path itself needed no fix because it never got that far)
1. Evidence lines (no change to verdicts): `IDXGIOutput6::CheckHardwareCompositionSupport` (`KFOVL hwcomp`, also in the JSON `note`), `ID3D11Device::CheckFormatSupport` of NV12/YUY2/P010
   (`KFOVL format_support`), `CheckOverlaySupport` and the description of every further output (`overlay_support_output`, `output index=`). Adds `#include <dxgi1_6.h>`.
2. `--format bgra` control format (see above); implies `--ignore-support`, no YUV swap-chain flag, no overlay-required abort.
3. `--inject-stall MS` (watchdog self-test).
4. New `native/run_overlay_it.ps1` (interactive-session runner, prints the KFOVL lines, JSON in `C:\kf\ovl\TAG.json`) and `native/edge_gpu_cdp.ps1` (below).

## Native Edge 155.0.4283.45 + Shorts (overlay default ON), 595.97 -- measured
`native/edge_gpu_cdp.ps1` (private profile, DevTools port on localhost, consent clicked by script, GPU page read as text; `raw/edge_gpu.txt`):
- A Short played (`<video>` paused false, 720x720 or 360x640, 2500+ frames, ~190 dropped in ~57 s), NVDEC active: engine `videodecode` 0.9-1.4 % in every sample, `3d` ~0.5 %, `copy` <0.1 %.
- `edge://gpu`: *Video Decode: Hardware accelerated*, *Direct Rendering Display Compositor: Disabled*, *Direct composition: true*, *Supports overlays: true*, and
  **`YUY2 / NV12 / BGRA8 / RGB10A2 / P010 overlay support: SOFTWARE`** (Chromium's value for "no hardware overlay plane": consistent with CheckOverlaySupport flags 0).
- **Does DWM promote the video to an overlay plane natively here? No (measured at the capability level: no overlay/hardware composition exists on this output).** A DxgKrnl/Dwm-Core ETW
  capture of the playing Short was taken (`logman ... -ets`, Base keyword and all keywords; tracerpt CSV not committed: 33-83 MB) but the events carry no field names on this box, so
  **no per-present plane count was decoded and none is claimed.** The DWM overlay state beyond the capability flags was not read.
- Occlusion/window-order question (task 5): not measurable (no overlay exists to demote/re-enable). The control scenarios show the window-order events themselves
  (topmost occluder over the middle third of the video, 5 s) do not stall presents or return OCCLUDED.

## What this means for kayfabe
- *Measured:* a real NVIDIA driver (580.88 and 595.97, Ada) on a headless head reports no overlay support and no hardware composition; Edge falls back to composed video with NVDEC decode.
  The probe's SETUP/NEVER_OVERLAY handling is the correct outcome there.
- *Inferred, not measured:* with a real display head NVIDIA drivers do expose MPO, and then a head-equipped native run would give the DIRECT/SCALING flags for NV12 etc. This box cannot confirm it.
- **Suggested next step (reference with a real head):** run the same `run_overlay_it.ps1` flow on the owner's trusted host, a VFIO Windows guest on the RTX 4070 with a physical or
  dummy-plug display on a connector (as the `vfio-dvi` reference traces were made): record `CheckOverlaySupport`, run the seven scenarios (no `--format bgra`), then Edge Shorts with the same
  `edge_gpu_cdp.ps1`. Compare its flags with kayfabe's 0x2: a DIRECT bit there and none on kayfabe means the kf3 display caps are a derived value to fix.

## Exact commands
```
git worktree add /data/kf-overlaynative -b claude/overlay-native-20261010 origin/integration/windows-20261010
bash scripts/bench/windows/appmatrix/build_tools.sh /root/ovl                      # mingw-w64: kf_overlayprobe.exe (pushed TO the box with scp)
scp kf_overlayprobe.exe native/run_overlay_it.ps1 native/edge_gpu_cdp.ps1 vast@BOX:/C:/kf/
powershell -File C:\kf\run_overlay_it.ps1 -Tag t3 -ProbeArgs "--scenario steady --duration 8 --ignore-support" -TimeoutSec 60   # via ssh; runs as scheduled task /it
... -ProbeArgs "--scenario soak --duration 600 --format bgra" -TimeoutSec 700
... -ProbeArgs "--scenario steady --duration 12 --format bgra --inject-stall 6000"
schtasks /create /tn kfedge /tr C:\kf\ovl\run_edge.cmd /sc once /st 00:00 /it /ru <box>\vast /rl highest   # run_edge.cmd = powershell -File C:\kf\edge_gpu_cdp.ps1
logman create trace kfmpo -p Microsoft-Windows-Dwm-Core 0xFFFFFFFFFFFFFFFF 5 -o C:\kf\ovl\mpo.etl -ets   # ETW attempt (not decoded)
```
Box cost: 0.53 USD/h for about 2 h (two earlier instances of ours, 55273136 and 55274607, were destroyed unused or after a VM-support failure; 55274611/55274617/55274623 were created by mistake in one loop and destroyed within seconds).
