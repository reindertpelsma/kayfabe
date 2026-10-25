# Linux kayfabe/native parity — LX2

STATUS: RESEARCH, 2026-10-25/26. Registered before any LX2 GPU execution.

## Authority and scope

Owner's 2026-10-26 native-reference addendum replaces the original VFIO arm: N is the host itself, K is the matching host-mode Linux guest, RTX 4070, kernel 7.0.0-34-generic and NVIDIA open driver/userspace 595.91.07. No GPU detach, VFIO launcher or owner-VM shutdown is authorized by this revision. Candidate source is 6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4; the existing binary's identity is verified privately before launch. Mirror is default ON; optional O changes only KF3_TSG_MIRROR=0. Raw evidence and host paths remain in the private lane report. No Windows vendor scores are published.

## Hypotheses and falsifiers (INFERRED predictions)

H1: Linux has a material shared-stack throughput deficit. Falsifier: each tested workload has a paired K/N throughput interval containing 1 with all arm medians within 5% and overlapping run ranges. This bounds the tested workloads only; it cannot prove every Linux application is at parity or isolate Windows causality.

H2: the memory/surface workloads show a larger deficit than compute. Falsifier: their normalized K/N ratios are no lower than compute outside run spread. A surface deficit alone does not establish missing compression: scheduling, placement and page size remain competing explanations.

H3: a slow frame consumes additional GPU execution time. Falsifier: GPU timer time is unchanged within run spread while CPU completion time increases, or additional GPU-active time is accounted by host walker/other clients. Host NVML utilization is aggregate, not engine-specific evidence.

H4: Linux shows a recurring long-gap class. Falsifier: no completion gap exceeds 100 ms in the registered steady windows; report sample duration and counts, never extrapolate absence to long sessions.

## Arms, replication and selection

Build/stage once on the host. Exact same workload executables, PTX/shaders, inputs, matching userspace libraries, kernel and modules are used in N and K. Hash all inputs and record resolved dependencies. Use a fixed offscreen EGL device path on NVIDIA in both arms: no desktop compositor or broker. GDM is stopped under the lifetime shared GPU lease, then restored to its captured initial state. Refuse any preexisting VM or competing GPU workload; never stop anyone else's QEMU. No device reboot or application result is assumed to be a correctness gate.

Primary plan: six matched N/K pairs with order N,K / K,N / N,K / K,N / N,K / K,N (six fresh K boots). The first N is also the bare-metal correctness prerequisite. Pair order is fixed before data. One same-configuration setup attempt and one launcher repair/retry are allowed before accepted runs; preserve attempts and exclusions. No post-result sample substitution. Each accepted run has new process state, at least 20 warmup iterations for graphics and first CUDA iteration (i=0) excluded from each 100-iteration row; shader caches disabled identically. Native and guest get the same eight-CPU affinity set, K uses eight vCPUs and 8 GiB RAM, fixed GPU store 8 GiB. Host scheduling/virtual CPU overhead and native/guest memory size remain disclosed limitations. Do not change clocks, power caps, persistence mode, polling constants or driver settings to improve a result.

Each workload requires n >= 5 per arm for a parity claim; target n=6. Every row reports n and run medians, min/max and IQR. Throughput ratio is K/N (higher is better); duration ratio is separately K/N (higher is slower). Report paired ratio median/range and deterministic paired bootstrap 95% interval (seed 20261025, 10000 resamples). Bootstrap units are complete paired runs, not individual frames. Primary equivalence band is [0.95,1.05]; a non-overlapping run range outside that band is a deficit even if small n makes inference imprecise. Otherwise label indeterminate; never equate insufficient evidence with parity.

## Workload table

| Priority | Workload | Metric and sample | Correctness and scope |
|---|---|---|---|
| 1 | Offscreen compressible RGBA8 render target + depth, then sampled color pass, 1920x1080, fixed 64 layers | >=1000 frames/run, CPU completion interval and GPU per-pass timestamps; fps, median/p90/p99, max, fraction >33/50/100 ms and >2x own median | Check FBO completeness, NVIDIA renderer, GL errors and deterministic color/depth/output readback; no scanout/present-fps claim |
| 1 | CUDA existing cup8bench, N=16 and N=2048, solo and batch | 100 solo iterations, 10 launches/batch, raw launch/sync/total times; iterations/s and effective matrix GFLOP/s | Existing poison-before-launch and full matrix verification; this is a naive GEMM, not cuBLAS TFLOPS |
| 1 | Existing CUDA streaming-read kernel, 16/64/256 MiB, one full traversal per iteration | 100 iterations/size, GB/s, per-iteration completion distributions | Existing poison/output verifier; do not call cached small-buffer results DRAM bandwidth |
| 2 | INFERRED plan (unmeasured), CUDA micro extension: polled/blocking tiny launch, events, empty-kernel throughput, copies 4 KiB..1 GiB, graph, async alloc/free, UVM fault cost | Separate named rows, >=5 runs and per-iteration times | Only use available verified executable/API coverage; unsupported rows UNRUN, no substitution |
| 2 | cuBLAS fp32/fp16 GEMM | TFLOPS, >=5 runs | Only if matching cached library available; naive GEMM cannot satisfy this row |
| 2 | Fixed offline LLM via scripts/bench/llm_parity.sh and run_llm.py | Decode tokens/s, TTFT, per-token timeline; same cached model/revision, seed and input, 512 tokens | Cache-only, no model download; matched interpreter/dependency environment; unavailable assets => UNRUN |
| 2 | vkmark all scenes; glmark2; existing gfx suite | Scene fps and frame distributions where available, hashes for gfx suite | Average-only tools cannot satisfy frame-tail requirement; no claims for unavailable app assets |
| 2 | NVENC H.264/HEVC, NVDEC via video_lane.sh | Frames/s on identical clip and ffmpeg, >=5 runs | Software/hardware decoded content hashes and explicit CUDA hwdownload required, no silent CPU fallback |
| 3 | 10-minute mixed graphics/compute loop | Completion-gap distributions, error journal | Last, only if primary pairs complete with sufficient time for teardown |

## Sampling, clocks and stop rules

Host sampler runs nvidia-smi every 100 ms in both arms: utilization.gpu, utilization.memory, clocks.sm, clocks.mem, clocks.gr, P-state, power, temperature, throttle reasons, memory use and PCIe link state. Capture query start/end using host CLOCK_REALTIME and CLOCK_MONOTONIC; actual cadence/query duration are reported. NVML's internal utilization averaging can be longer than 100 ms: ten queries are not ten independent hardware windows. Once/second capture pressure, CPU utilization, and identified QEMU per-vCPU/thread runtime. Retain output gaps and unsupported query fields explicitly.

INFERRED clock plan (unmeasured): all application records carry CLOCK_MONOTONIC and CLOCK_REALTIME origins. Guest serial clock probes are bounded by host send/receive timestamps before and after each workload (midpoint estimate plus half-roundtrip uncertainty); no hardcoded cross-boot offset. For iteration distributions use only local monotonic durations. Map guest workload windows onto host sample times with the measured clock bracket; reject GPU attribution if uncertainty exceeds 100 ms or clock drift exceeds 10 ms. Frames are recorded to memory and emitted after the timed window to avoid per-frame serial I/O cost. Steady window excludes only the preregistered warmups.

Capture host journal cursor before GPU activity and poll new records. Any new Xid, OOM, GPU/application instability, failed content check or named safety refusal stops the campaign: preserve exact logs, request only owned VM QMP powerdown, restore GDM after owned QEMU exits, and report. No Xid classification exception is inherited from another lane. No kill of anyone else's process. Timebox six hours from lane start; reserve teardown time. Lock held by correctness/PF1 campaigns is probed nonblocking at >=5 minute intervals; never break a lease or auto-stop a VM. GPU-free staging and host compilation may proceed without the GPU lease.

## Phase 2, conditional and separate from parity data

Only after valid paired ratios, select the largest beyond-spread deficit; if no such deficit is established, stop attribution and state the limits. Same tracer and exact workload/image on both sides. Collect strace -f -yy -e trace=ioctl -s 0, decode RM allocation/channel/TSG/ctxshare/VA/map attributes from matching open 595.91.07 headers. Plain strace prints pointed-to payload addresses for unknown ioctls and cannot recover those bytes retrospectively: mark undecoded fields, use a reviewed open-header logging method or matching guest ogkm debug build rather than inventing values. CUDA per-kernel timing needs Nsight/CUPTI or an explicit paired event-timed probe; graphics per-pass timestamps use the API's query mechanism. GL timer queries in the primary GL probe are not Vulkan timestamp-query evidence.

INFERRED diagnostic plan (unmeasured): use identical debug ogkm build/trace settings in native and guest only as a separate diagnostic arm; preserve baseline system/driver and restore afterwards. Do not replace the live host module while another VM holds the lease. Log kind/page-size/comptag per allocation where the matching source exposes it. Add host KF3_DIAG_KINDSTAT only if the PF1 implementation is available and reviewed; that uses a distinct binary/measurement arm, never silently replaces the pinned primary binary. Report first observed divergence, its source and competing hypotheses with falsifiers. No product behavior changes are in scope.
