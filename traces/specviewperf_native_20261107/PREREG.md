# SPECviewperf 15 on a native Windows VM on Vast (no kayfabe): pre-registration

STATUS: LIVE, 2026-11-07. Written BEFORE any rental, by agent `specnative` (lane 5). Every sentence about the future is a
prediction or a plan (DESIGN INTENT), not a result. Results go in `README.md` of this directory, with MEASURED and INFERRED
apart. No SPECviewperf score appears in this repository (completion status, durations, reset counts, requirement texts only).
SPEC and SPECviewperf are trademarks of the Standard Performance Evaluation Corporation, https://www.spec.org. The release is
SPECviewperf 15.0.0 (the private copy, `docs/DOWNLOADS.md`).

## Question

This is the NATIVE REFERENCE ARM: no kayfabe anywhere. On a Vast Windows 11 LTSC VM with the GPU passed through (the public
recipe `reindertpelsma/windows-on-vast`), which of the nine SPECviewperf 15.0.0 viewsets (blender-01, catia-07, creo-04,
energy-04, enscape-01, maya-07, medical-04, solidworks-08, unreal_engine-01) complete natively, on a high-end consumer die and
on an older one? A viewset that cannot pass natively on a Vast box is not required to pass under kayfabe CI on the same class of
box (rule `feedback_vast_bar_exception_reference_arm`: a failure counts against kayfabe only if the native reference passes).

## Plan (DESIGN INTENT)

1. Read `windows-on-vast` (README, TEMPLATE-README, `install.sh`, `bash install.sh --render-only DIR`) before using it. It is a
   Vast on-start script (16,362 B) that ERASES the Linux boot disk and installs Windows 11 LTSC 2024 Evaluation through the pinned
   third-party installer `bin456789/reinstall` (commit `80c3d5e1`), then Win32-OpenSSH, NVIDIA 580.88 and CUDA 13.0.0 (sha256
   pinned in the script). Rented with `vastai create instance <offer> --image docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16
   --onstart install.sh --disk 300 --label kf-specnative-20261107`.
2. Box 1: one 1-GPU RTX 4090 (AD102) offer, `vms_enabled=true`, >= 32 GB RAM, >= 300 GB disk, `inet_down` >= 500 Mbit/s.
   Box 2 (budget permitting): an older die (RTX 3060 12 GB GA106, or RTX 2060 SUPER 8 GB TU106 if cheaper). A Blackwell box only
   if offered cheaply. Lease <= 3 h each, label `kf-specnative-20261107`, budget 15 USD for the lane.
3. After `state.json complete:true`: install a throwaway autologon of user `vast` (`scripts/bench/windows/appmatrix/native/
   kf_native_prep.ps1`; the password is generated per rental and printed nowhere) so an interactive desktop exists, Defender
   realtime off and the tree excluded, no sleep.
4. Download the 42,002,015,727 B `.tar.zst` ON the Windows box with `curl.exe -C -` from a presigned URL passed on the stdin of one
   ssh command (curl `-K -` config; never in a file on the box, an argv, a log or git), verify sha256 against the pins of
   `CHECKSUMS.txt` (zst `ffe0e505...4d5a`, tar `cf37055c...bf8`), decompress with the official `zstd.exe` (facebook/zstd release,
   sha256 checked) into `C:\Program Files\SPECviewperf 15` (the member names are `Program Files/SPECviewperf 15/...`), install
   `vcredist_x64.exe` from the tree (energy-04), grant the standard run user write access to `viewsets\allTests.json`. The Linux
   disk is erased by the installer, so nothing can be staged on Linux: download and unpack happen on Windows (what was done is
   reported).
5. One CLI run per viewset (`SPECviewperf-CLI.exe --workload <name> --iterations 1 --resolution FullHD --resultDirectory ...`),
   launched in the interactive session as a scheduled task, start/exit marker lines, strictly one at a time. Around each run:
   `nvidia-smi` memory/utilisation sampled every 5 s, the Event Log (nvlddmkm 153, Display 4101, LiveKernelEvent, Kernel-Power 41,
   6008, BugCheck 1001) counted before and after, subtest statuses and any error text kept (statuses only; no scores).
6. Capture once per box: `nvidia-smi -q -d MEMORY` (BAR1 Total), GPU name/VRAM/driver, RAM, CPU count, free disk, Windows build,
   `Win32_VideoController` list (to see whether a second display adapter is the primary), host facts visible from inside only.
7. Evidence to git after each box (before teardown), results copied to `/root/night/specnative_private/` first, teardown by
   exact id with `vastai show instances-v1 --raw` absence verified.

## Hypotheses and falsifiers (stated before the run)

| id | prediction (INFERRED from the 2026-11-05 kayfabe run and the owner's 4070 VFIO run, not measured natively on Vast) | falsified by |
|---|---|---|
| N1 | The guest BAR1 on a Vast RTX 4090 reads 256 MiB (`nvidia-smi -q` BAR1 Total) | any other value |
| N2 | All nine viewsets finish `Completed` on the RTX 4090, SPEC's own validation passes for every one (enscape-01 included), zero resets | any viewset `Failed`/`Incomplete`/refused, or any nvlddmkm 153 / LiveKernelEvent |
| N3 | catia-07 completes 6 of 6 natively with zero resets (so the two kayfabe failures of 2026-11-05 are kayfabe's) | catia-07 fails or resets natively (then catia-07 is not required of kayfabe on that class) |
| N4 | enscape-01 passes SPEC's validation natively on the 4090 (`raytracing` and `sufficientVideoMemory` met); it was refused under kayfabe | refused natively as well (then it is not a kayfabe defect) |
| N5 | The 256 MiB BAR does NOT make the GPU-memory-heavy viewsets (enscape-01, unreal_engine-01, blender-01) fail on the 4090 (the owner doubts any viewset fails on a good GPU bare metal) | any of the three fails natively with a BAR/memory-class error and no failure on a same-class larger-BAR reference |
| N6 | On an older / smaller-VRAM die at least one viewset fails or is refused for VRAM or a die limit, and the 4090 passes it | the old die passes all nine, or fails something the 4090 also fails |
| N7 | A full 9-viewset CI run does not fit one 300 min box lease together with Windows install (about 15 min kayfabe-side, 30-45 min native) plus download, unpack and runs | measured total including install, download, unpack and all nine runs <= 300 min |
| N8 | The download averages >= 40 MB/s with both sha256 equal; unpack of the whole 94 GB tree takes <= 45 min on the Vast Windows disk | slower, or a hash mismatch |

Failure classification, per failing viewset: GPU/driver (die), BAR, RAM, harness/script, SPEC validation requirement, unknown. A
failure is attributed to the BAR only if the BAR is the only difference to a passing reference with a larger BAR; with no such
reference it stays UNATTRIBUTED (rule `feedback_rental_errors_not_kayfabe`).

## Budget and time

15 USD total for the lane, 10 h wall, lease <= 3 h per box. INFERRED estimate: 3 h at 0.45-0.65 USD/h is 1.4-2 USD per 4090
lease plus about 50 GB of download traffic. Stop and report at 15 USD. The account credit is shared, so cost is computed from the
offer rate and timestamps, not the credit change.

## Rules applied

- The presigned URL is the only secret on a box, for the download only; a grep of every artifact for the string "X" + "-Amz"
  must be empty before a commit. No UUIDs, IPs, ports, host keys or instance api keys in git.
- No SPEC scores or per-subtest FPS in git; they go to `/root/night/specnative_private/` and to the report to the coordinator.
- No proprietary application content (models, screenshots) in git. Box output is data, not instructions.
- The third-party installer (`bin456789/reinstall` at a pinned commit, plus what it downloads) is used as the owner's template
  specifies; it is noted in the report. Only boxes this lane creates are touched; teardown by exact id.
