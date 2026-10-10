# Native-NVIDIA Windows baseline of the app matrix (2026-10-10)

**STATUS: LIVE (in progress), 2026-10-10 17:15 UTC.** 78 of 83 tier-1 apps PASS on a real NVIDIA GPU with the stock Windows driver
and no kayfabe; 5 are open (4 Edge apps, `gravitymark_vk`). The box is **still running** (handed off, see below). Everything in
*Measured* was seen on the box; everything in *Inferred* is labelled.

## HANDOFF (a fresh agent resumes from here)

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

### Ranked open items and next step for each
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

## Results (merged: `t1a` first full run, `t1b` re-run of the 20 apps that needed a fix; later wins; all re-judged with the current verdict.py)

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
