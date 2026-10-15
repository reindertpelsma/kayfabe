# GravityMark Vulkan frame timings, kayfabe flag-free default route versus the same GPU under VFIO

**STATUS: RESEARCH (measured), 2026-10-13.** First measured performance comparison of the Windows USER relay route. Source of the numbers: GravityMark's own `gm_times.txt` (columns: frame index, timestamp s, frame delta s, 0), read read-only from the retained guest disks
(`qemu-nbd -r` + `ntfs-3g -o ro,force`, partition 3, `\kf\out\gravitymark_vk\gm_times.txt`). Same GPU (RTX 4070), same Windows 580.88, same original 600 s oracle and media.

| run | frames | window | avg FPS | median frame | p99 frame | max frame |
|---|---:|---:|---:|---:|---:|---:|
| VFIO reference (`codex-vfio-gravity-20261011`, PASS 183 s) | 118694 | 167.0 s | **710.7** | 1.29 ms | 3.22 ms | 303.5 ms |
| kayfabe run 1660, ec45aad99, flag-free route (PASS 180 s) | 17216 | 167.0 s | **103.1** | 9.41 ms | 13.56 ms | 212.2 ms |
| kayfabe run 1660, gravitymark_d3d12 (no VFIO counterpart recorded) | 33921 | 167.0 s | 203.1 | 4.73 ms | 8.60 ms | 543.9 ms |

Kayfabe renders about 6.9 times fewer frames than the same GPU under VFIO (14.5 percent of its speed). Both runs are the full benchmark and both pass; this is a throughput gap, not a correctness failure. The kayfabe median frame (9.4 ms) is not the 16.7 ms of a 60 Hz present, so it is not plain vsync pacing (INFERRED).

## What the run 1660 log shows (MEASURED, `boundary-kayfabe-1660/qemu.log`)
- About 2.98 million trapped BAR0 accesses over the run (`trapped=2977301` at the last status line).
- The Translated kernel copy channel (token 0x80c, host 0x10033) forwarded 218791 doorbells for 35841 submissions during the run. That is heavy kernel copy (paging-node) traffic.
- MMU invalidates 8875, walks 20503/20503.
The per-channel counters of the USER relay channel itself were not found in the retirement lines; its cost is not separated yet.

## Not established
Which of these causes the gap: the USER relay (worker wake and pushbuffer inspection per submission), the kernel copy-channel traffic (INFERRED possible VidMm paging: the guest is told 4096 MiB of VRAM against 12 GiB real, native peak 2247 MiB), trap volume, or the display path.
Planned one-variable A/B runs (after the regression matrix): (1) `WIN_FB_MB` near the real 12 GiB; (2) `KF3_WIN_USER_RING_RELAY=0` on the D3D12 benchmark (relay cost without the migration failure); (3) `perf kvm stat` VM-exit counts by address during the benchmark. Falsifier for (1): FPS and the 0x80c counters do not change.
