# Native-NVIDIA Windows baseline of the app matrix (2026-10-10)

**STATUS: LIVE (final, box destroyed), 2026-10-10 19:15 UTC.** 100 apps (tier 1 + 2 + 3) run on a real NVIDIA GPU (RTX 4060 Ti, driver 595.97)
with the stock Windows driver and no kayfabe: **99 PASS, 1 TIMEOUT (`gravitymark_vk`)**. Tier 1: 82/83 PASS; tier 2/3: 17/17 PASS. The vast
instance was destroyed at the end of the second session (the HANDOFF below is historical). Everything in *Measured* was seen on the box;
everything in *Inferred* is labelled.

## FINAL RESULTS (second session, 2026-10-10 17:10-19:15 UTC) -- these supersede the open items of the first session

Merged table: `results.md` / `results.tsv` (later runs win: `t1a`, `t1b`, Edge re-runs, `gravitymark_vk` 20-min run, tier 2/3 run, fix re-runs).

| area | result |
|---|---|
| tier 1 (83) | 82 PASS, 1 TIMEOUT (`gravitymark_vk`); sum of per-app runtimes 5107 s (incl. the 1234 s timeout) |
| tier 2/3 (17) | 17 PASS (sum 890 s tier 2, 220 s tier 3); `dxva2_h264` needed `session=interactive` (below) |
| Edge apps (5: webgl, webgl_headless, webgpu, video_h264, video_vp9) | **5 PASS** after two fixes (below) |
| stability pass (10 random tier-1 PASS apps, <= 150 s, seed 20261010) | 10/10 PASS again, same proof kinds; runtimes within +-1 s except `hist_t` 6 s -> 16 s (cold python import; `stability.tsv`) |
| D3D11 probe winding fix (override build of the committed `kf_dxprobe.cpp`) | `dxprobe_d3d11` PASS with `KFDX ok draw result matches (centre pixel 64,128,191,255)`; d3d12 PASS |
| Per-app runtimes | `results.md` (column `secs`); longest: gravitymark_vk 1234 (timeout), clpeak_cuda 1176, clpeak_ocl 1037, gravitymark_d3d11 204, gravitymark_gl 187, gravitymark_d3d12 184, vkpeak 136 |

### Edge and "YouTube Shorts plays natively on this driver" (the kayfabe reference), measured
- **Edge version 155.0.4283.45** (inbox Edge, Windows 11 LTSC 2024), NVIDIA 595.97, one display adapter (the NVIDIA GPU).
- The 4 Edge failures had two causes. (1) `kf_edge.ps1` orphaned pending `BeginGetContext` operations (fixed in the first session): after the fix the
  page loads and posts. (2) The matrix' GPU proof `pdh:VideoDecode` found nothing because Edge's browser tree is **not** a descendant of the
  supervisor (the sampled tree held 3-5 pids and none of Edge's 10+ processes); `kf_run_app.ps1` now also samples every `msedge.exe` for `edge_*` apps.
  After that: `edge_video_h264` PASS proof `pdh:videodecode` (VideoDecode engine peak 3.9 %), `edge_video_vp9` PASS (2.2 %), `edge_webgl` / `edge_webgpu`
  PASS with `3d` engine use (3.1 % / 1.3 %). No Edge first-run policy was needed for the test pages (the matrix profile uses `--no-first-run`).
- Test video page (`traces/.../edge/edge_probe1.txt`): `canPlayType probably`, `mediaCapabilities` supported, smooth, **powerEfficient true**; GPU process
  `engtype=videodecode` non-zero in 7 of 7 samples (2.2-4.1 %), 3d 0.5 %, copy; screenshot `edge/video_h264_test_playing.png` (frame counter running).
- **Real YouTube Shorts** (box has internet; `edge/shorts_probe.ps1` sets HideFirstRunExperience / AutoplayAllowed, opens `https://www.youtube.com/shorts`
  in a private profile): no consent page appeared (region CA), the Short (id luANrw9UQlk) autoplayed with sound icon. Two screenshots 12 s apart
  differ (`edge/shorts_t0.png`, `edge/shorts_t12s.png`; 144k pixels differ in the video area; caption text and the speaker's pose change). Over 120 s of
  playback the Edge **GPU process** showed engine `videodecode` in 46 samples (0.65-1.74 %, i.e. NVDEC hardware decode is active), `3d` 0.1-1.3 % (compositor),
  `copy` <= 0.12 %; no nvlddmkm event, no TDR. (`edge/shorts_probe.txt` has every sample.)
- Not measured: `edge://gpu` text (not scriptable here); hardware decode is inferred-from-counters (VideoDecode engine of the NVIDIA adapter attributed to the Edge GPU pid), which is direct evidence of NVDEC use, not of which codec path.

### gravitymark_vk (diagnosed: never finishes natively; not slowness; cause unknown)
- 20000 asteroids, 1230 s budget: still running at the end; CPU time 84 s per 90 s sampled (about one core busy), working set steady at ~520 MB, 20-25 threads,
  `Responding` True, GPU 71-92 % util, 2.1 GB, 2640 MHz, 82-91 W, no output, no image (`gravitymark_vk_diag/process_samples_1230s_run.txt`). The D3D11/D3D12/GL
  GravityMark runs with the same flags end in 177-204 s.
- 3000 asteroids, 400 s: TIMEOUT too. Two direct variants (`-vk`, with and without `-benchmark`, 1000 asteroids, `-close 1`, 150 s each): still running, ~0.9 core CPU, no image.
- **Measured:** it keeps the GPU busy and does not finish in 20 min whatever the load. **Inferred:** a benchmark loop that never reaches its end condition
  (the Vulkan path or its present on this box); not proven, since in-window frames cannot be screenshotted (`CopyFromScreen` is white for D3D/Vulkan). The kayfabe comparison should
  treat `gravitymark_vk` as "no native reference" (compare the D3D11/D3D12/GL GravityMark only), and `vkcube`/`vkpeak`/`vulkan_video_decode` as the native Vulkan references (all PASS).

### Other lane fixes this session (all in git)
- `kf_run_app.ps1`: Edge pids sampled for `edge_*` (above). `gen_apps.py`: `dxva2_h264` `session="interactive"` (in session 0 ffmpeg: `Failed to create Direct3D device`,
  rc -1313558101; interactive PASS with VideoDecode 36 %); `apps.json` and the design-doc tables regenerated, `pytest scripts/bench/windows/appmatrix/tests` 67 passed.
- New helpers in `native/`: `edge_probe.ps1`, `shorts_probe.ps1`, `click.ps1` (cursor click / SendKeys in the interactive session).
- Environment traps measured: after the second Edge run the box's sshd accepted keys but closed every session ("closed by remote host", also for `cmd /c`) for >10 min;
  `vastai reboot instance` recovered it (cause unknown; no TDR/WER seen in any run). The controller's root disk filled (2.6 GB free) during the first re-run (`OSError 28`): keep run dirs on /data.
- Caveats for the comparison: 4-vCPU guest; `clpeak_*` ~1040-1180 s; 595.97 instead of 595.91; the matrix' own clock on the box is not UTC-aligned (boot times in the logs are off), use `session.log` times.

## HANDOFF (first session; HISTORICAL, the box is destroyed)

**Instance** `55209252` (vast, our own; account is shared: act on this id only). Rented 12:20 UTC, $0.2657/h (~$1.3 at 17:15). **Cap: about
$6 or 8 h wall = destroy by 20:20 UTC at the latest.** Never print or store the vast API key or `instance_api_key`.
**Hardware** (vast offer 49744624, host in Quebec, CA): RTX 4060 Ti 16 GB (Ada, sm_89, PCIe gen4 x8-ish), AMD EPYC 7713; the Windows
guest sees **4 logical CPUs, 50 GB RAM, 150 GB disk** (vast lists 16 effective cores / 515 GB: the template VM is smaller). Inet 2.8 Gbps.
**OS** Windows 11 Enterprise LTSC 2024 Evaluation, build 26100 (the owner's template `Windows 11 KVM on Vast.ai`, hash
`183b10af7fd788d08aca02e31811d70d`, `docker.io/vastai/kvm`; it erases the Linux disk and installs Windows, ~25 min incl. 3 boots).
**Driver** the template installs GeForce 580.88 + CUDA 13.0.0; I upgraded it with the signed NVIDIA installer to **595.97** (NVIDIA WHQL,
sha256 `979ed00fea181c786f608967377d6d83ac82e6368275994a4182ec79d97b3122`, Authenticode checked). **595.91 does not exist** on
`us.download.nvidia.com` (404 for both `dch` and `nsd` names; 595.71, 595.79, 595.97 exist): 595.97 is the closest, newer one.
**Display** one adapter only (the NVIDIA GPU, 1024x768 "Default Monitor"); no virtual VGA, no WARP in use.

**Access.** Windows OpenSSH, public key only, user `vast` (administrator, default shell Windows PowerShell), the controller key
`~/.ssh/id_ed25519` (registered in the vast account). IP and port are NOT in git: take `public_ipaddr` and `ports["22/tcp"][0].HostPort`
from `vastai show instances-v1 --raw` (or `vastai ssh-url 55209252` for the proxy form). Pin the Windows host key from the install log:
`vastai logs 55209252 --tail 3000 | grep -a -A2 'WINDOWS SSH HOST KEY'` (the intermediate installer has another key). Quirks measured:
**each ssh call costs ~3 s** (sshd session spawn; ControlMaster does not help), and **stdin to the remote shell hangs after ~100 KB**, so
files go by `scp`/sftp (`/C:/path` form). `scp -3 trusted:... box:...` streamed the 6.8 GB image through the controller (6 min).

**On the box.** `C:\kfmedia\kfapps.iso` (the 6.8 GB app image, sha256 `183c13aa...a652` verified, pushed from the trusted host; mounted
read-only with `Mount-DiskImage` at every guest start, volume `KFAPPS`, image identity matches `manifest.json`); `C:\kf` (guest scripts, pushed
by the driver each guest), `C:\kfapps` (staged packages: python+wheels, cuda demo suite, ffmpeg, blender, ...), autologon of `vast` configured
(`kf_native_prep.ps1`: password is a throwaway, in controller scratch only; Defender real-time off + exclusions, Windows Update off, lock screen
off, OOBE privacy screen dismissed). Reboot-to-clean is what the adapter calls "a fresh guest".

**Run it** (from a controller with the repo checkout at branch `claude/windows-baseline-20261010`):
```
export KF_SSH_HOST=<ip> KF_SSH_PORT=<port> KF_SSH_USER=vast KF_SSH_KEY=~/.ssh/id_ed25519 KF_SSH_KNOWN_HOSTS=<file with the pinned key> KF_SSH_CTL_DIR=/tmp
N=scripts/bench/windows/appmatrix/native/native_windows_apps.py
python3 -I $N run --run-dir /root/winbase/runs/NAME --apps edge_webgl,edge_webgl_headless,edge_webgpu,edge_video_h264 --override-dir /root/winbase/override --no-isolate
python3 -I $N shot --out x.png         # screenshot of the interactive desktop
python3 -I scripts/bench/windows/appmatrix/native/reverdict.py RUN   # re-judge facts with the current verdict.py/apps.json
python3 -I scripts/bench/windows/appmatrix/native/baseline_report.py traces/windows_baseline_20261010 RUN_A RUN_B   # later runs win
```
`--override-dir` holds the locally built `kf_dxprobe.exe` (`x86_64-w64-mingw32-g++ -O2 -std=gnu++17 -static -static-libgcc -static-libstdc++
tools/src/kf_dxprobe.cpp -o override/kf_dxprobe.exe -ld3d11 -ld3d12 -ldxgi -ldxguid`; the image on the host still holds the old, buggy
binary: **rebuild the image or use the override before the kayfabe run**). `prep` (password via `$KF_GUEST_PW`) and `push-iso` are in the
adapter's `--help`. The adapter is `native/native_windows_apps.py`; `winapps.py`, `verdict.py`, `apps.json` and the guest scripts are the
lane's own (fixes below). A run with no `EXIT` line in `session.log` died.

**Destroy** (when done, also on any failure path): `vastai destroy instance 55209252 -y`, then `vastai show instances` must show none of ours.
Nothing on the box is needed after the results are in git. Evidence is text/JSON/PNG only; nothing executable came back from the box.

**Password self-check** (owner's accidentally pasted string, never used, never written): worktree NO, controller scratch NO, cloned reference
repo NO, git history NO, files on the box NO.

### Ranked open items and next step for each (SUPERSEDED by FINAL RESULTS above: items 1-3 are done except gravitymark_vk, which never finishes; phase-2 isolation was not exercised)
1. **`edge_webgl`, `edge_webgl_headless`, `edge_webgpu`, `edge_video_h264`** (all FAIL: "no result posted"). Edge starts and shows the page URL
   but sits on "Loading...". Cause found and fixed in git, **not yet re-run**: `kf_edge.ps1` re-issued `BeginGetContext` after every 1 s timeout,
   orphaning the pending operation that then received Edge's first request (falsifier: after the fix the page loads and posts; if not, test Edge
   against the listener by hand in the interactive session, check `--enable-unsafe-webgpu` banner / proxy). Then check the GPU proofs.
2. **`gravitymark_vk`** TIMEOUT at 600 s (window white in the screenshot, GPU 3D 48 %, nvidia-smi 96 %, 2.1 GB, no output, no `gm.png`); the D3D11/12
   versions finish in 184-204 s. Not yet known whether Vulkan GravityMark hangs or is just slower. Next: rerun with `-asteroids 5000` to see if it
   finishes; if it also hangs, document "cannot run natively on this box (Vulkan present path)" and compare with kayfabe only for D3D.
3. **Not done**: tier 2/3 apps (17), phase-2 isolation of non-PASS apps (adapter supports it; reboot per app), a second full pass for run-to-run
   stability, `furmark_*` bench runtimes, per-app digests compare (`PASS*`).
4. Kayfabe comparison caveats: runtimes below are on a 4-vCPU guest; `clpeak_*` need ~1040-1180 s here (new budget 1500 s).

## Results of the first session (SUPERSEDED by FINAL RESULTS above; kept for the record; merged: `t1a` first full run, `t1b` re-run of the 20 apps that needed a fix; later wins; all re-judged with the current verdict.py)

Counts, tier 1 (83 apps): **PASS 78, FAIL 4, TIMEOUT 1** (`results.md` has the per-category and per-app tables, `results.tsv` the machine form with
per-app seconds, proof, GPU engine peaks, nvlddmkm/153 events; `apps/ID.json|log`, `verdict/ID.json`, `screenshots/`). Sum of per-app runtimes 4706 s
(79 min) without staging/ssh overhead; wall time of the first full pass 2 h with ~60 s of ssh/staging overhead per app. Longest: clpeak_cuda 1176 s,
clpeak_ocl 1037 s, gravitymark_vk 604 s (timeout), gravitymark_d3d11 204 s, gravitymark_d3d12 184 s, vkpeak 136 s.

Expected difficulty for kayfabe (the inventory's reasoned rating) vs. the native result, tier 1:

| reasoned difficulty | apps | PASS natively | not PASS natively |
|---|---|---|---|
| low | 34 | 34 | 0 |
| medium | 16 | 15 | 1 (edge_video_h264 or a browser app: fix pending) |
| high | 33 | 29 | 3 FAIL (Edge apps, fix pending) + 1 TIMEOUT (gravitymark_vk) |

Of the inventory's 50 "high" apps (all tiers), 33 are tier 1 and were run: **none of them is a verified native failure**; the 4 Edge failures
were a script bug and the GravityMark Vulkan timeout is open (item 2). 17 high-rated apps in tier 2/3 were not run.

### Fixes made to the lane (all in git, each found by the native run; the kayfabe path shares them)
- `kf_run_app.ps1`: the facts file was never written (supervisor spun for minutes): `ConvertTo-Json -Depth 8` expanded the PSPath/PSDrive note
  properties of `Get-Content` strings (Windows PowerShell 5.1). Cast to `[string]`.
- `verdict.py`: `Get-Counter` lower-cases GPU Engine instance names (`engtype_3D` arrives as `3d`), so every `pdh:` proof failed; now case-insensitive
  (+ unit test). Re-judged: gravitymark_d3d11/12, nbody_demo_gl, oceanFFT_demo, randomFog_demo, ort_directml flip to PASS.
- `kf_edge.ps1`: `$page` shadowed the `-Page` parameter (URL `/C:/kf/web/webgl.html.html`); single pending `BeginGetContext` (see item 1).
- `kf_dxprobe.cpp`: the D3D11 fullscreen triangle was counter-clockwise and culled (read back the clear colour on any hardware); winding fixed.
- `win_cuda_equiv.py` `managed_t`: CuPy 14 `malloc_managed` already returns a `MemoryPointer`.
- `gen_apps.py` (apps.json regenerated): `nvenc_*` get `-pix_fmt yuv420p` (av1_nvenc rejected yuv444p); decode apps loop the clip 30x (the 0.7 s decode
  was shorter than the 2 s engine sampler, no proof possible); `furmark --glrenderer/--vkinfo` exit 1 by design (mapped to 0 with a note);
  `hashcat` cracks in 0 s but never exits (killed after a 15 s grace by the new `Invoke-KfUntil`; app teardown hang on this driver, measured);
  `opencl_icd` also reads `OpenCLDriverName` from the display class key (the ICD is no longer under Khronos\OpenCL\Vendors);
  `cupy_check`/`torch_burn`/`ort_directml`/`llama_cuda_gen` get an explicit device-name or `smi` proof (`kf_runpy.py` prints `KFDEV <name>`);
  `clpeak_*` budget 600 -> 1500 s, GravityMark 360 -> 660 s.
- New: `native/` adapter (SshQga + NativeVm; reboot = fresh guest; ISO mount; in-session PNG screenshots), `reverdict.py`, `baseline_report.py`.

### Measured findings worth carrying to the kayfabe comparison
- Cold first Python/torch import after staging: `atomics_t` hit the 120 s budget (no output for 121 s), 18 s warm. Order matters.
- `nvlddmkm` event 153 ("Error occurred on GPUID: 7", no display-reset event 4101, GPU kept working) fired 3x during `clpeak_cuda` natively: the
  lane's "153 = TDR" label is too strong; it is a GPU error report that also occurs on a healthy native run.
- The 2 s GPU Engine sampler misses sub-2 s work; CUDA-only apps show `smi`/`out` proofs, 3D apps show `3d`/`copy` engines.
- Screenshots of D3D/Vulkan windows via `CopyFromScreen` are white (GPU-composited content), also for apps that PASS; use them for desktop/Edge UI only.
- 1024x768 console: 1280x720 app windows extend past the screen.

### Unverified / inferred
- Edge apps and `gravitymark_vk` outcomes (above). No run-to-run repeat yet; tier 2/3 not run; phase-2 isolation not exercised.
- That the 4-vCPU guest, not the GPU, bounds `clpeak_*` runtime is inferred (CPU backend dominates the log); not measured separately.
- 595.97 instead of 595.91 (not obtainable); results may differ by a driver point.
