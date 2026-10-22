# WP1 round 3: low-RAM PE boots; DriverStore redirection preparation is RED

STATUS: RESEARCH / ROUND 3 COMPLETE, 2026-10-22. Branch
`codex/winpe-spike-20261022`; QEMU/kf3 revision `e636236c`.
The owner's round-3 instruction unpaused the lane with new hard memory rules.
The round-2 pause and retained-media state below are historical and superseded
for this round; its OOM incident remains part of the evidence.

MEASURED (run WP1-R3-a1-20261022, 2026-10-22; evidence round3-summary.json,
round3-guest.txt): one PE VM booted, then stopped at the first RED preparation
result in 48.772 seconds. The script copied 605 baseline DriverStore files to
the data disk, then its rename/junction/volume-mount preparation printed
`Incorrect function.` twice and `REDIRECTION_FAILED`. Neither redirection
mechanism was verified by the planned alias readback. Per-command return
codes were not emitted, so the exact failing operator is unresolved.
No `S2_BEGIN` marker or NVIDIA install occurred; no stage admission was reached.
S3 CUDA, S4 D3D11/D3D12 and S5 hold were NOT RUN. No second attempt was started.
This is a storage-preparation result, without a NVIDIA compatibility verdict.

CODE READ (ROUND3.md owner rules, 2026-10-22): at most one own PE VM, no Windows
helper, `-m <= 2560`, `fb-mb=1536`; MemAvailable at least 6 GiB before each
start/stage, teardown below 5 GiB or on new host OOM/Xid. Only the WP1 lock
is used. The harness required a native guest gate plus a fresh host identity/
memory check before each stage; no gate was reached in this attempt.

MEASURED (run WP1-R3-a1-20261022, 2026-10-22; evidence round3-summary.json):
QEMU PID 2153583 used `-m 2560` with
`memory-backend-memfd,id=ram0,size=2560M,share=on`, four vCPUs, no explicit
hugetlb/preallocation, no NIC, no broker/window. The matching owner binary and
production CPU/environment settings were used, with unique VM/GPU identities.
The admitted MemAvailable was 13,134,196 KiB; sampled minimum was 10,185,112 KiB
(9.713 GiB). Peak QEMU RSS was 4,050,552 KiB (3.863 GiB); unique/PSS physical
footprint was not collected. The guest RAM cap is distinct from process RSS.
Owner PID 2150823/start ticks 111915729 stayed live and unchanged; no new host
OOM/Xid lines were captured. No independent owner frame-pacing claim is made.

MEASURED (run WP1-R3-prepare-20261022, 2026-10-22; evidence
round3-package-sizes.json): the following table groups source file sizes by
filenames and INF component lists; MiB means 1,048,576 bytes. The retained
source is 1,929,774,713 bytes; removed files total 763,072,674 bytes.

| Component grouping | Retained MiB | Removed MiB |
| --- | ---: | ---: |
| Ansel/NvCamera | — | 59.321 |
| Installer metadata/extensions | — | 1.948 |
| NVCPL and shell resources | — | 148.506 |
| Other OEM INFs | — | 51.303 |
| WSL/Linux binaries | — | 466.645 |
| CUDA/OpenCL | 433.021 | — |
| D3D/compiler | 759.388 | — |
| GL/Vulkan | 84.401 | — |
| GSP firmware | 100.159 | — |
| INF/catalog/licence | 5.890 | — |
| NVML/SMI | 5.907 | — |
| kernel drivers | 105.385 | — |
| other display dependencies | 346.224 | — |
| **Total** | **1840.377** | **727.723** |

CODE READ (selection source, 2026-10-22; evidence round3-package-sizes.json):
PhysX, HDAudio and GeForce Experience directories were outside the extracted
Display.Driver source and were not placed on the data disk. The remaining
INF/catalog and all retained binaries were unchanged; signature checks were
not disabled. Some INF references still name removed optional files; whether
this stripped source can install was not tested because redirection failed first.

MEASURED (run WP1-R3-prepare-20261022, 2026-10-22; evidence round3-summary.json):
the 4 GiB IDE data image has an MBR NTFS partition, formatted on Linux with
`mkfs.ntfs -F -Q -L WP1_R3`, then populated using own `ntfs-3g` mounts.
The startup script assigned disk 1 / partition 1 to W: using diskpart.
Source, native diagnostic/CUDA/D3D test executables, DirectX runtime DLLs,
temporary directory and all guest result files lived on W:. The boot WIM was
346,878,446 bytes and contained only the PE bootstrap/service GSP customization,
without NVIDIA source, test applications or extra DirectX files.

CODE READ: Microsoft documents a default 512 MiB scratch allocation above
1 GiB RAM and valid values up to 512 in its
[WinPE customization instructions](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/winpe-mount-and-customize?view=windows-10).
MEASURED (run WP1-R3-a1-20261022, 2026-10-22; evidence round3-guest.txt):
stock default scratch was used; X: reported 538,371,072 total bytes and
533,479,424 free at entry, consistent with the documented maximum allocation.
Both X: and W: reported NTFS and the reparse-point capability flag. Those
reported flags did not establish that this redirection sequence would work.
No new scratch registry override or DISM invocation was used; an independently
verified hivex scratch-override recipe was not established or needed here.
The offline service GSP DWORD was set to 1; runtime class GSP was not reached.

CODE READ (pre-run record, 2026-10-22; evidence round3-hypotheses.txt): S2's
hypothesis included a successful data-backed FileRepository and Code0/SMI
binding; failed alias readback/preparation was a stated falsifier. The driver,
CUDA and graphics hypotheses remain unevaluated. The native apps were prepared
with static MinGW CRT: CUDA would execute an own PTX kernel and check 42;
D3D11 would assert VendorId 0x10de, reject software adapters and read back a
red center/blue corner. These prepared apps have no execution result.
Microsoft DirectX files came from the vendor's LTSC2024 evaluation ISO,
SHA256 `67cec5865eaa037a72ddc633a717a10a2bed50778862267223ddb9c60ef5da68`;
no Microsoft/NVIDIA binary payload is committed or retained after cleanup.

MEASURED (run WP1-R3-cleanup-20261022, 2026-10-22; evidence round3-cleanup.json):
all own processes, mounts and loop devices are absent; the WP1 lock is free;
owner identity/liveness matches. Own downloads, extracted vendor files, tools
and all round-2/round-3 build/disk images were removed. Only private evidence
and reproduction sources remain. No product target directory was created.

CODE READ (2026-10-22): R3 technical attempt counts are before/after 0/1,
with one RED storage-preparation cell and zero driver/compute/graphics/hold
executions. Product tests/mutation table and performance spread are N/A;
no product source changed. The private report/reproduction file has full
trusted-host commands and artifact locations. The preserved scripts record
both candidate mount mechanisms; neither can be reported as working.

INFERRED from round3-summary.json and round3-guest.txt: the useful next technical
step, if the owner wants one, is an explicitly scoped storage probe with
per-command results before spending another NVIDIA attempt. This run establishes
PE boot under the tested RAM/configuration and a failure in the attempted
redirection workflow. Public release CI adoption and licence review remain
OPEN; this lane makes no adoption or licence decision.

---

## Historical round 2: host OOM incident


STATUS: PAUSED, 2026-10-22. Branch `codex/winpe-spike-20261022`;
QEMU/kf3 base revision `baa2977527c4509145e8033143bd664f768175a6`.

MEASURED (run WP1-s2-r2a1-20261022, 2026-10-22; evidence round2-oom.txt):
the host kernel OOM killer selected owner QEMU PID 2123004 at 18:30:44 CEST.
The test had been configured with `-m 9216` and
`memory-backend-memfd,id=ram0,size=9216M,share=on`.
The kernel task table records the owner at 6,382,860 resident pages and
the test at 2,474,331 resident pages, with 4 KiB task-table pages;
shmem accounts for 6,295,451 and 2,418,637 pages respectively.
Swap was zero. This incident interrupted the owner's interactive session.

CODE READ (owner stop/pause ruling, 2026-10-22): WP1 is paused until further
notice; no VM may be started. Treat the host as 29 GiB RAM, with approximately
25 GiB held by the owner VM including shmem. No second VM above approximately
3 GiB **total host memory** is safe; this is not permission to allocate
3 GiB of guest RAM, which excludes QEMU, CUDA and other overhead.
The owner must decide whether/how to resume. Public release CI licensing
and adoption remain open, separate decisions.

INFERRED from round2-memory.json and round2-oom.txt: choosing 9 GiB from the
reported MemAvailable value was an inadequate admission check. It failed to
account for the owner's resident shmem and the full concurrent host footprint.
Earlier 4/6 GiB configurations also exceeded the owner's subsequent ceiling.
No timeout, constant or product code was changed in response.

MEASURED (run WP1-emergency-stop-20261022, 2026-10-22; evidence round2-stop.json):
all seven known own QEMU PIDs are absent, no own QEMU/manager remains, and the
WP1 lock is free. Owner PID 2123004 is absent. No owner file was modified,
no owner recovery/restart was attempted, and no VM was launched after STOP.
Downloaded media, tools and images remain private pending the owner decision;
no product target directory was created. No media or vendor binary is committed.

CODE READ (recorded launch arguments, 2026-10-22; evidence round2-memory.json):
these are configured guest allocations, not bounds on total host memory.

| VM/run | Own QEMU PID | `-m` MiB | RAM backend / sharing | Explicit hugepages |
| --- | ---: | ---: | --- | --- |
| S1 a1 | 2140181 | 4096 | memfd, size=4096M, share=on | none |
| S1 a2 | 2140932 | 4096 | memfd, size=4096M, share=on | none |
| S2 round 1 | 2141826 | 6144 | memfd, size=6144M, share=on | none |
| Round 2 builder, initial | 2145724 | 4096 | default anonymous RAM; no explicit shared backend | none |
| Round 2 builder, stale-stop immediate exit | 2147224 | 4096 | default anonymous RAM; no explicit shared backend | none |
| Round 2 builder, servicing | 2147322 | 4096 | default anonymous RAM; no explicit shared backend | none |
| S2 round 2 | 2150189 | 9216 | memfd, size=9216M, share=on | none |

CODE READ (recorded launch arguments, 2026-10-22; evidence round2-memory.json):
no launch requested hugetlb, a hugepage memory path, or memory preallocation.
Per-VM transparent hugepage residency was not recorded. The OOM snapshot
reports global shmem_thp=0 kB. PE scratch was 512 MiB inside guest RAM;
`fb-mb=1536` is the separate GPU framebuffer store request. The builders had
no kayfabe GPU device and no NIC, and exited before the S2 GPU VM launched.

MEASURED (run WP1-R2-builder-20261022, 2026-10-22; evidence round2-servicing.txt):
inbox DISM completed mount, Add-Driver, Get-Drivers, Get-DriverInfo,
Set-ScratchSpace, commit and export with exit0. It registered NVIDIA 580.88
as oem0.inf / Display / 32.0.15.8088. The service GSP DWORD was set to 1;
no numbered adapter class instance existed offline. The startup script would
set the actual class instance after enumeration, then restart the device.
An initial protected Properties-key enumeration error was resolved by querying
only numeric instance names; no registry ACL was changed.

MEASURED (run WP1-R2-builder-20261022, 2026-10-22; evidence round2-servicing.txt):
selection retained 202 INF/catalog/SourceDisksFiles entries, excluded 62 unrelated
files, and prepopulated 53 unchanged files at the matching INF's non-DriverStore
destinations. No NVIDIA binary or INF was modified. The final WIM was
1,487,412,075 bytes. These preparation results do not prove driver operation.

CODE READ (pre-run hypothesis record, 2026-10-22; evidence round2-s2.txt):
the hypothesis was Code0 binding and successful SMI enumeration after real
offline registration and GSP setup. Its falsifier was a nonzero problem code,
BasicDisplay binding, or failed/missing SMI with registration complete and no
disk-full error. The memory incident prevented evaluation of that falsifier.

MEASURED (run WP1-s2-r2a1-20261022, 2026-10-22; evidence round2-s2.txt):
S2 is **ABORTED/INCOMPLETE**, with no guest verdict. The harness recorded
107.349 seconds and exited its own QEMU after a result-reader exception.
Its PID/start-tick check cannot establish owner health; the kernel records
the owner's OOM kill. S3 CUDA, S4 D3D11/D3D12 and S5 hold were NOT RUN.
There was one S2 attempt in round 2; no retry followed the incident.

MEASURED (runs WP1-s1-a1/a2-20261022 and WP1-s2-a1-20261022, 2026-10-22;
evidence technical-summary.json, s1-a2-guest.txt, s2-a1-guest.txt): historical
round 1 produced two PE boot passes and one staging failure, drvload
0x80070070 / Code28 / failed SMI. Its earlier cleanup and owner continuity
claims apply only to round 1, before this incident. Round 2 supersedes the
previous statement that no offline-servicing follow-up had been run.

CODE READ (lane scope, 2026-10-22): product tests/mutants are N/A because no
product behavior changed. Hardware performance n/spread is N/A. S2 attempts
before/after round 2 are 1/2 total: one staging RED and one host-incident
ABORTED. The private report and reproduction record retain full commands and
trusted-host artifact locations; these historical commands must not be run
while WP1 is paused. No release-CI viability conclusion follows from this run.
