# V3_LINUX_PERF_BASELINE_6692e621 - the Linux performance baseline before the Windows integration

**STATUS: LIVE, 2026-10-10 (BEFORE end only; the AFTER end has not been run).** Measured on the trusted host (172.22.1.20: Ryzen 9 7900, 24 threads, RTX 4070, host driver 595.91.07 open module, kernel 7.0.0-34, **not nested**) with `scripts/bench/perf_compare.sh 6692e621 --reps 3`. Raw output and parsed numbers are in `traces/perf_baseline_6692e621_20261010/` (and on the host in `/var/lib/kf-windows-20261005/perf-baseline/6692e621/`). Purpose (owner): quantify the performance impact of everything added for Windows, by running the identical command on the integrated branch and diffing against this baseline.

## 1. Which revision, and why (read before using the diff)

**BEFORE = `6692e621`** (master code of 2026-10-08; the revision with "guest 67/71 = 61/65 apps + 6/6 probes" in `docs/STATUS_AND_HANDOFF.md`). It exists in git and is an ancestor of `claude/tmode-pieces-20261009` (`c6fff2e3`, the base of this branch).

Two measured/derived facts limit what "before Windows" means:

1. **`6692e621` is not purely pre-Windows (derived from git).** It already contains the default-off Windows chain (Translated deferred API, class 5080; opt-in private Translated spaces; software-runlist flag; per-VM GPU UUID). `git diff --shortstat 906a76a4 6692e621 -- crates qemu cuda` is 168 files, +50826/-3778. The last master before the first Windows commit (`f40cd822`, 2026-10-04) is `906a76a4`. So the AFTER-minus-BEFORE delta measured with this baseline **understates** the total Windows cost by whatever those default-off changes cost.
2. **`906a76a4` cannot run on this host (measured).** I built it (`kf3-bins/906a76a4`) and booted two fast-guest arms: both `CRASH`, and the QEMU log says `kf3: realize refused: RM bring-up failed at R2 host driver version: host driver 595.91.07 is not a measured tag of the driver matrix`. The 595 acceptance arrived later (`f7aad91c`, +2377 lines incl. a generated 523-line matrix); cherry-picking it would make a synthetic revision, which I did not do. To measure a true pre-Windows point use a host with a 580.x driver (vast; nested, so a different baseline) or decide on a cherry-pick. `6692e621` is also the first revision in which the raw client and the kf3 device accept this host's driver, which is why it is the only usable BEFORE here.

The AFTER end is `c6fff2e3` or whatever the integrated branch is; the 6692e621..c6fff2e3 diff in `crates qemu cuda` is 79 files, +25186/-6541 (derived).

## 2. Existing numbers (harvested from docs/ and traces/, not re-measured)

Nested = a vast.ai box that is itself a KVM guest (a VM exit costs 10-40x bare metal); none of these is comparable with the bare-host numbers in section 4.

| What | Value | Revision | Box / GPU / driver | Source |
|---|---|---|---|---|
| 30-arm raw client, bare, sum of arm wall | 56.6 s (median 109 ms, max 28.6 s gpga-reserve-probe) | `6c8bedc6` | this host, RTX 4070, 595.91.07, bare | `traces/rawclient_all_drivers_20261008/rawclient595bare2_suite.out` |
| 30-arm fast guest, sum of `client_ms` | 66.2 s (median 500 ms; ce-client-guest-ram 20.05 s; defer-liveness 12.5 s; gpga-reserve-probe 9.3 s); arm wall median 4 s, total 165 s | `0e64a960` | this host, 595.91.07, bare | `traces/merge_bar_20261008/mchost2_fast_suite.out` |
| same, nested | sum `client_ms` 189.9 s, total wall 814 s, ce-client-guest-ram 67.1 s | `b5c9f717` | vast `mcbox`, RTX 3060, 580.159.04, nested | `traces/merge_bar_20261008/mcbox_fast_suite.out` |
| CUDA ladder guest, N=16 batched per launch | 27 / 27 / 32 us (fast path off) | `ca7a5006` | vast, RTX 3060, 580.159.04, nested | `traces/v3_ioeventfd/dbl2_ca7a5006/dbl2.log`; `V3_DOORBELL_IOEVENTFD.md` 277-301 |
| cup8bench N=2048 | guest 784 GFLOPS vs host 786 | `b5c9f717` | vast `mcbox`, nested | `traces/merge_bar_20261008/README.md` |
| cuInit / ctx create (guest) | 3.2 s / 1.0 s (host GA106: 1.2 s / 0.13 s) | `ca7a5006` | vast, RTX 3060, nested | `traces/v3_ioeventfd/dbl3_ca7a5006/dbl3.log` |
| doorbell store, vCPU cost p50 | trapped 20.4 us, ioeventfd 15.7 us | `43293417` | vast, RTX 3060, nested | `V3_DOORBELL_IOEVENTFD.md` 228-266 |
| Qwen2-0.5B decode (eager PyTorch), warm | host 42.2 tok/s, guest 12.2 tok/s (0.29x) | `ca7a5006` | vast, RTX 3060, nested | `V3_DOORBELL_IOEVENTFD.md` 312-316 |
| llama-bench Qwen2.5-1.5B Q4_K_M | pp512 0.998x, tg64 0.877x of host | `9d82f259` | vast, RTX 3060, nested | `traces/v3_candidates/cand2_20261004/apps_*` |
| `bandwidthTest` 32 MB pinned H2D / D2H / D2D | host 7.8 / 8.0 / 327 GB/s, guest 9.7 / 10.3 / 322 GB/s | `9d82f259` | vast, RTX 3060, nested | same |
| NVENC 1080p h264 | host 198 fps, guest 142 fps | `d06833f0` (pre-`afb552ea`) | vast, RTX 3070, nested | `V3_GFX_TESTSET.md` 251-270 |
| app matrix, sum of per-app seconds | host 1075 s, guest 1193 s | `9d82f259` | vast, RTX 3060, nested | `traces/v3_candidates/cand2_20261004/apps_*` |

Never recorded anywhere before this baseline: any kf3 CUDA ladder, doorbell-exit, LLM, graphics or app number on a non-nested host; any v3-era `BWROW` kernel-bandwidth row; any max-pass-time or held-reply-age figure (`docs/design/V3_NONSTALL_THREADS.md`, named by `AGENTS.md`, does not exist in the tree).

The two bare-host suite files reproduce within about 1% in section 4 (sum `client_ms` 65.8 s vs 66.2 s; bare 55.6 s vs 56.6 s), a useful cross-check that the host is quiet and the harness is the same.

## 3. The harness

`scripts/bench/perf_compare.sh <rev> [--baseline <rev|dir>] [--lanes ...] [--reps N]` and `scripts/bench/perf_summarize.py` (stdlib only; `extract`, `summary`, `compare`). One command, the same set for any revision:

| lane | what it runs | numbers |
|---|---|---|
| `bare` | `scripts/fastguest/bare_metal_suite.sh`, raw client, no VMM | per-arm wall (100 ms resolution), sum |
| `fast` | `scripts/fastguest/fast_suite.sh` (30 arms, one boot each, kf3, `QEMU_BIN=<rev>`) | per-arm `client_ms` (10 ms resolution), per-arm wall s (1 s), sum, pass count |
| `ladder_host` | `scripts/bench/cuda_ladder.sh host` (cup2/cup3/cup8/cup8bench) | rung wall; cup8bench BSUM: launch, submit, sync, H2D, D2H, matmul GFLOPS; cuInit/ctx/module ms |
| `ladder_guest` | `cuda_ladder.sh guest`, one fat-guest boot per rung per rep, each under `flock -o` | the same, plus `trapped=` (BAR0 write exits) |
| `exit` | `dbfast_exit_hook.sh` in the fat guest, doorbell fast path off and on | p50/p90/p99/mean ns of one doorbell store; trapped reference; no-op MMIO floors |

Design points that make the AFTER run like-for-like:

- Harness scripts are the checkout's own for both ends; only `QEMU_BIN` (the kf3 device) differs. The fast guest's initrd (kernel, host-mode 595.91.07 modules, and the **reference revision's** raw client) is built once and reused (`PERF_FASTGUEST_DIR`, default `perf-baseline/6692e621/fastguest`), so `client_ms` differs between ends only by the device. The fat guest image is the same overlay for both ends.
- Locking: `fast` and `bare` lanes take `/tmp/kayfabe-fastguest.lock` themselves (per arm / per suite); fat-guest boots are wrapped in `flock -o`. A held lock means wait. Load average, other QEMU count and GPU memory/util are logged before every rep (`raw/noise.log`); in the baseline run every entry shows no tenant (qemu=0, GPU 9 MiB, load 0.3-1.5).
- cup8bench runs 20 timed iterations but 100 launches per timed batch (`KAYFABE_BENCH_BATCH=100`, the ladder default is 10), because the printed per-launch value has 1 us resolution; `derived.N16.batch_launch_us` = `BATCH_TOTAL_MS / BATCH` resolves 0.01 us.
- `compare` lists metrics whose medians differ by 10% or more and marks `OUT` when the new median is also outside the baseline's min..max by more than 2%. Several metrics are quantised (bare ms to ~100 ms, fast arm wall to 1 s, N=16 submit/sync to 1 us): read those deltas with the resolution in mind.

### Reproduce (BEFORE end)

```
W=/var/lib/kf-windows-20261005
git clone --shared $W/kayfabe-96d93c32 $W/kayfabe-perf-6692e621   # any host clone; then: git checkout --detach 6692e621
cp -a $W/irqflood-qemu/qemu-10.2.4 $W/perf-qemu/                    # private QEMU tree, so no shared build is touched
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=$W/target-perf-6692e621
(cd $W/kayfabe-perf-6692e621 && bash scripts/bench/build_kf3.sh $W/perf-qemu/qemu-10.2.4 $W/perf-qemu/qemu-build-kf3 \
  && KAYFABE_BUILD_REV=6692e621 cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder)
cp -a $W/perf-qemu/kf3-bins/6692e621 $W/kf3-bins/                    # keeps the pc-bios and qemu-bundle links
mkdir -p $W/perf-baseline/6692e621/bin && cp $CARGO_TARGET_DIR/release/kayfabe-rm-ladder $W/perf-baseline/6692e621/bin/
(cd $W/kayfabe-perf-6692e621 && KF_FROM_HOST=1 CLIENT=$W/perf-baseline/6692e621/bin/kayfabe-rm-ladder \
  bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 $W/perf-baseline/6692e621/fastguest)
# fat guest: the base image carries 580.159.04, which this host's 595.91.07 cannot GSP-boot (measured, section 5)
mkdir -p $W/perf-baseline/drivers/595.84 && ln -s /root/converge/nvidia/NVIDIA-Linux-x86_64-595.84.run $W/perf-baseline/drivers/595.84/
bash scripts/drivermatrix/stage_fat_guest.sh 595.84 $W/perf-baseline/drivers   # -> /workspace/bench/guest-595.84.qcow2 (overlay)
bash scripts/bench/perf_compare.sh 6692e621 --reps 3                          # ~23 min with the lock free
```

### Run the AFTER end (later)

```
# build <afterrev> exactly as above (kf3-bins/<afterrev>/ with its links), then, from the branch that holds perf_compare.sh:
bash scripts/bench/perf_compare.sh <afterrev> --baseline 6692e621 --reps 3     # prints and saves compare.txt
```

## 4. Baseline (6692e621, 3 reps, median (min-max))

Run 2026-10-10 00:41-01:04 CEST, harness at `c6fff2e3` + the committed scripts of this branch, QEMU binary sha256 prefix `2d5c835fde3b5580`, initrd `34f4700d6510c176`, client `391b4252b4473b9c`. All lanes 3/3 reps PASS (bare 30/30, fast 30/30, all four rungs).

**Suite totals (measured):**

| | median (min-max) |
|---|---|
| bare raw client, sum of 30 arms | 55.6 s (55.4-55.7) |
| fast guest, sum of `client_ms` (boot excluded) | 65.8 s (65.6-66.1) |
| fast guest, sum of per-arm wall (boot included) | 167 s (166-167) |

**Per arm** (bare = no VMM; fast = in the kf3 guest; `client_ms` is the raw client's own wall in the guest, so fast minus bare is the VMM cost; arm wall adds about 3.5 s of boot):

| arm | bare ms | fast client_ms | fast arm wall s |
|---|---|---|---|
| alias-two-vas | 109 (109-109) | 390 (379-390) | 4 (3-4) |
| alias-unmap-observe | 109 (109-109) | 390 (390-399) | 3 (3-4) |
| atomics-probe | 109 (109-109) | 379 (379-380) | 3 (3-4) |
| bar1-crossing | 109 (109-109) | 390 (379-390) | 4 (3-4) |
| blockage-coverage | 510 (510-510) | 810 (800-810) | 4 (4-4) |
| bus-info-sweep | 3319 (3318-3319) | 3600 (3599-3600) | 7 (7-7) |
| ce-client | 109 (109-109) | 430 (429-439) | 4 (3-4) |
| ce-client-guest-ram | 2110 (2009-2110) | 19959 (19880-20080) | 23 (23-24) |
| concurrency | 3012 (3011-3013) | 3340 (3340-3340) | 6 (6-7) |
| concurrent-fuzz | 209 (209-210) | 1410 (1299-1460) | 5 (5-5) |
| cross-client-leak | 109 (109-109) | 459 (450-459) | 4 (4-4) |
| defer-liveness | 12116 (12116-12117) | 12510 (12500-12530) | 16 (15-16) |
| dictated-ring | 109 (109-109) | 379 (379-390) | 4 (3-4) |
| dictated-ring-negative | 109 (109-109) | 429 (420-430) | 4 (4-4) |
| doorbell-census | 109 (109-209) | 820 (800-830) | 5 (4-5) |
| engines | 108 (108-108) | 669 (669-670) | 4 (4-5) |
| executor-vas | 109 (109-109) | 430 (429-439) | 4 (4-4) |
| gpga-reserve-probe | 27521 (27421-27523) | 9300 (9300-9310) | 13 (12-13) |
| gpu-info-sweep | 109 (109-109) | 390 (379-390) | 4 (3-4) |
| guest-ram-pin | 109 (109-109) | 420 (410-429) | 4 (4-4) |
| guest-ring-channel | 109 (109-109) | 410 (390-410) | 4 (4-4) |
| late-map-race | 510 (510-510) | 799 (799-810) | 4 (4-5) |
| map-propagation | 109 (108-109) | 379 (379-390) | 4 (4-4) |
| map-stress | 109 (109-109) | 650 (649-660) | 4 (4-4) |
| missing-page-fault | 3111 (3111-3112) | 3450 (3440-3460) | 7 (6-7) |
| pce-mask-probe | 109 (109-109) | 379 (370-379) | 3 (3-4) |
| rpc-mixed-allocs | 912 (912-912) | 1230 (1230-1250) | 5 (5-5) |
| timer | 109 (109-109) | 399 (399-410) | 4 (3-4) |
| uvm-invalidate | 108 (108-108) | 540 (529-549) | 4 (3-4) |
| uvm-mean | 209 (209-210) | 700 (690-710) | 4 (4-4) |

Readings (derived from the table): most arms cost a roughly fixed +270-300 ms in the guest (device open, GSP boot, RM init through the emulated path); `ce-client-guest-ram` is the largest gap (2.1 s bare, 20.0 s guest, 9.5x); `gpga-reserve-probe` is faster in the guest (9.3 s) than on bare metal (27.5 s) - measured, reproduces the earlier 28.6 s / 9.3 s pair, cause not established.

**CUDA ladder, cup8bench (N=16 isolates per-launch cost, N=2048 is a 2048x2048 float matmul), host vs fat guest:**

| metric | host (bare) | guest (kf3, 6692e621) |
|---|---|---|
| batched launch, N=16 (BATCH_TOTAL_MS/BATCH) | 1.71 us (1.70-1.93) | 10.21 us (10.20-10.21) |
| launch rate, N=16, batched | 585k/s (518k-588k) | 97.9k/s (97.9k-98.0k) |
| submit median, N=16 | 1 us | 10 us (10-10) |
| sync median, N=16 | 3 us | 3 us |
| cuInit | 40.0 ms (40.0-42.8) | 475.9 ms (467.7-479.3) |
| cuCtxCreate | 71.5 ms (69.8-72.1) | 243.7 ms (179.1-262.0) |
| N=2048 batched GFLOPS | 1914 (1913-1917) | 1917 (1914-1923) |
| N=2048 per-launch median | 8.80 ms (8.79-10.10) | 10.10 ms (8.76-10.10) |
| N=2048 H2D, 2 x 16 MiB (derived GB/s) | 2.695 ms = 12.45 GB/s | 2.918 ms = 11.50 GB/s (11.40-12.77) |
| N=2048 D2H median, 16 MiB (derived GB/s) | 1.204 ms = 13.93 GB/s | 1.446 ms = 11.60 GB/s (11.48-11.75) |
| trapped BAR0 write exits, cup2 / cup3 / cup8 / cup8bench | - | 12119 / 12117 / 12249 / 12959 |
| rung wall: cup2 / cup3 / cup8 / cup8bench | 207 / 206 / 307 / 1408 ms | 1 s resolution: cup8 1 s, cup8bench 2 s |

Readings: per-launch cost is about 6x the host's (10.2 vs 1.7 us, measured) and is the Linux-path number a Windows-side change to the doorbell path would show first; large-kernel throughput is equal. The N=2048 per-launch median is bimodal (8.76 or 10.10 ms) on both host and guest across reps; both are the same GPU doing the same kernel, so the 10.10 ms mode is a GPU clock state, not kayfabe (inference; clocks were not logged). Compare `batch_gflops` or `min_ms` for compute, not `med_ms`.

**Doorbell store cost seen by the guest vCPU (`dbfast_exit_hook`, 200000 stores x 3 per boot, 3 boots per mode):**

| store | fast path off | fast path on (`doorbell-ioeventfd=on`) |
|---|---|---|
| matched doorbell register (probe), p50 | 6095 ns (5905-6126) | 1839 ns (1835-2242) |
| never-registered register (always trapped), p50 | 6095 ns (5924-6119) | 6155 ns (5959-6160) |
| no-op MMIO in QEMU, lockless / under BQL, p50 | 5233 / 5368 ns | 5226 / 5366 ns |
| plain ioeventfd with no memslot (floor), p50 | 1285 ns | 1284 ns |

So one trapped doorbell write costs about 6.1 us of guest vCPU time on this host (nested vast boxes: about 20 us) and the optional fast path takes it to about 1.8 us; the no-op MMIO floors show the exit to QEMU itself is about 5.2 us.

## 5. What could not be run, and what was changed to run the rest

- **Fat-guest lanes needed a different guest driver (staging, not a test substitution).** The base image `/workspace/bench/guest.qcow2` carries 580.159.04 and does not boot on this host: `RmInitAdapter: Cannot initialize GSP firmware RM` (`GspStatusQueueInit: msgqRxLink failed`, `WPR2 already up`), with the device logging `HOST-ABI REFUSED ... host driver 595.91.07 is outside the interval` for host controls. Measured once (cup2 FAIL in a smoke run; `boot_capture` log). I staged a qcow2 overlay `/workspace/bench/guest-595.84.qcow2` with the repo's `scripts/drivermatrix/stage_fat_guest.sh 595.84` (the only 595 `.run` on the host is `/root/converge/nvidia/NVIDIA-Linux-x86_64-595.84.run`; there is no 595.91.07 `.run`), so **the fat guest runs driver 595.84 against host 595.91.07, not 580.159.04** as in the 61-app run. Same image for both ends; absolute numbers are not those of the vast runs.
- **App matrix (`scripts/apps/apps_matrix.sh`): not run.** Needs `/opt/apps/{bundle,venv,data}` on the host and in the guest image; the host has no `/opt/apps`, there is no apps receipt in `/workspace/bench`, and provisioning downloads several GB (`provision_guest_apps.sh`).
- **`llm_parity.sh`: not run.** The host has no `torch` (`python3 -c "import torch"` fails), no `LLM_ROOT`/model, and the guest image has no `llm_lane.receipt`. No tok/s baseline exists for this host.
- **`gfx_suite.sh`: not run.** `/workspace/bench/gfx_lane.receipt` is absent (`provision_guest_gfx.sh` has not run, host or guest). **`video_lane.sh`: not run**, same class (no provisioning receipt; not provisioned).
- **`dbfast_lane.sh` as such: not used.** It hard-codes `$BENCH/kf3-bins/$REV` and the checkout's own scripts; `perf_compare.sh` runs its `exit` step directly. Its `launch` (ON/spin variants of cup8bench), `gpufree` (KVM-less cargo benchmarks) and thin-suite-ON steps are not in the baseline.
- **Kernel-bandwidth rows (`KAYFABE_BENCH_BW`) not enabled**: documented allocation-cliff history, pre-v3 only; H2D/D2H GB/s are derived from the BSUM lines instead (2 x 16 MiB and 16 MiB transfers, small).
- **No pre-Windows baseline (`906a76a4`)**: refuses this host's driver, section 1.
- **Not measured, so no claim:** the drainer/act-thread stall metrics (`V3_NONSTALL_THREADS.md` does not exist), CPU use of the host-side threads, display and broker paths, anything with more than one VM.
- Harness note: `boot_nvkvm.sh` echoes `(rev c6fff2e3)` - that is the harness checkout, not the binary; the binary's revision is the `BINARY kf3-bin-rev:6692e621` line of `boot_capture` (in every `raw/*driver.log`).
- Noise: no other tenant was present in any rep of this run. The owner's Windows jobs (which hold the same lock) ran during an earlier throw-away test, not during the baseline. Rerun if `raw/noise.log` of the AFTER run shows `qemu` greater than 0 or load above ~2.
