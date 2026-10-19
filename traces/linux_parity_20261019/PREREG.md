# Linux guest parity preregistration

STATUS: RESEARCH, 2026-10-19. Lane LX1. Started 20:45:32 UTC;
hard stop 2026-10-20 04:45:32 UTC (eight hours). No LX1 GPU workload has run.
Labels: MEASURED = an observation made by this lane; CODE READ = source or a
named inherited report; INFERRED = hypothesis or interpretation. Planned values
below are not observations. No product/default change or merge is planned.

## Questions, falsifiers written before runs

1. INFERRED H1: Linux tiny-job latency has the same large relative penalty as
   Windows. FALSIFIER: five matched Linux boot blocks put A/C and B/C latency
   ratios within the measured VFIO spread, or substantially below the supplied
   Windows 266/61 ratio. The Windows ratio is a question, not a Linux prediction
   with a statistical confidence interval.
2. INFERRED H2: sharing a host TSG improves Linux compute or graphics. FALSIFIER:
   paired B-A differences overlap zero/run spread, or any correctness regression.
3. INFERRED H3: the approximately 100 ms hitch class also appears on Linux.
   FALSIFIER: all five ten-minute mixed runs per arm have no >100 ms measured
   frame gap. A finite negative result bounds this experiment, not all Linux use.
4. INFERRED H4: clocks/P-state/power explain a bad ratio. FALSIFIER: a gap persists
   in matched clock/state bins without power/throttle differences. Correlation
   alone cannot establish causation; no clock/power-limit change in primary arms.
5. INFERRED H5: extra submit-to-ISR/fence latency explains a bad tiny-job ratio.
   FALSIFIER: separately instrumented matched GPU timestamps and OGKM events show
   equivalent intervals while the CPU round trip remains worse. Timestamp domains
   must be calibrated; CPU marker-to-IRQ is not a GPU fetch timestamp.
6. INFERRED H6: a provisionable same-disk Linux baseline can be assembled from
   existing images/cache. FALSIFIER: exact driver/source/kernel dependencies or
   required workloads cannot be obtained and verified, the image fails stock
   boot, or its manifest changes between arms. Missing tools are UNRUN, not a
   zero score. Never relax a correctness or ownership guard to obtain numbers.
7. INFERRED H7: mirror preserves Linux results. FALSIFIER: wrong bytes/hash,
   unexpected refusal, hang, missing completion, new host/guest Xid, or regression
   versus matched A and mirror-OFF controls. Stop at an unexplained failure.

## Arms and run accounting

| Arm | Immutable product source | Runtime profile |
|---|---|---|
| A | f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e | candidate-5 production; no KF3 measurement/diagnostic variables |
| B | 0d7ba74389cd85ac65826417ad53f5c0bcd1ef2f | same profile plus KF3_TSG_MIRROR=1 only |
| C | A's QEMU executable, physical RTX 4070 via vfio-pci | same CPU/RAM/disk/network/firmware; GPU device differs |
| D, correctness only | B's executable | KF3_TSG_MIRROR=0; one complete Linux gate versus A |

CODE READ: inherited M1 hashes are MD5 c2ff1a02ed33a39e4b26d2dd002e62f6
(A) and 48ab1b70c2cf2c26d2c1287bf7311a9a (B). Recheck installed REV and
hash, record SHA256, and provide pc-bios/qemu-bundle links to the matching QEMU
build resources. Directory names alone are insufficient provenance.

Planned: five independent guest boots per arm, with all workloads in a fixed
order inside each boot. Order blocks: ABC, BCA, CAB, ACB, CBA. Each arm occurs
five times; fresh guest boot between arms avoids carrying driver/TSG state.
If time allows, add BAC as block six for complete position balance; never reduce
n below five or count repetitions within one process as independent boots.
Keep warmup separate; include all completed runs and preserve failed/partial rows.
Driver 580 is a second matrix, never pooled with matching-driver results.

Each row records UTC start/end, host boot id/kernel, GPU PCI identity/module
flavour, host/guest driver and OGKM source/build hashes, guest kernel, image and
tool/model/clip manifest hashes, binary revision/hash, actual environment and
argv, order block, n, median, min/max and IQR, correctness and Xid before/after.
Report raw kayfabe/VFIO metric ratios, naming latency's lower-is-better direction.
Also report paired ratios by block. A missing arm means ratio UNMEASURED.
"Within spread" means overlapping run min/max and a paired bootstrap interval
including 1; this is descriptive at n=5, not proof of equivalence.

## Workloads and metrics (n >= 5 independent boots per arm per row)

| Workload | Fixed recipe and primary metrics |
|---|---|
| W1 tiny kernel | Fixed empty kernel, 1,000 warmups then 10,000 launch+sync iterations; separate spin/polled and blocking scheduling modes; CPU monotonic wall median/p99/p99.9 us, checked completion marker; empty-kernel launch throughput kernels/s |
| W1 event/graph | Event record+sync and instantiated CUDA graph launch+sync, same iteration counts; us/round trip and launches/s; report CPU and CUDA-event time separately |
| W1 copies | Pageable and pinned H2D/D2H, D2D, powers of two 4 KiB through 1 GiB; 10 warmups, >=20 timed copies each; GB/s and us, full deterministic byte check; stay within 8 GiB usable allocation on all arms |
| W1 allocation | cudaMallocAsync/FreeAsync, fixed 4 KiB and 64 MiB, pool policy pinned; ops/s and synchronized batch us; distinguish warm pool from cold allocation |
| W1 managed | CPU first-touch, GPU first-touch and prefetch controls over fixed 64 MiB; elapsed us/page and correct data. Unsupported must return an explicit error; never print a successful parity value for an unsupported fault plane |
| W2 GEMM | cuBLAS FP32 and FP16, fixed 4096 matrices, alpha/beta, math mode and seed; correctness tolerance fixed before run; warmed CUDA-event TFLOPS plus CPU wall |
| W2 bandwidth | Fixed 256 MiB copy/triad kernel, verified output, device-event and wall GB/s |
| W2 LLM | Existing llm_parity.sh Qwen2-0.5B-Instruct cached snapshot, greedy seed 1, fixed prompt, 512 generated tokens, one cold + one warm; TTFT and steady tok/s, exact token ids/hash. Alternatively cached llama.cpp + one pinned small GGUF, identical binary/options/model hash on every arm; report as a separate row |
| W3 graphics | vkmark all scenes, glmark2, offline Heaven/Valley/Superposition, GravityMark Vulkan if available. Fixed 1920x1080 settings, no vsync, same guest offscreen path where supported; frame-completion timestamps, avg fps, gap p99/p99.9-derived 1%/0.1% low fps, >33/>50/>100 ms counts/min, maximum gap |
| W4 video | Identical ffmpeg bytes + deterministic 1280x720 90-frame video_lane.sh content gate; extended fixed 1080p clip, H.264/HEVC NVENC preset/rate-control fixed, NVDEC CUDA frames with hwdownload preventing software fallback; fps, wall time, stream/frame hashes, explicit GPU-copy throughput |
| W5 mixed | Ten minutes per boot, fixed alternating compute/graphics loop; same frame-gap metrics, host/guest kernel messages, failures and per-second clocks/pressure. n=5 ten-minute runs per arm; no stall-free claim from a shorter run |

Planned W3 path: guest offscreen rendering on both arms, with Xvfb only as an
X protocol endpoint if needed, and NVIDIA renderer/ICD verified. Do not count a
software renderer. A benchmark without an offscreen mode gets a separate matched
guest Xorg/fullscreen cell, not a mixed-path comparison. Broker presentation and
new-content/repeated-frame rates are a separate kayfabe display diagnostic;
offscreen throughput cannot answer broker new-content parity. Averages alone do
not satisfy W3 or W5: absent per-frame instrumentation means partial coverage.

## Provisioning plan

CODE READ: M1 inspected 62 candidates and found gcc/libcuda/modules but no
documented graphics source/build bundle, CUDA app bundle or LLM venv. The
existing graphics provisioner boots stock QEMU, refuses concurrent QEMU and
fetches distro packages/VirtualGL; it must not run while the owner holds the GPU
lease. LX1 provisions at the beginning of Part B after graceful owner shutdown.

Planned seed: a powered-off copy/overlay of the existing noble fat guest, pinned
kernel 6.8.0-142-generic (verify in guest, do not assume from inventory). Create
LX1's child and copy the benchmark guest SSH key privately into the lane host
directory. Never modify a source image or another lane's overlay. Provision in
stock QEMU under the global lock; freeze/hash the resulting seed. Primary test
overlay is the SAME file for A/B/C with clean shutdown between boots, same
kernel, driver, userspace and firmware. Record mutable filesystem changes;
freeze workload/tool manifests and disable automatic package/kernel upgrades.

First guest driver: 595.91.07, matching measured host open module. Use its
matching public open-kernel-module source and full userspace installer/libs
(CUDA driver, NVML, Vulkan/EGL/GLX, NVENC/NVDEC, GSP firmware); build against the
guest's exact installed headers, record source tag/hash, compiler, vermagic and
module SHA256. Unmodified open source release build first; a debug/ftrace build
is a separate diagnostic arm. Second: pinned 580.159.04 (or an explicitly
recorded obtainable 580.x); never combine releases' module and userspace.

Offline route: inventory existing installer/source packages, CUDA headers/libs,
ffmpeg and cached models; transfer only lane-local copies into the guest. Use a
lane-local HTTP/package spool from already authenticated distro packages if
guest internet is absent. Host package cache must match the guest distro: the
host currently has a newer Ubuntu userspace, so blindly installing host .debs
into noble is forbidden. If noble packages are absent, download with noble apt
inside the stock guest (or a noble root with signed Release/Packages verification)
and retain the cache in the lane directory. Never rewrite guest apt suites or
install unrelated host driver packages as a workaround. Build supplied gfx
sources, CUDA probes and any upstream benchmark from pinned public sources;
record exact receipts. Optional unavailable proprietary benchmark/model is
UNRUN with its reason. Check guest root disk/free RAM before each install.

## Hardware and restore gates

Planned sequence: record actual owner launcher/process/flags and log; write the
authorised stop flag; wait OWNER_SESSION_EXIT and QEMU/broker disappearance.
Acquire flock -o on the shared fastguest lock with a bounded wait, never break it.
Hold it through provisioning, controls, gates and parity. No nested attempt to
acquire that same lock; inherited harnesses need a private inner lock while the
shared outer lease is demonstrably held. Do not run their broad PID killers.

Before parity: native content controls; mirror Linux CUDA ladder/app/multistream,
Vulkan hash/Xid-69 shape, video and broker/interactive retest, plus A and D
controls. Preserve every unrun gate. Failure stops performance work, without
inventing a product fix; a fix needs its own EXPECTED RED then green and >=2
killing tests per mutant.

VFIO plan: inventory exact RTX4070 PCI functions and complete IOMMU group;
verify AMD drives the host desktop and no QEMU/broker/NVIDIA clients remain.
Bind only those GPU functions through driver_override to vfio-pci; record the
original drivers. No global NVIDIA module unload, no other PID kill. Restore
all recorded drivers in finally after own QEMU exit. Do not rebind/reset under a
live QEMU; failed guest shutdown retains the lock and is reported. Use clean
guest poweroff between arms, observe reset_method and post-rebind health; no
unproven reset guarantee and no firmware change.

Host sampler: one sample/s, UTC realtime_ns plus monotonic_ns/boottime and
sample start/end, SM/memory clocks, P-state, power, utilisation, PCIe generation/
width and throttle reasons; pressure cpu/memory/io; QEMU per-vCPU schedstat
runtime and wait, bound to PID/starttime. In VFIO NVML is unavailable on the
host: explicitly mark unavailable and collect equivalent nvidia-smi in the
guest. Calibrate host/guest clocks with >=10 bracketed SSH timestamp exchanges,
retain minimum-RTT offset/error bound; never subtract uncalibrated monotonic
clocks. Extra tracing is a separate diagnostic, not A's primary measurement.

At exit: verify NVIDIA binding and Xid journal before/after (ring wrap is not
zero); release the lane lease only once own VM exits. Restore the exact observed
perf6 owner profile using a byte-identical launcher copy, next unused run,
0d7ba743 and KF3_TSG_MIRROR=0; wait OWNER_SESSION_READY and verify actual process
exe/env. One launcher repair/retry allowed, no force termination. Stop/report
on any guard refusal. No rental, other-lane writes, master merge or force push.

## Prior art and limits

CODE READ: cuda_ladder.sh supplies four graded rungs; gfx_suite.sh compares five
steps and >=10 hashes; video_lane.sh distinguishes CUDA decode from fallback;
llm_parity.sh separates warm/cold/graph timings and binds counters to process
identity. dbfast_lane.sh is a different experiment and supplies no parity result.
fastguest scripts supply thin 30-arm coverage, not fat-guest workloads.
The requested submit_latency directory is absent in both available candidate-5
and mirror checkouts; Windows latency reports are inherited evidence only.
The 2026-10-16 parity report stopped on a Linux gate failure: do not reuse its
performance values or unfinished implementation as this lane's baseline.

## Hardware pause and GPU-free follow-up, 2026-10-19 21:08 UTC

MEASURED: the coordinator requested hardware release for the owner's Minecraft
session. LX1 stopped its restored run 3019 gracefully (EXIT=0), verified no own
QEMU/broker/lease and NVIDIA binding, and wrote the requested PAUSED marker.
Latest instruction supersedes automatic owner restoration: do not start/stop an
owner VM or acquire the GPU lock until the coordinator schedules the next slot.
GPU-free preparation continues; the eight-hour wall deadline remains unchanged.

INFERRED H8, before offline inspection: damaged/inconsistent installed guest
userspace may explain the CPU-only provisioning panic. FALSIFIER: bounded
read-only inspection finds the implicated systemd, libc, sudo and apt files
consistent with their recorded package manifests and no relevant I/O/package
errors. That would weaken a file-damage explanation, not prove CPU/KVM correctness.
No guest binary will be executed on the host. Inspection uses only LX1's stopped
private disk, read-only/no journal replay, and promptly releases its own NBD.
No provisioning VM, GPU operation or shared GPU lock is allowed during the pause.

INFERRED H9, before the new guard tests: an idempotent VFIO restore can leave
driver_override stale when the original driver is already bound. FALSIFIER:
fake-sysfs tests show both GPU and audio overrides cleared in that case.
This concerns unexecuted host tooling; no VFIO transition has occurred.
