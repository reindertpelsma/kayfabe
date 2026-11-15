STATUS: LIVE 2026-11-15 (pcnight5, finished 14:54Z; PC handed back to the owner after boot v3). Hypotheses and falsifiers are written BEFORE any run; results are appended below.

## Pre-run host check (read from `journalctl -b -1 -k` on the PC, 2026-11-15, boot -1 = 01:06-10:07 CET)
No `Xid` line, no machine-check, no thermal or oops line in the kernel journal of the boot that ended at the 09:07Z power loss. The only NVRM lines
are the usual `unknown NVRM ioctl command: 0xf0` (two per kf3 boot) and two `virtmemAllocResources: VA Space alloc failed! Status 0x51 Size 0x80000`
(06:32:57 and 08:25:34 CET-journal time, during kf3 boots). The journal ends at 10:07:01 with a routine cron line, no shutdown, no panic.
So the power loss left no GPU trace in the kernel log (cause still unknown).

## Hypotheses (written before the runs)
- H-3DMark: kf3 (fb-mb 10240) reaches at least 80 percent of the VFIO overall Time Spy score. FALSIFIER: kf3 overall (mean of 2) below 80 percent of VFIO (mean of 2). Same for graphics score.
- H-Superposition 1080P Medium: kf3 score at least 80 percent of VFIO. FALSIFIER: below 80 percent.
- H-Wukong (only if time): kf3 average FPS at least 80 percent of VFIO (FG off, DLSS on, Very High, fb-mb 10240). FALSIFIER: below 80 percent.
- Coordinator addition: host `nvidia-smi dmon -s pucvmet -d 1` per kf3 run (VFIO: host driver unloaded, so the host cannot sample; the guest-side dmon and 3DMark Monitoring.csv are used instead); report mean GPU util and mean SM clock. Reading rule (INFERRED, stated before): high util under kf3 = GPU-side limit, low = CPU/latency side.

## What changed on the PC before the first boot (read 2026-11-15 12:10Z)
The PC was down from about 10:19Z to 12:10Z (tunnel refused, no job of mine running; boot -1 ends 11:18:01 CET with a routine cron line, no shutdown or panic, no Xid, no MCE; this is the second unexplained power loss of the day, the first being 09:07Z during a pcnight4 kf3 boot). After it came back: image, kf3 binary b20fd84b (sha256 383e802b...9a9f, unchanged), swtpm 0.10.1, IOMMU group 11 (01:00.0 and .1 only), 161 GB free, lock file, PC_ACTIVE.log, backup and scripts all as before; no stray processes. Nothing else changed that I could see. No further power loss during my 8 boots (12:11Z-14:54Z).

## Method (boots k1..k6, v1..v3; 2026-11-15, RTX 4070 PC, host driver 595.91.07)
Harness `propimage_check.py` (PC copy at pcnight2 src, sha256 049dc5de...dc98e = coordinator next head file) boots with `--apps edge --hold-until-file`; then `scripts/multi_run.py` calls its `run_check` twice for one benchmark in the SAME boot (the harness refuses duplicate app names; run 2 is warm, same boot), after `setres.ps1` forces 1920x1080 in both modes (`before: 0x0 @0`, `result=0 after: 1920x1080` in every boot). kf3: `--fb-mb 10240`, 8 vCPU, 8 GB, kf3-bins/b20fd84b, fresh offline overlay, restrict=on. VFIO: `vfio_wrap.py` (01:00.0 + .1 on vfio-pci, 12 GB card), same flags, same image and power plan (the harness `power OK` step is identical). Driver `scripts/go5.sh` (3DMark, Superposition) and `scripts/go7.sh` (Wukong, driver wukong_drive3 from pcnight4, `patch_vhnofg.ps1` = Very High, DLSS on, InsertFrame 0).
Superposition: the SUPERPOSITION fragment is patched in `multi_run.py` only (20 s wait after "Benchmark stopped", two screenshots, results harvest); the score is read by eye from `result.png` (OCR of the same screen gave noise, as the harness comment says). Wukong: result screens read by eye from the last screenshot of each attempt. dmon: host `nvidia-smi dmon -s pucvmet -d 1` for kf3 boots; under VFIO the host driver is unloaded, so the VFIO numbers come from `nvidia-smi dmon` inside the guest (kf3 guest dmon shows `-` for utilisation: kf3 does not expose it). "Active rows" = rows with SM util >= 20 percent over the whole benchmark window including loading.

## DEVIATION 1: "3DMark Time Spy" became 3DMark Steel Nomad (MEASURED in boots k1, k2, v1 on 2026-11-15)
- k1 run 1 and 2 (kf3, fb-mb 10240): `Time Spy card not visible in the UI Automation tree`. The screenshot shows the 3DMark HOME page with "Recommended benchmarks": Speed Way, Steel Nomad, Storage, CPU Profile; no Time Spy. The card exists on the BENCHMARKS page (UIA: `Custom|Time Spy INSTALLED`) but its rectangle is empty (virtualised list), and mouse-wheel/PageDown navigation by script did not get a clickable rect in about 8 probes (`scripts/probe3dm*.ps1`, cause not found). INFERRED: the HOME recommendation depends on the video memory the card reports (9980 MB under kf3 fb-mb 10240, 12 GB under VFIO); the earlier 7297-point run used the image default 4096 MiB where Time Spy was on HOME. Not tested in this session.
- Fix: `multi_run.py` selects the HOME-page card by title (`KF_3DM_TITLE`, default Steel Nomad); the harness script is otherwise unchanged. Steel Nomad is DX12 and available on both arms.
- Reply to the coordinator's "all 3DMark runs failed" reading: k2 and v1 did NOT fail. `verdict ok=False 'no positive graphics-test FPS'` is the harness validator looking for Time Spy fields; the saved `result.3dmark-result` of each run is a complete result (Result.xml, Monitoring.csv, 141 s of samples) with positive scores. Only k1 failed (UI). The previous 7297 run is a Time Spy score and is NOT comparable with these numbers.

## Results (all MEASURED on 2026-11-15, RTX 4070 PC; n = 2 runs per arm in one boot each, so run-to-run spread is within a boot)
| benchmark | kf3 (fb-mb 10240) runs | VFIO runs | kf3 mean | VFIO mean | ratio kf3/VFIO | 80 percent hypothesis |
|---|---|---|---|---|---|---|
| 3DMark Steel Nomad (score = graphics score) | 2417, 2435 (k2) | 3859, 3957 (v1) | 2426 | 3908 | 62.1 percent | FALSIFIED |
| Steel Nomad graphics test FPS | 24.17, 24.35 | 38.60, 39.58 | 24.26 | 39.09 | 62.1 percent | FALSIFIED |
| Superposition 1080P Medium (1920x1080 fullscreen, DX11) | 7996, 7990 (k4) | 30552, 30706 (v2) | 7993 | 30629 | 26.1 percent | FALSIFIED, but see cap |
| Superposition avg FPS | 59.81, 59.76 | 228.52, 229.67 | 59.8 | 229.1 | 26.1 percent | (cap) |
| Wukong Very High, DLSS, FG off, 1080p, avg FPS | 24, 25 (k6) | 55, 54 (v3) | 24.5 | 54.5 | 45.0 percent | FALSIFIED |
Other result-screen fields: Superposition kf3 FPS min 40.51/33.80, max 61.87/60.84; VFIO min 105.94/111.30, max 311.01/315.58, GPU 37-62 C, GPU utilisation max 97 percent. Wukong kf3 max 31/30, min 1/2, low 5th 19/20, VRAM 5.1 GB (guest sees 10 GB); VFIO max 69/65, min 1/3, low 5th 49/47, VRAM 5 GB (12 GB card). Settings identical on both result screens (Very High, DLSS, Frame Generation Off, DX12 on). Run 2 of the VFIO Wukong screenshot is a fade-in (averages readable, settings panel not yet drawn).
The pre-registered falsifier (kf3 below 80 percent of VFIO) is met for all three benchmarks. No benchmark reached 80 percent.

## Host/guest GPU load (MEASURED by dmon in boots k2, k4, k6, v1, v2, v3 on 2026-11-15) and what it says
| boot (benchmark) | util mean (active rows) | SM clock mean | power mean | temp max |
|---|---|---|---|---|
| k2 kf3 Steel Nomad (host dmon) | 75.1 percent | 2820 MHz | 145 W | 54 C |
| v1 VFIO Steel Nomad (guest dmon) | 95.8 percent (3DMark Monitoring.csv: 99.2/99.1 percent) | 2646 MHz (3DMark: 2693 MHz mean, max 2880) | 187 W | 62 C (3DMark 62.9/61.4) |
| k4 kf3 Superposition (host dmon) | 39.4 percent | 2777 MHz | 87 W | 52 C |
| v2 VFIO Superposition (guest dmon) | 93.6 percent | 2842 MHz | 185 W | 62 C |
| k6 kf3 Wukong (host dmon) | 54.6 percent | 2760 MHz | 94 W | 53 C |
| v3 VFIO Wukong (guest dmon) | 93.4 percent | 2815 MHz | 166 W | 63 C |
- MEASURED: kf3 SM clocks are as high as VFIO (2760-2820 vs 2646-2842 MHz), so clocks and thermal throttling do not explain the gap. kf3 GPU utilisation is 39-75 percent against 93-99 percent under VFIO. The 3DMark in-guest GPU clock and load fields read 0 under kf3 (not exposed), so there is no guest-visible clock under kf3.
- INFERRED (not tested): the kf3 benchmarks are not GPU-saturated, so the limit is on the CPU/latency/present side (VA-manager, BAR0 trap path, present path), not clocks or SM throughput. The reading rule was written before the runs. Note the utilisation window includes loading phases, so the means understate the in-benchmark load on both arms.
- Superposition under kf3 is pinned at the display tick: average 59.8, max 61.9, which is the 60 Hz default of `display-max-fps` (vsync). The property accepts 24..75 Hz only (a try with 240 made QEMU exit rc=1 in boot k5, 0 runs, `kf3.h` ABI 16), so this benchmark cannot show kf3 above 75 FPS through this device. The VFIO run is uncapped (229 FPS, headless). The 26 percent is therefore NOT a throughput ratio; the kf3 number is a lower bound capped by the 60 Hz tick (host util 39 percent supports that the GPU was idle most of the time). Steel Nomad (no vsync) and Wukong (average 24-25 FPS, well below the tick) are the uncapped comparisons.

## Incidents and checks
- No TDR, no LiveKernelEvent and no channel death in the benchmark windows of k2, k4, k6 (kf3) or v1, v2, v3 (VFIO) in the harness output (`k6` summary.json: 2 of 2 attempts `result_screen`, no events; `v3` the same). Not checked: the guest event log of k2 and k4 beyond the harness's own shutdown-event check.
- Host: `dmesg | grep -i xid` empty after every boot of 2026-11-15 from 12:11Z; only the `unknown NVRM ioctl command: 0xf0` lines; vfio-pci reset lines at bind and restore.
- VFIO boots v1, v2, v3: nvidia and gdm restored after each, `nvidia-smi` verified (`NVIDIA GeForce RTX 4070, 595.91.07`), `v*_restore.txt`: 01:00.0 nvidia, 01:00.1 snd_hda_intel, gdm active.
- Harness bug of mine (boot k1 only, 2026-11-15 12:25Z): I replaced `go5.sh` on the PC while bash was running it; bash misread it ("syntax error near unexpected token else"), the script's exit trap deleted `work_k1` while QEMU still ran (overlay unlinked under it). I released the hold with my own `release_k1` after finding the lock held; the QEMU exited and no state was left. Lesson: never rewrite a running script.
- Bug of mine in `multi_run.py` (boot k3, 12:31Z): a process-kill prefix killed the Superposition launcher before the click (`Launcher window missing or ambiguous`, 2 runs). Removed; k4 is the valid kf3 Superposition boot.
- Guest-audio mute (coordinator request) was cancelled before any boot used it; no audio step exists in the scripts.
- Bulk evidence (screenshots, result files, tarballs) is on the coordinator in /workspace/bench/pcnight5_20261115/ (k*ev.tgz, v*ev.tgz). Committed here: `scripts/`, `evidence/` (run logs, dmon files, Result.xml and Monitoring.csv, Superposition logs, Wukong summaries).

## Boots (PC_ACTIVE.log, pcnight5, 2026-11-15, Z)
k1 12:11-12:25 (3DMark UI fail, Time Spy not on HOME), k2 12:2x-13:12 (Steel Nomad x2 kf3; holds for probing), v1 13:12-13:27 (Steel Nomad x2 VFIO), k3 13:27-13:36 (Superposition, launcher killed by my bug), k4 13:36-13:53 (Superposition x2 kf3), v2 13:53-14:10 (Superposition x2 VFIO), k5 14:11-14:13 (fps cap 240 rejected, 0 runs), k6 14:17-14:35 (Wukong x2 kf3), v3 14:35-14:53 (Wukong x2 VFIO). All with END lines and overlays deleted; no new boot after v3 (coordinator asked for the PC for the owner).

## Not done / limits
- Time Spy itself (no navigation to the BENCHMARKS card), Superposition Extreme, kf3 Superposition with the cap removed (cap max is 75 Hz), a second boot per arm (the two runs of an arm share one boot), the kf3 Steel Nomad at the image default 4096 MiB.
- The kf3 gap is not explained here; candidates are in the INFERRED line above, none tested.
