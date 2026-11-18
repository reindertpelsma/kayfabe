# V3 GPU-PV gap matrix: what a GPU paravirtualisation stack leaves behind, and where kayfabe stands

**STATUS: RESEARCH, 2026-11-18** (first version; every row says whether it is MEASURED, cited or INFERRED; no probe of this document has run on a GPU yet, section 3.4 says so again where hardware results would go).

**Summary (public, counts per kayfabe status): 132 rows: WORKS 20, BROKEN 11, UNKNOWN 94, N-A-BY-DESIGN 7.** **Target: BROKEN = 0. Patch-tier rows are BROKEN until the optional patch tier ships.** **Closure path of the BROKEN and UNKNOWN rows: IN-TREE FIX 88, PATCH TIER 4, OWNER DECISION 13 (and 7 N-A-BY-DESIGN rows, which close by their stated reason).** (IN-TREE FIX = kayfabe code; PATCH TIER = needs a host and/or guest kernel or driver patch, optional and later, section 6; OWNER DECISION = a policy; for an UNKNOWN row the path is the one that applies if a run fails.) UNKNOWN is the largest class on purpose: most rows name a probe that is authored and not yet run, and a row only leaves UNKNOWN when a run with a revision exists. The aim of the matrix is to find failures: a pass is hygiene, a failure is a result.

Owner goal (2026-11-17/18): kayfabe emulates full NVIDIA; its first target is the set of gaps that GPU paravirtualisation (GPU-PV) leaves behind. Every failing probe ends in exactly one of four buckets, and a difference is never called a kayfabe bug without a host control (`docs/OWNER_RULINGS.md` working rules):

1. a documented kayfabe limitation (named in a ruling, a design document or the README);
2. a kayfabe gap to close (a core defect: the GPU-free regression test is written FIRST, then the fix);
3. an application or harness bug confirmed by a control (the same case fails on the host too, or on bare metal);
4. our own configuration error.

"Not applicable by design" is a status, never a bucket, and every such row carries its reason in brackets.

## 1. How to read this document

- **Status.** WORKS: a run with a revision shows guest equals host for the row (named in the evidence column). BROKEN: a run shows a failure, with its bucket in the evidence (most are bucket 1). UNKNOWN: no run. N-A-BY-DESIGN: cannot or must not apply, reason given.
- **Source column.** A key into section 7. `R` = the page was fetched and read on 2026-11-18 (some pages only in part, said in the key). `P` = read in an earlier pass of this project, not re-read today. `I` = INFERRED: the author's reading of how the feature works, with no source; the next-step column says what would settle it. Nothing in a row says more about any third party than its cited page does.
- **Evidence column.** MEASURED = a run named here (trace directory or document, revision, date). A probe name with "authored" is a program that exists in `scripts/apps/gap_probes` and has run on no GPU.
- **Closure path column.** Every BROKEN and UNKNOWN row names how it closes: IN-TREE FIX, PATCH TIER, OWNER DECISION, or N-A-BY-DESIGN with its reason. The project's end state is BROKEN = 0 and this matrix is the tracker.
- **Not duplicated.** `docs/design/V3_FEATURE_GAPS.md` ranks what kayfabe lacks inside the display, video, memory and channel code (rows G01 to G17, L01 to L14); this matrix is organised by what users meet, and links those rows where they decide a status.

## 2. The matrix

Rows are grouped by area. Columns: id, feature, source, kayfabe status, evidence, next step, closure path.

### 2.0 CUDA memory, sharing, graphs, pools, launch and fault behaviour

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| CU-01 | CUDA IPC, legacy memory handles (cuIpcGetMemHandle/OpenMemHandle) between two guest processes | I | UNKNOWN | none; probe ipc_legacy authored | run ipc_legacy on host and guest | IN-TREE FIX |
| CU-02 | CUDA IPC events | I | UNKNOWN | none; probe ipc_event authored | run ipc_event | IN-TREE FIX |
| CU-03 | VMM export of a physical allocation as a POSIX fd and import in a second process | S4 | UNKNOWN | none; probe ipc_vmm_fd authored | run ipc_vmm_fd | IN-TREE FIX |
| CU-04 | Virtual memory management: cuMemCreate, cuMemMap, cuMemSetAccess, granularity | S4 | UNKNOWN | none; probes vmm_basic and vmm_granularity authored | run both | IN-TREE FIX |
| CU-05 | VMM remap: physical handles swapped under one VA range | S4 | UNKNOWN | none; probe vmm_remap authored | run vmm_remap | IN-TREE FIX |
| CU-06 | VMM compressible allocations (CU_MEM_ALLOCATION_COMP_GENERIC) | S4 | UNKNOWN | none | add a case to gap_probe.c (vmm_compress) | IN-TREE FIX |
| CU-07 | VMM fabric handles, IMEX channels, multicast objects | S4 | N-A(needs an NVLink/NVSwitch fabric; the target is one GeForce die per guest and NVLink is stubbed as absent by owner ruling H) | docs/OWNER_RULINGS.md section H | assert the attribute dump reports fabric handles unsupported (attr_dump) | N-A-BY-DESIGN |
| CU-08 | CUDA graphs: stream capture, instantiate, 1000 replays | I | UNKNOWN | none in the Linux app matrix; probes graph_capture_launch, graph_mem_nodes authored | run both | IN-TREE FIX |
| CU-09 | CUDA graph update (cuGraphExecUpdate) | I | UNKNOWN | none; probe graph_update authored | run graph_update | IN-TREE FIX |
| CU-10 | Conditional graph nodes, graph memory nodes, device-side graph launch | I | UNKNOWN | none | extend gap_probe.c after the first graph run | IN-TREE FIX |
| CU-11 | Stream-ordered allocator: cuMemAllocAsync/FreeAsync, release threshold, trim | I | UNKNOWN | none; probes pool_alloc_free, pool_threshold_trim authored | run both | IN-TREE FIX |
| CU-12 | Memory pool export as a shareable handle | I | UNKNOWN | none; probe pool_ipc_export authored | run pool_ipc_export | IN-TREE FIX |
| CU-13 | Managed memory with demand paging (a kernel touching a managed allocation) | S1 | BROKEN | MEASURED: the four UVM demand-paging apps (UnifiedMemoryStreams, UnifiedMemoryPerf, conjugateGradientUM, attach_verify) fail in the guest with one host Xid 31 each and pass on bare metal (kf3 144557e4, 2026-11-07, docs/design/V3_APP_MATRIX.md); documented limitation, not a release target (README.md, docs/OWNER_RULINGS.md section E) | owner decision; probes managed_* record the same failure per case | PATCH TIER (H3 host nvidia-uvm fault service plus the guest side; H4 pool; section 6) |
| CU-14 | Managed memory: prefetch, advise, read-mostly, accessed-by | S1 | UNKNOWN | none; probe managed_prefetch_advise authored; expected to hit the CU-13 limitation (INFERRED) | run it; a host pass and a guest failure is the listed limitation | PATCH TIER if the failure is a demand fault (INFERRED, H3 and H4), else IN-TREE FIX |
| CU-15 | Managed memory: concurrent CPU access during a kernel (concurrentManagedAccess) | S1 | UNKNOWN | none; probe managed_concurrent authored; expected to hit the CU-13 limitation (INFERRED) | run it | PATCH TIER (H3 and H4: concurrent access is demand faulting) |
| CU-16 | Managed memory oversubscription (more than the device holds) | S1 | UNKNOWN | none; probe managed_oversub authored (oversubscribes only where the guest frame buffer is below 8 GiB) | run it with fb-mb 4096 | PATCH TIER (H3 and H4: oversubscription is demand faulting) |
| CU-17 | Pinned host memory, 1 GiB and 4 GiB | S1 | UNKNOWN | none; probes pinned_1g, pinned_4g authored | run both | IN-TREE FIX |
| CU-18 | Host registration (cuMemHostRegister) and mapped zero-copy kernels | I | UNKNOWN | none; probes host_register_512m, mapped_zerocopy authored | run both | IN-TREE FIX |
| CU-19 | A guest request for an absurd pinned allocation is refused promptly (hostile guest rule: no unbounded guest-reachable resource) | I | UNKNOWN | none; probe pinned_huge_refused authored | run it | IN-TREE FIX |
| CU-20 | Cooperative kernel launch (grid-wide synchronisation) | I | UNKNOWN | none | DESIGN-ONLY: author a coop case (occupancy-sized grid, atomic barrier) | IN-TREE FIX |
| CU-21 | Thread block clusters (sm_90 and newer) | I | UNKNOWN | none; needs a Hopper or Blackwell die | DESIGN-ONLY: gate on the cluster-launch attribute (attr_dump) | IN-TREE FIX if a run on a Hopper or Blackwell die fails |
| CU-22 | Peer access and peer-to-peer copies | I | N-A(one die per guest: peer PTEs are refused by name and the P2P capabilities answer NOT_SUPPORTED; the guest must answer cudaDeviceCanAccessPeer with 0, which attr_dump records) | docs/design/V3_FEATURE_GAPS.md row L10 | check the clean answer in attr_dump | N-A-BY-DESIGN |
| CU-23 | NVRTC and nvJitLink compilation in the guest | I | UNKNOWN | none; driver PTX JIT of fresh modules is probe churn_module_200 | author an NVRTC case once the toolkit is on the box | IN-TREE FIX |
| CU-24 | PTX JIT of 200 fresh modules with the cache disabled | I | UNKNOWN | none; probe churn_module_200 authored | run it | IN-TREE FIX |
| CU-25 | Per-boot CUDA process budget | I | WORKS | MEASURED: 100 CUDA processes in one boot, 100 of 100 (docs/design/V3_APP_MATRIX.md row m20seq); the earlier host NoMemory at the 40th (no persistence mode) or 60th process, cause J, is gone | re-measure with fanout_procs_24 and churn_ctx_200 (concurrent, not sequential) | - |
| CU-26 | 24 simultaneous CUDA processes each running a kernel | I | UNKNOWN | none; probe fanout_procs_24 authored | run it | IN-TREE FIX |
| CU-27 | 2048 streams and 4096 events in one process | I | UNKNOWN | none; probe churn_streams_events authored | run it | IN-TREE FIX |
| CU-28 | Kernel running 5 s and 30 s (no watchdog the host does not apply) | I | UNKNOWN | none; probes spin_5s, spin_30s authored | run both | IN-TREE FIX |
| CU-29 | Illegal-address and trap faults are reported, and a fresh context works afterwards | I | UNKNOWN | none; probes fault_recovery, trap_recovery authored | run both; a host Xid 13 or 31 on the host control is expected and is not a finding | IN-TREE FIX |
| CU-30 | 100000 back-to-back launches | I | UNKNOWN | none; probe launch_flood_100k authored | run it | IN-TREE FIX |
| CU-31 | CUDA attribute parity (all attributes, memory total) | I | UNKNOWN | none; probe attr_dump authored; memory total and PCI ids are listed known differences | run it | IN-TREE FIX |

### 2.2 Tools, profiling, NVML and nvidia-smi

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| TL-01 | MPS (nvidia-cuda-mps-control daemon, two clients) | S5 | UNKNOWN | none | DESIGN-ONLY: start the daemon with a private pipe directory and run two canary clients | IN-TREE FIX |
| TL-02 | compute-sanitizer: memcheck, racecheck, synccheck, initcheck | S1 | UNKNOWN | none; probe tools_sanitizer authored | run it with the tools from provision_tools.sh | IN-TREE FIX |
| TL-03 | CUPTI callback API | I | UNKNOWN | none; probe cupti_callback authored | run it | IN-TREE FIX |
| TL-04 | CUPTI activity API (kernel records) | I | UNKNOWN | none; probe cupti_activity authored | run it | IN-TREE FIX |
| TL-05 | Nsight Compute kernel replay with performance counters | S6, S2 | N-A(profiler interface stubbed as capability absent by owner ruling H; hardware performance counters are a privileged RM profiler interface and the hostile-guest rule forbids serving it; the probe still records the exact answer) | docs/OWNER_RULINGS.md section H; probe tools_ncu authored | run tools_ncu: a clean not-supported answer is the expected pass | N-A-BY-DESIGN |
| TL-06 | Nsight Systems CUDA trace | I | UNKNOWN | none; probe tools_nsys authored | run it | IN-TREE FIX |
| TL-07 | Nsight Systems GPU metrics sampling | S6 | UNKNOWN | none; probe tools_nsys_gpumetrics authored | run it | IN-TREE FIX |
| TL-08 | cuda-gdb and the debugger interface | I | N-A(debugger stubbed as capability absent by owner ruling H) | docs/OWNER_RULINGS.md section H | none | N-A-BY-DESIGN |
| TL-09 | NVML field coverage in the guest | S1 | BROKEN | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): 132 NVML and 74 DCGM telemetry values (clocks, power, temperature, fan, PCIe throughput, utilisation, throttle reasons) answer N/A in the guest; documented limitation (docs/design/V3_UTILIZATION_FLAG.md section 10) | none until the owner wants the counters served | OWNER DECISION (serve the counters, docs/design/V3_UTILIZATION_FLAG.md) |
| TL-10 | nvidia-smi queries and persistence mode | S11 | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): persistence mode on, DCGM software diagnostic passes on host and guest; memory.used and encoder session counters served | none | - |
| TL-11 | nvidia-smi setters from guest root: compute mode, power limit, clock lock, ECC, accounting | S11 | UNKNOWN | none; probe smi_setters authored, with a host-state watcher that fails when the host sees a change | run smi_setters with hoststate watch on the host | IN-TREE FIX |
| TL-12 | Driver reset (nvidia-smi -r) | S11 | UNKNOWN | none; probe life_reset authored (destructive, guest only) | run life_reset with --destructive in the guest | IN-TREE FIX |
| TL-13 | ECC mode and counters | S11 | N-A(owner ruling H stubs ECC as capability absent; consumer dies have none to report) | docs/OWNER_RULINGS.md section H | none | N-A-BY-DESIGN |
| TL-14 | MIG | S11 | N-A(MIG partitions a data-centre die; one guest owns one whole GeForce die) | none | none | N-A-BY-DESIGN |
| TL-15 | NVAPI queries (Windows) | P12 | UNKNOWN | none | DESIGN-ONLY: NvAPI_Initialize, EnumPhysicalGPUs, GPU_GetFullName, GPU_GetMemoryInfo compared with a native run | IN-TREE FIX |
| TL-16 | GPU clocks and power through nvidia-smi dmon | S11 | BROKEN | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): same 132 N/A fields as TL-09 | none | OWNER DECISION (same decision as TL-09) |

### 2.3 Interop

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| IO-01 | CUDA and OpenGL interop (cuGraphicsGLRegisterBuffer) | S1 | UNKNOWN | none | DESIGN-ONLY: EGL device platform, GL 4.6 context, register a buffer, write from CUDA, read in GL | IN-TREE FIX |
| IO-02 | CUDA and Vulkan external memory and external semaphores | S4 | UNKNOWN | none; the capability bits are in attr_dump and vk_dump | DESIGN-ONLY: export a Vulkan buffer as an fd, import in CUDA, signal a timeline semaphore | IN-TREE FIX |
| IO-03 | Vulkan external memory fd and dma-buf import between processes | I | WORKS | MEASURED: gfxset pv_dmabuf_import, pv_egl_dmabuf_export, pv_xiso_sharing, pv_signal_restart_export in the 38-item set, 38/38 in the guest at kf3 d06833f0 (traces/v3_gfxset, docs/design/V3_GFX_TESTSET.md) | none | - |

### 2.4 Vulkan, OpenGL and display (Linux guest)

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| GX-01 | Vulkan extension set equal to the host | I | WORKS | MEASURED: extension sets equal in the Vulkan CTS subset guest run (open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending)) | none | - |
| GX-02 | Vulkan features, limits, queue families and format properties equal to the host | I | UNKNOWN | none; probe vk_dump authored | run vk_dump | IN-TREE FIX |
| GX-03 | Vulkan ray tracing pipeline and ray query | P7 | UNKNOWN | gfxset item pv_vk_rt_ext exists but is in no stage | run the gfxset item on a box; read feature bits in vk_dump | IN-TREE FIX |
| GX-04 | Mesh and task shaders | I | UNKNOWN | none | add the CTS mesh groups to the full-suite list (section 4) | IN-TREE FIX |
| GX-05 | Device-generated commands (multiview indexed draw) | I | WORKS | MEASURED: DeviceLost with host Xid 13 before the fix, Pass after it (PR 480, host_pte_kind), confirmed on a second box 2026-11-18 | none | - |
| GX-06 | Sparse binding and residency (buffer and multisampled image residency) | S14 | BROKEN | MEASURED: Timeout and host Xid 109 on texel_buffers.sparse_residency size_2_24 and on the 4-sample residency image case; binding-only cases pass; mechanism NOT shown (docs/design/V3_VULKAN_CTS_ROW.md section 12) | root cause first, then a GPU-free regression test | IN-TREE FIX (root cause first; H6 or H7 only if the cause is the mapping granularity, INFERRED) |
| GX-07 | Vulkan video decode and encode queues | I | WORKS | MEASURED: the 43 video cases of the CTS subset agree host versus guest (open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending)) | none | - |
| GX-08 | Present modes, vsync, tearing, swapchain recreation | I | UNKNOWN | none | display lane: needs a window system in the guest | IN-TREE FIX |
| GX-09 | Variable refresh rate (VRR, G-SYNC) | I | UNKNOWN | none; the virtual monitor is set by kayfabe | decide whether the virtual EDID should advertise a VRR range | OWNER DECISION (virtual monitor VRR range) |
| GX-10 | HDR output (advanced colour, PQ) | I | BROKEN | documented limitation: no HDR claim for the console path (docs/design/V3_FEATURE_GAPS.md row L04) | owner decision, then hdr_query on Windows | OWNER DECISION (HDR claim) |
| GX-11 | Fullscreen exclusive versus flip-model presentation | I | UNKNOWN | none | DESIGN-ONLY (Windows) | IN-TREE FIX |
| GX-12 | Multi-monitor and display mode changes | I | UNKNOWN | none | DESIGN-ONLY: xrandr and modetest with a GL client alive (section 5) | IN-TREE FIX |
| GX-13 | Windowed-copy overhead of the console path | I | WORKS | MEASURED on Linux: the GPU-resident console path offered and imported, gpucopy 169 of 170 sent (docs/design/V3_FEATURE_GAPS.md row L05); overhead ratio not measured | measure frame cost against the host | - |
| GX-14 | OpenGL 4.6 core and compatibility basics, GLX and EGL | I | WORKS | MEASURED: gfxset gl_micro, pv_validate_gl, gl_info, glx_vgl, egl_render in the 38/38 guest run (traces/v3_gfxset) | none | - |
| GX-15 | OpenGL extension list and limits equal to the host, NV_* extensions | I | UNKNOWN | none | DESIGN-ONLY: dump GL_EXTENSIONS and GL_MAX_* through EGL on both sides | IN-TREE FIX |
| GX-16 | OpenGL context sharing across threads and processes | I | UNKNOWN | none | DESIGN-ONLY: two shared contexts, fence sync, compute shader | IN-TREE FIX |
| GX-17 | EGL, GBM and Wayland compositors (weston headless, sway) | S9 | WORKS | MEASURED: gfxset pv_gbm_egl_import, weston_headless, weston_client_diff, sway_screencap in the 38/38 guest run | none | - |
| GX-18 | PRIME render offload | S9 | UNKNOWN | none | DESIGN-ONLY: needs a second (software or virtual) GPU in the guest | OWNER DECISION (whether a second virtual GPU is offered to the guest) |
| GX-19 | Display-less and headless operation | S9 | WORKS | MEASURED: the whole gfxset and the Linux app matrix run headless | none | - |

### 2.5 Video engines

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| VD-01 | NVENC H.264 and HEVC 8-bit, 10-bit, 4:4:4, above 4096, 8K, B-frames and lookahead | S7 | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): guest equals host on every encode case the die supports (31 pass, 16 not supported incl. the AV1 encode the die lacks) | none | - |
| VD-02 | NVENC concurrent sessions | S7, S2 | BROKEN | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): 8 sessions on the host, 3 in the guest (kf-qemu ENC_SESSIONS_PER_VM = 3, a design choice recorded in the source; the vendor sheet lists 12 for consumer cards) | owner decision on the per-VM cap | OWNER DECISION (the per-VM NVENC session cap) |
| VD-03 | NVENC AV1 encode | S7 | UNKNOWN | none; the 3090 die has no AV1 encode | run the matrix on an Ada or Blackwell die | IN-TREE FIX if a run on an Ada or Blackwell die fails |
| VD-04 | NVENC 4:2:2 | S7 | UNKNOWN | none; vendor sheet lists it for Blackwell only | run the matrix on a Blackwell die | IN-TREE FIX if a run on a Blackwell die fails |
| VD-05 | NVDEC H.264, HEVC, VP9, AV1 including 10-bit, 4:4:4 and 8K | S7 | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): every decode case agrees host versus guest | none | - |
| VD-06 | Optical flow accelerator (Vulkan) | I | WORKS | MEASURED: gfxset vk_ofa in the 38/38 guest run | none | - |
| VD-07 | VA-API on NVDEC (nvidia-vaapi-driver) and VDPAU | I | UNKNOWN | none | DESIGN-ONLY | IN-TREE FIX |
| VD-08 | NVJPG and SEC2 engines | I | N-A(not advertised by design: docs/design/V3_VIDEO_ENGINES.md; no app needs the engine) | docs/design/V3_FEATURE_GAPS.md row L12 | none | N-A-BY-DESIGN |
| VD-09 | Screen capture plus NVENC record (OBS, Sunshine class) | I | UNKNOWN | none | DESIGN-ONLY: OBS CLI display capture with the encoder session counter as the GPU-use proof | IN-TREE FIX |

### 2.6 Linux stack, containers, libraries and frameworks

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| LX-01 | Container toolkit and CDI specs (nvidia-ctk cdi generate) in the guest | S10 | UNKNOWN | none | DESIGN-ONLY: generate the spec in a guest with the toolkit installed from the official repository, run a container with --device nvidia.com/gpu=all | IN-TREE FIX |
| LX-02 | nvidia-modprobe and /dev/nvidia* node creation for an unprivileged user | S9 | UNKNOWN | none | DESIGN-ONLY | IN-TREE FIX |
| LX-03 | nvidia-persistenced start, hold and stop with CUDA alive | S9 | UNKNOWN | none; probe life_persistenced authored | run it | IN-TREE FIX |
| LX-04 | OpenCL (clinfo, clpeak) and hashcat | I | WORKS | MEASURED: rows clinfo, clpeak, hashcat in the Linux app matrix (docs/design/V3_APP_MATRIX.md) | none | - |
| LX-05 | CUDA libraries and frameworks: cuBLAS, cuFFT, PyTorch, CuPy, llama.cpp | I | WORKS | MEASURED: rows sgemm_cublas, fft_cufft, torch_correct, torch_ai_bench, cupy, llama_cpp_gen in the Linux app matrix | none | - |
| LX-06 | vLLM, TensorRT and JAX rows | I | UNKNOWN | rows built (scripts/apps/mlrows), first guest run pending | run apps_discovery | IN-TREE FIX |
| LX-07 | Two dies per family (slot pool) | I | UNKNOWN | none in this batch | DESIGN-ONLY: two guests on two dies, then reboot one with a busy GPU and read the census | IN-TREE FIX |

### 2.7 Lifecycle and fault events

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| LF-01 | Unload and reload the nvidia modules in the guest (3 cycles), with and without a CUDA client alive | I | UNKNOWN | none; probe life_module_cycle authored (guest only) | run it; the expected result is a quick refusal under a live client and a clean reload | IN-TREE FIX |
| LF-02 | Kill a process in the middle of a kernel (5 rounds), next process must work | I | UNKNOWN | none; probe life_kill_kernel authored | run it on host and guest | IN-TREE FIX |
| LF-03 | Kill an EGL client in the middle of rendering (5 rounds) | I | UNKNOWN | none; probe life_kill_gl authored | run it on host and guest | IN-TREE FIX |
| LF-04 | Guest power-off with a busy GPU: the host frees the slot, VA pool and channels | I | UNKNOWN | none; probe life_busy_poweroff authored (terminal); the host census check is DESIGN-ONLY | run, then read the host census and boot the same device again | IN-TREE FIX |
| LF-05 | Guest reboot with a busy GPU | I | UNKNOWN | a plain reboot scrubs the store (docs/design/V3_DEVICE_RESET.md); a busy-GPU reboot is not measured | DESIGN-ONLY: spin client then systemctl reboot, canary after boot | IN-TREE FIX |
| LF-06 | VT switch and screen lock/unlock under a display manager with a GL client alive | I | UNKNOWN | none | DESIGN-ONLY: commands in section 5 | IN-TREE FIX |
| LF-07 | S3 suspend and resume, Linux guest | I | WORKS | MEASURED: three suspend/resume cycles in one boot at kf3 559db30c, RTX 4070, 2026-10-14 (traces/linux_s3_resume_20261014, docs/design/V3_S3_RESUME.md); the release image VMs set disable_s3=1 by default, so this is opt-in; not re-run since | re-run at the current revision before any claim | - |
| LF-08 | S4 hibernate, Linux guest | I | UNKNOWN | none; the release image VMs set disable_s4=1 | DESIGN-ONLY: enable S4 in a throw-away VM | IN-TREE FIX |
| LF-09 | Sleep and wake, Windows guest (S3) | I | BROKEN | MEASURED: 0 clean cycles in 7 attempts at kf3 0c5fedff, 2026-10-14 (traces/win11_s3_20261014, docs/design/V3_S3_RESUME.md section 7a); not re-run since | re-run on the current revision | IN-TREE FIX |
| LF-10 | Windows hibernate and resume, fast startup, modern standby | I | UNKNOWN | none | DESIGN-ONLY: exact commands in section 5 | IN-TREE FIX |
| LF-11 | Windows lock and unlock with a DXGI client alive | I | UNKNOWN | none | DESIGN-ONLY: section 5 | IN-TREE FIX |
| LF-12 | Driver refresh (Win+Ctrl+Shift+B) and pnputil disable/enable cycles | I | UNKNOWN | none | DESIGN-ONLY: section 5 | IN-TREE FIX |
| LF-13 | Forced TDR and recovery (Windows) | I | BROKEN | MEASURED: a TDR in a game benchmark is an open item (docs/design/V3_FEATURE_GAPS.md rows G03, G10, G14 and docs/OWNER_RULINGS.md); no forced-TDR probe exists | DESIGN-ONLY: compute loop longer than TdrDelay, expect DXGI_ERROR_DEVICE_REMOVED then a fresh device renders | IN-TREE FIX |
| LF-14 | Device removal and re-add (hot unplug of the kf3 device) | I | UNKNOWN | none | DESIGN-ONLY: QMP device_del then device_add, host census | OWNER DECISION (whether hot unplug of the device is supported) |
| LF-15 | Display mode and HDR toggles at run time (Windows) | I | UNKNOWN | none | DESIGN-ONLY: section 5 | IN-TREE FIX |

### 2.8 Windows guest

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| WX-01 | Windows DX11 and DX12 basic rendering and compute | S3, S8 | WORKS | MEASURED: production smoke 4 of 4 and a 110-test Windows matrix in progress as of 2026-10-14 (docs/design/V3_FEATURE_GAPS.md row G15); current results not re-read | re-read the latest Windows matrix | - |
| WX-02 | D3D12 DXR, VRS, mesh shaders, sampler feedback | I | UNKNOWN | rows d3d12_dxr, d3d12_vrs, d3d12_mesh_shaders, d3d12_sampler_feedback exist in the Windows matrix; results not read | read the latest Windows matrix run | IN-TREE FIX |
| WX-03 | D3D12 work graphs | S12 | UNKNOWN | none | DESIGN-ONLY: CheckFeatureSupport OPTIONS21 WorkGraphsTier, then the work-graph sample | IN-TREE FIX |
| WX-04 | D3D12 ExecuteIndirect, reserved resources, multi-adapter | I | UNKNOWN | none | DESIGN-ONLY | IN-TREE FIX |
| WX-05 | DirectStorage | I | UNKNOWN | none | DESIGN-ONLY | IN-TREE FIX |
| WX-06 | Windows Graphics Capture and DXGI Desktop Duplication | S8 | UNKNOWN | none | DESIGN-ONLY: needs a display on the adapter | IN-TREE FIX |
| WX-07 | OpenGL on the Windows driver (an application crash in nvoglv64 reproduced 3 of 3 for one video-editing application; an application violation in one layout application) | I | BROKEN | reported by earlier Windows application runs of kayfabe (the traces are not in this tree); no native control on the same die yet | native-Windows control on a Vast box, then classify | IN-TREE FIX or bucket 3 after a native-Windows control |
| WX-08 | Reported video memory (apps see 3.8 GB at fb-mb 4096) | I | BROKEN | documented policy: the guest reports the frame-buffer size reserved for it, not the die total | owner decision on the default fb-mb | OWNER DECISION (default fb-mb) |
| WX-09 | Application GPU gating (adapter identity, driver version, certified lists) | P2, P9, P10 | WORKS | reported by an earlier Windows image-B run: an image-editing application's GPU gate passes under kf3 (RTX 4090 seen, GPU on); traces not in this tree; permutations not run | permutation matrix | - |
| WX-10 | DLSS, Reflex and NVAPI-dependent games | S9 | UNKNOWN | none | DESIGN-ONLY: Streamline feature-requirement call, NVAPI sleep mode | IN-TREE FIX |
| WX-11 | 30-bit OpenGL output | P4, P5 | UNKNOWN | none | DESIGN-ONLY: 10-bit pixel format and a gradient with 1024 distinct levels | OWNER DECISION (virtual monitor EDID bit depth) |
| WX-12 | Hardware-accelerated GPU scheduling (HAGS) | P13 | UNKNOWN | scripts exist that query and disable it; no run with it on | DESIGN-ONLY: run the gate rows with HwSchMode=2 | IN-TREE FIX |
| WX-13 | Multi-engine copy and overlay planes (MPO) | I | BROKEN | documented policy: no overlay planes advertised; Windows reaches a clean hold only without MPO (docs/design/V3_FEATURE_GAPS.md rows G01, G02) | owner decision | OWNER DECISION (MPO policy) |
| WX-14 | WSL2 CUDA inside a Windows guest (nested virtualisation) | S1, S2 | UNKNOWN | none | DESIGN-ONLY: needs nested KVM on the host and a pre-bundled rootfs | IN-TREE FIX |
| WX-15 | TCC versus WDDM driver model answers | P12 | UNKNOWN | none | DESIGN-ONLY: nvidia-smi driver_model fields, -dm refusal text equals a native GeForce | IN-TREE FIX |

### 2.9 Applications

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| AP-01 | Blender Cycles CUDA and OptiX rendering | I | WORKS | MEASURED: blender_cycles row on the Linux matrix and gfxset items (traces/v3_gfxset) | none | - |
| AP-02 | Blender OptiX denoiser and adaptive sampling | I | UNKNOWN | both Blender scripts set denoising off | add a denoise row | IN-TREE FIX |
| AP-03 | MATLAB GPU computing (gpuArray, kernels, parpool) | I | UNKNOWN | eight rows built, run on no box | run the matlab stage | IN-TREE FIX |
| AP-04 | Unreal Engine editor and cooked builds (DX12, Vulkan, ray tracing, Nanite, Lumen) | S13 | UNKNOWN | nine rows planned, nothing run | run the planned rows | IN-TREE FIX |
| AP-05 | SolidWorks, Ansys and similar certified-card applications | P9, P10 | UNKNOWN | none | owner-supplied offline licence needed | OWNER DECISION (an offline licence) |
| AP-06 | Steam and Proton games (DXVK, VKD3D-Proton) | I | UNKNOWN | none | DESIGN-ONLY: one Vulkan and one VKD3D title | IN-TREE FIX |
| AP-07 | 3DMark and Unigine benchmarks | P11 | UNKNOWN | manual rows only | none | OWNER DECISION (licence and manual lane) |
| AP-08 | Isaac Sim headless (pip package from NVIDIA's index: Vulkan ray tracing, GPU PhysX, OmniGraph, NVENC streaming) | I | UNKNOWN | none | DESIGN-ONLY: install from the official index at run time, run a headless scene with the encoder session counter as proof | IN-TREE FIX |
| AP-09 | RTX Remix (DX8/DX9 games to Vulkan path tracing, Windows) | I | UNKNOWN | none | DESIGN-ONLY: needs an owned game | OWNER DECISION (an owned game) |

### 2.10 Full suites

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| FS-01 | Vulkan CTS full groups, host versus guest per case | I | UNKNOWN | 291-case subset measured (open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending)); full groups not run | section 4 | IN-TREE FIX |
| FS-02 | CUDA samples, all samples | I | UNKNOWN | the Linux matrix builds a subset for sm_86 only | section 4 | IN-TREE FIX |
| FS-03 | OpenGL CTS (VK-GL-CTS GL targets), headless EGL | I | UNKNOWN | none | section 4 | IN-TREE FIX |
| FS-04 | DirectX 12 samples and conformance | I | UNKNOWN | a DirectX sample row set exists in the open Windows profile | section 4 | IN-TREE FIX |
| FS-05 | FFmpeg NVENC/NVDEC matrix | I | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): 47 cases, 38 agree, the 9 differences are explained (session cap, a different ffmpeg build) | none | - |

### 2.11 Interface coverage and fuzzing

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| CV-01 | Interface coverage per suite: which RM classes and controls each suite exercises (trace and ogkm method) | I | UNKNOWN | the method exists; no per-suite coverage table | section 4.5 | IN-TREE FIX |
| CV-02 | Boundary fuzzing of the RM verbs and ioctls a guest can reach | I | UNKNOWN | none | DESIGN-ONLY (section 4.5) | IN-TREE FIX |

### 2.12 Security

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| SE-01 | Guest root cannot change host GPU state (compute mode, clocks, persistence, power limit) | S11 | UNKNOWN | none; probe smi_setters plus the host-state watcher authored | run it | IN-TREE FIX |

## 3. The first batch of probes

### 3.1 What exists

`scripts/apps/gap_probes` holds 45 cases in 13 groups as DATA (`gap_cases.tsv`, one row per case with its falsifier, timeout and tags). Nothing is hard-coded per image: the standalone images are configuration, and no row names one (a test scans the directory). It is a DISCOVERY row of `scripts/apps/run_apps.sh` (name `gap_probes`, in no gate list), so it rides the existing open-guest lane: `apps_hook.sh` samples the host GPU census around the row, `apps_matrix.sh` compares the host log with the guest log.

| group | cases | falsifier of the first case (the full table has one per case) | GPU work |
|---|---|---|---|
| group | cases | falsifier of the first case (the full table has one per case) | GPU work |
|---|---|---|---|
| cuda_attr | attr_dump, canary | any device attribute, or the memory total, differs between the host and the guest run outside gap_known_limits.tsv | yes |
| cuda_pinned | pinned_1g, pinned_4g, host_register_512m, mapped_zerocopy, pinned_huge_refused | cuMemHostAlloc of 1 GiB fails or corrupts in the guest and passes on the host | yes |
| cuda_managed | managed_prefetch_advise, managed_concurrent, managed_oversub | managed allocation, advise, prefetch and a kernel give a wrong value or an error that the host does not | yes |
| cuda_ipc | ipc_legacy, ipc_event, ipc_vmm_fd | cuIpcGetMemHandle/OpenMemHandle between two processes fails in the guest and works on the host | yes |
| cuda_vmm | vmm_granularity, vmm_basic, vmm_remap | allocation granularity differs from the host or is not a power of two | yes |
| cuda_graph | graph_capture_launch, graph_update, graph_mem_nodes | a captured graph replayed 1000 times gives a wrong value or an error in the guest | yes |
| cuda_pool | pool_alloc_free, pool_threshold_trim, pool_ipc_export | stream-ordered allocation (cuMemAllocAsync/FreeAsync) fails or corrupts in the guest | yes |
| cuda_churn | churn_ctx_200, churn_module_200, churn_streams_events, fanout_procs_24 | 200 create/use/destroy cycles of a context fail (the per-boot CUDA process budget is about 60 on the same device) | yes |
| cuda_kernel_time | spin_5s, spin_30s, launch_flood_100k, fault_recovery, trap_recovery | a 5 s kernel is killed in the guest (a watchdog the host does not apply) | yes |
| profiling | cupti_callback, cupti_activity, tools_sanitizer, tools_ncu, tools_nsys, tools_nsys_gpumetrics | the CUPTI callback API does not see the driver API calls of a process in the guest while it does on the host (or cuptiSubscribe is refused) | yes |
| vulkan_caps | vk_dump | any Vulkan feature, limit, queue family, format or extension value differs from the host outside the known limits | no |
| smi_control | smi_setters | a setter answers differently from the host, hangs, or changes a value the host can see (guest root changing host GPU state) | no |
| lifecycle | life_persistenced, life_kill_kernel, life_kill_gl, life_module_cycle, life_reset, life_busy_poweroff | nvidia-persistenced does not start, hold and stop cleanly in the guest, or CUDA fails while it runs | yes |

How a case works:

- `gap_probe.c` (CUDA driver API through `dlopen`, no toolkit, no header) runs ONE case per process. It reads the device attribute that says whether the feature exists, runs the feature anyway, and judges: worked and the attribute says yes is PASS; refused and the attribute says no is NOT_SUPPORTED; worked against the attribute is MISMATCH; failed against the attribute, or returned wrong data, is FAIL. A crash, a hang and a missing result line are FAIL with a named reason, never NOT_RUN and never a pass.
- After a failing case a canary (context, kernel, copy) tells a cascade from a defect of that case; three dead canaries stop the run and the rest are NOT_RUN with the reason.
- The `py` cases drive `nvidia-smi`, `vulkaninfo`, the kernel modules, process kills and the NVIDIA tools. The NVIDIA tools (compute-sanitizer, Nsight Compute, Nsight Systems, CUPTI) are installed at run time by `provision_tools.sh` from NVIDIA's official CUDA apt repository at pinned versions; they are never stored in the repository, an image or R2. A missing tool is NOT_RUN with its name.
- Disruptive cases are tagged: `guest_only` (never run on the host oracle, because it would disturb the machine kayfabe runs on), `destructive` and `terminal` (opt-in flags; the terminal case, a guest power-off with a busy GPU, runs last).

### 3.2 The host run is the reference

Every case that can run on the host runs there first, twice (host against host gives the noise floor: a case that differs between two host runs is excluded from the guest comparison). The comparison is `scripts/apps/gap_probes/diffrun.py`: one verdict per case, only differences reported, with the vkcts category names (GUEST_ONLY_FAIL, GUEST_LOSES_SUPPORT, GUEST_FAIL_HOST_NS, GUEST_GAINS_SUPPORT, BOTH_FAIL, ...) plus VALUE_DIFF (both pass but a recorded value differs), timeouts and crashes kept apart from failures, the host Xid lines attached per case window, and a known-limits file (`gap_known_limits.tsv`: case, key, host class, guest class, bucket, reason; every limit carries a reason and explains only the difference it names). A difference with no listed limit stays a `kayfabe_gap_candidate` until someone classifies it. A case that fails on the host control too is `app_or_harness_bug`.

Listed limits at the time of writing: the frame-buffer total and the PCI ids of the guest's virtual topology, UVM demand paging (README, ruling E), and the profiler interface (ruling H).

### 3.3 Tests

GPU-free: `scripts/ci/test_gap_probes.py` (the case table, selection, probe output parsing, process classification, the C judge table compiled with `-DGAP_SELFTEST`, setters, Vulkan flattening, the comparison, runner end to end against fake probe programs, wiring). Mutants: `scripts/ci/gap_probes_mutants.py`, every mutant killed by two named tests. MEASURED: those tests and mutants pass; the C file compiles with `gcc -Wall`. NOT measured: any behaviour against libcuda.

### 3.4 Hardware results

PENDING: the hardware run (one Vast box through the open-guest lane) has not been done when this version was written. This section receives, per probe, the host result, the guest result, the kf3 census (Xid, refused, unserviced) and the bucket of every difference. Until then every probe row is UNKNOWN.

### 3.5 Probes not authored (DESIGN-ONLY rows in the matrix)

MPS (two clients on a private pipe directory), cooperative launch and cluster launch, an NVRTC case, CUDA-GL and CUDA-Vulkan interop, OpenGL extension and context-sharing parity through EGL, work graphs and the other Windows rows, the lifecycle cases that need a display (VT switch, lock and unlock, display mode change), the host census check after a busy power-off, Isaac Sim and RTX Remix. Each is a row with its next step; none is claimed.

## 4. FULL-SUITE section (DESIGN-ONLY except where marked)

Owner priority (2026-11-18): full-suite host-versus-guest comparisons come first. Sizes and times below are ESTIMATES unless marked MEASURED.

### 4.1 Per suite

| suite | size | how the per-case host-versus-guest diff is produced | expected runtime and box cost |
|---|---|---|---|
| Vulkan CTS full groups (`deqp-vk`, pinned commit) | the subset is 291 cases (MEASURED); the full main mustpass is large and has not been counted (INFERRED: order 10^5 or more; count with `--deqp-runmode=xml-caselist` on the pinned commit) | existing row `vkcts` writes one TSV row per case (`VKCTS_RESULTS`); `diffrun.py` reads it (`parse_vkcts_results`) and reports only differences. Run by group in shards of a fixed case count, each shard a fresh process, so a hang costs one shard | subset: host 104 s, guest 877 s with 2 timeouts (MEASURED, one RTX 3090 box). Full: ESTIMATE hours per side per group family; box cost about 0.4 to 0.6 USD per hour (MEASURED on two boxes 2026-11-17/18) |
| CUDA samples, all | about 150 samples at the pinned release (INFERRED, not counted); the Linux matrix builds a subset for sm_86 only (MEASURED) | one row per sample from the existing matrix format (exit code and output digest); adapter into `diffrun` rows; the host output is the oracle | ESTIMATE 1 to 3 hours per side; the toolkit is needed for the build only |
| OpenGL CTS (VK-GL-CTS GL targets, headless EGL) | not counted (INFERRED: order 10^5) | same QPA result format as `deqp-vk`, so the vkcts adapter applies once the GL binary is built from the same pinned repository | ESTIMATE as Vulkan CTS |
| DirectX 12 samples and conformance | the open Windows profile has a small DirectX sample set (MEASURED rows exist); the vendor conformance tests ship with a Windows hardware kit that is not redistributable (INFERRED) | per-sample exit code and image digest through the Windows lane; conformance stays DESIGN-ONLY | ESTIMATE hours; needs the Windows base image |
| FFmpeg NVENC/NVDEC matrix | 47 cases (MEASURED) | existing `nvcodec_matrix` compare, readable by `diffrun` (`parse_nvcodec_results`) | host 143 s, guest 245 s (MEASURED) |

### 4.2 One differential runner (design)

1. A suite adapter lists its cases deterministically (name, area, shard) and records the list digest, so host and guest run the same list.
2. The runner shards the list, runs each shard in a fresh process with a per-case timeout, and writes one row per case: status class (pass, not_supported, fail, timeout, crash, not_run), reason, seconds, start and end time. Timeouts and crashes are classified, never merged into failures.
3. Host first, twice (noise floor), then the guest. Cases that differ host against host are marked flaky and left out of the guest comparison.
4. `diffrun.compare` reports only the differing cases, each with the host Xid lines inside its time window, the bucket from the known-limits file or `kayfabe_gap_candidate`.
5. A confirm step re-runs each candidate alone in a fresh boot (what was done by hand on 2026-11-17/18 for the sparse and device-generated-commands cases), with the host control run twice.
6. The report lists candidates by area with their Xid, and the findings ledger records each with its bucket and its falsifier.

Implemented in this change (GPU-free, tested): items 2 and 4 for the gap probes, and the adapters for the vkcts and nvcodec result formats. NOT implemented: the shard planner, the noise-floor step, the confirm planner, a QPA log parser, the CUDA-samples and GL adapters. `diffrun.py` is the stable core other work should extend; its schema is the row dictionary (case, status, area, reason, secs, t0, t1, kv) and the comparison dictionary (case, area, host, guest, category, bucket, xid, host_xid, kvdiff, why).

### 4.3 Owner priority 2: interface coverage

Method (existing): the trace and ogkm interface coverage of what a workload asks the RM. Per suite, the kf3 census after each shard gives the refused and unserviced control ids; a suite's coverage table is the set of ids it touched. Row CV-01. Not built per suite.

### 4.4 Owner priority 3: boundary fuzzing

Row CV-02: fuzz the RM verbs and ioctls a guest can reach with out-of-range sizes, handles and counts, expecting a refusal, never a host fault or a hang (hostile-guest rule). DESIGN-ONLY; the probe `pinned_huge_refused` is the first single data point (an absurd allocation must be refused promptly).

## 5. Lifecycle and fault events (owner priority 4)

The VM must be free to hibernate, lock, refresh the driver and lose the device. Linux events are authored as probes (section 3); the commands for the rest are written here for the next lane. DESIGN-ONLY unless a probe name is given.

### 5.1 Linux guest

| event | command or probe | pass criterion |
|---|---|---|
| module unload and reload, with and without a client | probe `life_module_cycle` | refusal under a live client within 60 s, clean unload and reload without one, canary passes after each cycle |
| driver reset | probe `life_reset --destructive` | a clean answer (reset done or refused), no hang, canary passes |
| kill mid-kernel, kill mid-graphics | probes `life_kill_kernel`, `life_kill_gl` | the next process works, 5 rounds |
| persistence daemon | probe `life_persistenced` | starts, holds, stops on SIGTERM, CUDA works while it runs |
| power-off or reboot with a busy GPU | probe `life_busy_poweroff --terminal`; reboot variant: spin client then `systemctl reboot` | host census after: no leaked slot, VA pool or channel; the same device boots again and the canary passes |
| VT switch with a GL client alive | `chvt 3; sleep 2; chvt 1` with a looping EGL or `kmscube` client | the client keeps rendering, no Xid |
| screen lock and unlock under a display manager | `loginctl lock-session; sleep 5; loginctl unlock-session` with a GL client alive | the client survives |
| S3 suspend and resume | `systemctl suspend`, wake with the QEMU monitor `system_wakeup` (harness `scripts/bench/display/desktop_s3.sh`) | MEASURED working at 559db30c; the release image VMs disable S3 by default, so a run needs the flag |
| display mode change | `xrandr --output <out> --mode 1280x720` and back | no hang, new size seen |
| driver rebind | `echo <bdf> > /sys/bus/pci/drivers/nvidia/unbind` then `bind` | clean refusal or clean rebind |

### 5.2 Windows guest (exact commands for the base image later)

| event | command | note |
|---|---|---|
| hibernate and resume | `powercfg /h on` then `shutdown /h`; relaunch the same disk | needs S4 enabled in the VM (the release image VMs disable it); read the event log for the resume |
| sleep and wake | `rundll32.exe powrprof.dll,SetSuspendState 0,1,0` with hibernate off, wake with QMP `system_wakeup` | the earlier run used exactly this (traces/win11_s3_20261014); needs S3 enabled |
| modern standby | none | QEMU provides no S0ix state, so the platform offers S3 or nothing (INFERRED); the row stays UNKNOWN until a guest shows otherwise |
| lock and unlock with a DXGI client alive | `rundll32.exe user32.dll,LockWorkStation`; unlock by typing the password through QMP `send-key` | the client writes a heartbeat file; it must keep advancing |
| driver refresh | QMP `send-key` for left-windows, ctrl, shift and `b` together | the display blinks and the client survives or recreates its device |
| device disable and enable | `pnputil /enum-devices /class Display` to find the device identifier, then `pnputil /disable-device /instanceid "<device id>"`, `pnputil /enable-device /instanceid "<device id>"`, `pnputil /restart-device /instanceid "<device id>"` | repeat 5 times with a D3D client alive; expect device-removed then a clean recreate |
| forced TDR | a D3D12 compute shader looping longer than the TDR delay (2 s default) | expect DXGI_ERROR_DEVICE_REMOVED, then a fresh device renders; no BSOD; host sees no Xid cascade |
| fast startup | `powercfg /h on`, `shutdown /s /t 0`, boot the same disk | the driver comes up and a canary passes |
| display mode and HDR toggles | `ChangeDisplaySettingsEx` in a small program; the HDR toggle is the Win+Alt+B key through QMP | no hang; see GX-10 for the HDR claim |
| device removal and re-add | QMP `device_del` then `device_add` of the kf3 device | whether the device supports hot unplug is UNKNOWN |

## 6. Patch tier: rows that can only close with a host or guest patch

Policy (`docs/design/V3_HOST_PATCH_LIST.md` section 1, owner, 2026-10-10): the stock install works with no kernel patch and is the acceptance bar; a patched host kernel or driver is an optional, better tier. A row that can only close that way stays BROKEN until the tier ships, and its stock-path behaviour must stay loud and safe (never a silent wrong answer).

| rows | what the patch has to do | state (from the cited documents, not re-measured here) |
|---|---|---|
| CU-13, CU-14, CU-15, CU-16 (managed memory with demand paging, prefetch and advise through faults, concurrent CPU access, oversubscription; the Windows counterpart `managed_t`) | **H3**: a narrowly scoped patch of the host's open nvidia-uvm (about 1.3 kLoC) adds one per-address-space mode, divert instead of service: replayable faults of an opted-in guest address space are queued to that address space's own fd and parked; the owner (kayfabe) answers replay or cancel. The passthrough twin address space becomes UVM-owned on the host, so the publish executor moves from the RM map verb to UVM's external-mapping ioctls (stock and unprivileged). **Guest side (unbuilt):** the replayable fault buffer, the non-replayable shadow buffer and the replay, cancel and clear-faulted decode. **H4** (design only): a host-owned UVM pool where CPU VA and GPU VA differ, for the cases H3 cannot cover. Constraints from `docs/OWNER_RULINGS.md` section E: the VMM is not trusted, no MMIO reads of guest page tables (a copy-engine snapshot instead), no multi-second stall of other tenants' graphics work per parked fault (the host watchdog is about 4.3 s, Xid 109), and nothing merges until the per-fault hold time is measured with two VMs, one hostile. Unprivileged-only is believed impossible (section 4 of the UVM design: the fault buffer needs RM kernel privilege, and nvidia-uvm already holds it) | host-only proof of H3 passed on a GA106 with driver 580.159.04 (docs/design/V3_UVM_B3_IMPLEMENTATION.md; traces/v3_uvm_b3); guest side unbuilt; H4 not run. Tweaks that need no new host module are described in docs/design/V3_UVM_DEMAND_PAGING.md section 7 as stop-gaps only |

Rows that might become patch-tier depending on a root cause not yet found: GX-06 (sparse residency timeouts) only if the cause is the mapping granularity, in which case **H6** (exact partial unmap of `NV01_MEMORY_VIRTUAL` mappings, RM core) or **H7** (a scatter-map verb) are the candidates named in the patch list; INFERRED, the mechanism is not shown.

Patch-list items with no BROKEN or UNKNOWN row of this matrix depending on them today: H0 (tier infrastructure), H1 (in-kernel doorbell: performance for stock guests), H2 (a software vfio device for stock hypervisors), H5 (USERD-in-sysmem address-size fix for Windows on Turing, Ampere and Ada hosts; the relay is the stock-path fallback), H8 (own-channel FIFO and USERD migration). They are listed so that a future row that needs one is filed here.

## 7. What would change a status

An UNKNOWN row becomes WORKS or BROKEN only through a run that names the kayfabe revision, the box and the date, with the host control. A BROKEN row becomes WORKS only after a GPU-free regression test exists for the defect and a hardware run passes. A bucket 1 entry needs a document that names the limitation, and the known-limits file needs a reason for it.

## 8. Sources

| key | read | URL | what it is |
|---|---|---|---|
| S1 | READ 2026-11-18 | https://docs.nvidia.com/cuda/wsl-user-guide/index.html | CUDA on WSL2 user guide: known limitations (managed memory, pinned memory, NVML queries, ECC/compute mode/persistence, OpenGL-CUDA interop) |
| S2 | READ 2026-11-18 | https://docs.nvidia.com/vgpu/latest/grid-vgpu-release-notes-microsoft-windows-server/known-product-limitations.html | vGPU known product limitations (NVENC resolution cap, channel exhaustion, nested virtualisation) |
| S3 | READ 2026-11-18 | https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/gpu-partitioning | Microsoft GPU partitioning: supported GPUs and guests, limitations |
| S4 | READ 2026-11-18 | https://docs.nvidia.com/cuda/cuda-programming-guide/04-special-topics/virtual-memory-management.html | CUDA virtual memory management API surface and platform restrictions |
| S5 | READ 2026-11-18 | https://docs.nvidia.com/deploy/mps/index.html | MPS overview (table of contents only; the requirement pages were not read) |
| S6 | READ 2026-11-18 | https://docs.nvidia.com/nsight-compute/ProfilingGuide/index.html | Nsight Compute profiling guide (first 100000 characters read: mentions vGPU and MPS launch metrics) |
| S7 | READ 2026-11-18 | https://developer.nvidia.com/video-encode-decode-gpu-support-matrix | Video encode/decode support matrix (session caps, AV1, 4:2:2, 4:4:4, 8K) |
| S8 | READ 2026-11-18 | https://github.com/jamesstringerparsec/Easy-GPU-PV | A GPU-PV project README: stated limitations (Vulkan unavailable, OpenGL may not work, display and virtual display, host/guest version match) |
| S9 | READ 2026-11-18 | https://download.nvidia.com/XFree86/Linux-x86_64/580.126.18/README/index.html | NVIDIA Linux driver README, chapter list only (PRIME offload, persistenced, GBM, power management, GSP) |
| S10 | READ 2026-11-18 | https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/cdi-support.html | Container toolkit CDI support |
| S11 | READ 2026-11-18 | https://docs.nvidia.com/deploy/nvidia-smi/index.html | nvidia-smi: state-changing options and their restrictions (first half of the page) |
| S12 | READ 2026-11-18 | https://microsoft.github.io/DirectX-Specs/d3d/WorkGraphs.html | D3D12 work graphs specification (first 100000 characters; the device-support section was not read) |
| S13 | READ 2026-11-18 | https://dev.epicgames.com/documentation/en-us/unreal-engine/hardware-and-software-specifications-for-unreal-engine | Unreal Engine hardware and software specifications |
| S14 | READ 2026-11-18 | https://docs.vulkan.org/spec/latest/chapters/sparsemem.html | Vulkan sparse resources |
| P2 | earlier pass, not re-read | https://helpx.adobe.com/photoshop/kb/photoshop-cc-gpu-card-faq.html | Photoshop GPU FAQ (a search summary was read in an earlier pass; not re-read) |
| P4 | earlier pass, not re-read | https://www.pugetsystems.com/labs/articles/How-to-enable-30-bit-in-Photoshop-1243/ | 30-bit display setup (summary, earlier pass) |
| P5 | earlier pass, not re-read | https://www.anandtech.com/show/14682 | 30-bit support on GeForce (summary, earlier pass) |
| P7 | earlier pass, not re-read | https://forums.developer.nvidia.com/t/nvidia-vgpu-19-0-enables-graphics-and-ai-virtualization-on-nvidia-blackwell-gpus/341340 | a user report about Vulkan ray tracing extensions on a virtualised GPU (earlier pass) |
| P9 | earlier pass, not re-read | https://support.citrix.com/external/article/492431/solidworks-doesnt-consume-gpu-in-virtual.html | a vendor article title on a CAD application in virtual desktops (404 on fetch; title only) |
| P10 | earlier pass, not re-read | https://knowledge.cadimensions.com/knowledge/the-solidworks-use-software-opengl-setting | the software-OpenGL setting of a CAD application (secondary, earlier pass) |
| P11 | earlier pass, not re-read | https://support.benchmarks.ul.com/support/solutions/articles/44001873452-the-frame-rate-exceeded-the-nvidia-max-frame-rate-setting | benchmark validity and the driver frame-rate cap (summary, earlier pass) |
| P12 | earlier pass, not re-read | https://docs.nvidia.com/datacenter/tesla/driver-installation-guide/gaming.html | NVIDIA gaming notes: NVAPI and DLSS (earlier pass) |
| P13 | earlier pass, not re-read | https://ntdoc.m417z.com/d3dkmt_wddm_2_7_caps | WDDM 2.7 caps and HAGS (secondary, earlier pass) |
| I | INFERRED | - | no source: this is the author's inference from how the feature works; the row says what would settle it |
