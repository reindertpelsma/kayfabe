# V3 — Windows (and Linux) GPU performance, kayfabe vs VFIO native, 2026-10-15: where the time goes

**STATUS: RESEARCH, 2026-10-15 (measured on the trusted host the same evening; every number below names its source revision and run directory under `/data/claude-perf/ev/`, small summaries are in `traces/perf_win_20261015/`).** Labels: **[measured]** = a number from a run named here; **[read]** = kayfabe code; **[inferred]** = reasoning, never evidence. The hypotheses and their falsifiers were written to `traces/perf_win_20261015/PREREG.txt` before the runs they govern (addenda are dated; several are post-observation and say so).

## 0. Answer in five lines

1. **Windows D3D11 (Heaven, 1080p Ultra) ran at 6 % of native and the cause is one defect, now fixed:** `userstore::Cache::prepare` released every CPU view of the framebuffer store that the next snapshot did not need, so each USER-ring snapshot cost one RM map + mmap + munmap + RM unmap (~16 500 cycles/s on the two kf3 workers, 106 s of SYSTEM time per worker per 293 s). Keeping the views armed (commit `0a7461173`, branch `claude/perf-fix-20261015`) gives **20.9 -> 97.7 fps** (native 330), arms 0 per second.
2. **Windows OpenGL is 65-67 % of native and Linux OpenGL is 84 %.** Linux (true Passthrough, no ring mediation) is GPU-bound at the same SM clock; the 16 % is not clocks (locked: no change), not traps/exits (0.18 % of time), not the display path. Windows adds ~17 points on top (GPU only 82 % busy: latency).
3. **The large-buffer bandwidth knee is the 4 KiB PTE pin** (store slices are mapped with `NVOS46_FLAGS_PAGE_SIZE_4KB`): 2 MiB host PTEs make a 1 GiB streaming read 95.8 % of native instead of 87.6 % (the rest is the P2 memory clock, 10251 vs 10501 MHz). **It barely moves Heaven** (285 vs 281 fps with 98 % of the store-slice bytes on 2 MiB/64 KiB pages) and Blender OptiX only by -8 % (5.3 -> 4.87 s).
4. **Blender OptiX is 1.57x slower than native (5.3 s vs 3.4 s)** on a pure compute workload with no display, and the matmul is 97 %: a second, unexplained gap on RT/BVH-heavy work (page size explains at most ~8 % of it: with 2 MiB/64 KiB host pages OptiX is 4.87 s (4.85-5.43), still 1.43x native; section 6).
5. Not solved: the remaining Linux 16 %, the Windows GL extra 17 points, OptiX 1.57x. Ranked candidates in section 9.

## 1. Configurations (what was compared, and the confounds)

| tag | what | source revision |
|---|---|---|
| **K** | kayfabe production: no `KF3_*` flags, display on (virtual head + broker window), 8 vCPU / 16 GiB (Windows), 6 vCPU / 8 GiB (Linux) | kf3 `666ea966` (broker-resize branch tip) |
| **K-scratch** | `666ea966` + a 20-line scratch change in `userstore.rs` (keep ready views) | tree state = `666ea966` + uncommitted diff (`/data/claude-perf/wt-exp`), NOT a source revision |
| **K-fix** | the committed fix | `0a7461173` (kf3-bins `0a746117`) |
| **K-grain** | scratch experiment, 2 MiB / 64 KiB host page flags for aligned store slices (+ MAPSTAT/KINDSTAT counters) | branch `claude/perf-grain-experiment-20261015` (never merged) |
| **V** | VFIO native: the 4070 bound to vfio-pci, stock KVM irqfd/ioeventfd (no observer, no tracing), same CPU flags, RAM, SMBIOS, tap/slirp, CDs | QEMU of `666ea966` without kf3 |

Same guest disk for K and V (qcow2 overlays `K`/`V` over a common prep overlay `P` of the owner's Windows image; Linux: overlay `L` of `broker-interactive/desktop.qcow2`), Windows NVIDIA 580.88, Linux NVIDIA 580.159.04. **Display:** the 4070 has a monitor on HDMI-A-1 (host `card1-HDMI-A-1` connected), so under VFIO the guest drives it natively at 1920x1080; there is no hybrid/headless-presentation confound for Windows V. Guest `nvidia-smi` is **useless on K** (pstate/clocks/util `[N/A]`): K GPU numbers come from the host's `nvidia-smi dmon` (1 Hz) and V numbers from the guest's. Heaven's frame rate is read from the engine's own `log.html` after the in-app benchmark (F9, 26 scenes, 260.6 s): no PresentMon, no eye reading. The first run of each API on a fresh overlay is a discarded warm-up.

**Sources of error stated.** Reps are 3 unless said (Linux native 1, Linux kayfabe 2-3, as ordered). Heaven repeats agree within 0.2-6.6 %. K's GPU clocks differ from V's: the host driver keeps the card in P2 under kayfabe (SM 2790-2850 under load, memory 10251 MHz) vs V in P0 (SM 2805-2835, memory 10501 MHz).


**Which number came from which tree/revision.**

| label used below | tree / revision | what it contains |
|---|---|---|
| `666ea966` | committed `666ea966` (kf3-bins `666ea966`) | production, before the fix |
| `K-scratch` | `666ea966` + uncommitted 20-line diff (`wt-exp`), installed as `kf3-bins/666ea966-dirty` (overwritten several times, do not trust that directory) | persistent views, no limit logic |
| `0a746117` | committed fix `0a7461173` on `claude/perf-fix-20261015` (kf3-bins `0a746117`) | the fix, no experiment code |
| `b133dd66` **no flag** | `claude/perf-grain-experiment-20261015` at `b133dd66c` = experiment commit `69b349e30` + the fix cherry-picked; `KF3_GRAIN_EXP` was NOT set (launcher override bug, verified in `command.json` and the process environment) | fix + MAPSTAT/KINDSTAT counters, 4 KiB PTEs: `g_*_NOFLAG_fixonly` |
| `b133dd66` **flagged** | same binary, `KF3_GRAIN_EXP=1` verified | fix + 2 MiB/64 KiB host PTEs for aligned store slices: `g_*` |
| grain-only Linux builds | `666ea966` + experiment diff (uncommitted at the time, later committed as `69b349e30`) | Linux Heaven 285.0 (HUGE) / 284.5 (HUGE+BIG), BW sweep |
| FULLMAP build | experiment branch tip `5533398b0` (`KF3_FULLMAP_EXP`, view-copy counters) | built (`kf3-bins/b133dd66-dirty`), **NOT RUN** |

## 2. Results

### 2.1 Heaven 4.0 Basic, 1920x1080 windowed, Ultra, no AA (fps, median and (min-max); gap = kayfabe/native)

| API | OS | K `666ea966` | K-fix `0a746117` | native V | gap before | gap after fix |
|---|---|---|---|---|---|---|
| Direct3D11 | Windows | **20.89** (20.56-20.99) | **97.65** (96.04-102.44) | **330.05** (329.48-330.18) | 6.3 % | 29.6 % |
| OpenGL | Windows | 228.9 (221.6-231.6) | 222.5 (215.3-222.7) | 341.9 (339.6-347.1) | 67 % | 65 % |
| OpenGL | Linux | 281.4, 280.9 (n=2; a third, PROF+perf run 283.0) | - | 334.2 (n=1) | 84 % | - |

K-scratch (persistent views, scratch tree): D3D11 102.8, 103.8; GL 235.2 (n=1). K-fix GL is 2.8 % below K: within the 4-5 % GL spread, not a regression claim.

### 2.2 Other benchmarks

| benchmark | kayfabe | native | gap | note |
|---|---|---|---|---|
| Blender Cycles 5.2.2 OptiX, 1080p 128 spp, 3.9 M tris, render_s (Windows) | 5.33 s (5.23-5.40; K `666ea966`), 5.29 s (K-fix) | 3.40 s (3.39-3.48) | **1.57x** | pure GPU compute, no display; section 6 gives the huge-page re-run |
| Blender CUDA same scene (Windows) | 7.43 s (7.36-7.51) | not run (owner skipped) | - | |
| cup8bench (Windows) N=16 launch->complete | 28 us p50, 44-47 us per launch in a batch of 100 | not run | - | N=2048 matmul 1902-1914 GFLOPS |
| cup8bench (Linux, same binary) N=16 batch/launch | 11 us | 2 us | 5.5x | the doorbell trap (~9 us); 2048^2 fp32 matmul 1887 vs 1949 GFLOPS (96.8 %) |
| vkpeak 0 (Windows, n=1) | fp32 22.3 TFLOPS, fp16-matrix 132.9, int8-matrix 264, copy d2d 195 GB/s | not run | - | |
| clpeak (Windows), Vulkan backend (n=1) | fp32 32.2 TFLOPS, global BW 415-435 GB/s, h2d 26.6 GB/s, dispatch 29 us, roundtrip 237 us | not run | - | |
| clpeak (Windows), **OpenCL backend** | **2 nvlddmkm 153 TDRs** (guest 16:43:37-39 and again 16:48:58-05), run hung, killed by me | not run | - | known item; not repeated on native (owner skipped) |
| cup8bench BW rows, Linux, read GB/s, 64 MiB / 128 / 256 / 512 / 1 GiB / 2 / 4 / 6 | 454-456 / 459-460 / 422-435 / 421-427 / 415-419 / 404-420 / 421-433 / 419 | 445-448 / 455-457 / 463-465 / 470 / 473-474 / 473-475 / 475 / 476 | 88-94 % above 256 MiB | the knee between 128 and 256 MiB |

### 2.3 GPU state during the timed Heaven runs (host dmon for K, guest nvidia-smi for V; busy seconds only)

| run | GPU util | SM MHz | mem MHz | power | pstate |
|---|---|---|---|---|---|
| Windows D3D11, K `666ea966` | **27.9 %** | 2439 | 10009 (P2: 10251) | 44 W | P2 |
| Windows D3D11, K-scratch | 76.4 % | 2793 | 10008 | 91 W | P2 |
| Windows D3D11, K-fix | 75.8 % | 2796 | 10008 | 90 W | P2 |
| Windows D3D11, native V | **97.6 %** | 2835 | 10501 | 191 W | P0 |
| Windows GL, K `666ea966` | 84.3 % | 2780 | 10009 | 137 W | P2 |
| Windows GL, K-fix | 81.9 % | 2780 | 10007 | 131 W | P2 |
| Windows GL, native V | 98.9 % | 2805 | 10501 | 196 W | P0 |
| Linux GL, K | **96.5 %** | 2792 (2850 under load) | 10043 (10251) | 183 W | P2 |
| Linux GL, native V | 99.1 % | 2820 | 10501 | 193 W | P0 |

(the dmon means include the first and last seconds of the benchmark; under load SM is 2850/2835 MHz on K and 2820-2850 on V.)

## 3. Where the Windows D3D11 time went (and the fix)

**Before (K `666ea966`, 293 s Heaven run, per-thread census).** kf3-worker0 and worker1: 46 s user + 106 s SYSTEM each, ~120 000 involuntary context switches each; the eight vCPU threads only 70-80 s each (guest mostly waiting); GPU 28 % busy. `perf trace -s` on the two workers over 40 s: ioctl 568 127 calls (8.8 s of the non-wait 13.3 s = **66 %**), munmap 284 052, mmap 284 039, openat 283 562, close 284 063, epoll_wait 33 410 (blocking, 20 s). The ioctl census (8 s): 131 791 `NV_ESC_RM_MAP_MEMORY` (0x4e) + 131 791 `NV_ESC_RM_UNMAP_MEMORY` (0x4f) = **100 %** of the sampled ioctls, i.e. ~16 500 arm+release cycles per second (~840 per frame), vs OpenGL 4 184 pairs/8 s (~523/s, ~2.3 per frame). Host kernel profile: `gvaspaceInvalidateTlb`, `_mmuWalkPdeRelease`, `kgmmuInvalidateTlb*`, `smp_call_function_many` (munmap shootdowns), user side `RawVec::finish_grow` 8.4 %. Cumulative BAR0 traps are NOT the cost: 3 743/s, handler 0.5 us average, 2.3 ms per second; RPC doorbells 9/s; drainer wake p99 41 us.

**Caller named (H12 falsified, my H12b held).** `perf record -e syscalls:sys_enter_openat --call-graph dwarf` on both workers, 27/27 sampled stacks: `openat <- CharDevice::openat_mode <- HostRm::open_view_node <- HostRm::arm_cpu_view <- userstore::Native::arm <- userstore::Cache::prepare <- userread::snapshot <- userpump::Pump::snapshot <- Pump::turn_body <- Pump::turn <- ChanPlane::serve <- Device::run_translated <- Plane::worker_pass <- worker::run_with_relay`. **Not** `chan.rs StoreViews` (0 of 27), which the coordinator had named.

**The access pattern** (`KF3_USTORE_TRACE`, scratch build, 7.2 s of Heaven D3D11 at 103 fps, `ev/ustore/d3d11_trace_analysis.txt`): per frame 3 166 snapshots (most RAM-backed, 0 windows), **867 view copies, 232 KB**; copy sizes 96-416 bytes (419 438 of 645 199 are <= 256 B, **35 % are 257-416 B**, none larger); each copy is an uncached BAR1 read, p50 2.9 us (<= 256 B) and 6.5 us (257-416 B), ~16 ns/byte, total **38 % of one core-equivalent (~3.7 ms per frame)**, worst copy 0.93 ms. Only **7 distinct 64 KiB windows** across 3 channels and **zero shared between channels**: offline LRU replay, per-channel 1 view 54 % hits, 2 views 85 %, 4 views 99.94 %, 8 views 100 %; a shared pool keyed by store offset (16/64/256 views) also 100 %. A shared pool gains nothing measurable here.

**The fix (`0a7461173`, `docs/design/V3_USER_STORE_VIEWS_20261015.md`).** Views are kept; released only at the per-channel limit (LRU, never a view the read needs) or when RM refuses a mapping (evict own LRU + retry once, then today's arm/release for that read, counted and logged by name). Property `user-view-keep` (ABI 30, default 16 = the old quota), no mutex, no shared structure, nothing derived from BAR1. Counters at shutdown: `USER_STORE_VIEWS keep=16 armed=245 evicted_at_quota=46 rm_refused_*=0` for the whole ~40-minute K-fix Windows session (about six Heaven runs, Blender, cup8bench); before the fix that was ~16 500 arms **per second**. After: **0 openat/mmap/munmap/ioctl in a 10 s window** (D3D11 and GL), worker SYSTEM time 106 s -> 2 s per 293 s.
Tests: `cargo test -p kf-qemu --lib userstore` 13/13 (the churn case, LRU order, quota, refusal -> retry -> fallback -> refused, failed release/mmap ownership) plus a real-mmap 8-thread stress (memfd mapped per view, every byte checked against an offset pattern, injected arm/release/mmap failures, one hostile thread): the first version of the stress found a real defect in the first draft of the change (the retry path discarded the RM mapping a failed mmap left behind: `rm_maps 5311` vs `rm_unmaps 5309`), fixed by `try_arm`. 3 s runs only (`KF3_STRESS_SECS` lengthens); no loom, TSAN, 60-min soak (owner trimmed the scope).

## 3b. Per-frame breakdown (the original question: which component takes the time), Windows Heaven, fix rev `0a746117`

Frame = 1/fps: D3D11 10.24 ms at 97.65 fps (native 3.03 ms at 330.05), OpenGL 4.49 ms at 222.5 fps (native 2.92 ms at 341.9). "per frame" values are the run's totals divided by frames (fps x 260.6 s). M = measured, I = inferred/derived, - = not measured.

| component | D3D11 K-fix | D3D11 native | OpenGL K-fix | OpenGL native | label |
|---|---|---|---|---|---|
| GPU `utilization.gpu` (any resident work, includes stalls) | 75.8 % = **7.76 ms** busy of 10.24 | 97.6 % = **2.96 ms** of 3.03 | 81.9 % = 3.68 ms of 4.49 | 98.9 % = 2.89 ms of 2.92 | M util, I product |
| GPU memory-controller busy (dmon mem %) | 11.4 % = 1.17 ms | not recorded | 21.4 % = 0.96 ms | not recorded | M, native I do not have it |
| SM clock / memory clock under load | 2796 / 10008 (P2) | 2835 / 10501 (P0) | 2780 / 10007 | 2805 / 10501 | M |
| GPU power (host dmon / guest smi) and energy per frame above idle (idle K 36 W, native 9.5 W) | 90 W; (90-36) x 10.24 ms = **0.55 J** | 191 W; (191-9.5) x 3.03 ms = **0.55 J** | 131 W; 0.43 J | 196 W; 0.54 J | M power, I energy |
| worker threads (kf3-worker0+1) CPU | user 294 s + sys 4 s per 293 s window = ~1.0 core = **~10.2 ms core-time per frame**, spinning included | n/a | user 294 + sys 7 s = ~1.0 core = ~4.5 ms | n/a | M (spin and work not separable: I) |
| of which uncached view copies | **~3.7 ms** (867 copies x 4.29 us, 232 KB; K-scratch trace at 103 fps, trace overhead included) | n/a | - (no trace of the GL run) | n/a | M (K-scratch), the rest ~6.5 ms: snapshot, scan, forward, wake/spin (I) |
| vCPU threads (8 + main) | user 500 s + sys 53 s per 293 s = 1.9 cores = ~21.7 ms vCPU-time per frame (guest mostly waiting) | - | user 533 + sys 56 = ~2.0 cores | - | M (native host side not collected) |
| BAR0 traps (PROF, `666ea966` D3D11 at 19.6 fps: 3 743/s) | per frame at 97.7 fps: not measured (3 743/s was at the old rate) | - | OpenGL 13 380/s = ~60 per frame at 222 fps (666ea966 PROF, 229 fps) | - | M rates; handler 0.5 us avg => at most ~0.03-0.1 ms per frame (derived upper bound) |
| KVM exits per frame by reason | **not measured** (`perf kvm stat` was scripted, not run) | - | not measured | - | - |
| doorbell -> GPU start | not measurable on the user-ring path from here; tiny-kernel data points: Windows cup8bench N=16 submit 19-21 us + launch-to-complete 28 us p50; Linux per launch 11 us vs 2 us native (~9 us trap) | - | - | - | M (micro) |
| display flips / copies | flips = app fps (D3D11) ; Linux flips 59/s (compositor) | - | - | - | M, not on the critical path |

**Which of the worker time is ON the submission critical path [inferred]:** a USER pushbuffer segment must be copied (the view reads) and scanned before its authored submission reaches the GPU, so the ~3.7 ms of copies and the scan/forward time of each segment are on that segment's critical path; with two workers and several channels some of it overlaps, so the per-frame latency added is between ~3.7 ms (copies only, fully serialised) and the whole ~10 ms worker core-time. The wake/spin/epoll part (~6.5 ms of the 10.2 ms) is off the path only if a worker is idle when a doorbell arrives; the 345 000 involuntary switches per worker say the workers are often runnable at the same time as the vCPUs.

**Checking the coordinator's inference ('GPU busy ~2.5x native for the same frame'):** the busy-time product is right (7.76 vs 2.96 ms, 2.6x) but it is **not evidence of 2.6x more compute**: the energy above idle per D3D11 frame is equal (0.55 J on both) and the memory controller is only 11 % busy (1.2 ms), so the GPU does the same switching work stretched over 3.4x the time at ~half the power: 'busy' includes stalls (semaphore acquires that wait for a late release / a late submission), i.e. a **latency symptom, not slower compute** [inferred from power/energy and mem %, not from a direct stall counter: the host exposes no SM-activity counter here]. OpenGL: 3.68 ms busy vs native 2.89 ms (1.27x) and 0.43 vs 0.54 J: mostly the same work, ~22 % less energy per frame, i.e. no inflation of compute there either.

**Ranking by ms per frame, D3D11 K-fix (10.24 ms frame vs 3.03 native; 7.2 ms excess):** (1) GPU waiting on late submissions/releases: ~4.8 ms of the 'busy' time above the native 2.96 ms plus the 2.5 ms (24 %) it is idle: I; (2) worker view copies ~3.7 ms of worker time (M, K-scratch) on the segment critical path; (3) rest of worker time ~6.5 ms (I); (4) traps <= 0.1 ms (M rate x measured handler time); (5) display: 0 on the critical path (M).

## 4. What remains after the fix, Windows D3D11 (97.7 vs 330 fps)

Measured on K-fix (`f_d3d11_r1`, 293 s): GPU 75.8 % busy at SM 2796 / mem 10008 MHz / 90 W (native 97.6 % / 2835 / 10501 / 191 W) so the GPU is **no longer saturated; ~24 % idle = latency**. kf3-worker0/1 each ~147 s USER + 2 s SYSTEM (50 % of a core, spin/event loop, 345 000 involuntary switches each), vCPU threads 81-107 s user. Costs per frame at 103 fps (9.7 ms): view copies ~3.7 ms core-time (38 % of one core-equivalent over both workers, trace, **inferred largest single item**, 867 uncached reads), 3 166 snapshots x ~3 us pump overhead, 3.7 k BAR0 traps/s (0.2 %). The 44 % of OpenGL's rate cannot be split further without a worker off-CPU/on-CPU profile of the fixed build (not run: time).

## 5. Linux (true Passthrough) and the Linux/Windows decomposition

**Confirmed passthrough on Linux [measured, run-K1 qemu.log]:** 8 `act birth passthrough` (user channels, USERD in the framebuffer store) + 2 `act birth translated` (the two RM-internal kernel-privilege channels, engine 0xb); the three `DOORBELL-LEDGER` lines are all `route=passthrough` (the busy channel `rung=289887 forwarded=289887 trap=289887`, ~1.1 k doorbell traps/s); **zero** `USER_RING`/userpump/userrelay/userstore/`USER_STORE_VIEW` lines and no `USER_RING_LOOP SUMMARY` at shutdown (printed only when a relay existed); `KF3_*` environment of the qemu process: none (the only relevant switch, `KF3_WIN_USER_RING_RELAY`, is read in `userrelay.rs` only); `perf trace -s` on the workers over 40 s of the Heaven run (PROF run): epoll_wait 39.8 s, write 14-22 k calls (0.04-0.06 s), read < 100, **no openat/mmap/ioctl**; workers busy 320 ms per 100 s, 572 items served; BAR0 traps 2 963/s with handler 1.8 ms per second (0.18 %), 75 RPC doorbells in 100 s.

**Why Linux is 84 % (not a CPU-side cost):** GPU 96.5 % busy at equal SM clock; locking the host clocks to the native SM value (2820 MHz; memory could not be forced above 10251) left the result at 278.2 fps vs 281/283 (pre-registered falsifier 'within 3 %' met: the P2 SM clock is not the cause; memory clock is at most 2.4 %); display flips are the compositor's 59-60/s (Heaven runs uncapped inside it), so the display path is not the limiter (H8 refuted on Linux and on Windows D3D11: flips = app fps, ticks on time, `late_max_us` 5-60 ms sticky max). What is measured: the GPU memory path above 128 MiB (sections 2.2, 6). What is read, not measured: compression stripped (section 7).

**Windows OpenGL = Linux x extra:** 65-67 % = 84 % x ~0.78-0.80, i.e. **~17 points of native are Windows-specific**. Evidence: Windows GL GPU util 82-84 % (Linux 96.5 %), power 131-137 W (Linux 183 W), so ~15 % GPU idle time = latency-bound by the USER-ring mediation (workers 147 s user at 50 % each plus 555 000 involuntary switches, vCPUs 156-171 s) on top of the same GPU-side gap. Not decomposed further.

## 6. The page-size experiment (scratch, `claude/perf-grain-experiment-20261015`)

Step 0 [measured, `MAPBIG` lines]: the BW-sweep buffers are single host map calls through a micro reservation (108 MiB, 256 MiB, 1 GiB, 4 GiB), but all carry `flag=0x100` = `NVOS46_FLAGS_PAGE_SIZE_4KB`: **4 KiB PTEs** by the store-slice pin (`kf_abi::bringup::nvos46_page_size_flag_for_store_slice`, measured w744/w755e: RM ignores `DMA_OFFSET_FIXED` without the pin; the code's own doc: 'this costs TLB reach and the cost is real'). Experiment: 2 MiB (`PAGE_SIZE_HUGE`, 0x400) for slices whose VA, store offset and length are all 2 MiB aligned, then also 64 KiB (`BIG`, 0x200). RM honoured both (flags seen, no relocation, no refusal, no Xid, host Xid count unchanged at 74).
BW sweep, GB/s: 64 MiB 453, 128 443, 256 447, 512 450, 1 GiB 453, 2 GiB 454, 4 GiB 452, 6 GiB 460 (native 445/455/463/470/473/475/475/476; K 4 KiB 454/460/422/421/415/404-420/421-433/419): knee gone, 1 GiB 87.6 -> 95.8 %, 6 GiB 88 -> 96.7 % of native; clock-normalised (native x 10251/10501) 98-99 %. Pre-registered falsifier ('1 GiB < 95 %') NOT met: **the 4 KiB PTEs cause the streaming-bandwidth knee**.
Heaven, Linux: HUGE only **285.0** fps; HUGE + BIG **284.5** fps (vs 281-283): bytes by flag during that run [MAPSTAT, store-slice maps]: 4K 84 MiB (0.8 %), BIG 142 MiB (1.4 %), HUGE 10 048 MiB (97.8 %). Pre-registered: >= 95 % of bytes big/huge and fps <= 288 => **page size is excluded for Heaven**. Windows, b133dd66 flagged (fix + grain flags, `KF3_GRAIN_EXP=1` verified in the process environment, MAPSTAT 1.7 % 4K / 2.7 % BIG / 83.4 % HUGE of all mapped bytes, i.e. 98 % of the store-slice bytes on big/huge pages): Heaven D3D11 97.85 fps (same build without the flag by mistake: 94.83), Heaven OpenGL 223.9 (213.8 without), Blender OptiX 4.85 / 5.43 / 4.87 s (median 4.87; K-fix 5.29, 666ea966 5.33, native 3.40), Blender CUDA 6.91 / 8.22 / 6.96 s (median 6.96; the no-flag counters build gave 8.14 / 8.55, 666ea966 7.43). Huge pages give +3 % D3D11, +5 % GL, -8 % OptiX time, -6 % CUDA time: small, partly inside the 5 % noise, not the source of any gap (OptiX stays 1.43x native).

## 7. The compression suspect (read, partly measured; not decided)

[read] `kf-mem/src/apply.rs:276` and `kf-host/src/channel.rs:846-848`: *this device never backs comptags*; every compressible PTE kind of the guest is rewritten to an uncompressed kind (`kf_chip::uncompressed_pte_kind`), so colour/depth surfaces run without framebuffer compression on kayfabe. [measured, KINDSTAT in the Linux Heaven run, cumulative runs by guest PTE kind] the guest writes almost only kinds 0x06 (33 078 MiB / 8 180 runs), 0x09 (7 924 MiB, 1 run: the flat FB alias), 0x00 pitch (976 MiB), 0x08 (637 MiB), 0x0c (512 MiB): **no compressible kind appears in what the guest writes on kayfabe**. Whether a native guest on the same GPU writes compressible kinds is NOT known (the guest driver may decide from the comptag capability kayfabe reports): the native PTE kinds were not captured. So the suspect stays open: a native PTE-kind capture (the VFIO observer exists) or a fill-rate/MSAA micro native vs kayfabe is the cheap A/B (not run).

## 8. Hypothesis verdicts (falsifiers in PREREG.txt)

| H | verdict | basis |
|---|---|---|
| H7 'compute is near native' | **falsified** for Blender OptiX (1.57x); held for the matmul (96.8 %) and vkpeak/clpeak throughput (no native number) | 2.2 |
| H3 clocks / P-state | **falsified as the Linux cause** (lock to 2820 MHz: 278 vs 281 fps); a real but small part is the memory clock (P2 10251 vs 10501, -2.4 %); Windows D3D11 before the fix was latency, not clocks (GPU 28 % busy) | 2.3, 5 |
| H1 trap cost | **refuted**: cumulative BAR0 handler 0.18-0.2 % of wall, drainer wake p99 41 us (the single worst trap 2 ms sits in the first minutes) | 3, 5 |
| H2 completion/backstop | **not the cause**: `backstop_found(work=0)` in 4 520 / 2 823 backstop wakes; doorbell->work not measurable from here | USER_RING_LOOP SUMMARY |
| H4/H8 display path | **refuted** (Windows D3D11: flips = app fps; Linux: flips 59/s = compositor, ticks on time) ; the display=off control was NOT run | 3, 5 |
| H5 host CPU contention | no thread saturated, perf sched max delays only for idle tasks; not the cause | harvest `sched-latency.txt` |
| H6 BAR1/IOMMU H2D/D2H | not tested natively; clpeak h2d 26.6 GB/s on K | 2.2 |
| H9 worker syscalls dominated by one class | **not falsified**: ioctl 66 %, and 100 % of the sampled ioctls = RM map/unmap | 3 |
| H10 D3D11-specific | not tested (GravityMark is vsync-capped at ~58 fps by the virtual display at default settings: 9 701 frames in 167 s on K) | - |
| H11 doorbells per frame | **refuted**: D3D11 fewer BAR0 traps/s (3 743) than OpenGL (13 380) | 3 |
| H12 caller = StoreViews | **falsified** (0/27), caller = `userstore::Cache::prepare` | 3 |
| H13 persistent views fix D3D11 | **confirmed** (20.9 -> 97.7-103 fps, arms 16 500/s -> 0) | 3 |
| H14 4 KiB PTE grain | **confirmed for streaming bandwidth, excluded for Heaven** | 6 |

## 9. What to fix first (ranked, with expected gain)

1. **Ship the view-retention fix** (`0a7461173`): measured +368 % on Windows D3D11, zero risk to the measured working sets, counters will name any workload that exceeds 16 views per channel. *Gain: D3D11 6 % -> 30 % of native.*
2. **Windows ring-mediation latency** (D3D11 30 % and GL 65 % vs Linux 84 %): GPU idle 18-24 %. Candidates in order of measured weight: the 867 uncached view copies per frame (~3.7 ms of worker time per 9.7 ms frame; none above 416 B; a GPU-side gather or WC mapping is the lever, microbench not run); worker spin/wake structure (345 000-555 000 involuntary switches per worker per 293 s); 3 166 snapshots per frame. *Expected: up to the remaining 24 % idle GPU.*
3. **PTE page size for store slices** (2 MiB/64 KiB when VA, offset, length are aligned and RM honours it, 4 KiB fallback): measured fix for the streaming-bandwidth knee (87.6 -> 95.8 % at 1 GiB); no Heaven/OptiX gain, so low priority; needs the exactness work the micro reservations already provide (RM honoured HUGE/BIG in the experiment but only the flag was tested, not partial unmaps).
4. **The remaining Linux 16 %, OptiX 1.57x:** unexplained; compression (section 7) is the leading read-from-code suspect; capture native PTE kinds first.

## 10. Not done / limits (and what is handed over)

**Done on the committed fix rev `0a746117`:** Windows Heaven D3D11 and OpenGL 3 reps each, Blender OptiX (K-fix reps 5.99/5.29/5.29), cup8bench, the census (arms 0/10 s), `cargo test -p kf-qemu --lib userstore` 13/13 and `--test wire_mirror` 4/4 (ABI 30). **NOT done, handed to Codex (P1):** `cargo test` of the whole `kf-qemu` crate and of `kf-chan`/`kf-mem`, `v3_gates.sh` 9/9, the fast suite 30/30 (the baseline fast guest `perf-baseline/6692e621/fastguest` can be reused: the raw client crate is frozen), the Linux broker lane / `interactive.sh` retest on `0a746117` (my Linux runs used `666ea966`), the Blender CUDA x3 repeat on `0a746117` (the 8.1-8.5 s seen on the no-flag b133dd66 counters build vs 7.4 s on `666ea966` is unexplained: counters overhead or noise; the flagged b133dd66 gave 6.96 s), the 20-minute D3D11 + Minecraft soak. **Scripted but NOT run:** `perf kvm stat` exits per reason and the PROF BAR0 census on the fix rev (`scripts/bench/windows/perf/kvmstat.sh`), the `KF3_FULLMAP_EXP` full-store-view test (code on the experiment branch tip `5533398b0`, built as `kf3-bins/b133dd66-dirty`; prediction pre-registered by the coordinator: no gain because arms are already ~0; falsifier: D3D11 fps +5 % over 3 reps vs 3 reps, or worker copy ms per frame -10 %; the view-copy counters it adds print `view_copies`, `view_copy_ms` at shutdown).
No VFIO Blender CUDA, clpeak, GravityMark, vkpeak (owner skipped); native OptiX 3.40 s was already running when told (kept). No display=off control, no view-copy microbench (access width, WC mapping, in-flight reads), no GPU-gather latency probe, no compression A/B. **The 4 KiB-PTE pin experiment tested only the page-size FLAG (RM honoured BIG/HUGE, no relocation, no refusal, no Xid); partial-unmap exactness under BIG/HUGE host pages, guest leaf splits and migration were NOT tested: no recommendation to ship it.** Host-patch-tier items untouched. The scratch builds overwrote `kf3-bins/666ea966-dirty` repeatedly: never use it as evidence of a revision.

## 11. Open decisions

1. Review/merge the fix commit `0a7461173` after Codex's gate runs.
2. Next target for Windows D3D11/GL (GPU idle 18-24 %): the view READ cost (867 uncached reads per frame, none above 416 B; access width, WC mapping or a GPU gather) vs the worker spin/wake structure vs ring-mediation latency: the FULLMAP test (if run) separates rotation from read cost.
3. Whether to pursue Blender OptiX (1.43x after huge pages, no display involved) via the native PTE-kind capture (compression) or the GMMU page-table placement (PTEs in vidmem vs sysmem) first.
4. Whether the 4 KiB PTE pin deserves a proper design (the streaming-BW knee is real; Heaven/OptiX gains are 3-8 %).
