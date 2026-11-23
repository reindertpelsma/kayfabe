# docs/design/ index

**STATUS: LIVE, 2026-11-23 (generated once from each document's own STATUS line, not kept in sync).** A finding aid for the design documents. The authority is the `STATUS` line at the top of each document; where this index and a document disagree, the document wins. `scripts/ci/doc_status_check.py` checks that every design document has a dated STATUS; `scripts/ci/doc_freshness.py` reports old LIVE documents with dead references.

Start with [`THE_ARCHITECTURE_v3.md`](THE_ARCHITECTURE_v3.md) (the architecture), [`THE_V3_PLAN.md`](THE_V3_PLAN.md) (the phased plan), [`V3_BUILD.md`](V3_BUILD.md) (execution rules) and [`THE_CONSTRAINTS.md`](THE_CONSTRAINTS.md) (the owner's constraints). Owner decisions are in [`../OWNER_RULINGS.md`](../OWNER_RULINGS.md) and win over any design text.

Meaning of the status words: LIVE describes the current tree (as of the document's date); DESIGN-ONLY is intent that is not built; ANSWERED is a question that has an answer and is kept for it; RESEARCH is an investigation, not a description of the product; SUPERSEDED is history and links its successor.

## LIVE (91)

| Document | Title | Date in STATUS |
|---|---|---|
| [`OWNER_DIRECTIVES_RECOVERY_20261008.md`](OWNER_DIRECTIVES_RECOVERY_20261008.md) | Recovered owner directives: what was kept and what was dropped (2026-10-08) | 2026-10-08 |
| [`THE_ARCHITECTURE_v3.md`](THE_ARCHITECTURE_v3.md) | kayfabe v3 — the architecture after the owner's review | 2026-10-28 |
| [`THE_CONSTRAINTS.md`](THE_CONSTRAINTS.md) | THE CONSTRAINTS — what kayfabe must do, not just do | 2026-11-07 |
| [`THE_DESIGN.md`](THE_DESIGN.md) | kayfabe — the design | 2026-11-20 |
| [`THE_OPEN_QUESTIONS.md`](THE_OPEN_QUESTIONS.md) | What is still open | 2026-09-20 |
| [`THE_OVERNIGHT_DIRECTIVE.md`](THE_OVERNIGHT_DIRECTIVE.md) | ★★★★★ THE OWNER'S OVERNIGHT DIRECTIVE — 2026-09-13 | 2026-11-20 |
| [`THE_PDF_FRONT_FINAL.md`](THE_PDF_FRONT_FINAL.md) | How to read this | 2026-09-21 |
| [`V3_APP_LANE_PLANNER.md`](V3_APP_LANE_PLANNER.md) | The app lane planner | 2026-11-18 |
| [`V3_APP_MATRIX.md`](V3_APP_MATRIX.md) | V3 app matrix — which real CUDA apps work in a kayfabe v3 fat guest | 2026-11-07 |
| [`V3_BAR0_TRACE_MODE.md`](V3_BAR0_TRACE_MODE.md) | V3 — the BAR0 trace mode: kf3 recorded by the VFIO reference's own tracer | 2026-10-09 |
| [`V3_BAR1_DOORBELL.md`](V3_BAR1_DOORBELL.md) | V3 — the Hopper+ BAR1 doorbell: follow where the guest RM puts it | 2026-11-07 |
| [`V3_BLACKWELL_FIELD_PHASE_20261022.md`](V3_BLACKWELL_FIELD_PHASE_20261022.md) | V3_BLACKWELL_FIELD_PHASE_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_GUARD_REVIEW_20261022.md`](V3_BLACKWELL_GUARD_REVIEW_20261022.md) | V3_BLACKWELL_GUARD_REVIEW_20261022.md | 2026-10-22 |
| [`V3_BOUNDARY_FUZZING.md`](V3_BOUNDARY_FUZZING.md) | V3 — boundary fuzzing: GPU-free property tests at the guest-to-host boundary | 2026-11-18 |
| [`V3_BROKER_XOR_CURSOR_20261017.md`](V3_BROKER_XOR_CURSOR_20261017.md) | V3 broker — the true XOR cursor, composed by the broker (D2), and the cursor size rule | 2026-10-17 |
| [`V3_BUILD.md`](V3_BUILD.md) | V3_BUILD — how v3 is being built (branch `v3`) | 2026-11-20 |
| [`V3_CHANNEL_BUDGET.md`](V3_CHANNEL_BUDGET.md) | V3 channel budget — the count the guest is told is the cap kayfabe enforces | 2026-10-11 |
| [`V3_CI_VAST_FIXES_20261023.md`](V3_CI_VAST_FIXES_20261023.md) | Worker boundary and independent cleanup | 2026-10-23 |
| [`V3_CI_VAST_GPU_20261021.md`](V3_CI_VAST_GPU_20261021.md) | Vast GPU tripwire with a fixed paid-account capability | 2026-11-04 |
| [`V3_CI_VAST_RUNNER_TRACE_20261022.md`](V3_CI_VAST_RUNNER_TRACE_20261022.md) | Approved code, untrusted GPU tripwire, GitHub build provenance | 2026-10-22 |
| [`V3_CLOCK_RULE.md`](V3_CLOCK_RULE.md) | V3: the guest clock rule (network decides; a recorded clock is never earlier than loaded software) | 2026-11-16 |
| [`V3_CONSTANT_COVERAGE_20261022.md`](V3_CONSTANT_COVERAGE_20261022.md) | V3 constants coverage: same-die evidence and remaining gaps | 2026-10-22 |
| [`V3_CONSTANT_INVENTORY_20261011.md`](V3_CONSTANT_INVENTORY_20261011.md) | V3 constant inventory: guest-visible values answered from something other than ogkm, kayfabe state or a host q | 2026-10-11 |
| [`V3_CONSTANT_LISTS_20261011.md`](V3_CONSTANT_LISTS_20261011.md) | V3 constants: the three lists (obtainable / measure / reverse engineering) | 2026-10-11 |
| [`V3_CONSTANT_STRATEGY.md`](V3_CONSTANT_STRATEGY.md) | V3 per-die and per-driver constants: the strategy | 2026-10-22 |
| [`V3_DEVICE_RESET.md`](V3_DEVICE_RESET.md) | V3 — the device reset: every guest life starts on the device realize left | 2026-10-13 |
| [`V3_DISPLAY.md`](V3_DISPLAY.md) | V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe | 2026-10-18 |
| [`V3_DISPLAY_CURSOR_FORMATS.md`](V3_DISPLAY_CURSOR_FORMATS.md) | V3 display — cursor formats: the Windows I-beam (A1R5G5B5) | 2026-10-15 |
| [`V3_DISPLAY_PACING_20261017.md`](V3_DISPLAY_PACING_20261017.md) | V3 display pacing — the guest's flips are never parked; copies are bounded; who ticks | 2026-11-07 |
| [`V3_DISPLAY_PLANES_ORACLE.md`](V3_DISPLAY_PLANES_ORACLE.md) | V3 display — the Linux plane oracle | 2026-10-14 |
| [`V3_DOORBELL_IOEVENTFD.md`](V3_DOORBELL_IOEVENTFD.md) | V3 — the doorbell fast path: one KVM ioeventfd per live token, serviced by the register drainer | 2026-11-07 |
| [`V3_DRIVER_MATRIX.md`](V3_DRIVER_MATRIX.md) | V3 DRIVER MATRIX — both driver axes, measured per ogkm tag | 2026-10-24 |
| [`V3_FAMILY_COVERAGE.md`](V3_FAMILY_COVERAGE.md) | GPU family coverage and archived evidence | 2026-10-27 |
| [`V3_FEATURE_GAPS.md`](V3_FEATURE_GAPS.md) | V3 feature gaps: what kayfabe lacks, ranked, with a plan | 2026-10-14 |
| [`V3_FIX_TEST_AUDIT_20261113.md`](V3_FIX_TEST_AUDIT_20261113.md) | Fix test audit, 2026-11-13 | 2026-11-13 |
| [`V3_FN1_NEGOTIATION_ORACLE.md`](V3_FN1_NEGOTIATION_ORACLE.md) | V3 fn-1 (SET_GUEST_SYSTEM_INFO) negotiation oracle | 2026-10-13 |
| [`V3_GPU_USE_ASSERTION.md`](V3_GPU_USE_ASSERTION.md) | GPU use is asserted, never assumed: the audit of every row that can run on the CPU, and the shared judge | 2026-11-17 |
| [`V3_GPU_UUID.md`](V3_GPU_UUID.md) | Per-VM GPU UUID (`gpu-uuid=`) | 2026-10-08 |
| [`V3_GUEST_INFLUENCE_AUDIT.md`](V3_GUEST_INFLUENCE_AUDIT.md) | V3 guest-influence audit: host-side paths a guest can steer, and their bounds | 2026-11-20 |
| [`V3_GUEST_INGRESS_TABLE.md`](V3_GUEST_INGRESS_TABLE.md) | V3 guest-ingress table: the control/class allowlist, generated from ogkm (task I7) | 2026-11-20 |
| [`V3_H5_USERD_DMA_PATCH.md`](V3_H5_USERD_DMA_PATCH.md) | V3 — H5: the USERD-in-sysmem DMA-address fix (host `nvidia.ko` patch) | 2026-10-10 |
| [`V3_HELD_HOLE_HANDOFF_20261010.md`](V3_HELD_HOLE_HANDOFF_20261010.md) | V3 — the HELD-hole (micro-reservation geometry): fix, evidence, handoff | 2026-10-10 |
| [`V3_KFROOT_AUDIT.md`](V3_KFROOT_AUDIT.md) | V3: the audit of every `kfroot` step (are you really sure a user has to run this?) | 2026-11-20 |
| [`V3_KFROOT_ROOTLESS.md`](V3_KFROOT_ROOTLESS.md) | V3: the Windows gate and the box harness without root, and the explicit `kfroot` step | 2026-11-15 |
| [`V3_LATE_TSG_JOINER.md`](V3_LATE_TSG_JOINER.md) | V3 — the late TSG joiner: a twin born after the guest's one schedule | 2026-10-10 |
| [`V3_LINUX_PERF_BASELINE_6692e621.md`](V3_LINUX_PERF_BASELINE_6692e621.md) | V3_LINUX_PERF_BASELINE_6692e621 - the Linux performance baseline before the Windows integration | 2026-10-10 |
| [`V3_MULTI_GPU_AUDIT.md`](V3_MULTI_GPU_AUDIT.md) | V3 multi-GPU audit — is it still possible, and is it required? | 2026-11-20 |
| [`V3_NOT_IN_GIT.md`](V3_NOT_IN_GIT.md) | V3 — what is intentionally NOT in git | 2026-10-10 |
| [`V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md`](V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md) | V3 open lanes: nvbandwidth, llama.cpp on Vulkan, LibreHardwareMonitor (NVAPI) | 2026-11-23 |
| [`V3_OPEN_ML_ROWS.md`](V3_OPEN_ML_ROWS.md) | V3 open ML rows: vLLM, TensorRT and JAX / XLA (discovery lane) | 2026-11-17 |
| [`V3_OPEN_ROWS_FFMPEG_DCGM.md`](V3_OPEN_ROWS_FFMPEG_DCGM.md) | V3 open rows: the FFmpeg NVENC/NVDEC capability matrix and the DCGM + NVML sweep | 2026-11-17 |
| [`V3_P1P2_TSPACE.md`](V3_P1P2_TSPACE.md) | P1 + P2: no window in any space an unprivileged guest channel uses; Translated work in its own space | 2026-10-10 |
| [`V3_PARALLEL_FETCH.md`](V3_PARALLEL_FETCH.md) | Parallel CI downloads | 2026-11-12 |
| [`V3_PINS_ARE_DEPLOYMENT_DATA.md`](V3_PINS_ARE_DEPLOYMENT_DATA.md) | Pins are deployment data: identity pins are strict-only, a standalone run records, and anyone can run their ow | 2026-11-17 |
| [`V3_PROPIMAGE_CONFIG.md`](V3_PROPIMAGE_CONFIG.md) | V3: propimages are configuration (bundles, the resolver, the generic stage) | 2026-11-19 |
| [`V3_PROVENANCE.md`](V3_PROVENANCE.md) | V3 provenance: where kayfabe's data and interfaces come from | 2026-10-11 |
| [`V3_QUEUE_PRIORITY.md`](V3_QUEUE_PRIORITY.md) | V3 queue priority: `VK_KHR_global_priority` in the guest | 2026-11-18 |
| [`V3_RECOVERY_WALL.md`](V3_RECOVERY_WALL.md) | V3 — the recovery wall: the System process (pid 4) as the kernel driver's process | 2026-10-09 |
| [`V3_REPLAY_TESTS.md`](V3_REPLAY_TESTS.md) | V3 — replay tests: a logged failure becomes a GPU-free Rust test | 2026-11-21 |
| [`V3_S3_RESUME.md`](V3_S3_RESUME.md) | V3 — ACPI S3 suspend/resume of a kf3 guest | 2026-10-14 |
| [`V3_SECURITY_MODEL.md`](V3_SECURITY_MODEL.md) | V3 security model | 2026-10-03 |
| [`V3_SPARSE_RESIDENCY_RCA.md`](V3_SPARSE_RESIDENCY_RCA.md) | V3 — sparse residency: why residency cases hang in the guest (root cause, GPU-free reproduction) | 2026-11-18 |
| [`V3_STANDALONE_FULL_RUN.md`](V3_STANDALONE_FULL_RUN.md) | The standalone full run: what it covers, what each part needs, what cannot run | 2026-11-20 |
| [`V3_STANDALONE_TRUST.md`](V3_STANDALONE_TRUST.md) | Standalone trust: what a repo-defined download is anchored by, and what a third party has to trust | 2026-11-17 |
| [`V3_TRANSLATED_RC_20261022.md`](V3_TRANSLATED_RC_20261022.md) | V3_TRANSLATED_RC_20261022.md | 2026-10-22 |
| [`V3_TSPACE_HARDWIRED.md`](V3_TSPACE_HARDWIRED.md) | The T-space is hardwired: what was deleted, what rule 4 delivers, what hardware must show | 2026-10-10 |
| [`V3_USERD_RELAY.md`](V3_USERD_RELAY.md) | V3 USERD relay — a Passthrough twin whose USERD the host cannot adopt | 2026-10-08 |
| [`V3_USER_DESCRIPTOR_DECISION_20261022.md`](V3_USER_DESCRIPTOR_DECISION_20261022.md) | Oversized USER descriptors: the decision | 2026-11-12 |
| [`V3_USER_DESCRIPTOR_PORT_20261109.md`](V3_USER_DESCRIPTOR_PORT_20261109.md) | USER descriptor admission on next | 2026-11-09 |
| [`V3_USER_RING_EVENT_LOOP_20261014.md`](V3_USER_RING_EVENT_LOOP_20261014.md) | V3 — the USER-ring relay as an event loop: no timer carries correctness | 2026-11-07 |
| [`V3_USER_RING_LIFECYCLE_20261016.md`](V3_USER_RING_LIFECYCLE_20261016.md) | V3 — the Windows USER-ring lifecycle: Scan, Pointer, Window | 2026-10-17 |
| [`V3_USER_RING_SCAN_RACE.md`](V3_USER_RING_SCAN_RACE.md) | V3 — the Windows USER-ring relay: reading too early, and not reading at all | 2026-10-14 |
| [`V3_USER_STORE_VIEWS_20261015.md`](V3_USER_STORE_VIEWS_20261015.md) | V3 — USER-ring store views: keep them armed (the D3D11 view churn) | 2026-10-15 |
| [`V3_UTILIZATION_FLAG.md`](V3_UTILIZATION_FLAG.md) | Guest GPU utilisation: the `report-utilization` device property (design note) | 2026-11-17 |
| [`V3_VA_PROFILE.md`](V3_VA_PROFILE.md) | V3 — bounded phase profiling of the VA apply path | 2026-11-16 |
| [`V3_VIDEO_ENGINES.md`](V3_VIDEO_ENGINES.md) | V3 video engines — NVENC and NVDEC for guest userspace | 2026-11-15 |
| [`V3_VULKAN_CTS_ROW.md`](V3_VULKAN_CTS_ROW.md) | V3 Vulkan CTS row (open Linux row, `deqp-vk`, discovery lane) | 2026-11-17 |
| [`V3_WINDOWS_APP_MATRIX.md`](V3_WINDOWS_APP_MATRIX.md) | V3 Windows app matrix — the Linux app matrix on a Windows guest | 2026-11-07 |
| [`V3_WINDOWS_BRING_UP_20261009.md`](V3_WINDOWS_BRING_UP_20261009.md) | V3 WINDOWS BRING-UP — what hung it, what fixed it, what is left (2026-10-09) | 2026-10-09 |
| [`V3_WINDOWS_CAPTURE_MODULE.md`](V3_WINDOWS_CAPTURE_MODULE.md) | V3: the Windows GSP capture module (`tools/windows-capture/`, "kfwc") | 2026-10-14 |
| [`V3_WINDOWS_DISCOVERY.md`](V3_WINDOWS_DISCOVERY.md) | V3 — the first Windows guest on kf3 (discovery run, `vwin`) | 2026-10-04 |
| [`V3_WINDOWS_GATE.md`](V3_WINDOWS_GATE.md) | V3: the Windows stage of the merge bar (`KF_MERGE_WINDOWS=1`, `windows_gate.sh`) | 2026-11-09 |
| [`V3_WINDOWS_ONLY_CONSTANTS_20261013.md`](V3_WINDOWS_ONLY_CONSTANTS_20261013.md) | V3: constants no ogkm tag defines (the Windows-capture list) | 2026-10-13 |
| [`V3_WINDOWS_SUPPORT_PROFILE.md`](V3_WINDOWS_SUPPORT_PROFILE.md) | V3 Windows support profile: one switch for the Windows-only capabilities | 2026-11-15 |
| [`V3_WINDOWS_TIMER_MAPPING.md`](V3_WINDOWS_TIMER_MAPPING.md) | Optional native timer mapping for the Windows experiment | 2026-10-11 |
| [`VAST_SETUP_CACHE_20261023.md`](VAST_SETUP_CACHE_20261023.md) | Vast setup dependency cache — V8 | 2026-10-23 |
| [`bar1_simultaneous_view_ceiling.md`](bar1_simultaneous_view_ceiling.md) | BAR1: how much of one reserved vidmem object can be CPU-mapped at once | 2026-11-14 |
| [`gpga_is_one_reserved_object.md`](gpga_is_one_reserved_object.md) | GPGA is ONE reserved object | 2026-11-10 |
| [`the_all_dma_baseline.md`](the_all_dma_baseline.md) | The all-DMA baseline — what passed while guest "vidmem" was host memory | 2026-09-14 |
| [`the_aperture_store_lifetime.md`](the_aperture_store_lifetime.md) | The aperture store: when to punch, how big it gets, and what exhaustion does | 2026-11-14 |
| [`the_three_channel_kinds.md`](the_three_channel_kinds.md) | The three channel kinds — passthrough, translated, emulated | 2026-10-28 |
| [`the_write_trap_contract.md`](the_write_trap_contract.md) | THE WRITE-TRAP CONTRACT — what a vCPU may do inside an MMIO exit | 2026-09-21 |

## DESIGN-ONLY (46)

| Document | Title | Date in STATUS |
|---|---|---|
| [`V3_APPS_IMAGE_RECIPE.md`](V3_APPS_IMAGE_RECIPE.md) | Linux apps image recipe | 2026-11-08 |
| [`V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md`](V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md) | V3_BLACKWELL_HOST_PRECOMP_VALIDATION_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_OTHER_DIES_PLAN_20261022.md`](V3_BLACKWELL_OTHER_DIES_PLAN_20261022.md) | V3_BLACKWELL_OTHER_DIES_PLAN_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_PRECOMP_CONSTTAB_20261023.md`](V3_BLACKWELL_PRECOMP_CONSTTAB_20261023.md) | V3_BLACKWELL_PRECOMP_CONSTTAB_20261023.md | 2026-10-23 |
| [`V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md`](V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md) | Box downloader and build supply chain | 2026-11-13 |
| [`V3_BROKER_ROADMAP_20261015.md`](V3_BROKER_ROADMAP_20261015.md) | V3_BROKER_ROADMAP_20261015 — what the broker becomes, and what kayfabe has to wire for it | 2026-10-15 |
| [`V3_CI_TIERS_AND_IGNORE_LADDER.md`](V3_CI_TIERS_AND_IGNORE_LADDER.md) | CI tiers, the admin start route, the tree-hash key and the ignore ladder | 2026-11-12 |
| [`V3_COOPERATIVE_TIERS.md`](V3_COOPERATIVE_TIERS.md) | V3 — the stock tier and the opt-in cooperation stages | 2026-10-03 |
| [`V3_CUSTOM_HOSTS.md`](V3_CUSTOM_HOSTS.md) | Custom hosts: Worker, daemon and seeder contract | 2026-11-12 |
| [`V3_DRIVER_AVAILABILITY.md`](V3_DRIVER_AVAILABILITY.md) | NVIDIA release driver availability and selection | 2026-11-08 |
| [`V3_GPU_FAIR_SHARE.md`](V3_GPU_FAIR_SHARE.md) | V3 — fair GPU sharing between VMs ("vGPU-like" scheduling) | 2026-10-02 |
| [`V3_GSP_CAPTURE_FORMAT.md`](V3_GSP_CAPTURE_FORMAT.md) | V3: the GSP queue capture format (`KFGSPCAP`) and the census tool | 2026-10-13 |
| [`V3_GUEST_DOORBELL_MODULE.md`](V3_GUEST_DOORBELL_MODULE.md) | V3 — optional guest doorbell helper module | 2026-09-28 |
| [`V3_HOST_PATCH_LIST.md`](V3_HOST_PATCH_LIST.md) | V3 — the host patch list: every optional host-kernel/driver enhancement, in one plan | 2026-10-11 |
| [`V3_KFCI_STEPS.md`](V3_KFCI_STEPS.md) | Python CI steps and migration contract | 2026-11-08 |
| [`V3_MIDDLE_REAL_APPS.md`](V3_MIDDLE_REAL_APPS.md) | Middle-tier real applications | 2026-11-09 |
| [`V3_NON_GSP_CTX.md`](V3_NON_GSP_CTX.md) | V3 NON-GSP CONTEXT — what a non-GSP guest does with the golden context, and what we would serve | 2026-09-25 |
| [`V3_PACKAGING_AND_DISTRIBUTION.md`](V3_PACKAGING_AND_DISTRIBUTION.md) | Packaging and distribution: the QEMU release, the build kit, the command-line surface, trust and the manual | 2026-11-20 |
| [`V3_PLATFORM_TESTING.md`](V3_PLATFORM_TESTING.md) | Platform testing: a coverage service behind CI, and a public status site | 2026-11-06 |
| [`V3_PROPIMAGE_APPS_STAGE.md`](V3_PROPIMAGE_APPS_STAGE.md) | The `propimage_apps` payload stage | 2026-11-15 |
| [`V3_PROPIMAGE_APP_AUTOMATION.md`](V3_PROPIMAGE_APP_AUTOMATION.md) | One proprietary-image check pipeline | 2026-11-13 |
| [`V3_PROPIMAGE_B_APPS.md`](V3_PROPIMAGE_B_APPS.md) | Application rows for the second proprietary image | 2026-11-16 |
| [`V3_PROPIMAGE_LINUX.md`](V3_PROPIMAGE_LINUX.md) | A Linux proprietary image (image C): design and first slice | 2026-11-16 |
| [`V3_PROPIMAGE_RUNNER.md`](V3_PROPIMAGE_RUNNER.md) | The generic image runner: any number of Windows images, their applications, and the clock/network rule | 2026-11-16 |
| [`V3_RAW_CLIENT.md`](V3_RAW_CLIENT.md) | V3 RAW CLIENT — the grader moves into the live tree; the refresh stress is its first new arm | 2026-10-31 |
| [`V3_RELEASE_MATRIX.md`](V3_RELEASE_MATRIX.md) | Release matrix: dies, die families and driver families | 2026-11-12 |
| [`V3_RELEASE_PLANNER.md`](V3_RELEASE_PLANNER.md) | Flat release requirements and executor assignments | 2026-11-08 |
| [`V3_RELEASE_REHEARSAL.md`](V3_RELEASE_REHEARSAL.md) | Release rehearsal: the large tier on one die, through the Worker, without a tag | 2026-11-12 |
| [`V3_RELEASE_RESILIENCE.md`](V3_RELEASE_RESILIENCE.md) | Release resilience: a failed rental is replaced, it never means the test failed | 2026-11-12 |
| [`V3_RELEASE_SOFTWARE_WISHLIST.md`](V3_RELEASE_SOFTWARE_WISHLIST.md) | Release software wishlist and image-size planning (SPEC excluded) | 2026-11-07 |
| [`V3_RESOLVE_GPU_LANE.md`](V3_RESOLVE_GPU_LANE.md) | DaVinci Resolve in image A: real GPU-rendering coverage with a GPU-use assertion | 2026-11-16 |
| [`V3_SECURITY_AUDIT_PLAN.md`](V3_SECURITY_AUDIT_PLAN.md) | V3 security audit — plan and gate | 2026-10-04 |
| [`V3_SPEC_GATE.md`](V3_SPEC_GATE.md) | V3 SPECviewperf gate: the `specviewperf` stage of the Vast release tier | 2026-11-08 |
| [`V3_SWEEP_AND_INSTALL.md`](V3_SWEEP_AND_INSTALL.md) | V3 sweep and install — every driver pair on every GPU family, tested on the binary users install | 2026-10-02 |
| [`V3_UNREAL_IMAGE.md`](V3_UNREAL_IMAGE.md) | Unreal Engine as a cached installed build, run in the open Linux guest | 2026-11-17 |
| [`V3_VFIO_USER_FRONTEND.md`](V3_VFIO_USER_FRONTEND.md) | V3 — a vfio-user frontend: can an unpatched VMM host kayfabe? | 2026-10-01 |
| [`V3_VIOMMU.md`](V3_VIOMMU.md) | V3 vIOMMU — a guest IOMMU in front of the kf3 device (design only) | 2026-10-08 |
| [`V3_WINDOWS_APPS_GATE.md`](V3_WINDOWS_APPS_GATE.md) | V3 Windows apps gate: the Windows app matrix as a stage of the Vast gate | 2026-11-08 |
| [`V3_WINDOWS_ON_VAST.md`](V3_WINDOWS_ON_VAST.md) | V3: one Windows 11 desktop boot on a rented Vast box (the verification plan for the owner's bar (b)) | 2026-10-31 |
| [`V3_WINDOWS_OPEN_ROWS.md`](V3_WINDOWS_OPEN_ROWS.md) | Open Windows rows: VK-GL-CTS and DirectX-Graphics-Samples extras | 2026-11-17 |
| [`V3_WINDOWS_PROPRIETARY_IMAGE.md`](V3_WINDOWS_PROPRIETARY_IMAGE.md) | Reproducible proprietary Windows 11 image on the PC | 2026-11-12 |
| [`V3_WINDOWS_RELEASE_SOFTWARE_DEFINITIONS.md`](V3_WINDOWS_RELEASE_SOFTWARE_DEFINITIONS.md) | Windows release software definitions | 2026-11-07 |
| [`V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md`](V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md) | Windows user-channel FIFO/USERD migration: next design decision | 2026-10-11 |
| [`V3_WINDOWS_USER_RING_RELAY.md`](V3_WINDOWS_USER_RING_RELAY.md) | V3 Windows user-channel ring relay: private GPFIFO + private USERD, with a safe migration path | 2026-10-12 |
| [`V3_WORKER_PRIVATE_PRESIGN.md`](V3_WORKER_PRIVATE_PRESIGN.md) | The Worker's private-object presign, the lease-time trust scope, releaseSeq and the estimate as the price | 2026-11-17 |
| [`V3_WORKER_PROPIMAGES.md`](V3_WORKER_PROPIMAGES.md) | The Worker's proprietary-image configuration: one instruction for the Worker run and the standalone run | 2026-11-16 |

## ANSWERED (22)

| Document | Title | Date in STATUS |
|---|---|---|
| [`OVERNIGHT_NOTES_w825.md`](OVERNIGHT_NOTES_w825.md) | OVERNIGHT NOTES — w825 (2026-09-23/24) | 2026-09-24 |
| [`THE_REWRITE.md`](THE_REWRITE.md) | The rewrite — what is deleted, what is kept, what is rewritten | 2026-11-20 |
| [`THE_TRANSLATED_PLANE.md`](THE_TRANSLATED_PLANE.md) | THE TRANSLATED PLANE — how a guest's work reaches the real GPU | 2026-11-20 |
| [`THE_V3_PLAN.md`](THE_V3_PLAN.md) | The v3 plan — what gets built, in what order, and what proves each step | 2026-09-26 |
| [`V3_BLACKWELL_NEXT_VALIDATION_20261022.md`](V3_BLACKWELL_NEXT_VALIDATION_20261022.md) | V3_BLACKWELL_NEXT_VALIDATION_20261022.md | 2026-10-22 |
| [`V3_BROKER_PRESENT_TIER_20261017.md`](V3_BROKER_PRESENT_TIER_20261017.md) | V3 broker — why the later brokers of run 3001 presented through wl_shm, and the fix that is the broker's | 2026-10-17 |
| [`V3_CDP.md`](V3_CDP.md) | V3 CDP — CUDA dynamic parallelism: why the child grid never ran, and the fix | 2026-09-30 |
| [`V3_FAMILY_PORT_ADA.md`](V3_FAMILY_PORT_ADA.md) | V3 FAMILY PORT — Ada (AD106, RTX 4060 Ti): the first non-GA106 boot | 2026-09-26 |
| [`V3_FAMILY_PORT_BLACKWELL.md`](V3_FAMILY_PORT_BLACKWELL.md) | V3 FAMILY PORT — Blackwell (GB203, RTX 5080): the first FSP-booted family on hardware | 2026-09-26 |
| [`V3_FLOORSWEPT_GR.md`](V3_FLOORSWEPT_GR.md) | V3 — floor-swept GR: the host's own GPC mask, logical rows, physical placement | 2026-09-26 |
| [`V3_GFX_TESTSET.md`](V3_GFX_TESTSET.md) | V3 headless-graphics test set — nvkvm-pv's headless graphics workloads on kayfabe v3 | 2026-09-26 |
| [`V3_HEADLESS_GRAPHICS.md`](V3_HEADLESS_GRAPHICS.md) | Headless graphics in kayfabe v3: what it takes, and whether channels are sufficient | 2026-09-26 |
| [`V3_P2_PORT_MAP.md`](V3_P2_PORT_MAP.md) | V3 P2/P3 PORT MAP — the old control plane, file by file | 2026-11-20 |
| [`V3_REFUSAL_AUDIT.md`](V3_REFUSAL_AUDIT.md) | V3 refusal audit — every non-OK status kf3 returns where bare metal returns OK | 2026-11-20 |
| [`V3_USER_RING_LIFECYCLE_REVIEW1_20261016.md`](V3_USER_RING_LIFECYCLE_REVIEW1_20261016.md) | Review: claude/win-lifecycle-p1-20261016 (6f9163848) vs claude/perf-fix-20261015 (0a7461173) | 2026-10-16 |
| [`V3_UVM_B3_IMPLEMENTATION.md`](V3_UVM_B3_IMPLEMENTATION.md) | b3: opt-in external fault service in host nvidia-uvm | 2026-09-30 |
| [`V3_WINDOW_EXPOSURE_REVIEW.md`](V3_WINDOW_EXPOSURE_REVIEW.md) | What kayfabe maps into a guest channel's GPU address space, and who can reach it | 2026-10-10 |
| [`dirty_tracking_without_uffd.md`](dirty_tracking_without_uffd.md) | Dirty tracking for one store, without userfaultfd | 2026-11-14 |
| [`fable_off_vcpu_design.md`](fable_off_vcpu_design.md) | Fable — the off-vCPU design for the w742 violation (constraints 4, 6, 7, 8) | 2026-11-16 |
| [`the_uvm_map_external_allocation_wall.md`](the_uvm_map_external_allocation_wall.md) | The `UVM_MAP_EXTERNAL_ALLOCATION` wall | 2026-09-14 |
| [`w754_gsp_queue_off_vcpu.md`](w754_gsp_queue_off_vcpu.md) | w754 — the GSP command-queue doorbell, and what is ACTUALLY on the vCPU behind it | 2026-11-16 |
| [`what_i_think_is_rotten_w433.md`](what_i_think_is_rotten_w433.md) | What I think is rotten, and what I would delete | 2026-11-10 |

## RESEARCH (39)

| Document | Title | Date in STATUS |
|---|---|---|
| [`THE_BAR0_DISPOSITION_MAP.md`](THE_BAR0_DISPOSITION_MAP.md) | THE BAR0 DISPOSITION MAP — every page, every family, GSP and non-GSP | 2026-11-20 |
| [`THE_COMMENT_AUDIT.md`](THE_COMMENT_AUDIT.md) | The comment audit — what the source says that the design no longer means | 2026-11-20 |
| [`THE_COMMENT_AUDIT_APPENDIX.md`](THE_COMMENT_AUDIT_APPENDIX.md) | Comment audit — full appendix (Tables A–E) | 2026-11-20 |
| [`THE_OGKM_RESIDUE.md`](THE_OGKM_RESIDUE.md) | ogkm residue — what NVIDIA left behind about monolithic RM, pre-Turing, and Windows | 2026-11-20 |
| [`THE_SURFACE_v3.md`](THE_SURFACE_v3.md) | The surface, at constant level — every RPC, class, control, register and method we touch | 2026-11-20 |
| [`THE_WINDOWS_AXIS.md`](THE_WINDOWS_AXIS.md) | The Windows axis — what a Windows guest actually changes | 2026-11-20 |
| [`V3_APPS_IMAGE_CHROOT_EVALUATION.md`](V3_APPS_IMAGE_CHROOT_EVALUATION.md) | Linux apps image without KVM during the build | 2026-11-08 |
| [`V3_BATCHED_MAP.md`](V3_BATCHED_MAP.md) | V3 — batched guest-RAM maps: N scattered runs, O(1) host calls | 2026-11-15 |
| [`V3_BLACKWELL_CONTINUATION_20261022.md`](V3_BLACKWELL_CONTINUATION_20261022.md) | V3_BLACKWELL_CONTINUATION_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_HOST_CAPS_20261022.md`](V3_BLACKWELL_HOST_CAPS_20261022.md) | V3_BLACKWELL_HOST_CAPS_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md`](V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md) | V3_BLACKWELL_PRECOMP_TIER_AUDIT_20261022.md | 2026-10-22 |
| [`V3_BLACKWELL_RESUME_20261021.md`](V3_BLACKWELL_RESUME_20261021.md) | 1. What the committed evidence says (new cross-reads, not in SW2.md) | 2026-10-21 |
| [`V3_COMPLETION_AUDIT.md`](V3_COMPLETION_AUDIT.md) | V3 completion audit — completions, interrupts and notifications a Windows guest can wait on | 2026-10-09 |
| [`V3_CONSTANT_MATRIX_20261014.md`](V3_CONSTANT_MATRIX_20261014.md) | V3 constant matrix: family x constant class x tier x status | 2026-10-22 |
| [`V3_DOORBELL_BASELINE.md`](V3_DOORBELL_BASELINE.md) | Doorbell baseline and ioeventfd prerequisites | 2026-09-28 |
| [`V3_ENS8_OWED_RING_TESTS.md`](V3_ENS8_OWED_RING_TESTS.md) | Translated-slot contention must preserve its consumed ring | 2026-10-27 |
| [`V3_FLAG_INVENTORY.md`](V3_FLAG_INVENTORY.md) | V3 FLAG INVENTORY — every `KF3_*` the Windows run profile sets or this line of work adds, what it does, and ho | 2026-10-09 |
| [`V3_HW_BOUNDARY_INVENTORY.md`](V3_HW_BOUNDARY_INVENTORY.md) | V3 — THE HARDWARE-BOUNDARY INVENTORY: every hardware-defined interface kayfabe touches, per die group | 2026-11-20 |
| [`V3_NATIVE_FIFO_RELOCATION_20261013.md`](V3_NATIVE_FIFO_RELOCATION_20261013.md) | Native deferred FIFO relocation: recovered positive reference | 2026-10-13 |
| [`V3_P4_PORT_MAP.md`](V3_P4_PORT_MAP.md) | V3 P4 PORT MAP — memory, VA and the BARs | 2026-10-03 |
| [`V3_P5_PORT_MAP.md`](V3_P5_PORT_MAP.md) | V3 P5 PORT MAP — the guest kernel's channels, born as host channels, run on the host GPU | 2026-10-07 |
| [`V3_PRECOMP_CSC_PRECISION_20261022.md`](V3_PRECOMP_CSC_PRECISION_20261022.md) | V3_PRECOMP_CSC_PRECISION_20261022.md | 2026-10-22 |
| [`V3_SDR_COLOR.md`](V3_SDR_COLOR.md) | Bounded SDR colour scanout | 2026-10-06 |
| [`V3_TMO_COLOR.md`](V3_TMO_COLOR.md) | Bounded pre-composition tone mapping | 2026-10-06 |
| [`V3_UVM_DEMAND_PAGING.md`](V3_UVM_DEMAND_PAGING.md) | V3_UVM_DEMAND_PAGING — managed memory and HMM in a kf3 guest | 2026-09-28 |
| [`V3_UVM_STATE_MACHINE.md`](V3_UVM_STATE_MACHINE.md) | V3_UVM_STATE_MACHINE — the managed-page state machine, read from nvidia-uvm | 2026-10-02 |
| [`V3_VGPU_COMPAT_RESEARCH_20261013.md`](V3_VGPU_COMPAT_RESEARCH_20261013.md) | V3 vGPU / guest-host version skew — what ogkm 595.84 says, and what kayfabe presents | 2026-10-13 |
| [`V3_VIOMMU_NEED_RESEARCH_20261013.md`](V3_VIOMMU_NEED_RESEARCH_20261013.md) | Does a Windows 11 guest need a (virtual) IOMMU? How much of a blocker is "no vIOMMU"? | 2026-10-13 |
| [`V3_WINDOWS_DOORBELL_RESEARCH.md`](V3_WINDOWS_DOORBELL_RESEARCH.md) | Windows analogue of the V3 guest doorbell module — feasibility research | 2026-09-26 |
| [`V3_WINDOWS_POOL_AUDIT_20261005.md`](V3_WINDOWS_POOL_AUDIT_20261005.md) | Independent review of the Windows pool experiment | 2026-10-05 |
| [`V3_WINDOWS_POOL_EXPERIMENT.md`](V3_WINDOWS_POOL_EXPERIMENT.md) | Windows graphics-preemption pool experiment | 2026-10-05 |
| [`V3_WINDOWS_USER_RING_REFUSALS_20261012.md`](V3_WINDOWS_USER_RING_REFUSALS_20261012.md) | Windows USER worker refusals and observation plan | 2026-10-12 |
| [`V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md`](V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md) | Windows user-ring relay: review against v3 | 2026-10-12 |
| [`V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md`](V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md) | Windows user-ring relay: P0 review and restricted prototype | 2026-10-12 |
| [`completion_wait_architecture.md`](completion_wait_architecture.md) | The completion-wait architecture — what exists, what serialises, what is unbuilt | 2026-11-20 |
| [`fable_leg_b_solution_space.md`](fable_leg_b_solution_space.md) | Leg B — the USERD naming problem: an independent survey of the solution space (w749) | 2026-11-20 |
| [`l1_os_shell.md`](l1_os_shell.md) | L1-M2 — the OS shell: reactor, raw module, VMM seam, and the reclamation lifecycle (decision #38) | 2026-11-20 |
| [`the_interrupt_arming_model.md`](the_interrupt_arming_model.md) | THE INTERRUPT ARMING MODEL — fire what the guest armed, not what the channel is bound to | 2026-11-20 |
| [`w750_route_k_prereg.md`](w750_route_k_prereg.md) | w750 PRE-REGISTRATION — route K, phase 1: the birth-client probe | 2026-11-20 |

## SUPERSEDED (3)

| Document | Title | Date in STATUS |
|---|---|---|
| [`THE_MACHINE.md`](THE_MACHINE.md) | The machine, as it is — and what I propose to change | 2026-09-26 |
| [`THE_SURFACE_v2.md`](THE_SURFACE_v2.md) | The surface we present — every RPC, register and channel operation, and the plan for it | 2026-09-20 |
| [`V3_GSS_NATIVE.md`](V3_GSS_NATIVE.md) | V3 — GSS-legacy controls carried to the host RM (`KF3_GSS_NATIVE`) | 2026-11-13 |
