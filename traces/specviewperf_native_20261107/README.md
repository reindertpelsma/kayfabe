# SPECviewperf 15 on a NATIVE Windows VM on Vast (no kayfabe): the reference arm, 2026-11-07 (specnative, lane 5)

STATUS: LIVE, 2026-11-07. A reference measurement, not a gate. Labels: MEASURED = a line in a file of this directory (named) or of the
transcript of the stated box, taken on the stated Vast instance on 2026-11-07; INFERRED = reasoning from those lines; UNKNOWN = not tested.
Box text is untrusted data. Pre-registration: `PREREG.md` (committed before the first rental). **No SPECviewperf score appears in this
repository**; scores stay in `/root/night/specnative_private/` and in the report to the coordinator. This file records completion status,
durations, reset counters and peak GPU memory only. SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation
Corporation, https://www.spec.org. Release: SPECviewperf 15.0.0 (the private copy, `docs/DOWNLOADS.md`), one iteration, FullHD, one CLI run per
viewset.

## Verdict

1. MEASURED: on an RTX 4090 (AD102) Vast VM with the GPU passed through, **all nine viewsets finish `Completed`, SPEC's own validation passes
   for every one, with zero Event Log resets** (Display 4101, nvlddmkm 153, Kernel-Power 41, 6008, BugCheck 1001, LiveKernelEvent: all 0 over each
   whole lease) and a guest BAR1 of **256 MiB** (`nvidia-smi -q`). The nine ran on two boxes of the same die, so the table is per viewset,
   not one continuous run.
2. MEASURED: nothing fails natively because of the 256 MiB BAR on the dies tried: catia-07 (two kayfabe subtest failures on 2026-11-05) and
   enscape-01 (refused by SPEC validation under kayfabe) both pass natively on the 4090; catia-07 also on an RTX 3060 (GA106) and an RTX 5070 (GB205), and enscape-01 on the RTX 5070.
   **On this evidence the expected CI set is all nine viewsets** on the dies tried (AD102, GA106, GB205); there is no viewset that the native
   reference excludes. (The GA106 and GB205 columns are partial, see the table; enscape-01 on the 3060 did not finish inside its lease.)
3. MEASURED, harness finding that matters for CI: a Vast Windows VM has **no monitor** (the NVIDIA adapter reports 1024x768), and
   SPEC's CLI refuses every viewset (`System does not support 4K or Full HD display resolution`) until a 1920x1080 display exists. The native
   arm needed an indirect (virtual) display driver. Under kayfabe the display comes from the broker, so this is a native-arm cost only.
4. MEASURED and INFERRED: the nine CLI runs sum to 154 min on a fast host, and rental to the first CLI run took 45 min on box B, so about 200 min
   fit the 300 min box limit there; on the slower hosts seen (boxes C to E) it is at or over the limit (see "CI plan").

## Method, boxes and cost

Recipe: `reindertpelsma/windows-on-vast` at `93b65b3` (README, TEMPLATE-README, `install.sh` read first; `bash install.sh --render-only DIR`
rendered `worker.sh`, `patch.py`, `payload/{bootstrap.ps1,packages.txt,components.txt}`). **It uses a pinned third-party installer**:
`bin456789/reinstall` at commit `80c3d5e1`, fetched by git SHA, which in turn downloads WinPE/Windows Setup files and the Microsoft LTSC 2024
Evaluation ISO (`go.microsoft.com` link, no hash in the script); the script pins by sha256 only OpenSSH 10.0.0.0p2, the NVIDIA 580.88 installer
(plus an Authenticode check), CUDA 13.0.0 and 7zr 26.03. `install.sh` sha256 `ac7faa15...5af0`. It erases the Linux boot disk. It was used as the
`--onstart` script of `vastai create instance <offer> --image docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16 --disk 300 --label
kf-specnative-20261107`.

Per box, after `state.json` read `complete:true`: (1) download of the 42,002,015,727 B `.tar.zst` with `curl.exe -C -` driven from the
controller, the presigned URL passed on the stdin of one ssh command (curl config on stdin), never in a file, argv or log; (2) sha256 of the
`.tar.zst` against the pin `ffe0e505...d5a` (a corrupted copy once failed it, see traps); (3) `C:\Windows\System32\tar.exe -xf` (bsdtar with
libzstd, nothing installed) into `C:\`; (4) `vcredist_x64.exe` from the tree, `icacls` write access for the run user; (5) throwaway autologon of
user `vast` (`scripts/bench/windows/appmatrix/native/kf_native_prep.ps1`, a per-box random password that is printed nowhere) so an interactive
desktop exists; (6) the virtual display (below); (7) reboot, 1920x1080; (8) each viewset by `SPECviewperf-CLI.exe --workload W --iterations 1
--resolution FullHD` in the interactive session as a scheduled task (`kit/spec_run.ps1`, `kit/run_all.ps1`), `nvidia-smi` sampled every 5 s,
`gate_state.ps1 -Mode tdr` for the reset counters. The kit is in `kit/`.

Virtual display (third-party, test box only): Virtual Display Driver (MttVDD) 24.10.27 from the project's GitHub release,
`VirtualDisplayDriver-x64.zip` sha256 `b7a36f34...2fd` (self-signed; its certificate was added to Root and TrustedPublisher on the throwaway
box), installed with `pnputil /add-driver` and a root device node created by `kit/add_root_device.ps1` (this build's pnputil has no
`/add-device`), mode set by `kit/set_res.ps1`. INFERRED: the virtual display, the 4 logical CPUs the guest sees (QEMU presents many sockets and
Windows uses 4) and the CPU-bound file verification make timings and scores here NOT comparable with a native machine with a monitor, and
kayfabe's runs have their own display path; only completion status is compared.

| instance (own id) | offer / die | host | lease (UTC) | rate (API `dph_total`) | cost (INFERRED: rate x time) | note |
|---|---|---|---|---|---|---|
| 59493519 | 58548031, RTX 4090 | California | 01:04:16 - 02:06:34 | 0.523 | 0.54 | the console showed Windows installing, but its ssh port timed out for 60 min; destroyed |
| 59499570 | 58507066, RTX 4090 AD102 | Romania | 02:07:45 - 05:01:27 | 0.806 | 2.33 | box A: medical, enscape, catia, unreal, blender (creo cut by the 3 h limit) |
| 59505094 | 58791375, RTX 2060S | Quebec | 03:12:23 - 03:24:13 | 0.128 | 0.03 | stuck in `created` for 12 min, libvirt domain never started |
| 59506269 | 59022146, RTX 3060 GA106 | Quebec | 03:24:26 - 06:19:19 | 0.172 | 0.50 | box C: catia (first run lost to the sysinfo trap) |
| 59517211 | 59273392, RTX 4090 | Texas | 04:28:46 - 05:48:48 | 0.683 | 0.91 | Windows never reachable over ssh in 80 min; destroyed, nothing ran |
| 59529251 | 58676490, RTX 4090 AD102 | Romania | 05:40:18 - 07:18:42 | 0.806 | 1.32 | box B: creo, energy, maya, solidworks, add-on probes |
| 59532831 | 59022146, RTX 3060 GA106 | Quebec | 06:25:27 - 09:16:37 | 0.172 | 0.49 | box D: unreal; enscape unfinished at lease end (7 of 10 subtests) |
| 59537364 | 58781474, RTX 5070 GB205 | Quebec | 07:18:54 - 09:51:53 | 0.244 | 0.62 | box E: catia, enscape (stopped at the coordinator's wrap-up, nothing else run) |

Total INFERRED cost 6.74 USD of the 15 USD cap (sum of the column), plus inbound traffic of about 50 GB per box that the offers priced at 0.0004 to 0.013 USD/GB (at most about 0.7 USD per box, not itemised). The account credit is shared, so cost is rate x time, not the credit change. Every box was torn down by exact id; `vastai show instances-v1 --raw` listed no instance at 2026-11-07T09:51:53Z.

## Results (statuses only; MEASURED on the Vast instances named in each row, windows-on-vast 93b65b3, 2026-11-07)

Cell = status (CLI wall seconds, peak GPU memory MiB at the 5 s sampler). Every Completed row has all its subtests Completed, CLI rc 0 and
`resets_total=0` (`<box>/<viewset>/tdr.txt`, `END.txt`, `cli.log`, `smi.csv`; `kit/summarize.py` prints them).

| viewset | RTX 4090 (AD102, boxes A and B) | RTX 3060 (GA106, boxes C, D) | RTX 5070 (GB205, box E) |
|---|---|---|---|
| blender-01 | PASS (1876 s, 6790 MiB) | not run | not run |
| catia-07 | PASS (803 s, 4593 MiB) | PASS (1611 s, 5757 MiB) | PASS (1806 s, 4810 MiB) |
| creo-04 | PASS (1243 s, 2472 MiB) | not run | not run |
| energy-04 | PASS (323 s, 3464 MiB) | not run | not run |
| enscape-01 | PASS (2042 s, 5157 MiB), SPEC validation passed | unfinished at the 3 h lease end (7 of 10 subtests done, 8th running, 0 resets) | PASS (3298 s, 4675 MiB), SPEC validation passed |
| maya-07 | PASS (400 s, 1139 MiB) | not run | not run |
| medical-04 | PASS (318 s, 5832 MiB) | not run | not run |
| solidworks-08 | PASS (989 s, 2229 MiB) | not run | not run |
| unreal_engine-01 | PASS (1236 s, 5754 MiB) | PASS (1419 s, 5532 MiB) | not run |

BAR1 Total in the guest, `nvidia-smi -q -d MEMORY`: 256 MiB on every box (`box1/box_facts.txt`, `box4/box_facts_tdr_queue.txt`,
`box2/box_facts_and_tdr.txt`, `box5_facts_tdr_enscape_partial.txt`); the Linux stage of box A also showed `Region 1: ... [size=256M]` for the
4090 before the installer erased the disk (transcript). RAM seen: 50 to 58 GB; logical CPUs 4. Driver 580.88 on all, GSP on.

### Failures and refusals, classified

No viewset failed on a loaded benchmark box. What did fail, all outside the GPU:

| event | exact text | class | evidence |
|---|---|---|---|
| every viewset, before the display driver | `Error: System does not support 4K or Full HD display resolution. No compatible resolution found.` (`No graphics card supports 4K or Full HD display resolution.`) | SPEC validation requirement (no monitor on a Vast VM) | box A transcript (medical-04 and a `-d` debug run, 2026-11-07 02:55 and 02:57Z); cured by the virtual display |
| first CLI run after setup on boxes C and D (catia-07 126 s, enscape-01 128 s) | `Error: Unable to gather system details: ... gwpg_sysinfo.exe -s -d` | harness: the run started within about 1 to 2 min of the reboot (a rerun a few minutes later, and every later run, worked) | `box2/queue.log`, `box5/queue.log` |
| download of box C (first copy) | sha256 mismatch after a resumed `curl` | harness: two `curl` processes wrote the same file for a minute | transcript; re-download matched |

UNKNOWN: the enscape-01 slowness on the 3060 (the UltraHQ subtests run at 100 % GPU for 15 to 20 minutes each); not tested whether it
completes with more lease time.

## Add-on probes (coordinator request): RTX 4090, box B, native, virtual display present

`box4_addon_probes.txt`: `overlay_yuv_probe` (the kayfabe lane's `kf_overlayprobe.exe`, steady scenario): FAIL natively with `KFOVL RESULT SETUP
CheckOverlaySupport reports no DXGI_OVERLAY_SUPPORT_FLAG_DIRECT for the chosen format` (nv12, yuy2, p010 all `flags=0x0`). INFERRED: the
indirect display driver offers no overlay plane, so this does not show that a native machine with a real monitor fails. `nbody_demo_gl`
(CUDA demo suite 12.9.79 `nbody.exe -numbodies=16384`): `ALIVE_AT 25 s`; PDH GPU Engine maximum 3d=26 %, copy=0.1 % (no `Compute` instance
shows in PDH), `nvidia-smi` 37 % after.

## CI plan (INFERRED from the measurements above)

Expected to pass under kayfabe CI: all nine viewsets (blender-01, catia-07, creo-04, energy-04, enscape-01, maya-07, medical-04,
solidworks-08, unreal_engine-01). No viewset is excluded by the native reference. Required harness items, each seen on Vast instances 59499570 and 59529251 (windows-on-vast 93b65b3, 2026-11-07): a 1920x1080
display; `vcredist_x64.exe` (energy-04); write access to `viewsets\allTests.json`; at least 16 GB guest RAM; an interactive session; wait
about 5 minutes after the last reboot or retry once on `Unable to gather system details`; one CLI run per viewset.

Time on a good host (boxes A/B, 4090): rental to first CLI run 45 min on box B (Windows ssh up after 17 min; install, driver and CUDA
complete 28 min after the rental on box A; download 14.6 min at 48 MB/s overlapped with the driver install; sha256 1.5 min; unpack 3.7 min;
display, reboot, 1080p 6 min). Sum of the nine CLI walls
= 9,230 s = 154 min (blender 31, catia 13, creo 21, energy 5, enscape 34, maya 7, medical 5, solidworks 16, unreal 21; each includes the
CLI's file verification). Total about 200 min (INFERRED sum of parts timed on instances 59499570 and 59529251, 2026-11-07), under the 300 min box limit: the nine should fit in ONE lease on a fast host. On slow
hosts it does not: box C/D spent 5 min on the hash and 21 to 25 min on the unpack, and the file verification alone took 12 to 15 min for catia-07 (1611 s vs 803 s)
on a 2.2 GHz Xeon, so the same sequence would take about 275 to 305 min there (INFERRED from a 1.3 to 1.5 times longer run phase plus 75 min of setup); **split into two lanes (enscape + blender + creo + energy | catia +
unreal + maya + solidworks + medical) if the lane cannot pick the host**. Disk need: 42 GB download + 94 GB tree + Windows, 300 GB requested
(291 GB root): after the unpack 135 GB were free, so 220 GB is the floor; deleting the `.tar.zst` after the hash frees 42 GB. Download cost:
Vast bills inbound traffic on some hosts (`inet_down_cost` 0.0004 to 0.013 USD/GB in the offers), so 50 GB per box can be 0.02 to 0.65 USD.

## Falsifiers, as registered

| id | outcome |
|---|---|
| N1 BAR1 = 256 MiB on the 4090 | **held** (256 MiB, box A and B) |
| N2 all nine Complete on the 4090, validation passes, zero resets | **held** (two boxes of the same die, per viewset) |
| N3 catia-07 completes natively, zero resets | **held** (4090, 3060, 5070) |
| N4 enscape-01 passes validation natively on the 4090 | **held** |
| N5 the 256 MiB BAR does not make enscape/unreal/blender fail on the 4090 | **held** (all three Completed) |
| N6 on an older/smaller-VRAM die at least one viewset fails or is refused | **not shown**: no failure on the 3060 (12 GB) for the two viewsets that finished, none on the 5070 (12 GB) for the two that ran; enscape-01 did not finish on the 3060 inside 3 h (7 of 10 subtests, 0 resets), where seven subtests plus most of the eighth took longer (about 3,500 s) than all ten on the 5070 (3,298 s; n=1 each, different host CPUs); no 8 GB or Turing box ran (the 2060 SUPER lease never started) |
| N7 the nine do not fit one 300 min lease | **falsified on a fast host (about 200 min, INFERRED sum of parts), borderline on a slow one (275 to 305 min, INFERRED)** |
| N8 download >= 40 MB/s and sha256 equal; unpack <= 45 min | **held**: 48 to 50 MB/s on boxes A/B, hash equal on every box that was used (one corrupted copy was re-downloaded); unpack 3.7 min (A, B) to 25 min (C, D) |

## What this does not show

n=1 per cell, one iteration, FullHD, a virtual display, 4 logical CPUs; no comparison with an owner machine with a monitor; no kayfabe run;
no Turing/8 GB result; the overlay result is specific to the virtual display; two Vast hosts (California, Texas) never became reachable
over ssh after the Windows install and one (Quebec, 2060 SUPER) never started: rental flakiness, not a result about the GPU.

## Files

`PREREG.md`; `STATUS_TABLE.txt` (output of `kit/summarize.py` over every box dir); `kit/` (scripts that ran on the boxes and `summarize.py`); `box1/` (box A: statuses, resets, `smi.csv`, queue), `box4/` (box B),
`box2/` (box C, 3060), `box5/` (box D, 3060), `box6/` (box E, 5070) with `box_facts*`/`*_tdr*` files; `box4_addon_probes.txt`. Raw vendor results (scores, screenshots)
are not in git. A grep of this directory for the presigned-URL signature parameter is empty.
