# Current-master Heaven performance preregistration

**STATUS: RESEARCH, preregistration, 2026-10-28.** PF1 task PF2. No new performance result is asserted here. Evidence will live in `traces/master_perf_20261028/`; the report and raw logs stay private until sanitized results are ready.

## Question and hypotheses

**CODE READ at master `2108fb38d7288c82dfb7f5b257228e28fb36a13a`:** `kf-chan/src/user_lazy.rs` enables the GP_PUT cache and lazy private-USERD observation by default; the observation interval is 16, bounded by ring capacity. The production Pointer lifecycle replaces the earlier scan-off experiment. Therefore the historical 2026-10-17 labels do not establish current-master performance. This code read does not establish a new fps number.

**INFERRED H1:** master with its standard Windows functional launch profile and no performance overrides reaches median Heaven D3D11 >=250 fps and OpenGL >=330 fps across three fresh boots. The owner's explicit strong falsifier is a D3D11 median <200 fps. Values from 200 to <250 do not meet H1, even though they do not meet that stronger falsifier; OpenGL <330 also fails its stated target. Any correctness failure defeats acceptance regardless of fps.

**INFERRED H2:** the qualified 6d candidate with the owner's launch flags has an accepted median at least as high as arm A for each API. A lower candidate median for either API falsifies this descriptive comparison; any candidate correctness failure defeats H2 before performance is considered. With n=3 on one host, medians and ranges do not establish statistical equivalence.

**INFERRED control prediction:** disabling both lazy observation and the PUT cache lowers D3D11 throughput toward the historical pre-lazy range. Failure to reproduce that range is reported; it does not license changing another flag or relabelling the earlier experiment.

## Arms and immutable inputs

| Arm | Product revision | Configuration |
|---|---|---|
| A | `2108fb38d7288c82dfb7f5b257228e28fb36a13a` | Standard Windows functional flags below; lazy/cache switches unset |
| B | Same A executable and hash | A plus `KF3_LAZY_OBSERVE=0 KF3_PUT_CACHE=0` only |
| C | Qualified 6d `714ec29d3fdca0a013d9d478755346b0f1af6fa7` | Owner perf8 functional profile plus its lifecycle count/period flags |

The task explicitly permits the qualified 6d binary the owner uses. Its historical HP1 executable receipt names revision `714ec29d3` and SHA256 `ae2b346763fb4d4be291086d18d2c881a19282db90972009c4bb7f139b81c0ef`; verify the bytes under the lease before use. The task's `integration/prod-candidate-6d-20261022` ref is absent; the extant 20261023 integration ref has that qualified revision. This is a named qualified-6d comparison, not a claim that a different TSG-mirror branch was measured.

**CODE READ at master `2108fb38d`, `kf-qemu/src/chan.rs`:** functional Windows paths including kernel GR work remain opt-in. “No performance flags” is interpreted as retaining the standard Windows functional profile, not disabling graphics support. This interpretation was raised with the owner before execution. Any correction is folded into this preregistration and committed before the first run. The final table must label the profile explicitly rather than imply an empty KF3 environment.

The standard profile is the owner's perf8 launcher's underlying Windows flag list after its `KF3_TIMER_MAP` removal:

```text
KF3_WIN_USER_CHANNELS_PASSTHROUGH=1
KF3_WIN_TWIN_DEFAPI_OBJECT=1
KF3_KERNEL_GR_CE=1
KF3_KERNEL_GR_WORK=1
KF3_TRANSLATED_CE_RELAY=1
KF3_DEFERRED_API=1
KF3_PREEMPT_BIND_PROBE=1
KF3_DISPLAY_IMP_ENABLE=1
KF3_DISPLAY_CTRL_PROBE=1
KF3_ZCULL_BIND_PROBE=1
KF3_SW_RUNLIST_HOST_OWNED=1
KF3_ASYNC_PREEMPT=1
KF3_GFX_POOL_PROBE=1
KF3_SW_RUNLIST_PROBE=1
KF3_MEMORY_LIST_PROBE=1
KF3_DISPLAY_SDR_COLOR=1
KF3_DISPLAY_CAPS_PROBE=1
KF3_DISPLAY_LUT_MIRROR=1
KF3_DISPLAY_ILUT_OFFSET_256=1
```

C additionally keeps the owner's `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5`. A/B add no lifecycle census, profiler, interrupt storm, DIAG_PROGRESS, tracing or experimental scan-off switch. Each QEMU environment is constructed from a KF3-clean parent plus the complete recorded arm manifest. The old HP1 capture's extra DIAG_PROGRESS observer is not silently imported into the owner's perf8 profile.

Owner recipe provenance: perf8 launcher SHA256 `a804acf9869548bc2ab8b9af04f4873306371d1a80580c4699eccf04a8456b62`; its underlying broker launcher SHA256 `43bed8abda37a8116f8f6be83795bed57b70ca9d99b5b7e14d292650df305b64`. Full private paths, argv and executable hashes are recorded separately. Never execute or modify the owner's launcher or disk.

## Workload, order and acceptance

Reuse the Heaven job and F9 interaction from `claude/d3d11-gap-20261016`, pinned `890c6643c6022df22a5bc02804ab1dad5449e9a7`, under `scripts/bench/d3d11_gap/perf/`. Heaven 4.0, Windows 11, guest NVIDIA 580.88, host 595.91.07, RTX 4070; windowed 1920x1080, multisampling 0, `RELEASE,QUALITY_ULTRA`. Start F9 after the historical 38-second launch settle and focus click. Accept only the engine's completed in-app benchmark result, including its approximately 260-second duration, FPS, min/max FPS, score and frame count. Record the actual duration and settings. The generic `kfagent_ogl/heaven.ps1` windowed default is 1280x720 High/Normal and is deliberately not substituted.

Nine fresh QEMU boots, with both APIs once in each boot: `A1 B1 C1 B2 C2 A2 C3 A3 B3`. Repetitions 1 and 3 run D3D11 then OpenGL; repetition 2 reverses that API order. Thus each API/arm has n=3 independent boots. Both APIs share each boot, and that dependence is disclosed. All boots use new owned CoW disks and copied OVMF variables from one immutable backing chain, the same eight-vCPU/12-GiB host CPU profile, 8-GiB GPU store/BAR1, display geometry and fixed owner broker. Record installed driver, adapter status and TDR configuration before each benchmark. Never replace the guest driver or change TDR constants to make an arm finish.

**UNMEASURED procedure:** hold GDM/host desktop configuration constant and active. Record host power/utilisation using the historical 1-Hz host sampler when available, bounded to each benchmark window. These are shared-host readings, not isolated guest engine measurements. Heaven's min FPS is its own statistic, not a measured 1% low. No new present-gap or ISR/frame claim is made by this experiment.

Every boot/run must finish with no driver reset (153/141/4101, including a recorded LiveKernelEvent 141 signature), no new unapproved host Xid, and no host instability. The performance phase has no approved new Xid; the set is empty. Record a readable journal cursor/timestamp before launch and watch new events, not a dmesg line-count delta. Preserve first-failure evidence and stop the campaign on a new reset/Xid/instability. A fast failed run is **FAIL**, with no accepted fps. Incomplete/missing results are not zero or a pass. Do not replace a failed benchmark boot to improve the sample; any setup-only retry must be identified before its scores are examined.

**UNMEASURED procedure:** before Windows measurements, run the applicable native gates and current-master Linux fast-suite check if their existing assets can be reused within the lease. Record exact source, start/end/verdict and any approved fault-test oracle separately; a missing prerequisite or failed qualification is reported rather than passed over. No new product fix belongs in this measurement branch.

Per-run rows and the corrected table are generated from accepted records by a stamping script. Report accepted n, attempted n, individual FPS/min FPS, median and min/max spread. A missing or incorrect arm cannot receive a complete-arm median headline. Keep failed-attempt diagnostics alongside accepted results. Compare against the historical 2026-10-15 VFIO values D3D11 330.05 and OpenGL 341.9 only as historical context, not a fresh paired native control. Do not unbind the host GPU for a new VFIO reference.

## Lease and restoration

FIFO: ENS11 finishes, then the correctness lanes already waiting (WC1/PE1 and SW3/Z1 in their request order), then PF2. A momentarily free lock does not authorize skipping them. Notify the coordinator that PF2 is waiting; do not ask another campaign to stop. Take lifetime `flock -o /tmp/kayfabe-fastguest.lock` only at PF2's turn. Check RAM before each launch; never touch an owner's or another lane's QEMU. Four hours of host time include own build, qualification, runs and restoration, with time reserved for shutdown.

Build master using `build_kf3.sh` with nice 19 and bounded build jobs in owned host source/target/build directories. Install into a per-revision `kf3-bins` directory and seal the binary hash before the first boot. Never serve the mutable shared build output or another revision. Reuse C's qualified immutable binary only after its seal matches.

On exit, power down only owned guests through QMP, stop owned samplers/broker/agent, verify backing-chain metadata unchanged, GDM active, cuInit return 0, no QEMU, `/dev/null` still a character device, and release the lock. A cleanup failure is reported explicitly. Evidence and draft PR target `next`; no merge is authorized.
