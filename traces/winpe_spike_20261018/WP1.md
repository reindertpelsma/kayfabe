# WP1: PE boots; NVIDIA driver staging is red

STATUS: RESEARCH, 2026-10-22. Branch `codex/winpe-spike-20261022`;
device/QEMU base rev `baa2977527c4509145e8033143bd664f768175a6`.

CODE READ: the owner authorized this local private spike using vendor downloads,
with no media/driver redistribution. A later WP1-only ruling replaced the shared
fastguest lock with `flock -o /tmp/kayfabe-wp1.lock`. The owner VM was to remain
running. Earlier preparation stops are superseded for this spike; their records
remain in branch history and `host-preflight.txt`. Public release CI adoption
and licensing remain separate open questions; this lane decides neither.

MEASURED (run WP1-s1-a2-20261022, 2026-10-22; evidence s1-a2-guest.txt): S1 GREEN.
Microsoft's amd64 WinPE 26100.1 reached its startup script and returned
`WP1_S1_PASS` on the partitioned FAT results disk in 72.725 seconds including
VM start/teardown.

MEASURED (run WP1-s2-a1-20261022, 2026-10-22; evidence s2-a1-guest.txt): S2 RED.
`drvload` returned `0x80070070`; the NVIDIA PCI device remained at problem code
28 with no assigned driver/service. NVML/SMI could not communicate with the
driver. The lane stopped at this first red stage. S3, S4, D3D12 and S5 were NOT RUN.

CODE READ: Win32 error `0x70` is `ERROR_DISK_FULL` (112), per
[Microsoft's error reference](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-).
INFERRED: the driver-staging writable space is the leading preparation limit;
the failing volume's exact free-space/scratch accounting was not collected.

MEASURED (run WP1-s2-a1-20261022, 2026-10-22; evidence s2-a1-guest.txt): the full
unmodified INF source package was injected into the WIM, with nvlddmkm.sys, NVML
and SMI also placed at their diagnostic/system paths. The offline service GSP
DWORD was 1. Runtime class-key setup returned `GSP_CLASS_KEY_ABSENT` both before
and after the failed staging attempt. Completed offline DriverStore registration
and the adapter class GSP key were NOT achieved. The prepared source package
plus runtime `drvload` did not fulfill that part of the requested offline integration.

INFERRED: this result establishes a staging failure in the attempted recipe.
NVIDIA WDDM, CUDA and D3D compatibility in PE remain unverified. A follow-up
would need proper offline DriverStore servicing before testing driver binding;
no such follow-up was run after S2 went red.

CODE READ: the hypotheses and falsifiers below were recorded before the runs.

| Stage | Hypothesis | Falsifier | Result |
| --- | --- | --- | --- |
| S1 | Driver-free PE boots on the kayfabe model. | No PE boot within the attempt budget. | GREEN, 2 control boots |
| S2 | NVIDIA binds with problem code 0 and SMI lists the GPU. | Nonzero device problem or missing/failed SMI enumeration. | RED, 1 attempt; code 28 and SMI failure |
| S3 | A simple CUDA sample completes correctly. | CUDA initialization, execution or result error. | NOT RUN |
| S4 | Hardware D3D11 draws correct offscreen pixels. | Device creation, software adapter or pixel failure. | NOT RUN |
| S5 | The S4 loop holds for ten minutes without GPU errors. | Attributable host Xid, guest 153 or TDR. | NOT RUN |

MEASURED (runs WP1-s1-a1-20261022, WP1-s1-a2-20261022, WP1-s2-a1-20261022,
2026-10-22; evidence technical-summary.json): all attempts used `fb-mb=1536`,
the owner's QEMU/kf3 binary and production CPU/behavior flags, a unique VM/GPU
identity, and no display broker/window or VNC. Guest RAM was 4096 MiB for S1
and 6144 MiB for S2, with four vCPUs. Storage used separate IDE boot/results images.

CODE READ (rev baa29775, crates/kf-chip/src/bar0.rs): the firmware carve-out is
`0x10420000` bytes; 1536 MiB exceeds that layout minimum. This is not a bound
on the Windows driver's eventual memory demand.

MEASURED (run WP1-prepare-20261022, 2026-10-22): the host reported 2389 MiB free
VRAM before the control attempt. The S2 WIM was 1,496,453,038 bytes. No store
above 1536 MiB was requested. MEASURED (run WP1-final-20261022, 2026-10-22;
evidence technical-summary.json): every attempt has zero new host Xids and owner
PID/start-tick continuity.

MEASURED (run WP1-s2-a1-20261022, 2026-10-22; evidence s2-a1-qemu.txt): QEMU
reports a 1536 MiB store and 236 MiB of walker pools. The pools are additional
GPU memory beyond the store; total context overhead was not separately sampled.

| Run on 2026-10-22 | Stage assessment | Harness/transport | Total wall seconds | New host Xids |
| --- | --- | --- | ---: | ---: |
| WP1-s1-a1 | GREEN: PE shell/version observed | COM1 unavailable; initial unpartitioned results disk; explicitly stopped after headless screenshot | 359.714 | 0 |
| WP1-s1-a2 | GREEN | Partitioned FAT result contains version and success marker | 72.725 | 0 |
| WP1-s2-a1 | RED | FAT captures drvload/ConfigManager/SMI failure | 195.925 | 0 |

MEASURED (run WP1-final-20261022, 2026-10-22; evidence technical-summary.json):
technical attempt counts before/after are 0/3: two control boots and one driver
attempt. These wall times include different workflows and are not comparable
boot-time samples; no performance spread is claimed. Product tests/mutants are
N/A: there is no product behavior change or proposed product fix.

MEASURED (run WP1-s2-a1-20261022, 2026-10-22; evidence s2-a1-qemu.txt): filtered
QEMU output shows the boot framebuffer and guest-RAM mapping. The last device
phase was Cold, as it was on both controls. No guest-driver GPU workload was
established. The guest lacked `sc.exe`; ConfigManager and the service property
supplied the binding check. Numeric command exit-code echoes were not captured
reliably; the HRESULT, problem code, helper result and stage marker are retained.

CODE READ: the Microsoft download entry is
[the ADK/WinPE page](https://learn.microsoft.com/en-us/windows-hardware/get-started/adk-install).
The bootstrap URL was `https://go.microsoft.com/fwlink/?linkid=2289981`.
Payloads were fetched from its Microsoft download directory and checked against
the bootstrap's declared hashes. The amd64 WIM cabinet was
`a6296c5a6eec9ca02b431ed0461b952d.cab`; WIM image 1 identifies WindowsPE 26100.1.

MEASURED (run WP1-prepare-20261022, 2026-10-22): the NVIDIA runtime download was
`https://us.download.nvidia.com/Windows/580.88/580.88-desktop-win10-win11-64bit-international-dch-whql.exe`,
SHA256 `90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9`.
No Microsoft or NVIDIA files are committed. Only small text evidence is exported.

CODE READ: the private retained `repro/` directory contains the exact preparation,
image-build, guest diagnostic source and run-manager scripts. The owner's local
report records its trusted-host location and full regeneration commands.
The launch commands were:

```sh
flock -w 5 -o /tmp/kayfabe-wp1.lock python3 "$WP1_ROOT/run_stage.py" s1 a1
flock -w 5 -o /tmp/kayfabe-wp1.lock python3 "$WP1_ROOT/run_stage.py" s1 a2
flock -w 5 -o /tmp/kayfabe-wp1.lock python3 "$WP1_ROOT/run_stage.py" s2 a1
```

CODE READ: the final control script uses `wpeinit`, locates the `WP1.TAG` FAT
volume, writes the OS version and `WP1_S1_PASS`, then idles. The driver script
adds the dynamic class/service GSP setup, `drvload`, a device restart, binding
queries and SMI. Each manager owns its QEMU child, checks the owner identity,
stops on a new host Xid, and caps the attempt at 1190 seconds plus teardown.
S1 used a new-message dmesg reader; S2 used an independent `/dev/kmsg` read
cursor established before launch. No shared kernel log was cleared.

MEASURED (run WP1-final-20261022, 2026-10-22; evidence cleanup.json): all three
own QEMU PIDs are absent; the owner PID 2123004/start ticks 110376932 are unchanged.
The WP1 lock was released. Media, extracted vendor files, tools and build images
were deleted; only private evidence/reproduction text and diagnostic PNGs remain
on the host. No product target directory was created. Owner frame pacing was
not independently sampled; no claim about its FPS is made.

INFERRED from the stage table in WP1.md: demonstrated coverage is PE boot on this device/configuration.
Driver binding, compute, D3D rendering and a ten-minute accelerated hold remain
outside demonstrated coverage. Public CI licensing/adoption is still open.
