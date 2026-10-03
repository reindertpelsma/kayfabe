# V3 app matrix — which real CUDA apps work in a kayfabe v3 fat guest

★ *2026-10-03 (§R5, BUILT on branch `v3-loud-uvm`, NOT yet run on a box): release item §I
(`OWNER_RULINGS.md` §I). Managed memory is unsupported and now fails loudly: kf3 posts a guest
`Xid 31 … kayfabe:` and a named host line, and `loud_verdict.sh` scores the managed-memory rows
EXPECTED_LOUD / KF3_DEFECT / SILENT. The CUDA virtual-memory API gets three rows (`vectorAddMMAP`,
`vmm_probe`, `torch_expseg`) and `um_probe` gets eight. The matrix grows from 71 to 82 rows.
⊘ CORRECTED by the same section: the four managed-memory failures were never silent at the CUDA
level. `conjugateGradientUM` prints SUCCESS only because it discards its cuBLAS statuses.*
★ *2026-09-30 (§R4, partial): the same matrix at kf3 `738c90e5` (= `3f67ed95`, pre-CDP-fix) on an RTX 3070 —
guest OFF 60/65 + 6/6, identical to R3; PM, 100-process and fast-path-ON runs not run.*
**STATUS: LIVE, 2026-09-28 — current result is §R3 (kf3 `4c48ca0c` = master `8ab92bf4`'s code,
measured 01:20–02:39 UTC): 60/65 apps work, 6/6 stream probes, 100/100 processes in one boot.**
★ *2026-09-30 (`v3-cdp`, §R3's 2026-09-30 bullet): the CDP child never ran because the guest's
SKED-reflected page was mapped as memory; fixed. Full matrix at kf3 `2830988f` (branch `v3-cdp`, not yet
on master): **61/65 apps + 6/6 probes** in one boot, host 71/71; the four failures are the UVM four.*
⊘ *Superseded by §R3 (the headline below is R2's):* **LIVE, 2026-09-26 — current result is §R2 (kayfabe `670bd310`, measured 05:00–07:30 UTC):
58/65 apps work (was 35/65).** §0–§5 below are the first measurement at `79848341`, kept unchanged
as the baseline R2 is compared against; their cause list is SUPERSEDED by §R2.3 (A, B, D, E, F fixed).

## R5 — release item §I: managed memory fails loudly; the CUDA VMM API joins the sweep (2026-10-03)

**STATUS: BUILT 2026-10-03 on branch `v3-loud-uvm` (cut from master `5d70e9c2`). GitHub CI run
`37131299412` at `539a04cb` was green: it ran the new unit tests (`kf-abi` `oserrorlog`, `kf-qemu`
`chan::rc_delivery_tests`, `kf-rm` `rpc`) and the 42 verdict fixtures. NOT run on a box: every
guest/host expectation below is a prediction until §R5.7 runs.** The
item is `OWNER_RULINGS.md` §I: *"unsupported must fail loudly … every such fault must reach the app as an
error"*, plus *"add a sample such as `vectorAddMMAP` to the sweep"*.

### R5.1 ⊘ CORRECTED — what the four managed-memory rows already showed at R3

Evidence: R3 at kf3 `4c48ca0c` (2026-09-28), each row run alone in a fresh boot
(`traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz`, `m20/iso/`):

| row | what the app saw | host | kf3 | guest dmesg |
|---|---|---|---|---|
| `UnifiedMemoryPerf` | `code=719(cudaErrorLaunchFailure)` at `cudaStreamSynchronize`, rc 1 | Xid 31 | 8 × `RC_TRIGGERED posted` | no Xid |
| `UnifiedMemoryStreams` | `code=13(CUBLAS_STATUS_EXECUTION_FAILED)`, then SIGSEGV (rc 139) | Xid 31 `FAULT_PTE` | 8 posted | no Xid |
| `attach_verify` | `cudaDeviceSynchronize() -> 719`, rc 1 | Xid 31 | 8 posted | no Xid |
| `conjugateGradientUM` | rc 0, `result = SUCCESS`, `Error amount = 1.000000` | Xid 31 `FAULT_PDE` | 8 posted | no Xid |

- **The fault already reached every app.** The path is as follows:
  - guest UVM gets no fault;
  - the host twin faults, and the host RCs its group (one host Xid 31; eight twin notifier records);
  - kf3 posts `RC_TRIGGERED`;
  - libcuda reads the host-written record and returns 719.
- **`conjugateGradientUM` is silent by its own code.** Read upstream at cuda-samples `v12.5`,
  `Samples/4_CUDA_Libraries/conjugateGradientUM/main.cpp`:
  - `r1` is uninitialised (`:90`);
  - the first SpMV launch is asynchronous, so its `checkCudaErrors` (`:190`) sees success;
  - `cublasSaxpy` (`:192`) and `cublasSdot(…, &r1)` (`:195`) are unchecked;
  - stack garbage below `tol²` skips the loop (`:199`), so its checked SpMV (`:208`) never runs;
  - `result = SUCCESS` and the exit code come from the iteration count (`:270-272`).
  
  On bare metal it does the same after any fatal fault. No VMM can make an app that discards its
  statuses fail. The `um_*` rows (§R5.5) are the deterministic proof that the status reaches the app.
- **⊘ `UnifiedMemoryStreams` is not C′ at R3.** C′ is a refused host map before a `FAULT_PTE`
  (§R2.3). Its R3 slice has no `not applied` and no `Other(31)` line. So at R3 it is the same class as
  the other three: demand paging.
- **What was missing:** everything bare metal shows besides the status. The guest dmesg had no Xid,
  because a GSP-client guest prints an RC Xid only on the GSP `OS_ERROR_LOG` event, and kf3 never sent it.
  The kf3 log had no line saying *why*. The boot-report sentence (`kf_abi::faultbuffer::DELIVERY_UNBUILT`)
  still predicted a hang.

### R5.2 What kf3 adds (`kf-qemu` `deliver_rc` / `rc_scan`, `kf-abi` `oserrorlog`)

- **A guest Xid.** kf3 posts one `OS_ERROR_LOG` (`0x1006`) per (guest client, exception) group of an
  RC batch, before the group's `RC_TRIGGERED`s.
  - The guest prints `NVRM: Xid (PCI:…): 31, pid=<pid>, name=<comm, 15 chars>, kayfabe: GPU MMU fault;
    N channel(s) of this process stopped. kayfabe services no GPU page faults: CUDA managed memory or
    pageable (HMM) access to a non-resident page is unsupported. Otherwise this is a kayfabe bug - please
    report.` The source is `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:769-806` →
    `kernel_rc.c:297-412`. The print needs `RmLogonRC=1`, the default.
  - The wire layout is versioned. Three layouts exist across 535–615, and the encoder takes offsets by
    field name for the guest driver version the served chain answers as. It refuses an unmeasured
    version and text over 255 bytes.
  - **Attribution never names another process.** The chid is that of a member twin still live
    (checked on the drainer, under the GSP lock, the same thread and lock that apply the guest's FREE)
    with a runlist the served FIFO table names. Otherwise kf3 posts `INVALID_CHID`, and the Xid prints
    without `pid=`/`name=`.
  - `xid_done` makes a requeued group (GSP queue full) never post a second Xid.
  - An event from an earlier GSP life (the phase left `Running`: unload, teardown) is dropped whole and
    counted `rc[stale=]`. A guest reboot needs a QEMU restart: kf3 has no reset path.
- **⊘ Deviation from the design, named:** a group member whose twin the guest freed before delivery
  gets no `RC_TRIGGERED` either (`rc[freed=]`). Its chid may already name a new channel of another
  process, and `_kgspRpcRCTriggered` would notify that channel. The design kept the post unchanged.
- **A named host line**, once per guest client per RC scan, rate-limited per client (10 s; at most 64
  clients tracked, the held count carried into the next line). Example:
  `kf3: UNSERVICED-GPU-FAULT guest client 0x… chids [0x7 0x8 …] host Xid 31 — kayfabe services no GPU page
  faults; guest gets RC_TRIGGERED + Xid 31`.
- **The silent holes counted.** Two kinds of twin turn a fault into a silent hang:
  - a twin whose notifier could not be armed (`RC-UNARMED`, counted before);
  - a twin whose guest declared **no** error notifier. It was counted nowhere; it is now named at birth
    (`RC-NONE`) and counted in `rc[none=]`.
  
  The status reads `rc[armed= unarmed= none= wakes= seen= posted= xid= unserviced= stale= freed=]`.
- **The boot-report sentence**, `DELIVERY_UNBUILT`, now names the error path. kf3 prints it once, the
  first time a guest registers a replayable fault buffer.
- **Error code.** The app gets **719**, where bare metal's unserviced fault (HMM off) gives **700**
  (`V3_UVM_DEMAND_PAGING.md` §1.2). Both are sticky. The cause is UNVERIFIED. The candidates are a host RC
  of a fatal fault versus a UVM cancel, and kf3 refusing `0x83de030c` (`READ_ALL_SM_ERROR_STATES`, the
  query libcuda sends after the fault; `fn76/0x83de030c=0x56x1` in the R3 slices). Documented, not chased.

### R5.3 Verdict classes for the managed-memory rows (`scripts/apps/loud_verdict.sh`)

The list is `UnifiedMemoryStreams UnifiedMemoryPerf conjugateGradientUM attach_verify um_cpuinit
um_gpufirst um_pageable`. It is the one exception to *"bare metal passes + guest fails ⇒ kayfabe bug"*
(`OWNER_RULINGS.md` §A.10), sanctioned by §I. `apps_hook.sh` appends `loud=<class>` and the boot's
`rc_unarmed=`/`rc_none=` (from the last status line in the slice) to every `guest.res` row.
`triage.py` and `summarize.py` count the classes separately.

| class | when | release |
|---|---|---|
| PASS | the row passed | — |
| EXPECTED_LOUD | all hold: (1) the guest dmesg has `NVRM: Xid (…): 31, … kayfabe:`; (2) the kf3 slice has `UNSERVICED-GPU-FAULT` and an `RC_TRIGGERED posted`; (3) no `not applied`, `REFUSED VasKey`, `RC-UNARMED` or `RC-NONE` line; (4) the app saw an error (`code=7(00\|19)`, `-> 7(00\|19)`, `CUBLAS_STATUS_EXECUTION_FAILED`, or a non-timeout nonzero rc). (4) is waived only for `conjugateGradientUM` (§R5.1) | sanctioned |
| KF3_DEFECT | the slice has a `not applied`, `REFUSED VasKey` or `RC-UNARMED` line (the C′ signature) | **blocker** |
| SILENT | anything else, including a hang (TIMEOUT, HANG, GUEST_DEAD) and an `RC-NONE` birth | **blocker** |
| UNTESTED | NOTRUN or BOOT_FAIL | — |

⚠ A kf3 publication defect that leaves none of the condition-(3) lines still scores EXPECTED_LOUD.
The Xid text says *"otherwise this is a kayfabe bug"* for that reason. The design's optional step 2
would tell the two apart: read the fault's VA from the host twin with `0x906f0106`, which is
unprivileged. It is gated on a box probe and not built. The classifier is tested offline by
`scripts/apps/test_verdicts.sh`, on the R3 slices plus synthetic lines (42 cases, CI step
"App-matrix verdict fixtures").

### R5.4 The CUDA virtual-memory API rows

The ioctls these calls issue on bare metal come from the census at
`/workspace/nvidia-gpu-passthrough/traces/w388_cuda_api_census/` (RTX 3060, open 560.35.03, CUDA 12.6;
stage `vmm`):

| call | ioctls (bare metal) |
|---|---|
| `cuMemAddressReserve` | none |
| `cuMemCreate` (PINNED, DEVICE) | `NV_ESC_RM_ALLOC` class `0x40` under the device |
| `cuMemMap` + `cuMemSetAccess` | `UVM_CREATE_EXTERNAL_RANGE` + `UVM_MAP_EXTERNAL_ALLOCATION` |
| alias at a second VA, then unmap | the same pair again + `UVM_FREE` |
| export to a POSIX fd / import | RM ALLOC `0x40`; NV0000 `0x202`/`0x201`; `ATTACH_GPUS_TO_FD`; `0x3d05`/`0x3d08`/`0x3d06`; NV0041 `0x410110`; then the UVM pair |
| teardown | `UVM_FREE` + RM `FREE` |

**Prediction (read, not run).** A GSP-client guest sends no RPC for the memory object: vidmem allocation
is not RPC'd (`ogkm-580: src/nvidia/src/kernel/mem_mgr/video_mem.c:963,994`), so its dups send none
either. kf3 sees only guest UVM's page-table writes and invalidates, which is the `cuMemAlloc` path.
The exception is the **first FD export**: `RmExportObject` allocates an RM-internal client, device and
subdevice (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/rmobjexportimport.c:257-261,423-427,441-445`).
The device alloc is RPC'd, so kf3 sees new `GSP_RM_ALLOC`s under a kernel client. Whether kf3 accepts
them is UNVERIFIED; `vmm_probe`'s `fd_import` check is the test.

| row | regex | what it covers |
|---|---|---|
| `vectorAddMMAP` | `Result = PASS` | cuda-samples `v12.5` `0_Introduction/vectorAddMMAP`: three reserve/create/map/release/set-access rounds (d_A, d_B, d_C), then a checked sum. `EXIT_WAIVED` (VMM unsupported) scores FAIL |
| `vmm_probe` | `CHECK` | `scripts/apps/src/vmm_probe.cu`, ported from nvkvm's `nvd_apis.c` stage `vmm`. Checks: `vmm_supported`, `map`, `alias` (write VA1 / read VA2 through the copy engine and a kernel, and back), `remap_same_va` (VA1 unmapped and its handle released while VA2 keeps the backing; a fresh handle at VA1 must read new data, VA2 old data — a stale leaf would cross them), `ro_map` (PROT_READ reads correctly), `fd_import`, `teardown`, `ro_write` (a kernel write through a PROT_READ mapping, in a child process, must fail) |
| `torch_expseg` | `CHECK expandable_segments ok` | `torch_correct.py` under `PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True`, then a segment grown by four 256 MiB tensors, two freed + `empty_cache()` (unmap), reallocated (remap), kernel-written, every element verified, a `DIGEST`. The CHECK is printed last and fails if the allocator snapshot has no `is_expandable` field (field name UNVERIFIED on the bundle's torch 2.6.0) |

⚠ `vmm_probe`'s `ro_write` faults on purpose. One Xid 31 per run is expected on both lanes; in a kf3
guest that includes a guest `Xid 31 … kayfabe:` for the child process. That row is the one exception to
"`guest_xid=0` on every passing row". The bundle builds `vmm_probe` against the toolkit's libcuda stub
(`build_bundle.sh`), and `apps_hook.sh` pushes `samples/vectorAddMMAP` and
`samples/vectorAdd_kernel64.fatbin` beside `bin/`, so neither needs a re-provisioned guest image. The
fatbin's name collides with `vectorAddDrv`'s, which defines the identical `VecAdd_kernel`.

### R5.5 The `um_*` rows — the status reaches the app

`scripts/apps/src/um_probe.cu` is `traces/v3_appfix/um_probe.cu` (the shapes of
`V3_UVM_DEMAND_PAGING.md` §1), unchanged. It checks every CUDA status and every value.

| row | bare metal | kf3 guest (predicted from `V3_UVM_DEMAND_PAGING.md` §1) |
|---|---|---|
| `um_cpuinit`, `um_gpufirst`, `um_pageable` | `CHECK <mode> ok` | `CHECK <mode> FAIL … -> 719` + a guest Xid 31 naming kayfabe ⇒ EXPECTED_LOUD |
| `um_prefetch`, `um_advise`, `um_malloc`, `um_hostalloc`, `um_d2h` | ok | ok — must PASS |

### R5.6 What a user is told (README "Not yet", release notes)

- CUDA **managed memory** (`cudaMallocManaged`) and **HMM pageable access** to a page that is not
  resident and mapped on the GPU are **unsupported**. Such an access fails with CUDA error 719 at the
  next sync, plus a guest kernel line `NVRM: Xid (…): 31, …, kayfabe: …`.
- Two managed-memory patterns work, from the 2026-09 `v3-appfix` runs on an RTX 3060
  (`V3_UVM_DEMAND_PAGING.md` §1):
  - CPU-initialised, then `cudaMemPrefetchAsync` to the GPU;
  - CPU-initialised, then `cudaMemAdviseSetAccessedBy`.
  
  Any other access to a non-resident page fails as above, including a page the GPU touches first or one
  migrated back to the CPU.
- Known opt-ins: llama.cpp's `GGML_CUDA_ENABLE_UNIFIED_MEMORY`, and RAPIDS cudf.pandas (which, as far
  as the review knows, defaults to managed memory when the GPU reports concurrent managed access).
- Pinned and zero-copy host memory work (`bandwidthTest`, `simpleZeroCopy`).

### R5.7 Pending box tests (owner approval; one GA10x; record the kf3 revision on every claim)

```sh
# on a box provisioned as in §5, the tree at this branch's head, kf3 built by scripts/bench/build_kf3.sh
bash scripts/apps/build_bundle.sh 2>&1 | grep -E '^BUILD_(sample_vectorAddMMAP|vmm_probe|um_probe)='   # all =ok
R5="vectorAddMMAP vmm_probe torch_expseg um_cpuinit um_gpufirst um_pageable um_prefetch um_advise um_malloc um_hostalloc um_d2h"
UM="UnifiedMemoryStreams UnifiedMemoryPerf conjugateGradientUM attach_verify um_cpuinit um_gpufirst um_pageable"
bash scripts/apps/apps_matrix.sh host r5host $R5 $UM          # every new row PASS on bare metal first
KF3_BIN=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 APPS_PER_BOOT=1 \
  bash scripts/apps/apps_matrix.sh guest r5iso $UM            # each managed row alone, fresh boot
grep -E 'loud=|APPS_WEDGE' /workspace/apps/results/r5iso/guest.res   # want loud=EXPECTED_LOUD, rc_unarmed=0 rc_none=0, no WEDGE
for a in $UM; do grep -h 'NVRM: Xid' /workspace/apps/results/r5iso/$a.guest_dmesg.log; grep -hc UNSERVICED-GPU-FAULT /workspace/apps/results/r5iso/$a.kf3.log; done
KF3_BIN=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 APPS_PER_BOOT=8 \
  bash scripts/apps/apps_matrix.sh guest r5full all          # the full 82-row matrix
python3 scripts/apps/summarize.py /workspace/apps/results/r5full; python3 scripts/apps/triage.py /workspace/apps/results/r5full
```

Expected:
- `build_bundle.sh` prints `BUILD_sample_vectorAddMMAP=ok`, `BUILD_vmm_probe=ok` and `BUILD_um_probe=ok`.
- Host lane: every new row PASS, including `CHECK expandable_segments ok` and every `vmm_probe` CHECK.
  This validates the regexes and the snapshot field before any guest verdict counts.
- Guest, each managed row alone:
  - `loud=EXPECTED_LOUD`;
  - at least one guest `NVRM: Xid (…): 31, pid=<pid>, name=<15-char comm>, kayfabe:` per faulting
    process (`conjugateGradie`, `UnifiedMemorySt`, `UnifiedMemoryPe`, `attach_verify`, `um_probe`);
  - `UNSERVICED-GPU-FAULT` in the kf3 slice and `rc[… none=0 … xid=≥1 …]`;
  - the post-row `vectorAdd` passes.
- Full matrix: no regression from master's 61/65 apps + 6/6 probes (kf3 `2830988f`, §R3). `guest_xid=0`
  on every passing row except `vmm_probe` (§R5.4). The `torch_expseg` digest equals the host's.
- Still open (design §6): the optional `0x906f0106` fault-identity probe after an RC; one non-Ampere die
  (`um_probe cpuinit` attribution, `vmm_probe remap_same_va` against the Hopper/Blackwell walker gaps
  L4/L5 of `V3_HW_BOUNDARY_INVENTORY.md`), whose bundle must be built for that die.

## R4 — re-run at kf3 `738c90e5` (code of `3f67ed95`, before the CDP fix), RTX 3070, 2026-09-30 — PARTIAL

**[M]** Box vast `53563077` (destroyed), **RTX 3070 (GA104, 8 GiB)**, EPYC 7532, **nested** KVM, host + guest
580.159.04 open; kf3 `kf3-bins/738c90e5` (sha256 `fe3270ef…a69e0eeda`); guest `fb-mb=6144`, 16 GiB, 6 vCPUs;
same harness and shapes as R3. Evidence: `traces/v3_app_matrix/vast53563077_rtx3070_738c90e5/`. ⊘ The
measured code **predates the CDP fix** (`46509dce`, merged as `afb552ea`), so it does not certify master
`cb0bae8f`/`8644e477`; the CDP result of record stays `2830988f`'s 61/65 (above).

| run | result | vs R3 (`4c48ca0c`, RTX 3060) |
|---|---|---|
| host (bare metal) | **71/71**, 0 host Xid | same |
| guest, fast path OFF, no PM, all 71 rows in one boot + every non-PASS app alone | **60/65 apps + 6/6 probes**; the same 5 fail alone | **identical** |
| guest PM (one boot), 100 processes, the `doorbell-ioeventfd=on` lane | **not run** — the owner paused work; the PM boot was stopped before its first app | — |

- Failures, alone, classified against bare metal (which passes all five): the UVM four (`UnifiedMemoryStreams`,
  `UnifiedMemoryPerf`, `conjugateGradientUM` — silent `Error amount = 1.000000` — `attach_verify`): host **Xid 31**,
  kf3 `RC host twin … except_type=0x1f`, 0 guest Xid; refusal ledger = the boot baseline plus GSP
  `0x83de030c` (as R3). `cdpSimpleQuicksort`: TIMEOUT, no Xid, no RC, no extra refusal (fixed after this code).
- Digests equal this box's bare metal: `torch_correct`, `hf_generate`, `llama_cpp_gen`. No regression vs R3.
- 0 of 6 boots show §4.9's startup-race signature (`no fd-backed guest RAM block`).

## R3 — re-run at master (kf3 `4c48ca0c`, 2026-09-28)

**[M]** Box: vast 53004208, RTX 3060 (GA106), EPYC 7K62 host, nested KVM, host + guest 580.159.04 open.
The kf3 binary is `kf3-bins/4c48ca0c` (the v3-mc21 bar revision, `traces/v3_mc21/`); scripts from a
worktree at `8ab92bf4` (= `4c48ca0c` + docs). Evidence: `traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/`
(`m20` = no persistence mode, several apps per boot then every non-PASS app alone; `m20pm` = persistence
mode, one boot; `m20seq` = 100 `vectorAdd` processes in one boot; `all_logs.tgz` = every per-app log).

| run | result | vs R2 (`670bd310`) |
|---|---|---|
| host (bare metal) `m20` | **71/71** (65 apps + 6 probes) | same |
| guest, no PM, batched `m20` | **66/71 = 60/65 apps + 6/6 probes** | 58/65 |
| guest, alone (every non-PASS) | the same 5 fail alone (`m20/iso_guest.res`) | — |
| guest, PM, ONE boot `m20pm` | **66/71**, the identical 5, no wedge | 58/65 |
| 100 CUDA processes, one boot, no PM `m20seq` | **100/100** — the per-boot process budget J (§R2.2) is gone | R2 `seq2`: 39, the 40th hung |

- **Fixed since R2 (now PASS):** `gpu_burn` (G), `torch_ai_bench` (C/J), `clpeak` (C) — the mapfix /
  BAR1-view leak work. Output digests equal the host's for `torch_correct`, `hf_generate`, `llama_cpp_gen`
  (`m20/guest.dig` vs `m20/host.res`).
- ⊘ **CORRECTED 2026-10-03 (§R5.1): the next bullet's "silent wrong answer" is wrong.** The CUDA error
  reached all four apps. `conjugateGradientUM` lost it at an unchecked `cublasSdot` and computed
  `result = SUCCESS` from its iteration count. Also, `UnifiedMemoryStreams` shows no C′ signature at R3:
  its slice has no `not applied` or `Other(31)` line, and the host fault is `FAULT_PTE`.
- **Still failing, the UVM demand-paging four** (host twin RC `except_type=0x1f` = Xid 31 in `m20/triage.txt`):
  `UnifiedMemoryStreams`, `UnifiedMemoryPerf`, `conjugateGradientUM` (silent wrong answer, `Error amount =
  1.000000`), `attach_verify` — the owner-decision route (`STATUS_AND_HANDOFF.md` §3.2).
- ★★★ **2026-09-30 — ANSWERED + FIXED (`v3-cdp`, `docs/design/V3_CDP.md`).** The child never ran because
  libcuda's SKED-reflected page (`UVM_MAP_DYNAMIC_PARALLELISM_REGION`: a 4 KiB PTE of kind
  `SMSKED_MESSAGE`, aperture VIDEO, address 0) was mirrored as a **memory** row — the store at offset 0,
  kind PITCH — so a device-side launch was an ordinary store and never reached the host GPU's scheduler.
  Fix `46509dce`: a SKED-reflected leaf is placed as a message-kind host mapping (`MapTarget::map_sked`).
  **[M] kf3 `090b20d9`, vast 53004208 (RTX 3060), fresh boots:** the probe's four launch shapes (NULL
  stream, fire-and-forget, tail launch, device-created stream × auto/spin/block) all run their child
  (`child_ran=1`, `out[1]=0xc0ffee`); `cdpSimpleQuicksort` validates at 128 / 1 000 / 10 000 elements;
  the app-matrix lane gives `verdict=PASS rc=0 secs=7`; the ioctl trace is in lockstep with bare metal.
  The `kf3_refusals=4` of every CDP boot are the per-init set every app shows (`V3_CDP.md` §2.3) — not CDP's.
  **Full matrix re-run at kf3 `2830988f`** (the bar's revision, 18:49–19:34 UTC, same box, same harness):
  host **71/71**; guest, one boot, no PM **67/71 = 61/65 apps + 6/6 probes** — the four failures are the
  UVM four above, failing alone too; digests equal the host's; the boot placed 109 SKED pages (every CUDA
  context has one), none refused. Evidence: `traces/v3_cdp/` (`app_matrix_2830988f/`).
- ⊘ *Superseded 2026-09-30 for the CAUSE by the line above (its measurements stand):* **2026-09-28 recovery correction (folded onto master 2026-09-30):** the bisect finished: quicksort
  passes at `0667b784` and times out at its successor `56032c46` (the MC_SERVICE_INTERRUPTS completion
  fix). A dedicated CDP probe on the older revision reports successful synchronization **without
  executing its child** (`child_ran=0`, wrong output, `RESULT ... BAD`); the bare-metal control runs the
  child correctly. At `56032c46` the parent runs, the child does not, and synchronization hits its
  watchdog. ⇒ R2's PASS rode on the forged completion; the child has never run in a guest. Do not
  revert the completion fix to regain the label; diagnose child execution with fresh-boot controls.
  Logs: `traces/recovery_20260928/53004208-cdp4.log.txt` (+ `-cdp-host.log.txt`, the bare-metal
  control); probe source in the bundle on branch `recovery/resume-2026-09-28`.
- ⊘ *Superseded by the correction above:* **REGRESSION: `cdpSimpleQuicksort`** (CUDA dynamic parallelism) **PASSED in R2** and now TIMES OUT
  (60 s, quiet, no guest Xid, no kf3 RC line) — batched, alone and with PM. Host PASS. Being bisected over
  the 75 first-parent master revisions `670bd310..4c48ca0c` (2026-09-28, branch `local/cdpfix` of the
  cloud session; result will be folded in here, above this line).


## R2 — re-run at kayfabe `670bd310` (2026-09-26)

Since `79848341`, master gained: one host TSG per guest context share (the 2nd-CUDA-stream fix),
pooled/host-managed walker capacity (the no-PM multi-process fix), NVENC/NVDEC, headless
Vulkan/EGL, the GSP reopen fix, and batched host maps. Same app set, same predicates (two
tightened, §R2.4), same harness.

**Box:** vast 52624429 (`vh`), **RTX 3060 12 GB (GA106)**, AMD EPYC 7452, host driver 580.159.04
(kernel-open); kf3 binary `/workspace/bench/kf3-bins/670bd310/qemu-system-x86_64` (boot_capture
stamp `kf3-bin-rev:670bd310`); guest = a **copy** of the box's LLM fat guest (`guest_apps.qcow2`,
Ubuntu 24.04, kernel 6.8.0-139, stock 580.159.04) provisioned with `provision_guest_apps.sh`, so the
LLM guest stayed untouched; `fb-mb=8192`, 16 GiB RAM, 6 vCPUs. The bundle was rebuilt on this box
(`build_bundle.sh`, bundle sha `4ec119b39ec2395e`, llama.cpp pinned to `4b1a27f`). No RTX 3070 this
round — cause I (3070-only) is not re-measured.

### R2.0 Headline

- **Host (bare metal, same box): 71/71 PASS** (`h1`; `h2` re-ran clpeak + llama_cpp_gen under the
  tightened predicates: PASS).
- **Guest: 58/65 apps PASS, and all 6 stream probes PASS** — without persistence mode, several apps
  per boot. **With persistence mode: the identical 58/65 and the identical 7 failures** (`pm2`).
- **Cause A (non-default streams, Xid 13 SKEDCHECK05) is gone**: all 21 apps it blocked and all 5
  non-default `stream_probe` shapes pass — PyTorch (`torch_correct`, CNN-train digest = host),
  llama.cpp gen (token-identical to host) + bench, CuPy, hashcat, Blender CUDA+OptiX, Geekbench,
  every CUDA-graph / multi-stream sample.
- **NVENC/NVDEC, EGL, Vulkan now work**: nvenc_h264/hevc, nvdec_h264, egl_offscreen, vulkaninfo,
  vkpeak (every vkpeak figure within 1 % of bare metal).
- **No-PM multi-app boots now work** (§R2.2): 39 CUDA processes in one boot without PM (was 5–6).
  The budget that remains is a host `NoMemory` at the ~40th process (no PM) / ~60th (PM) — cause J.
- **What is left: one family** — a host **Xid 31 MMU fault on the guest's GR work** (5 apps
  `FAULT_PDE`, 1 app `FAULT_PTE`), incl. the known **silent wrong answer** in `conjugateGradientUM`
  — plus the known `gpu_burn` SIGSEGV. `clpeak` moved to FAIL: it was a **false PASS** of the old
  predicate (§R2.4), and it shows the fault is **not CUDA-managed-memory-only** (OpenCL hits it).
- Digests host vs guest: `torch_correct` cnn_train_step `763c693a5a53f948` ==, `hf_generate`
  `0d973108a6251e14` == (also == the 79848341 run), `llama_cpp_gen` generated text
  `caf613a5d956b7e3` == (pm2; nb2/iso1 compared by text, token-identical).

### R2.1 Results

Runs (results under `traces/v3_app_matrix/vh_rtx3060_670bd310/<run>/`, `triage.txt` = one evidence
line per app from `scripts/apps/triage.py`):
- `nb1` — no PM, **all 71 rows in one boot**. Stopped by hand after the boot wedged at
  `UnifiedMemoryStreams` (row 18; see C′) — the harness then had no wedge detection (added, §R2.4).
- `nb2` — no PM, rows 19–71 batched (`conjugateGradientUM` onward) with the wedge probe: 2 boots.
- `iso1` — every non-PASS app alone in a fresh boot (the verdict of record for failures).
- `seq2` — no PM, 100× `vectorAdd` in one boot (the process budget).
- `pm2` — guest PM on, all 71 rows batched with the wedge probe: 2 boots.

A row PASSes if it passed in a batched boot (a pass in a shared boot is still a pass); a failure is
confirmed alone (`iso1`). Old = the 79848341 3060 column of §2.

| app | host | guest no-PM (batched) | guest alone | guest PM (batched) | old 3060 | failure point / evidence |
|---|---|---|---|---|---|---|
| nvidia_smi … simpleIPC (16 rows¹) | PASS | PASS | - | PASS | 9 PASS, 7 A | ¹ nvidia_smi, deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, simpleIPC |
| UnifiedMemoryStreams | PASS | TIMEOUT (wedge) | **TIMEOUT (C′)** | FAIL 719 | FAIL (A) | host `Xid 31 … GPC1 … @0x7c70_d8489000 FAULT_PTE VIRT_READ`; kf3 `REFUSED VasKey(..) root 0x201000: 1 run(s) not applied: map 0x7c70d8400000+0x10000: Other(31)` (RM `NV_ERR_INVALID_ARGUMENT`) + `split ticket REFUSED`; no PM ⇒ the boot is **wedged** afterwards (sanity vectorAdd fails) |
| UnifiedMemoryPerf | PASS | TIMEOUT (after the wedge) | **FAIL (C)** | FAIL | FAIL (A) | `matrixMultiplyPerf.cu:435 code=719 cudaStreamSynchronize`; host `Xid 31 … @0x7c7d_22000000 FAULT_PDE VIRT_READ` |
| conjugateGradientUM | PASS | FAIL | **FAIL (C)** | FAIL | FAIL (C) | ⊘ **silent wrong answer**: `Error amount = 1.000000, result = SUCCESS`; host `Xid 31 … @0x791b_b6200000 FAULT_PDE VIRT_READ` |
| cudaTensorCoreGemm … memcpy2d (27 rows²) | PASS | PASS | - | PASS | 23 PASS, 4 A | ² incl. globalToShmemAsyncCopy, graphMemoryNodes, simpleCudaGraphs, MersenneTwisterGP11213 (were A), all nvkvm-pv realapp kernels |
| attach_verify | PASS | FAIL | **FAIL (C)** | FAIL | FAIL (A) | `cudaDeviceSynchronize() -> 719`, the buffers verified `0 mismatched` first; host `Xid 31 … @0x7280_a8007000 FAULT_PDE VIRT_READ` |
| stream_default … stream_created2nd (6 probes) | PASS | PASS | - | PASS | 1 PASS, 5 A | every stream shape, incl. `created2nd` |
| gpu_burn | PASS | FAIL | **FAIL (G)** | FAIL | FAIL (G) | SIGSEGV right after `cuInit returned 0` (guest `segfault at 7ffc83508000 … error 4 in gpu_burn`); no Xid, no kf3 refusal — known, branch `v3-appfix` |
| torch_correct | PASS | PASS | - | PASS | FAIL (A) | digest == host |
| torch_ai_bench | PASS | FAIL (J: no CUDA) | **FAIL (C)** | TIMEOUT (J) | FAIL (C) | alone: `RuntimeError: CUDA error: unspecified launch failure`, host `Xid 31 … @0x79af_c2127000 FAULT_PDE VIRT_WRITE`; in nb2 it was the ~41st process of the boot: kf3 `act birth translated REFUSED (0x56): USERD view of store 0xc0000: NoMemory` ⇒ `torch.cuda.is_available()` False; in pm2 the ~60th: `NV_ESC_RM_MAP_MEMORY … NoMemory` |
| hf_generate, cupy, llama_cpp_gen, llama_bench | PASS | PASS | - | PASS | 1 PASS, 2 B, 1 A | digests == host |
| vulkaninfo, vkpeak, egl_offscreen | PASS | PASS | - | PASS | F, F, E | vkpeak fp32 9335 vs host 9275 GFLOPS, fp16-matrix 55690 vs 55538 |
| clinfo | PASS | PASS | - | PASS | PASS | |
| clpeak | PASS | PASS (old predicate) | **FAIL (C)** | FAIL | PASS (old predicate) | ⊘ false PASS: GPU integer, integer-24bit, transfer and launch-latency groups `clFinish (-36)` → `Tests skipped`; host `Xid 31 … @0x772b_de422000 FAULT_PDE VIRT_WRITE`. Host: 0 skipped |
| nvenc_h264, nvenc_hevc, nvdec_h264 | PASS | PASS | - | PASS | D | 600 frames each, NVDEC used (no software fallback) |
| hashcat, blender_cycles, geekbench_gpu | PASS | PASS | - | PASS | A | hashcat cracks; Blender CUDA+OptiX `mean=0.6405` == host; GB6 OpenCL completes (`internal code 35` is printed on the host too) |

**Totals (65 apps, probes excluded): host 65/65; guest 58/65 — no PM and PM alike** (was 35/65 on
the 3060 at `79848341`; 34/65 on the 3070). Apples-to-apples with the old, weaker clpeak predicate:
59/65. Stream probes 6/6 (was 1/6).

### R2.2 No-PM multi-app boots — they hold now, up to ~40 CUDA processes

- `seq2` (no PM, 100× `vectorAdd`, one boot): **39 PASS**, the 40th hangs: kf3
  `REFUSED VasKey(18446744069414584321) root 0x1f1cac000: 1 run(s) not applied: window map
  0x110000+0x200000 (store @0x1000000): arm: NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory`
  — the **same** signature as §3 J; the boot is then wedged (sanity vectorAdd fails). At `79848341`
  the same experiment stopped after **5–6** (cause B, `slot N is full`): **B is gone** —
  `slot … is full` appears in no log of this round.
- `nb2` boot 1 ran **37 apps** (CUDA samples, cuBLAS/cuFFT, the realapp kernels, torch_correct,
  gpu_burn's crash, two Xid-31 faults) with every verdict equal to its isolated verdict; the 38th row
  (`torch_ai_bench`, the ~41st CUDA process counting 3 wedge probes) failed `cuInit` on a second
  NoMemory spelling (`USERD view of store 0xc0000: NoMemory`).
  `nb2` boot 2 ran the remaining 15 rows (LLMs, Vulkan, EGL, OpenCL, video, hashcat, Blender,
  Geekbench) — all PASS except clpeak (C).
- With PM (`pm2`) the budget is ~60 processes (J at `torch_ai_bench`, the 56th row + 5 wedge
  probes), matching §3 J's ~60th at `79848341`. ⇒ **J is now THE per-boot process budget** in both
  modes; it is the "~60th-process host OOM leak" already being worked on (branch `v3-appfix`).
- ⊘ One no-PM-only hazard: after `UnifiedMemoryStreams`' fault (C′) the no-PM boot is **wedged**
  (nb1: the next two apps hung silently, then a guest kernel channel `host 0x1001a
  REFUSED-AND-POISONED (§7)`; iso1: the sanity vectorAdd hung). With PM the same app fails fast with
  719 and the boot carries on.

### R2.3 Causes, ranked by apps blocked (670bd310)

| rank | cause | apps (of 65) | status vs 79848341 | signature |
|---|---|---|---|---|
| 1 | **C — host Xid 31 `FAULT_PDE` on the guest's GR work** | **5**: conjugateGradientUM (**silent wrong answer**), attach_verify, UnifiedMemoryPerf, torch_ai_bench, clpeak | was 2 (most were masked by A) — known, `v3-appfix` | host `Xid 31, MMU Fault: ENGINE GRAPHICS GPCn … FAULT_PDE ACCESS_TYPE_VIRT_READ/WRITE` at a user VA (`0x72xx…0x7exx`); kf3 `RC host twin … except_type=0x1f (Xid 31) — forwarding RC_TRIGGERED` on every channel of the context; no kf3 refusal precedes it. ⚠ **clpeak is OpenCL** — the class is not CUDA-managed-memory-only |
| 2 | **C′ — `FAULT_PTE` behind a refused host map** | 1: UnifiedMemoryStreams | new signature (was masked by A) | kf3 `1 run(s) not applied: map <va>+0x10000: Other(31)` (`NV_ERR_INVALID_ARGUMENT`) then host `Xid 31 … FAULT_PTE VIRT_READ` at/near that VA (nb1: map `0x7111f8600000+0x10000`, fault `0x7111_f8603000` — inside it); no PM ⇒ boot wedged afterwards |
| 3 | G — gpu_burn SIGSEGV | 1 | unchanged — known, `v3-appfix` | segfault just after `cuInit`; no Xid, no refusal |
| — | **J — host `NoMemory` process budget** | 0 alone; every app past the ~40th (no PM) / ~60th (PM) process of a boot | **now the only per-boot budget** (B gone) — known, `v3-appfix` | `NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory`, or `act birth … USERD view of store 0xc0000: NoMemory` |
| ✔ | A — non-default streams (Xid 13 SKEDCHECK05) | 0 (was 21 + 5 probes) | **FIXED** | 0 host `Xid 13` in the 13 captured host-dmesg windows (the same windows hold every Xid 31 of §R2.1, so the capture works); 0 `SKEDCHECK` in any per-app kf3 log |
| ✔ | B — walker slot exhaustion | 0 (was 5 + the per-boot budget) | **FIXED** | 0 `is full and nothing can be retired` in any per-app kf3 log (the same logs carry the `REFUSED VasKey` lines of J, so they capture `mem` refusals) |
| ✔ | D — video engines | 0 (was 3) | **FIXED** | |
| ✔ | E — EGL / F — Vulkan | 0 (was 1 / 2) | **FIXED** | vulkaninfo returns in 2 s; 0 `scrubberDestruct` in any per-app guest dmesg |
| ? | I — walk beyond the FB span (`fb-mb=6144`, 3070 only) | – | not re-measured (no 3070 box) | |

### R2.4 Harness changes in this round (branch `v3-apps2`, scripts only)

- **Wedge probe** (`apps_hook.sh`): after any non-PASS row, run `vectorAdd` (60 s); if it fails,
  write `APPS_WEDGE` and end the boot — `apps_matrix.sh` reboots and continues with the rest. Without
  it, nb1 burned every later app's full timeout after one wedge.
- **clpeak predicate**: FAIL on `clFinish (-N)` / `Tests skipped` (its bandwidth line — the old
  predicate — prints even when the compute groups abort).
- **llama_cpp_gen digest**: over the generated text only. This llama.cpp rev logs `VRAM: <n> MiB`
  (7871 in the guest vs 11909 on the host) and timings between tokens, so three different digests
  came out of one token-identical output.
- **Bench lock**: `apps_matrix.sh` takes `/tmp/kayfabe-fastguest.lock` per boot (and for the host
  run) and releases it between boots.
- **`KF_GUEST_IMG`** (`boot_nvkvm.sh`, `provision_guest_apps.sh`): run on a copy of the guest image so
  another lane's guest (here the LLM guest) stays intact.
- `build_bundle.sh` pins llama.cpp to `4b1a27f` (it cloned HEAD).
- `triage.py`: per-app evidence line (verdict, guest Xid, first kf3 RC line, first non-baseline kf3
  refusal, note).

Reproduce (on `vh`-shaped box): as §5, plus `KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2` for
provisioning and every guest run; `APPS_PER_BOOT=100` for the batched no-PM run, `APPS_GUEST_PM=1`
for the PM run, `APPS_PER_BOOT=1` for isolation.

---

# R1 — the first measurement, kayfabe `79848341`

**STATUS of R1: ANSWERED, 2026-09-26** (measured 00:10–04:00 UTC). kayfabe **`79848341`** (origin/master; the kf3 binary was built
from it, `kf3-bins/79848341`); harness commits on branch `v3-apps` touch only `scripts/apps/`,
`traces/v3_app_matrix/` and this file. Two rented vast KVM boxes, host driver **580.159.04**
(kernel-open), guest Ubuntu 24.04 with the stock **580.159.04** guest driver, kf3 device, 16 GiB
guest RAM, 6 vCPUs:
- **va1 — RTX 3060 12 GB (GA106)**, vast 52660722, guest FB `fb-mb=8192` (default);
- **va2 — RTX 3070 8 GB (GA104)**, vast 52660725, guest FB `fb-mb=6144` (8192 would exceed the card).

Question (owner): nvkvm-pv (the shipped Mode-1 sibling) validated a set of real CUDA apps. Run the
same set inside a kayfabe **v3 fat guest** and record which ones **work** — correct output, no
hang, no crash, no Xid. Not parity, not timing. Every app ran on the **host of the same box first**
(bare metal, identical binaries/runtime), so a guest-only failure indicts kayfabe, not the app.

## 0. Headline

- **Host: 71/71 PASS on both boxes** (65 apps + 6 `stream_probe` shapes). Every guest failure below is
  a guest-only failure.
- **Guest: 35/65 apps work on the 3060, 34/65 on the 3070** (one app per fresh boot). What works:
  nvidia-smi, deviceQuery, every **default-stream** CUDA program (vectorAdd, the nvkvm-pv HPC kernels
  incl. cuBLAS SGEMM and cuFFT, reduction/scan/sort/histogram, tensor-core GEMMs, dynamic
  parallelism, cooperative groups, simpleCUBLAS/CUFFT, conjugateGradient), HF transformers greedy
  generate (**token-identical to the host**), OpenCL (clinfo, clpeak).
- **One defect blocks most of the rest: any kernel launched on a non-default CUDA stream** faults the
  host GPU with **`Xid 13 SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE`** (§3 A) — 21 of the 65 apps, including
  PyTorch, llama.cpp, Blender, hashcat, Geekbench and every CUDA-graph / multi-stream sample.
- **Without guest persistence mode a guest can run only ~5–6 CUDA processes per boot**; the next one
  hangs forever and so does every one after it (§3 B). **With persistence mode that wall is gone**
  (59 processes in one boot, identical verdicts on both boxes, B-class apps such as CuPy pass) — until a
  host-side `NV_ESC_RM_MAP_MEMORY … NoMemory` wedges the boot at the ~60th process (§3 J).
- **One silent wrong answer**: `conjugateGradientUM` prints `result = SUCCESS` with
  `Error amount = 1.000000` after a host `Xid 31` MMU fault (§3 C).
- Not exposed at all: **NVENC/NVDEC** ("unsupported device"), **EGL** (`eglInitialize failed`),
  **Vulkan** (`vulkaninfo` hangs, vkpeak "No vulkan device").

## 1. The app set (inventory of nvkvm-pv) and what "pass" means here

Sources: `nvkvm-pv/tests/perf/{realapp_matrix.md,matrix_remote.sh,graphics_remote.sh,apps/}`,
`tests/integration/attach_verify.cu`, `docs/reference/{parity,blender-opendata,openbenchmarking-clpeak}.md`,
`tests/uvm_multiarch/build_apps.sh`, `tests/validate.sh`.

Harness: `scripts/apps/`. One binary set is built on the host (`build_bundle.sh`: CUDA 12.6
toolkit, `-arch=sm_86`, cuda-samples v12.5, gpu-burn, llama.cpp `4b1a27f` CUDA, clpeak 1.1.2,
vkpeak 20250531) and the **same files** run on both sides; `setup_side.sh` installs the identical
runtime on host and guest (torch 2.6.0+cu124, torchvision 0.21.0, transformers 4.46.3,
cupy-cuda12x 13.3.0, distro ffmpeg / hashcat / vulkan-tools / clinfo, Blender 4.5.0, Geekbench
6.4.0). `run_apps.sh` holds every app's command and pass predicate: **rc 0 and the app's own success
string and no `CHECK … FAIL`** (predicates were calibrated on the host baseline; many cuda-samples
verify on the CPU and report only through their exit code).

| group | apps | pass predicate |
|---|---|---|
| nvidia-smi | `nvidia_smi` | lists the RTX GPU |
| cuda-samples v12.5 | deviceQuery, vectorAdd, vectorAddDrv, matrixMul, matrixMulDrv, bandwidthTest, simpleStreams, asyncAPI, simpleAtomicIntrinsics, simpleCallback, simpleOccupancy, simpleZeroCopy, simpleCooperativeGroups, concurrentKernels, simpleIPC, cudaTensorCoreGemm, bf16TensorCoreGemm, globalToShmemAsyncCopy, cdpSimpleQuicksort, graphMemoryNodes, simpleCudaGraphs, simpleCUBLAS, simpleCUFFT, conjugateGradient, MersenneTwisterGP11213, reduction, sortingNetworks, scan, histogram, BlackScholes, fastWalshTransform, transpose | the sample's own CPU verification (PASS line or exit code) |
| nvkvm-pv realapp kernels | stream_triad, reduce, nbody, blackscholes, mandelbrot, conv2d, sgemm_cublas, fft_cufft, sha256, memcpy2d | their `CHECK ok` (value checks; several are only `isfinite`) |
| managed memory | attach_verify, conjugateGradientUM, UnifiedMemoryStreams, UnifiedMemoryPerf | own check (UMStreams/UMPerf: completion only) |
| stress | gpu_burn 60 s | `GPU 0: OK` (0 compare errors) |
| PyTorch | torch_correct (new: GPU vs CPU matmul fp32/fp16, cuDNN conv, CNN train step, 1 GiB round trip, sort/cumsum, 4 streams), torch_ai_bench (nvkvm-pv `ai_bench.py`) | every `CHECK ok`; torch_correct is numerical vs CPU |
| LLM | hf_generate (transformers greedy, Qwen2-0.5B fp16), llama_cpp_gen (llama.cpp, Qwen2.5-1.5B Q4_K_M greedy), llama_bench | completes; greedy output digest compared host vs guest |
| CuPy | cupy (NVRTC kernel, cuBLAS, cuFFT, cuRAND vs NumPy) | every `CHECK ok` |
| Vulkan / OpenGL / OpenCL | vulkaninfo, vkpeak, egl_offscreen (EGL device platform), clinfo, clpeak, geekbench_gpu (GB6 OpenCL) | NVIDIA device / GFLOPS / no GL error / NVIDIA platform / bandwidth / all GB workloads run |
| video | nvenc_h264, nvenc_hevc, nvdec_h264 (`-hwaccel cuda`, fails on software fallback) | 600 frames encoded (ffprobe) / decoded on NVDEC |
| crypto / render | hashcat (md5 mask attack), blender_cycles (scripted Cycles scene, CUDA then OptiX) | cracks the plaintext / both renders finish |
| probe (added) | stream_probe {default, created, nonblocking, perthread, two, created2nd} | one launch + full verify on that stream shape |

**Out of scope on a 1-GPU 8–12 GB box**: multi_gpu_app.py, nccl_allreduce.py, vLLM tensor-parallel
(≥2 GPUs); vLLM Qwen2.5-32B-AWQ (48 GB); llama.cpp 7B → 1.5B (same code path, fits the 8 GB card);
RAPIDS cuDF, glmark2 + weston, Unsloth, `validate.sh` (nvkvm-specific bring-up; its CUDA/Vulkan/GL
checks are covered above). Blender's Open Data launcher was replaced by a scripted render because
its mirror TLS-timed-out on the first box.

## 2. Results

One app per **fresh boot** (`APPS_PER_BOOT=1`), because a boot degrades after ~6 CUDA processes (§3
B) — a batched verdict would be contaminated by the apps before it. Guest cells carry the root-cause
letter of §3. `cupy`, EGL, Vulkan, NVENC/NVDEC and the stream probes are from the harness-corrected
re-run `r2` (r1 lacked the CUDA headers CuPy's NVRTC path needs, and did not load `nvidia_drm` in the
guest as the host has it — neither changed a verdict except CuPy). The **PM column** is a second
experiment: guest persistence mode on (`nvidia-smi -pm 1`) and ALL rows in ONE boot, in table order;
its verdicts were **identical on both boxes** for every row it reached, and the boot wedged at
`llama_bench` (the 60th process, cause J) — every later row in that boot hung.

| app | host 3060 | guest 3060 | host 3070 | guest 3070 | guest + PM, ONE boot (both boxes) | failure point (first failing box) |
|---|---|---|---|---|---|---|
| nvidia_smi | PASS | PASS | PASS | PASS | PASS |  |
| deviceQuery | PASS | PASS | PASS | PASS | PASS |  |
| vectorAdd | PASS | PASS | PASS | PASS | PASS |  |
| vectorAddDrv | PASS | PASS | PASS | PASS | PASS |  |
| matrixMul | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at matrixMul.cu:206 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| matrixMulDrv | PASS | PASS | PASS | PASS | PASS |  |
| bandwidthTest | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at bandwidthTest.cu:834 code=719(cudaErrorLaunchFailure) "cudaDeviceSynchronize()" |
| simpleStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleStreams.cu:371 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| asyncAPI | PASS | PASS | PASS | PASS | PASS |  |
| simpleAtomicIntrinsics | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleAtomicIntrinsics.cu:122 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| simpleCallback | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCallback.cu:146 code=719(cudaErrorLaunchFailure) "status" |
| simpleOccupancy | PASS | PASS | PASS | PASS | PASS |  |
| simpleZeroCopy | PASS | PASS | PASS | TIMEOUT (B) | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| simpleCooperativeGroups | PASS | PASS | PASS | PASS | PASS |  |
| concurrentKernels | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at concurrentKernels.cu:196 code=719(cudaErrorLaunchFailure) "cudaEventSynchronize(stop_event)" |
| simpleIPC | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleIPC.cu:161 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(&verification_buffer[0], ptrs |
| UnifiedMemoryStreams | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at UnifiedMemoryStreams.cu:221 code=13(CUBLAS_STATUS_EXECUTION_FAILED) "cublasDgemv(handle[tid + 1] |
| UnifiedMemoryPerf | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Running .CUDA error at matrixMultiplyPerf.cu:435 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(strea |
| conjugateGradientUM | PASS | FAIL (C) | PASS | FAIL (C) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC1 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ — Test Summary: Error amount = 1.000000, result = SUCCESS |
| cudaTensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| bf16TensorCoreGemm | PASS | PASS | PASS | PASS | PASS |  |
| globalToShmemAsyncCopy | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at globalToShmemAsyncCopy.cu:863 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| cdpSimpleQuicksort | PASS | PASS | PASS | PASS | PASS |  |
| graphMemoryNodes | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at graphMemoryNodes.cu:322 code=719(cudaErrorLaunchFailure) "cudaMemcpyAsync(hostArrays->square, d_ |
| simpleCudaGraphs | PASS | FAIL (A) | PASS | TIMEOUT (B) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at simpleCudaGraphs.cu:273 code=719(cudaErrorLaunchFailure) "cudaGraphLaunch(graphExec, streamForGr |
| simpleCUBLAS | PASS | PASS | PASS | PASS | PASS |  |
| simpleCUFFT | PASS | PASS | PASS | PASS | PASS |  |
| conjugateGradient | PASS | PASS | PASS | PASS | PASS |  |
| MersenneTwisterGP11213 | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CUDA error at MersenneTwister.cpp:115 code=719(cudaErrorLaunchFailure) "cudaStreamSynchronize(stream)" |
| reduction | PASS | PASS | PASS | PASS | PASS |  |
| sortingNetworks | PASS | PASS | PASS | PASS | PASS |  |
| scan | PASS | PASS | PASS | PASS | PASS |  |
| histogram | PASS | PASS | PASS | PASS | PASS |  |
| BlackScholes | PASS | PASS | PASS | PASS | PASS |  |
| fastWalshTransform | PASS | PASS | PASS | PASS | PASS |  |
| transpose | PASS | PASS | PASS | PASS | PASS |  |
| stream_triad | PASS | PASS | PASS | PASS | PASS |  |
| reduce | PASS | PASS | PASS | PASS | PASS |  |
| nbody | PASS | PASS | PASS | PASS | PASS |  |
| blackscholes | PASS | PASS | PASS | PASS | PASS |  |
| mandelbrot | PASS | PASS | PASS | PASS | PASS |  |
| conv2d | PASS | PASS | PASS | PASS | PASS |  |
| sgemm_cublas | PASS | PASS | PASS | PASS | PASS |  |
| fft_cufft | PASS | PASS | PASS | PASS | PASS |  |
| sha256 | PASS | PASS | PASS | PASS | PASS |  |
| memcpy2d | PASS | PASS | PASS | PASS | PASS |  |
| attach_verify | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — FAIL cudaStreamSynchronize(s[b]) -> 719 (unspecified launch failure) |
| gpu_burn | PASS | FAIL (G) | PASS | FAIL (G) | FAIL | SIGSEGV in gpu_burn right after cuInit (guest dmesg: segfault at the stack top); no Xid, no kf3 refusal |
| torch_correct | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: GET was unable to find an engine to execute this computation |
| torch_ai_bench | PASS | FAIL (C) | PASS | FAIL (A) | FAIL | Xid 31, MMU Fault: ENGINE GRAPHICS GPC2 faulted @VA. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE — RuntimeError: CUDA error: unspecified launch failure |
| hf_generate | PASS | PASS | PASS | TIMEOUT (I) | PASS | silent hang; kf3 `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` (3070 box ran `fb-mb=6144`) |
| cupy | PASS | TIMEOUT (B) | PASS | PASS | PASS | silent hang (no output until the kill); kf3: slot N is full and nothing can be retired |
| llama_cpp_gen | PASS | TIMEOUT (B) | PASS | FAIL (A) | FAIL | 3060: silent hang, kf3 `slot 16 is full and nothing can be retired`; 3070: Xid 13 SKEDCHECK05 |
| llama_bench | PASS | FAIL (A) | PASS | FAIL (A) | TIMEOUT (J) | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — /workspace/apps/srcs/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu:109: CUDA error |
| vulkaninfo | PASS | TIMEOUT (F) | PASS | TIMEOUT (F) | hung (after J) | blocks inside an RM ioctl (strace: last call `NV_ESC_RM_CONTROL`, then nothing); guest dmesg: `scrubberDestruct: Timed out`, `ce_utils.c:349` assert |
| vkpeak | PASS | FAIL (F) | PASS | FAIL (F) | hung (after J) | No vulkan device |
| egl_offscreen | PASS | FAIL (E) | PASS | FAIL (E) | hung (after J) | CHECK egl_gl_Mtri_s FAIL |
| clinfo | PASS | PASS | PASS | PASS | hung (after J) |  |
| clpeak | PASS | PASS | PASS | PASS | not reached |  |
| nvenc_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264_nvenc @ 0x5aeb9435b9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvenc_hevc | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [hevc_nvenc @ 0x5b9be2c6a9c0] OpenEncodeSessionEx failed: unsupported device (2): (no details) |
| nvdec_h264 | PASS | FAIL (D) | PASS | FAIL (D) | not reached | [h264 @ 0x6003c48bbf80] decoder->cvdl->cuvidGetDecoderCaps(&caps) failed -> CUDA_ERROR_NO_DEVICE: no CUDA-capa |
| hashcat | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — Watchdog: Temperature abort trigger set to 90c |
| blender_cycles | PASS | TIMEOUT (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — RuntimeError: Error: Launch failed in CUDA queue synchronize (integrator_shade_surface) |
| geekbench_gpu | PASS | FAIL (A) | PASS | FAIL (A) | not reached | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — [0926/022702:ERROR:optimizer.cpp(122)] build_patches_padding: optimization failed for size { 32, 1, 1, }: Wait |
| stream_default | PASS | PASS | PASS | PASS | PASS |  |
| stream_created | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK created FAIL sync=719(unspecified launch failure) |
| stream_nonblocking | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK nonblocking FAIL sync=719(unspecified launch failure) |
| stream_perthread | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK perthread FAIL sync=719(unspecified launch failure) |
| stream_two | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK two_a FAIL sync=719(unspecified launch failure) |
| stream_created2nd | PASS | FAIL (A) | PASS | FAIL (A) | FAIL | Xid 13, SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed — CHECK c2_created FAIL sync=719(unspecified launch failure) |

A 26 ['MersenneTwisterGP11213', 'UnifiedMemoryPerf', 'UnifiedMemoryStreams', 'attach_verify', 'bandwidthTest', 'blender_cycles', 'concurrentKernels', 'geekbench_gpu', 'globalToShmemAsyncCopy', 'graphMemoryNodes', 'hashcat', 'llama_bench', 'llama_cpp_gen', 'matrixMul', 'simpleAtomicIntrinsics', 'simpleCallback', 'simpleCudaGraphs', 'simpleIPC', 'simpleStreams', 'stream_created', 'stream_created2nd', 'stream_nonblocking', 'stream_perthread', 'stream_two', 'torch_ai_bench', 'torch_correct']

Totals (65 apps, probes excluded): **host 65/65 on both; guest 35/65 (3060), 34/65 (3070)**.
Digests: `hf_generate` greedy tokens **identical** host vs guest on the 3060 (`0d973108a6251e14`).
Raw rows: `traces/v3_app_matrix/<box>/{r1,r2,seq}/{host,guest}.res` + `triage.txt`; evidence excerpts
in `traces/v3_app_matrix/va1_rtx3060/evidence_excerpts.txt`.

## 3. Failure causes, ranked by apps blocked

| rank | cause | apps blocked (of 65) | signature |
|---|---|---|---|
| 1 | **A — kernels on a non-default stream fault the host GR** | **21** (+ all 5 non-default stream probes) | host `Xid 13, Graphics Exception: SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE failed`, class `c7c0`; kf3 `RC host twin … (guest chid 0x8/0x9, engine 0x1) … Xid 13 — forwarding RC_TRIGGERED`; app sees `cudaErrorLaunchFailure (719)` |
| 2 | **B — VA-space slot exhaustion ⇒ silent hang** | **5** directly (cupy, llama_cpp_gen, simpleZeroCopy, simpleAtomicIntrinsics, simpleCudaGraphs — nondeterministic, box-dependent) **and every app from the ~6th–7th CUDA process of a boot on** | kf3 `REFUSED VasKey(..) root 0x201000: slot N is full and nothing can be retired — raise WalkCfg::runs_per_pdb` + `split ticket N REFUSED`; the process then makes no progress, no Xid |
| 3 | **D — video engines absent** | 3 (nvenc_h264, nvenc_hevc, nvdec_h264) | `OpenEncodeSessionEx failed: unsupported device`; `cuvidGetDecoderCaps … CUDA_ERROR_NO_DEVICE`; no kf3 refusal (the guest never asks) |
| 4 | **C — MMU fault on a managed/UVM mapping** | 2 (conjugateGradientUM — **silent wrong answer**, torch_ai_bench on the 3060) | host `Xid 31 … MMU Fault: ENGINE GRAPHICS … FAULT_PDE`, first compute channel (guest chid 0x7) |
| 5 | **F — Vulkan** | 2 (vulkaninfo, vkpeak) | vulkaninfo blocks in an RM ioctl; vkpeak `No vulkan device`; guest RM `scrubberDestruct: Timed out`, `ce_utils.c:349` |
| 6 | E — EGL | 1 (egl_offscreen) | `eglInitialize failed`, no Xid, no kf3 refusal |
| 6 | G — gpu_burn | 1 | SIGSEGV in gpu_burn right after `cuInit` (reads past its stack); no Xid |
| 6 | J — host map `NoMemory` late in a long PM boot | 1 (llama_bench as the ~60th process; the boot then hangs for every later app) | kf3 `REFUSED VasKey(..) root 0x1f1cac000: 1 run(s) not applied: window map 0x110000+0x200000 (store @0x1000000): arm: NV_ESC_RM_MAP_MEMORY store@0x1000000+0x200000: NoMemory` |
| 6 | I — walk refused beyond the FB span | 1 (hf_generate, 3070 only) | `REFUSED walk: run[0] leaves the guest's GPGA: gpga=0x1d5ed0000 … span is 0x180000000` with `fb-mb=6144` |

### A — non-default streams (rank 1)
The split is exact and measured by `stream_probe` (r2, both boxes): **`default` PASS; `created`,
`nonblocking`, `perthread`, `two`, `created2nd` all FAIL with 719**, and `created2nd` passes its
default-stream half then fails the created-stream half in the same process. Every other A row names
a stream in its failing call (`cudaStreamSynchronize(stream)`, `cudaEventSynchronize`,
`cudaGraphLaunch`, cuBLAS/cuDNN handles bound to streams). The faulting host channel is always the
guest's **second or third** compute channel (guest chid 0x8/0x9); the first (chid 0x7) runs
default-stream work fine. `SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE` means the QMD asked for more shader
local memory than that channel's context has configured — i.e. the per-channel (per-subcontext)
local-memory setup that the driver does for the first channel is not in effect for the additional
channels on the host. ⚠ Mechanism is a hypothesis; the signature and the stream split are measured.
None of the failing user kernels use local memory themselves (`cuobjdump -res-usage`: `LOCAL:0`).

### B — slot exhaustion (rank 2)
`seq` experiment: 10× `vectorAdd` in ONE boot → **6 PASS then 4 TIMEOUT on the 3060, 5 PASS then 5
TIMEOUT on the 3070**, with `slot 58 is full and nothing can be retired` at root `0x201000` at the
first hang. The first r1 attempt (all apps batched in one boot) wedged at the 3rd CUDA app and every
later app timed out silently (`traces/v3_app_matrix/va1_rtx3060/r1_batched_contaminated/`). Single
processes with many mappings (llama.cpp loading a 1 GB model, CuPy) hit it on their own. ⇒ In
practice this blocks *every* app for any guest that has already run a handful of CUDA processes; it
is ranked 2 only because the per-boot matrix isolates it. With guest persistence mode the budget disappears (§4).

### C — Xid 31 on managed memory (rank 4)
`conjugateGradientUM` finishes, prints `result = SUCCESS`, and reports `Error amount = 1.000000`
(host `0.000000`): a **silent wrong result** behind a host MMU fault (`FAULT_PDE`, VA
`0x7d09_ac200000`, a UVM managed range). `torch_ai_bench` hit the same fault class on the 3060 (the
3070 run died earlier, on A).

## 4. Persistence mode, and what is still open

- ★ **Persistence mode removes B's per-boot budget.** `seqpm` (3060, `nvidia-smi -pm 1` in the guest,
  then `stream_created` + 9× `vectorAdd` in ONE boot): **all 9 vectorAdd PASS** (without PM: 6 then
  hang), and `stream_created` still FAILs with 719 (A is independent of PM). ⇒ B is tied to the guest
  RM tearing the adapter down and re-initialising it per process (what happens without PM), not to the
  processes themselves. `traces/v3_app_matrix/va1_rtx3060/seqpm/`. A full one-boot PM run of all 71
  rows (`pm1`) is the PM column of §2: B vanishes, A/C/G are unchanged, J appears at the 60th process.
- EGL / Vulkan / gpu_burn root causes are not localised beyond the signatures above (no kf3 refusal is
  logged for any of them).

## 5. Reproduce

    # on a box provisioned by scripts/bench/{provision_box,provision_host_driver,provision_bench_tree,build_kf3}.sh
    ln -sfn /workspace/apps /opt/apps
    bash scripts/apps/build_bundle.sh            # host: CUDA 12.6 toolkit + the bundle
    bash scripts/apps/setup_side.sh              # host runtime (identical to the guest's)
    bash scripts/apps/provision_guest_apps.sh    # guest image: bundle + runtime (stock qemu, slirp)
    bash scripts/apps/apps_matrix.sh host  <run> all
    KF3_BIN=/workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 bash scripts/apps/apps_matrix.sh guest <run> all
    #   (KF3_FB_MB=6144 on an 8 GB card; APPS_PER_BOOT=N to batch; diag_hook.sh for strace diagnostics)
