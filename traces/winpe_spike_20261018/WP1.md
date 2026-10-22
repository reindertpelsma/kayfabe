# WP1: paused after host OOM killed the owner VM

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
