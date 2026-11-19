# V3 GPU-PV gap matrix: what a GPU paravirtualisation stack leaves behind, and where kayfabe stands

**STATUS: RESEARCH, 2026-11-18** (first version; every row says whether it is MEASURED, cited or INFERRED; one hardware run of 2026-11-18 on one RTX 3090 box is recorded in sections 3.4 and 3.5 and in `traces/gap_probes_run_20261118/`; nothing else here was run on a GPU).

**Summary (public, counts per kayfabe status): 148 rows: WORKS 60, BROKEN 22, UNKNOWN 58, N-A-BY-DESIGN 8.** **Target: BROKEN = 0. Patch-tier rows are BROKEN until the optional patch tier ships.** **Closure path of the BROKEN and UNKNOWN rows: IN-TREE FIX 59, PATCH TIER 3, OWNER DECISION 18 (and 8 N-A-BY-DESIGN rows, which close by their stated reason).** (IN-TREE FIX = kayfabe code; PATCH TIER = needs a host and/or guest kernel or driver patch, optional and later, section 6; OWNER DECISION = a policy; for an UNKNOWN row the path is the one that applies if a run fails.) A row leaves UNKNOWN only through a run with a revision, and the UNKNOWN rows left are either probes not yet written (DESIGN-ONLY, section 3.6) or not yet run (Windows, other dies, tools in the guest). The aim of the matrix is to find failures: a pass is hygiene, a failure is a result.

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
| CU-01 | CUDA IPC, legacy memory handles (cuIpcGetMemHandle/OpenMemHandle) between two guest processes | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): ipc_legacy PASS on host and guest (a second process wrote through the imported handle) | none | - |
| CU-02 | CUDA IPC events | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): ipc_event PASS on host and guest | none | - |
| CU-03 | VMM export of a physical allocation as a POSIX fd and import in a second process | S4 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): ipc_vmm_fd PASS on host and guest (fd sent over a unix socket, imported, mapped and written by a second process) | none | - |
| CU-04 | Virtual memory management: cuMemCreate, cuMemMap, cuMemSetAccess, granularity | S4 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): vmm_basic and vmm_granularity PASS on host and guest; granularity values equal | none | - |
| CU-05 | VMM remap: physical handles swapped under one VA range | S4 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): vmm_remap PASS on host and guest | none | - |
| CU-06 | VMM compressible allocations (CU_MEM_ALLOCATION_COMP_GENERIC) | S4 | UNKNOWN | none | add a case to gap_probe.c (vmm_compress) | IN-TREE FIX |
| CU-07 | VMM fabric handles, IMEX channels, multicast objects | S4 | N-A(needs an NVLink/NVSwitch fabric; the target is one GeForce die per guest and NVLink is stubbed as absent by owner ruling H) | docs/OWNER_RULINGS.md section H | assert the attribute dump reports fabric handles unsupported (attr_dump) | N-A-BY-DESIGN |
| CU-08 | CUDA graphs: stream capture, instantiate, 1000 replays | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): graph_capture_launch and graph_mem_nodes PASS on host and guest; a captured graph replays at 20.7k launches/s in the guest against 174k/s on the host (performance, not correctness) | performance row, owner priority 'decent' | - |
| CU-09 | CUDA graph update (cuGraphExecUpdate) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): graph_update PASS on host and guest | none | - |
| CU-10 | Conditional graph nodes, graph memory nodes, device-side graph launch | I | UNKNOWN | none | extend gap_probe.c after the first graph run | IN-TREE FIX |
| CU-11 | Stream-ordered allocator: cuMemAllocAsync/FreeAsync, release threshold, trim | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): pool_alloc_free and pool_threshold_trim PASS on host and guest | none | - |
| CU-12 | Memory pool export as a shareable handle | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): pool_ipc_export PASS on host and guest | none | - |
| CU-13 | Managed memory with demand paging (a kernel touching a managed allocation) | S1 | BROKEN | MEASURED: the four UVM demand-paging apps (UnifiedMemoryStreams, UnifiedMemoryPerf, conjugateGradientUM, attach_verify) fail in the guest with one host Xid 31 each and pass on bare metal (kf3 144557e4, 2026-11-07, docs/design/V3_APP_MATRIX.md); documented limitation, not a release target (README.md, docs/OWNER_RULINGS.md section E) | owner decision; probes managed_* record the same failure per case | PATCH TIER (H3 host nvidia-uvm fault service plus the guest side; H4 pool; section 6) |
| CU-14 | Managed memory: prefetch, advise, read-mostly, accessed-by | S1 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): managed_prefetch_advise FAIL in the guest (cuCtxSynchronize) with host Xid 31 and 13 on the qemu pid, PASS on the host: the CU-13 limitation, bucket 1 | same closure as CU-13 | PATCH TIER (H3 and H4, same as CU-13) |
| CU-15 | Managed memory: concurrent CPU access during a kernel (concurrentManagedAccess) | S1 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): managed_concurrent PASS on host and guest (CPU writes while a kernel runs; that kernel does not touch managed pages, so this does not cover demand paging, which is CU-13) | none | - |
| CU-16 | Managed memory oversubscription (more than the device holds) | S1 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): managed_oversub FAIL in the guest (cuCtxSynchronize) with host Xid 31, PASS on the host (the host did not oversubscribe: its 24 GB exceeds the 8 GiB base, the guest sees 7.9 GB): the CU-13 limitation, bucket 1 | same closure as CU-13 | PATCH TIER (H3 and H4, same as CU-13) |
| CU-17 | Pinned host memory, 1 GiB and 4 GiB | S1 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): pinned_1g and pinned_4g PASS on host and guest | none | - |
| CU-18 | Host registration (cuMemHostRegister) and mapped zero-copy kernels | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host_register_512m and mapped_zerocopy PASS on host and guest | none | - |
| CU-19 | A guest request for an absurd pinned allocation is refused promptly (hostile guest rule: no unbounded guest-reachable resource) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): pinned_huge_refused PASS on host and guest: a 1 TiB pinned request is refused promptly | none | - |
| CU-20 | Cooperative kernel launch (grid-wide synchronisation) | I | UNKNOWN | none | DESIGN-ONLY: author a coop case (occupancy-sized grid, atomic barrier) | IN-TREE FIX |
| CU-21 | Thread block clusters (sm_90 and newer) | I | UNKNOWN | none; needs a Hopper or Blackwell die | DESIGN-ONLY: gate on the cluster-launch attribute (attr_dump) | IN-TREE FIX if a run on a Hopper or Blackwell die fails |
| CU-22 | Peer access and peer-to-peer copies | I | N-A(one die per guest: peer PTEs are refused by name and the P2P capabilities answer NOT_SUPPORTED; the guest must answer cudaDeviceCanAccessPeer with 0, which attr_dump records) | docs/design/V3_FEATURE_GAPS.md row L10 | check the clean answer in attr_dump | N-A-BY-DESIGN |
| CU-23 | NVRTC and nvJitLink compilation in the guest | I | UNKNOWN | none; driver PTX JIT of fresh modules is probe churn_module_200 | author an NVRTC case once the toolkit is on the box | IN-TREE FIX |
| CU-24 | PTX JIT of 200 fresh modules with the cache disabled | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): churn_module_200 PASS on host and guest (200 fresh PTX JIT modules, cache disabled) | none | - |
| CU-25 | Per-boot CUDA process budget | I | WORKS | MEASURED: 100 CUDA processes in one boot, 100 of 100 (docs/design/V3_APP_MATRIX.md row m20seq); the earlier host NoMemory at the 40th (no persistence mode) or 60th process, cause J, is gone | re-measure with fanout_procs_24 and churn_ctx_200 (concurrent, not sequential) | - |
| CU-26 | 24 simultaneous CUDA processes each running a kernel | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): fanout_procs_24 PASS on host and guest (24 concurrent processes, each with a context and a kernel); churn_ctx_200 PASS on both | none | - |
| CU-27 | 2048 streams and 4096 events in one process | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): churn_streams_events PASS on host and guest | none | - |
| CU-28 | Kernel running 5 s and 30 s (no watchdog the host does not apply) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): spin_5s and spin_30s PASS on host and guest: no watchdog kills a long kernel | none | - |
| CU-29 | Illegal-address and trap faults are reported, and a fresh context works afterwards | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): fault_recovery and trap_recovery PASS on host and guest, judged across processes (a fault in a second process is reported, the first process and a fresh process still work). The first version judged it inside one process and FAILED on the host too: a harness defect found by the host control | none | - |
| CU-30 | 100000 back-to-back launches | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): launch_flood_100k PASS on host and guest (about 4 s in the guest against under 1 s on the host) | none | - |
| CU-31 | CUDA attribute parity (all attributes, memory total) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): attr_dump PASS on host and guest: of every CUDA device attribute only the memory total (24124 against 7871 MiB) and the PCI device id differ, both listed known differences | none | - |

### 2.2 Tools, profiling, NVML and nvidia-smi

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| TL-01 | MPS (nvidia-cuda-mps-control daemon, two clients) | S5 | UNKNOWN | none | DESIGN-ONLY: start the daemon with a private pipe directory and run two canary clients | IN-TREE FIX |
| TL-02 | compute-sanitizer: memcheck, racecheck, synccheck, initcheck | S1 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): tools_sanitizer: PASS on the host (memcheck, racecheck, synccheck, initcheck all report 0 errors); in the guest the first CUDA call of the target returns 999 under each tool (the tool's own output: target application returned an error), and the host answers the positive control. Bucket 1 by INFERENCE from ruling H (debugger stubbed); mechanism not shown | read the injection failure: which RM control the tool needs | OWNER DECISION (serve the debugger interface that ruling H stubs) |
| TL-03 | CUPTI callback API | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): cupti_callback PASS on host and guest | none | - |
| TL-04 | CUPTI activity API (kernel records) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): cupti_activity PASS on host and guest (kernel activity records returned) | none | - |
| TL-05 | Nsight Compute kernel replay with performance counters | S6, S2 | N-A(profiler interface stubbed as capability absent by owner ruling H; hardware performance counters are a privileged RM profiler interface and the hostile-guest rule forbids serving it; the probe still records the exact answer) | docs/OWNER_RULINGS.md section H; probe tools_ncu authored | run tools_ncu: a clean not-supported answer is the expected pass | N-A-BY-DESIGN |
| TL-06 | Nsight Systems CUDA trace | I | UNKNOWN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): tools_nsys PASS on the host; NOT_RUN in the guest (the 1 GB tool was not pushed into the guest) | push the tool into the guest or install it in the image | IN-TREE FIX |
| TL-07 | Nsight Systems GPU metrics sampling | S6 | UNKNOWN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): tools_nsys_gpumetrics PASS on the host; NOT_RUN in the guest (tool not pushed) | as TL-06 | IN-TREE FIX |
| TL-08 | cuda-gdb and the debugger interface | I | N-A(debugger stubbed as capability absent by owner ruling H) | docs/OWNER_RULINGS.md section H | none | N-A-BY-DESIGN |
| TL-09 | NVML field coverage in the guest | S1 | BROKEN | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): 132 NVML and 74 DCGM telemetry values (clocks, power, temperature, fan, PCIe throughput, utilisation, throttle reasons) answer N/A in the guest; documented limitation (docs/design/V3_UTILIZATION_FLAG.md section 10) | none until the owner wants the counters served | OWNER DECISION (serve the counters, docs/design/V3_UTILIZATION_FLAG.md) |
| TL-10 | nvidia-smi queries and persistence mode | S11 | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): persistence mode on, DCGM software diagnostic passes on host and guest; memory.used and encoder session counters served | none | - |
| TL-11 | nvidia-smi setters from guest root: compute mode, power limit, clock lock, ECC, accounting | S11 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): smi_setters: in the guest the compute-mode and accounting setters answer NOT_SUPPORTED where the host answers OK, and power limit and clock fields are N/A so those setters were not attempted; every answer is a clean answer and the device works afterwards. Capability absent (ruling H pattern), bucket 1 | owner decision whether a guest may hold a compute mode | OWNER DECISION (whether a guest may hold a compute mode) |
| TL-12 | Driver reset (nvidia-smi -r) | S11 | N-A(a guest-triggered GPU reset is a privileged host action that kayfabe never authors from guest bytes: the hostile-guest rule; measured: nvidia-smi -r in the guest answers NOT_SUPPORTED with rc 3 and the device works afterwards) | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_reset (guest only, destructive flag) PASS: clean refusal, canary after it passes | none | N-A-BY-DESIGN |
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
| GX-02 | Vulkan features, limits, queue families and format properties equal to the host | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): vk_dump: 250 of 250 device extensions equal and 172 shared feature, limit and property names equal but for 4 format-related ones; the host's vulkaninfo is an older version than the guest's and dumps fewer properties (679 against 1266 tokens), so full parity needs one pinned tool | pin one vulkaninfo build for both sides | - |
| GX-03 | Vulkan ray tracing pipeline and ray query | P7 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): CTS sample: ray_tracing_pipeline 234 and ray_query 232 cases, guest equals host but for one case that is only slow (CT-02, CT-03) | none | - |
| GX-04 | Mesh and task shaders | I | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): CTS mesh_shader sample: 3 guest Timeouts without PR 480 (all depth_only or partial-usage smoke cases), none with it (CT-04) | merge PR 480 | IN-TREE FIX (merge PR 480) |
| GX-05 | Device-generated commands (multiview indexed draw) | I | WORKS | MEASURED: DeviceLost with host Xid 13 before the fix, Pass after it (PR 480, host_pte_kind), confirmed on a second box 2026-11-18 | none | - |
| GX-06 | Sparse binding and residency (buffer and multisampled image residency) | S14 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): CTS sparse_resources sample: 11 of 70 differ, with and without PR 480, host Xid 109 and 31 (CT-01) | root cause first, then a GPU-free regression test | IN-TREE FIX (root cause first; H6 or H7 only if the cause is the mapping granularity, INFERRED) |
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
| GX-20 | Vulkan queue global priority (VK_KHR_global_priority): LOW, HIGH and REALTIME queues | P7 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): vk_global_priority: the guest reports the host's priorities (family 0: LOW, MEDIUM, HIGH, REALTIME) but vkCreateDevice with LOW, HIGH or REALTIME on any family fails with VK_ERROR_INITIALIZATION_FAILED (-3) and only the default MEDIUM succeeds; the host accepts all reported ones; same uid 0 and same capability sets on both sides. The kf3 log of every failing boot has one refusal that the neighbour boot lacks: GSP REFUSED fn76/0x20801115=0x56, NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY (the link is INFERRED). CTS: api.device_init.create_device_global_priority_khr.basic and _query_khr.basic, synchronization.global_priority_transition high and realtime | owner decision on the host-wide runlist scheduling policy, then an in-tree change | OWNER DECISION (the host-wide runlist scheduling policy, section 3.5) |
| GX-19 | Display-less and headless operation | S9 | WORKS | MEASURED: the whole gfxset and the Linux app matrix run headless | none | - |

### 2.5 Vulkan CTS groups, sampled (section 3.5)

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| CT-01 | Vulkan CTS sparse_resources (70 sampled of 19,402) | S14 | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 53 pass, 17 NS; guest 42 pass, 17 NS, 10 Timeout, 1 Fail, with and without PR 480 (11 cases: image_sparse_residency 2d_array/cube/cube_array, multisampled_image_sparse_residency r32ui/rgba16f/rgba32f samples_2 and rgba32i/rgba32ui/rgba8i samples_4, buffer texel_buffers.sparse_residency size_2_24 r64ui, shader_intrinsics.cube_sparse_read r32i); host Xid 109 (channel 0x31) and one Xid 31 per timeout; mechanism not shown (CU/GX-06) | root cause first, then a GPU-free regression test | IN-TREE FIX |
| CT-02 | Vulkan CTS ray_tracing_pipeline (234 sampled of 22,241) | P7 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 205 pass, 29 NS; guest 204 pass, 29 NS, 1 Timeout at 60 s (misc.recursiveTraces_AABB_15), which passes alone in the guest at 240 s: slowness, not a hang | none | - |
| CT-03 | Vulkan CTS ray_query (232 sampled of 48,771) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 217 pass, 15 NS; guest identical | none | - |
| CT-04 | Vulkan CTS mesh_shader (254 sampled of 28,061) | I | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): without PR 480 the guest times out on 3 ext.smoke cases (optimized_lib depth_only_points_position_components, optimized_lib partial_usage_without_compaction, shader_objects depth_only_triangles) and the host Xid 13 3D-Z SYSMEM KIND Violation appears; with PR 480 the guest equals the host (238 pass, 16 NS). PR 480 is open, so the tree under test is BROKEN | merge PR 480 | IN-TREE FIX (merge PR 480) |
| CT-05 | Vulkan CTS synchronization2 (235 sampled of 81,510) | I | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): without PR 480 14 none_stage depth/stencil cases fail (12 Fail, 2 Timeout) with host Xid 13 3D-Z SYSMEM KIND Violation; with PR 480 the guest equals the host (112 pass, 123 NS). PR 480 is open, so the tree under test is BROKEN | merge PR 480 | IN-TREE FIX (merge PR 480) |
| CT-06 | Vulkan CTS synchronization (249 sampled of 64,872) | I | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): 2 global_priority_transition cases (realtime and high) pass on the host and are NotSupported in the guest (vkCreateDevice -> VK_ERROR_INITIALIZATION_FAILED), reproduced alone in fresh boots; the rest equal (see GX-20) | see GX-20 | OWNER DECISION (see GX-20) |
| CT-07 | Vulkan CTS transform_feedback (269 sampled of 133,719) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 170 pass, 99 NS; guest identical | none | - |
| CT-08 | Vulkan CTS device_group (21 of 21) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 12 pass, 9 NS; guest identical | none | - |
| CT-09 | Vulkan CTS memory (240 sampled of 6,364) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 162 pass, 78 NS; guest 162 pass, 77 NS, 1 Timeout at 60 s (allocation.basic.size_4KiB.mixed.count_4000) which answers NotSupported alone, as the host does; vkAllocateMemory costs 4,931 us in the guest against 154.6 us on the host (performance) | none (performance row) | - |
| CT-10 | Vulkan CTS memory_model (229 sampled of 18,530) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 214 pass, 15 NS; guest identical | none | - |
| CT-11 | Vulkan CTS api (263 sampled of 267,509) | I | BROKEN | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): guest Fail on api.device_init.create_device_global_priority_khr.basic and ..._query_khr.basic (Pass on the host, reproduced alone, see GX-20); api.maintenance3_check.support_count_inline_uniform_block_extra_bindings_create_layout fails on the host too (bucket 3); the rest equal | see GX-20 | OWNER DECISION (see GX-20) |
| CT-12 | Vulkan CTS pipeline (265 sampled of 1,172,232) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 224 pass, 41 NS; guest identical | none | - |
| CT-13 | Vulkan CTS compute (243 sampled of 67,553) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 195 pass, 48 NS; guest identical | none | - |
| CT-14 | Vulkan CTS subgroups (254 sampled of 48,621) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 228 pass, 26 NS; guest identical | none | - |
| CT-15 | Vulkan CTS video (263 sampled of 9,471) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): host 3 pass, 260 NS (the die lacks most of the video features); guest identical | none | - |

### 2.6 Video engines

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

### 2.7 Linux stack, containers, libraries and frameworks

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| LX-01 | Container toolkit and CDI specs (nvidia-ctk cdi generate) in the guest | S10 | UNKNOWN | none | DESIGN-ONLY: generate the spec in a guest with the toolkit installed from the official repository, run a container with --device nvidia.com/gpu=all | IN-TREE FIX |
| LX-02 | nvidia-modprobe and /dev/nvidia* node creation for an unprivileged user | S9 | UNKNOWN | none | DESIGN-ONLY | IN-TREE FIX |
| LX-03 | nvidia-persistenced start, hold and stop with CUDA alive | S9 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_persistenced PASS on host and guest: start, hold with CUDA alive, stop on SIGTERM | none | - |
| LX-04 | OpenCL (clinfo, clpeak) and hashcat | I | WORKS | MEASURED: rows clinfo, clpeak, hashcat in the Linux app matrix (docs/design/V3_APP_MATRIX.md) | none | - |
| LX-05 | CUDA libraries and frameworks: cuBLAS, cuFFT, PyTorch, CuPy, llama.cpp | I | WORKS | MEASURED: rows sgemm_cublas, fft_cufft, torch_correct, torch_ai_bench, cupy, llama_cpp_gen in the Linux app matrix | none | - |
| LX-06 | vLLM, TensorRT and JAX rows | I | UNKNOWN | rows built (scripts/apps/mlrows), first guest run pending | run apps_discovery | IN-TREE FIX |
| LX-07 | Two dies per family (slot pool) | I | UNKNOWN | none in this batch | DESIGN-ONLY: two guests on two dies, then reboot one with a busy GPU and read the census | IN-TREE FIX |

### 2.8 Lifecycle and fault events

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| LF-01 | Unload and reload the nvidia modules in the guest (3 cycles), with and without a CUDA client alive | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_module_cycle PASS in the guest: three cycles, the unload is refused quickly under a live client, the client survives, unload and reload without one work, CUDA passes after each reload | none | - |
| LF-02 | Kill a process in the middle of a kernel (5 rounds), next process must work | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_kill_kernel PASS on host and guest (5 rounds) | none | - |
| LF-03 | Kill an EGL client in the middle of rendering (5 rounds) | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_kill_gl PASS on host and guest (5 rounds, EGL client killed mid-render) | none | - |
| LF-04 | Guest power-off with a busy GPU: the host frees the slot, VA pool and channels | I | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): life_busy_poweroff in the guest (a busy kernel running, then power-off): kf3 retired all 6 channel tokens (released=true, stale binds 0, unresolved 0), no process left, host idle and no new Xid, and the next guest boot on the same die passed its canary and kill probes. VA-pool occupancy was not read | read the VA pool counter after the power-off | - |
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

### 2.9 Windows guest

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

### 2.10 Applications

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

### 2.11 Full suites

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| FS-01 | Vulkan CTS full groups, host versus guest per case | I | UNKNOWN | 291-case subset measured (open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending)); full groups not run | section 4 | IN-TREE FIX |
| FS-02 | CUDA samples, all samples | I | UNKNOWN | the Linux matrix builds a subset for sm_86 only | section 4 | IN-TREE FIX |
| FS-03 | OpenGL CTS (VK-GL-CTS GL targets), headless EGL | I | UNKNOWN | none | section 4 | IN-TREE FIX |
| FS-04 | DirectX 12 samples and conformance | I | UNKNOWN | a DirectX sample row set exists in the open Windows profile | section 4 | IN-TREE FIX |
| FS-05 | FFmpeg NVENC/NVDEC matrix | I | WORKS | MEASURED open-rows guest run 2026-11-17 (kf3 04613288, RTX 3090; evidence PR pending): 47 cases, 38 agree, the 9 differences are explained (session cap, a different ffmpeg build) | none | - |

### 2.12 Interface coverage and fuzzing

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| CV-01 | Interface coverage per suite: which RM classes and controls each suite exercises (trace and ogkm method) | I | UNKNOWN | the method exists; no per-suite coverage table | section 4.5 | IN-TREE FIX |
| CV-02 | Boundary fuzzing of the RM verbs and ioctls a guest can reach | I | UNKNOWN | none | DESIGN-ONLY (section 4.5) | IN-TREE FIX |

### 2.13 Security

| id | feature | source | kayfabe status | evidence | next step | closure path |
|---|---|---|---|---|---|---|
| SE-01 | Guest root cannot change host GPU state (compute mode, clocks, persistence, power limit) | S11 | WORKS | MEASURED gap-probe run 2026-11-18 (kf3 4065c82a, RTX 3090, driver 580.159.04 on both sides; traces/gap_probes_run_20261118): smi_setters with a host watcher: 207 samples at 0.5 s of compute mode, persistence mode, power limit and application clocks show no change on the host while the guest ran every setter (the first watcher included a live clock and cried leak: a harness defect, fixed) | none | - |

## 3. The first batch of probes

### 3.1 What exists

`scripts/apps/gap_probes` holds 46 cases in 13 groups as DATA (`gap_cases.tsv`, one row per case with its falsifier, timeout and tags). Nothing is hard-coded per image: the standalone images are configuration, and no row names one (a test scans the directory). It is a DISCOVERY row of `scripts/apps/run_apps.sh` (name `gap_probes`, in no gate list), so it rides the existing open-guest lane: `apps_hook.sh` samples the host GPU census around the row, `apps_matrix.sh` compares the host log with the guest log.

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
| vulkan_caps | vk_global_priority, vk_dump | a global queue priority (low, medium, high, realtime) that the host reports or accepts in vkCreateDevice is not reported or is refused in the guest | no |
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

GPU-free: `scripts/ci/test_gap_probes.py` (the case table, selection, probe output parsing, process classification, the C judge table compiled with `-DGAP_SELFTEST`, setters, Vulkan flattening, the comparison, runner end to end against fake probe programs, wiring). Mutants: `scripts/ci/gap_probes_mutants.py`, every mutant killed by two named tests. MEASURED: those tests and mutants pass; the C file compiles with `gcc -Wall`; the probes then ran against a real libcuda on the host and in the guest (section 3.4).

### 3.4 Hardware results

MEASURED, one run on one box on 2026-11-18, text evidence in `traces/gap_probes_run_20261118/` (README there). Nothing here is a claim about another die, driver or day.

- **Box.** One Vast KVM VM (nested KVM), RTX 3090 (GA102, 24 GB), Xeon W-2133 VM with 11 vCPU and 98 GB RAM, Ubuntu 22.04, host and guest driver 580.159.04, guest image from the apps stage (fast guest, `KF_DEVICE=kf3`, 16 GB RAM, 6 vCPU). Rented about 02:16 UTC, destroyed after the run (the record of the teardown is in the trace README).
- **Revisions.** kf3 `4065c82a` for every run unless the row says otherwise: the crates are byte-identical to `claude/accept-final-tree-20261118` (`2fab69396`, no PR 480). The probe code (the box checkout of this branch, short revisions) was `a41eaaa2` for H1, `a8545da3` for H1c and G1 to G4, `6345785b` for H2 and G5, and `8aa94098` for H3, G6 and the CTS runs; the box tree moved forward as probes were corrected, and the table below takes the latest run of each case. Runs with the PR 480 control binary use kf3 `8aa94098-dirty` = the same crates plus the diff of PR 480's two commits `e0e6b15d1` and `fa87599a6` (`kf-mem/src/apply.rs` only), built on the box in 40 s.
- **Runs.** H1 host (first probe version: 39 pass, 3 FAIL, 3 not run), H1c host control (fixed probes: 42 pass, 3 guest-only not run; host against host differs only in one performance number), G1 guest (37 pass, 1 NOT_SUPPORTED, 2 FAIL, 5 not run), G2 guest `life_reset`, G3 guest busy power-off, G4 guest kill probes, H2 and G5 host and guest reruns of the cases whose probes had been corrected or whose output was missing, H3 and G6 the Vulkan dump with format-independent tokens, H4 and G7 the global-priority probe.

The per-case table (the latest run of each case on each side; `NOT_RUN` on the host means a guest-only case):

| case | group | host | guest | note |
|---|---|---|---|---|
| attr_dump | cuda_attr | PASS | PASS | memory total and PCI device id differ (listed) |
| canary | cuda_attr | PASS | PASS |  |
| pinned_1g | cuda_pinned | PASS | PASS |  |
| pinned_4g | cuda_pinned | PASS | PASS |  |
| host_register_512m | cuda_pinned | PASS | PASS |  |
| mapped_zerocopy | cuda_pinned | PASS | PASS |  |
| pinned_huge_refused | cuda_pinned | PASS | PASS |  |
| managed_prefetch_advise | cuda_managed | PASS | FAIL | UVM demand paging, documented limitation (CU-13) |
| managed_concurrent | cuda_managed | PASS | PASS |  |
| managed_oversub | cuda_managed | PASS | FAIL | UVM demand paging, documented limitation (CU-13) |
| ipc_legacy | cuda_ipc | PASS | PASS |  |
| ipc_event | cuda_ipc | PASS | PASS |  |
| ipc_vmm_fd | cuda_ipc | PASS | PASS |  |
| vmm_granularity | cuda_vmm | PASS | PASS |  |
| vmm_basic | cuda_vmm | PASS | PASS |  |
| vmm_remap | cuda_vmm | PASS | PASS |  |
| graph_capture_launch | cuda_graph | PASS | PASS | replay 174k/s host, 20.7k/s guest (performance) |
| graph_update | cuda_graph | PASS | PASS |  |
| graph_mem_nodes | cuda_graph | PASS | PASS |  |
| pool_alloc_free | cuda_pool | PASS | PASS |  |
| pool_threshold_trim | cuda_pool | PASS | PASS |  |
| pool_ipc_export | cuda_pool | PASS | PASS |  |
| churn_ctx_200 | cuda_churn | PASS | PASS |  |
| churn_module_200 | cuda_churn | PASS | PASS |  |
| churn_streams_events | cuda_churn | PASS | PASS |  |
| fanout_procs_24 | cuda_churn | PASS | PASS |  |
| spin_5s | cuda_kernel_time | PASS | PASS |  |
| spin_30s | cuda_kernel_time | PASS | PASS |  |
| launch_flood_100k | cuda_kernel_time | PASS | PASS | under 1 s host, about 4 s guest (performance) |
| fault_recovery | cuda_kernel_time | PASS | PASS | first host version failed (harness defect, fixed) |
| trap_recovery | cuda_kernel_time | PASS | PASS | first host version failed (harness defect, fixed) |
| cupti_callback | profiling | PASS | PASS |  |
| cupti_activity | profiling | PASS | PASS |  |
| tools_sanitizer | profiling | PASS | NOT_SUPPORTED | first CUDA call under the tool returns 999 in the guest (INFERRED: debugger stub, ruling H) |
| tools_ncu | profiling | PASS | NOT_RUN | guest: tool not pushed (1.2 GB) |
| tools_nsys | profiling | PASS | NOT_RUN | guest: tool not pushed (1 GB) |
| tools_nsys_gpumetrics | profiling | PASS | NOT_RUN | guest: tool not pushed |
| vk_global_priority | vulkan_caps | PASS | PASS | see section 3.5 (global priority) |
| vk_dump | vulkan_caps | PASS | PASS | two vulkaninfo versions: 250 of 250 extensions equal, 172 shared names equal except 4 format-related (token counts 679 against 1266) |
| smi_setters | smi_control | PASS | PASS | guest answers NOT_SUPPORTED for compute-mode and accounting setters, power and clock fields N/A (listed limitation); no host-visible change (207 samples) |
| life_persistenced | lifecycle | PASS | PASS | first host version failed (harness defect, fixed) |
| life_kill_kernel | lifecycle | PASS | PASS |  |
| life_kill_gl | lifecycle | PASS | PASS |  |
| life_module_cycle | lifecycle | NOT_RUN | PASS | guest only, 3 cycles |
| life_reset | lifecycle | NOT_RUN | PASS | guest only: nvidia-smi -r answers NOT_SUPPORTED rc 3, device usable after |
| life_busy_poweroff | lifecycle | NOT_RUN | NOT_RUN | guest only, terminal: guest dead by design; host retired all channel tokens, next boot passed |

**Differences and their buckets** (the host run is the control for each):

| case | host / guest | bucket | basis |
|---|---|---|---|
| managed_prefetch_advise, managed_oversub | PASS / FAIL (cuCtxSynchronize), host Xid 31 and 13 on the qemu pid in the guest window | 1, documented | UVM demand paging is a documented limitation (README, ruling E); CU-13 |
| attr_dump | PASS / PASS, memory total 24124 against 7871 MiB and PCI device id differ; every other CUDA attribute equal | 1, documented (listed in `gap_known_limits.tsv`) | the guest reports the frame buffer reserved for it and its own virtual PCI topology |
| smi_setters | PASS / PASS, setters differ: compute mode and accounting OK on the host, NOT_SUPPORTED in the guest; power and clock fields N/A in the guest | 1, documented (listed) | ruling H: a stub answers as a GPU without the feature; telemetry N/A, `V3_UTILIZATION_FLAG.md` section 10. No change was visible on the host: 207 samples at 0.5 s of compute mode, persistence mode, power limit and application clocks |
| tools_sanitizer | PASS / NOT_SUPPORTED: under the tool the first CUDA call of the target returns 999 in the guest (the tool says the target application returned an error) | 1, INFERRED (listed as such) | ruling H stubs the debugger; the mechanism is not shown |
| tools_ncu, tools_nsys, tools_nsys_gpumetrics | PASS / NOT_RUN | 4, our configuration | the tools (1.2 GB and 1 GB) were not pushed into the guest; the host runs pass. The compute-sanitizer tools (38 MB) and libcupti were pushed |
| vk_dump | PASS / PASS, token digests differ | 4, our configuration | the host's vulkaninfo is an older version than the guest's and dumps fewer properties; 250 of 250 extensions are equal and 172 shared names are equal but for 4 format-related ones. A pinned single tool is the next step |
| graph_capture_launch, launch_flood_100k | PASS / PASS, a captured graph replays at 174k/s on the host and 20.7k/s in the guest; 100000 launches take under 1 s and about 4 s | none (performance) | kept as `perf.*` evidence, never a difference |
| fault_recovery, trap_recovery, life_persistenced (first host version) | FAIL on the host control | 3, harness defect | the host control failed the same case: a sticky fault poisons its own process (recovery is now judged across processes) and the daemon forks (now found by its kernel name). Caught by the host control before any guest claim |
| smi_setters watcher (first version) | HOST_STATE_LEAK=1 | 3, harness defect | the watcher read a live clock; with the four control fields only, 207 samples show one distinct state |
| vk_global_priority | PASS / PASS, create results differ: in the guest vkCreateDevice with LOW, HIGH or REALTIME fails (-3) on every family, only MEDIUM works | none yet, candidate | section 3.5 |
| life_reset | guest only: NOT_SUPPORTED rc 3, device usable after | N-A-BY-DESIGN (TL-12) | a guest-triggered GPU reset is a privileged host action |
| life_busy_poweroff | guest only: guest dead by design | none | kf3 retired all 6 channel tokens (released=true, stale binds 0, unresolved 0), no process left, host idle, no new Xid, the next guest boot on the same die passed |

**kf3 census per guest run** (from the harness's per-row line, `traces/gap_probes_run_20261118/gap/census_rows.tsv`): guest Xid 0 in every run; kf3 refusals 29 to 82 per run (the counter counts every kf3 line containing `refus`, floor about 15 to 50); `unserviced_in_row`: G1 2975, G2 61, G3 33, G4 353, G5 520, all of one id (`code:76`, the unserviced GSP RM control already known from the open-rows run). Host Xid during the gap probes: Xid 43 (host control faults, expected), and Xid 31 and 13 with the qemu pid while the guest ran the two managed cases.

### 3.5 Vulkan CTS groups and the global-priority lead (MEASURED, same box and day)

**What ran.** The Vulkan CTS (`deqp-vk`, pinned `vulkan-cts-1.4.7.1`, commit `0d18b347943e5e843529a05c95220e27fe30c538`, built on the box) over a fixed-seed, breadth-first sample of the main mustpass (`scripts/apps/gap_probes/cts_groups.py`; each group is split by the next two name components and one case is taken from each split in turn). Sample 1: seed 20261118, 70 cases per group of sparse_resources, ray_tracing_pipeline, ray_query, mesh_shader, synchronization2, synchronization, transform_feedback, device_group (all 21), memory, memory_model, api, pipeline, compute, subgroups, video = 1,001 cases. Sample 2: seed 20261119, up to 200 more per group (sparse_resources left out, because its open failures are known) = 2,621 cases; because of a defect of mine in the deduplication 301 of them repeat sample 1 and ran twice with equal results, so **3,323 distinct cases** (3,321 and the 2 info cases) were compared. Each side runs the same list with `vkcts.py run`: host first (0 Fail except one case that fails on the host too), then the guest, 60 s per case in the guest (90 s on the host), in chunks of at most 380 cases (the row refuses a list over 400). The sample is a few tenths of a percent of each group (mustpass sizes in `cts_groups.py`: for example api 267,509 cases, pipeline 1,172,232, transform_feedback 133,719): it is a sample, never a full run, and the unreached part of every group is the rest.

**Without and with PR 480.** Chunk 1 (382 cases: sparse_resources 70, ray_tracing_pipeline 70, ray_query 70, mesh_shader 70, synchronization2 70, 30 synchronization) ran in the guest twice: on kf3 `4065c82a` (no PR 480) and on `8aa94098-dirty` (PR 480's two commits applied, see section 3.4). Every later guest run used the PR 480 binary. Host Xid lines in the two windows: without PR 480 Xid 13 (3D-Z SYSMEM KIND Violation) 141, Xid 109 12, Xid 31 1; with it Xid 13 0, Xid 109 10, Xid 31 1.

| chunk 1, guest | pass | NS | Fail | Timeout |
|---|---|---|---|---|
| host (control) | 291 | 91 | 0 | 0 |
| without PR 480 | 262 | 92 | 16 | 12 |
| with PR 480 | 279 | 92 | 1 | 10 |

**Flip list (17 cases go from Fail or Timeout to Pass with PR 480; all depth or stencil):** mesh_shader.ext.smoke.optimized_lib.depth_only_points_position_components, mesh_shader.ext.smoke.optimized_lib.partial_usage_without_compaction, mesh_shader.ext.smoke.shader_objects.depth_only_triangles (Timeouts), and 14 synchronization2.none_stage cases: depth_stencil_attachment_to_depth_attachment_stencil_read, depth_stencil_attachment_to_generic_depth_stencil_read, generic_depth_stencil_attachment_to_depth_attachment_stencil_read, generic_stencil_attachment_to_transfer_src, legacy_depth_stencil_attachment_to_depth_attachment_stencil_read, legacy_general_to_stencil_read, legacy_generic_depth_stencil_attachment_to_generic_color_read, legacy_stencil_attachment_to_shader_read, legacy_stencil_attachment_to_stencil_read, old_access_general_to_depth_attachment_stencil_read (Timeout), old_access_generic_depth_stencil_attachment_to_generic_color_read, old_access_generic_depth_stencil_attachment_to_transfer_src, old_access_stencil_attachment_to_generic_color_read, old_access_transfer_dst_to_depth_attachment_stencil_read (Timeout). The list is in `traces/gap_probes_run_20261118/cts/flips_pr480.txt`. PR 480 is open, so these rows stay BROKEN for the tree under test.

**Per group, all samples, guest on the PR 480 binary** (`cts/report_all_with_pr480.txt`; host pass/NS/Fail against guest pass/NS/Fail/Timeout):

| group | host | guest |
|---|---|---|
| sparse_resources | pass=53 not_supported=17 | pass=42 not_supported=17 fail=1 timeout=10 |
| ray_tracing_pipeline | pass=205 not_supported=29 | pass=204 not_supported=29 timeout=1 |
| ray_query | pass=217 not_supported=15 | pass=217 not_supported=15 |
| mesh_shader | pass=238 not_supported=16 | pass=238 not_supported=16 |
| synchronization2 | pass=112 not_supported=123 | pass=112 not_supported=123 |
| synchronization | pass=19 not_supported=230 | pass=17 not_supported=232 |
| transform_feedback | pass=170 not_supported=99 | pass=170 not_supported=99 |
| device_group | pass=12 not_supported=9 | pass=12 not_supported=9 |
| memory | pass=162 not_supported=78 | pass=162 not_supported=77 timeout=1 |
| memory_model | pass=214 not_supported=15 | pass=214 not_supported=15 |
| api | pass=94 not_supported=168 fail=1 | pass=92 not_supported=168 fail=3 |
| pipeline | pass=224 not_supported=41 | pass=224 not_supported=41 |
| compute | pass=195 not_supported=48 | pass=195 not_supported=48 |
| subgroups | pass=228 not_supported=26 | pass=228 not_supported=26 |
| video | pass=3 not_supported=260 | pass=3 not_supported=260 |

**Still different with PR 480 (18 cases, `cts/report_all_with_pr480.txt`)**, with the confirmation of each non-sparse one alone in a fresh guest boot (240 s per case; the host passes them twice):

| case | host / guest | alone in a fresh guest boot | reading |
|---|---|---|---|
| 11 sparse_resources cases (10 Timeout, 1 Fail: cube_sparse_read.r32i) | Pass / Timeout or Fail | not repeated (known open row, reproducible before) | CT-01 / GX-06; host Xid 109 channel 0x31 and Xid 31 |
| api.device_init.create_device_global_priority_khr.basic | Pass / Fail | Fail | global priority, GX-20 |
| api.device_init.create_device_global_priority_query_khr.basic | Pass / Fail | Fail | GX-20 |
| synchronization.global_priority_transition.high.no_sync.sparse.from_graphics_to_compute | Pass / NotSupported | NotSupported | GX-20 |
| synchronization.global_priority_transition.realtime.semaphore.no_modifiers.from_compute_to_transfer | Pass / NotSupported | NotSupported | GX-20 |
| ray_tracing_pipeline.misc.recursiveTraces_AABB_15 | Pass / Timeout (60 s) | Pass | slowness, not a hang |
| memory.allocation.basic.size_4KiB.mixed.count_4000 | NotSupported / Timeout (60 s) | NotSupported (as the host) | slowness: 4000 live allocations cost 19.7 s in the guest against 0.6 s on the host |
| api.maintenance3_check.support_count_inline_uniform_block_extra_bindings_create_layout | Fail / Fail | not repeated | fails on the host too: bucket 3 |

**The global-priority lead (GX-20), what was measured.**

1. `gap_vkprio` (a small C program, `vk_global_priority` case) on host and guest: the guest reports the host's priorities (family 0: LOW, MEDIUM, HIGH, REALTIME; families 1, 3 and 4: LOW, MEDIUM, HIGH; the host also reports REALTIME on families 2 and 5) but `vkCreateDevice` with a non-default global priority fails with VK_ERROR_INITIALIZATION_FAILED (-3) for LOW, HIGH and REALTIME on every family, and succeeds only for MEDIUM (the default). The host accepts every priority it reports and answers VK_ERROR_NOT_PERMITTED_KHR (-1000174001) for REALTIME only where it does not report it.
2. The process is the same on both sides: uid 0 and the capability sets CapEff = CapBnd = `000001ffffffffff`; a missing CAP_SYS_NICE is not the difference.
3. kf3 log delta: in each of the four failing confirm boots exactly one refused RM control is absent from the neighbour boot (the two info cases only): `GSP REFUSED fn76/0x20801115=0x56`, which is `NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY` (the runlist scheduling policy, a host-wide setting; `docs/reference/IMPLEMENTATION_SURFACE.md` names it); the other refused kinds are identical (`confirm/kf3_refused_delta.txt`). INFERRED: that the driver's non-default queue priority path issues this control and fails when it is refused; the falsifier is that answering it (a policy choice for the owner: a host-wide setting must not come from guest bytes, and accepting it silently as a default-policy state is the ruling-S option) makes the four cases pass.
4. The exact failing call is `vkCreateDevice` (the CTS reports `createUncheckedDevice ... VK_ERROR_INITIALIZATION_FAILED`).

**Performance observations (evidence, not defects):** vkAllocateMemory 154.6 us on the host against 4,931 us in the guest (4000 live 4 KiB allocations: 0.6 s against 19.7 s); a captured CUDA graph replays 174k times per second on the host and 20.7k in the guest; 100000 launches take under 1 s on the host and about 4 s in the guest.

### 3.6 Probes not authored (DESIGN-ONLY rows in the matrix)

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
