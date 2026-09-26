# V3 refusal audit — every non-OK status kf3 returns where bare metal returns OK

**STATUS: LIVE, 2026-09-26.** Measured on `vrf` (vast 52742480, RTX 3090 **GA102**, host
`580.159.04`, guest `580.159.04`). Baseline kf3 `131f4841` (= v3-mapfix `56032c46` + the refusal
ledger, which only logs); fixes first measured at `f8e6a513`; branch `v3-refusals` then rebased
onto master `dd3aed08` (which had meanwhile landed v3-mapfix and v3-blackwell) as `393012fd` and
re-verified there (§6). ⚠ `131f4841` and `f8e6a513` are the pre-rebase commits of this same patch
series; after the rebase they are on no branch — the citable revision is `393012fd`, and every
merge-bar number is re-measured at it. The instruments are in the tree and re-runnable (§2).

## 1. Why this audit exists

v3-mapfix (`56032c46`) found that kf3 answered `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` with
`0x56`, a blocked CUDA/OpenCL waiter read that as the end of its wait, `clFinish` returned 1 s
early, and the guest freed a buffer the host GPU was still writing (host Xid 31). The rule it
taught, from the live oracle (`CLAUDE.md` of the research artifact, `tests/mode2/nvdiff/`):
**bare metal returns a non-OK RM status once in 613 records** (`0x2080012f`, in `cuInit`), so
every other non-OK status kf3 returns is a divergence until proven harmless.

The question has two halves, and one instrument sees only one of them:

- **Userspace-visible**: statuses the guest's user libraries read (libcuda, cudart, the Vulkan and
  EGL UMDs, libnvidia-encode, NVML). Measured by recording every `/dev/nvidia*` ioctl on the host
  and in the guest for the same workloads and comparing status censuses.
- **Kernel-side**: refusals between guest RM and our faked GSP that never reach userspace but
  change what guest RM does next (a swallowed flush, a skipped teardown, a fallback). Measured by
  a new ledger in the FSM that records every non-OK reply the guest read.

## 2. Method and instruments

| instrument | what it records | where |
|---|---|---|
| nvdiff shim | every `/dev/nvidia*` ioctl, params before AND after, per process tree (`LD_PRELOAD`) | `archive/nvkvm/tests/mode2/nvdiff/nvdiff_shim.c` (byte-identical to the research tree's) |
| nvdiff selftest | the differ detects (479 / 5) and has a zero noise floor | `nvd_selftest.sh` → **SELFTEST PASS** (run first, 2026-09-26) |
| `scripts/nvdiff_status.py` | **status** census per op (escape + control id / class / UVM ioctl); a guest `(op, status)` the host never read for that op is a divergence, independent of order | `census`, `compare`, `matrix`, `selftest` (6 host r1/r2 pairs → 0 divergent; one flipped status → exactly 1) |
| `kf_gsp::RefusalLedger` | every non-OK reply the FSM **posted**, keyed `(function, control id / class, status)`, at both posting sites (immediate, and a held reply's final status) | `crates/kf-gsp/src/refusal.rs`; kf3 logs `kf3: GSP REFUSED …` once per row and `gsp_refusals[…]` in the heartbeat |
| `rf_ledger.py` | the ledger's per-workload delta (heartbeat at workload start vs end) | `scripts/bench/refusal_audit/` |
| `rf_workloads.sh` / `rf_hook.sh` / `rf_run.sh` | the same 23 workloads on the host (bare metal) and in the kf3 fat guest, one capture each | `scripts/bench/refusal_audit/` |

UVM statuses are read at `offsetof(<X>_PARAMS, rmStatus)`, compiled from ogkm's `uvm_ioctl.h`
(`nvdiff_status.py gen-uvm`). Control and status names are read from ogkm at run time.

**Workloads** (host and guest, same binaries — the v3-apps2 bundle): CUDA ladder `cup2` `cup3`
`cup8`; `blocksync` (new: three blocking-sync waits on a 2.5 s `%globaltimer` spin — the class-A
probe); `deviceQuery`; `torch_correct` (includes an SGD step) and `torch_train` (ResNet-18, fp32 +
AMP, a >1 s synchronize); `llama.cpp` generate; `clpeak`; `vulkaninfo` + `vkpeak`;
`egl_offscreen`; ffmpeg NVENC h264/hevc and NVDEC h264; `nvidia-smi` incl. `-l 1`; `gpu_burn 10`;
cuda-samples `simpleStreams`, `simpleCudaGraphs`, `graphMemoryNodes`, `simpleIPC`, `asyncAPI`,
`simpleCallback`.

**Baseline run** (`h2`/`g2`, kf3 `131f4841`): host 22/22 PASS, guest 22/22 PASS; 26 295 host
records, 41 divergent userspace `(op, status)` rows, 70 distinct kernel-side ledger rows.

## 3. Classes

- **A** — on a wait / completion / poll or data-ordering path: the runtime may end a wait, skip a
  sync, or believe a flush happened. The dangerous class.
- **B** — changes a capability or feature decision: a different path, a disabled feature, a wrong
  device attribute.
- **C** — cosmetic or telemetry (`nvidia-smi` fields), or a status the guest logs and ignores with
  no state left different.

## 4. The dangerous and decision-changing rows (A and B)

| control | who calls it | host status | our status (before) | class | fixed? | evidence |
|---|---|---|---|---|---|---|
| `0x20801702` `MC_SERVICE_INTERRUPTS` | libcuda / NVIDIA OpenCL after each 1 s blocking-wait slice (guest `intr.c:200-225` returns our status) | NV_OK | `0x56` | **A** | **yes** — v3-mapfix `56032c46` (on master since `01b8cb5a`) | clpeak FP64 449.9 vs 218 GFLOPS, host Xid 31 (mapfix); here `blocksync` 3/3 waits dt=2.50 s ≥ kernel 2.50 s in the guest, clpeak FP64 638.95 vs host 639.16, and the row never appears in the ledger |
| `0x20800a70` `INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` | guest RM `kbusSendSysmembarSingle_KERNEL` (`kern_bus.c:419-434`): before every L2 op in `kmemsysCacheOp_HAL` (map/unmap/free of GPU-cached sysmem, UVM external-allocation PTE build, RM CPU mapping of GPU-written sysmem), and userspace `FB_FLUSH_GPU_CACHE(FB_FLUSH_YES)` | (physical RM performs it) | `0x56`, **swallowed** — `kbusSendSysmembar_IMPL` keeps only `NV_ERR_TIMEOUT` (`kern_bus.c:400-406`), so the caller proceeds as if flushed and the user control returns NV_OK | **A** | **yes** — `501ed2ea` | 4–5 per CUDA process, 119 per `vkpeak`, 19 per `vulkaninfo` (§5.1); after: `sysmembars=65` served by the host verb in 250–440 µs, 0 refusals |
| `0x00801814` `DMA_UNSET_PAGE_DIRECTORY` | nvidia-uvm at VA-space teardown (`uvm_gpu_va_space_unset_page_dir`, `deconfigure_address_space`) | NV_OK | `0x56`; the guest overwrites it with its local revoke's status (`dma.c:606-641`), so the refusal's effect was on **kf3**: the plane kept the withdrawn root and later all-PDB invalidates walked page-directory memory UVM frees right after the reply | **A (latent, kf3-side)** | **yes** — `a5062dfb` | 2 per CUDA process; after: `root_unsets=10`, 0 refusals |
| `0x2080a026` (GSS-legacy, 532 B) + `0x2080a084` (4 B) | libcudart init, every process (22× `torch_correct`, 54× `llama.cpp`) | NV_OK: `{…, GPC 1695000 kHz, MCLK 9751000 kHz}`; `a084` writes nothing | `0x56` → cudart took the `0x2080a001` fallback (`kf_abi::cudartinit`, a captured GA106 row) | **B** | **yes** — `393012fd` (`kf_abi::gssreplay` rows, asked of the host at realize) | guest `deviceQuery`: **GPU Max Clock rate 420 MHz**; host: **1695 MHz** — `cudaDevAttrClockRate` wrong in every guest process (§5.3) |
| `0x2080012b` `GPU_PROMOTE_CTX` (golden-image channel) | guest RM `kgraphicsCreateGoldenImageChannel` at every `RmInitAdapter` (hObject `0xbaba0045`, kernel GR channel) | NV_OK | `0x56` (`kf3: GPU_PROMOTE_CTX: no passthrough twin`; kernel GR channels are not born, "P7") | B | no | golden image falls back to lazy creation (`kernel_graphics.c:459-463`); `gpuStatePostLoad` maps `0x56` to NV_OK (`gpu.c:3437-3438`) — **any other status would abort `RmInitAdapter`**. Every workload passes. Truthful answer needs the kernel GR channel birthed |
| RM_ALLOC `0xc36f` `VOLTA_CHANNEL_GPFIFO_A` | RC watchdog, `krcWatchdogInit` (`osinit.c:2159-2162`) | NV_OK | `0x56` (`AllocClassNotPermitted`, not on the allowlist) | B→C | no | watchdog never initialised; Linux disables it right after init anyway (`osinit.c:2165`); `RmInitAdapter` treats `0x56` as benign (`osinit.c:2168-2172`) |
| RM_ALLOC `0x402c` `NV40_I2C` | `RmUnixAllocRmApi` (`osinit.c:1764-1778`) | NV_OK | `0x56` (`NoPhysicalBoardBus`) | B (no compute effect) | no — **correct to refuse**: a guest must not reach the host board's I2C | guest registers no I2C adapters |
| `0x20800a34` `INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER` | `kgraphicsLoadStaticInfo_KERNEL` once per GPU life; NULL cache ⇒ the user control `0x20801230` returns `0x56` | (physical) | `0x56` | **B, and fatal on Blackwell**: GB203 libcuda asks `0x20801230` and `cuInit` returned `CUDA_ERROR_NO_DEVICE` | **yes, on master** — v3-blackwell `256ec854` (host fact from the host's `GR_GET_SM_ISSUE_RATE_MODIFIER`) | no GA102 workload calls `0x2080123x` (host census: 0 calls in 23 workloads) |
| `0x20800b03` / `0x20800b05` `…SM_ISSUE_RATE_MODIFIER_V2` / `…SM_ISSUE_THROTTLE_CTRL` | same issuer; NULL cache ⇒ `0x2080123c` / `0x2080123d` return `0x56` | (physical) | `0x56` | B where a runtime asks the user control; unreached on GA10x | no | the same host-fact path as `0x20800a34` applies; no measured caller yet |
| `0x20800ab8` `INTERNAL_GET_PCIE_P2P_CAPS` | `_kp2pCapsGetStatusOverPcie` (`p2p_caps.c:555-569`) | NV_OK | `0x56` → P2P caps 0, no loopback bit | B (low: one GPU, no peers) | no | UVM reports peer access unsupported; no single-GPU workload differs |
| `0x20801210` `GR_SET_CTXSW_PREEMPTION_MODE` on a **copy-engine** channel | Vulkan UMD (`hChannel 0xbeed0101`, CE engine, `cilp = CTA`) | NV_OK | `0x56` (no GR twin behind a CE channel) | B (low) | no | 1 of 4 in `vulkaninfo`, 3 of 29 in `vkpeak`; the GR channels' requests are served; vkpeak within 1 % of host |
| `0x2080200a` `PERF_BOOST` (→ kernel `INTERNAL_PERF_BOOST_SET_2X` `0x20800a9a`) | libcuda at context creation (`flags 0x12` = BOOST_TO_MAX \| CUDA, duration ∞); Vulkan / EGL (`0x2`, 2 s); guest RM returns our status unchanged (`kern_perf_boost.c:64-65, 94`) | NV_OK | `0x56` (the user call fails; no wait involved) | B (performance, low) | no — measured without effect | during a guest run the host GPU sat in **P2 at 1695 MHz** (the board's rated boost) at 0 % utilisation; guest vkpeak fp32 27 049 vs host 27 146 GFLOPS (−0.4 %), clpeak FP64 638.95 vs 639.16, cup8bench N=2048 within the host's own spread. The truthful answer would be an **authored** host `PERF_BOOST` on kf3's own subdevice (the control is `NON_PRIVILEGED` on the host), held for the guest's duration — deferred until a workload shows a cost |
| RM_ALLOC `0x9072` `GF100_DISP_SW` → `0x1f`; `/dev/nvidia-modeset` ioctl → `EPERM` | Vulkan / EGL UMD init | NV_OK / 0 | refused by the **guest's own** RM and NVKMS: the display engine is amputated (`0x20800a4b`, `AmputationIntended`) | B (by design: headless) | no — design (`V3_HEADLESS_GRAPHICS.md`) | EGL and Vulkan workloads pass |

## 5. What each fix does

### 5.1 `INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` — the host GPU's sysmembar (`kf_rm::sysmembar`)

Refused, the flush was a completion the guest read without anything completing — the
MC_SERVICE_INTERRUPTS shape, one level down. The old tree's triage (`crate::sweep`, row
`0x20800a70`) kept it refused until *"the day forwarding is on"*; in v3 guest work runs on the host
GPU, so it is served: `SysmembarPolicy` (seated with the memory plane) carries
`MemStatement::Sysmembar` to the VA thread, which performs the authored, unprivileged host verb
`NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE(FB_FLUSH_YES)` — the same verb that already serves a Hopper+
guest's token-register sysmembar (`kf_trap::cacheop::CacheOp::FbFlush`) — and the NV_OK is **held**
until that statement settles. Nothing blocks the drainer or runs under the GSP lock. Without a
memory plane the control stays refused (never an unbacked NV_OK). Tests: `kf-rm/tests/sysmembar.rs`.

### 5.2 `DMA_UNSET_PAGE_DIRECTORY` — withdraw the root it names (`kf_rm::barpde`)

`PageDirPolicy` answers it NV_OK (params echoed) and carries `MemStatement::UnsetPageDir` for the
**original** VA-space object (UVM unsets through its dup, as it set); the VA thread clears the root
(`VasTable::clear_root` — rows stay until the object retires; nothing unmaps on a guess) and the
reply is held until that has settled, so no walk reads the root after UVM frees it. `hVASpace = 0`
or a short params image is declined to the ledger as before. ⊘ Not done: unmapping the mirror's
rows at unset (MPS-style detach). The object retires with its free a few ms later in every
measured teardown. Tests: `barpde` unit test and `kf-rm/tests/sysmembar.rs`.

### 5.3 cudart's clock pair — answered from the host (`kf_abi::gssreplay`)

`0x2080a026` / `0x2080a084` are GSS-legacy (bit 15; no open header names them). They are now
`gssreplay` rows: at realize kf3 asks its host subdevice exactly the request cudart sends (the
words constant across all 270 recorded `a026` calls, 135 bare metal and 135 guest) and keeps what the host wrote; a guest request is
answered only if every named input matches. The host answers the **max** clocks (1 695 000 and
9 751 000 kHz, identical in 135 calls), so a realize-time answer is the die's answer, not a
snapshot. ⊘ The earlier verdict *"measured innocent"* (`kf_abi::cudartinit`) measured
`cudaGetDeviceCount` only; it is corrected in place.

`gssreplay` now asks each row **twice** — over zero and over `0xa5` — because one zero probe cannot
tell *"the host wrote 0"* from *"untouched"*: the whole `u32` the host writes at `0x0c` would have
kept three bytes of the guest's uninitialised stack above its low byte. A byte is the host's if it
changed in either probe and carries the same value in both; otherwise the zero-probe answer is kept.
`[measured vrf, every boot]` both cudart rows took the two-probe answer; the two older
`0x2080a028` rows (libnvidia-encode's clock query) did not — the host reads more of that request
than the words the row names, so the `0xa5` probe was refused or answered differently — and they
kept exactly their previous zero-probe answer, as designed.
Tests: `kf-abi` `gssreplay` unit tests, `kf-rm/tests/cudart_clocks.rs`.

## 6. Verification

### 6.1 The divergences are gone — re-recorded, same box, same workloads

`g4` = the 23 workloads re-recorded on kf3 `f8e6a513` (fixes applied) against the same bare-metal
captures (`h2`, `h3`):

| measure | baseline `131f4841` | fixed `f8e6a513` |
|---|---|---|
| userspace divergent `(op, status)` rows | 41 | **39** — gone: `0x2080a026`, `0x2080a084`; none new |
| kernel-side ledger rows (distinct) | 70 | **66** — gone: `0x20800a70`, `0x00801814`, `0x2080a026`, `0x2080a084`; none new |
| kernel-side refusals, summed over the workloads | 2 264 | **1 713** |
| guest `deviceQuery` `GPU Max Clock rate` | **420 MHz** | **1695 MHz** (host: 1695 MHz) |
| ioctl records, guest vs host — `torch_correct` / `llama_gen` | 949 / 1 018 vs 839 / 748 | **839 / 748** — identical to bare metal (cudart no longer takes its fallback branch) |
| `blocksync` (three 2.5 s blocking-sync waits) | 3/3 ok | 3/3 ok (dt 2.50 s ≥ the kernel's own 2.50 s) |
| host sysmembars performed for the guest (`mem[… sysmembars=]`) | 0 (refused) | 65 in one 8-workload boot, 250–440 µs each |
| workloads PASS, host / guest | 22/22 / 22/22 | 23/23 / 23/23 |

### 6.2 The merge bar

| check | `f8e6a513` (pre-rebase) | **`393012fd`** (rebased on master `dd3aed08`) |
|---|---|---|
| `kf-*` crate tests (all 17 `kf-*` crates, `--no-fail-fast`) | 1 482 passed / 0 failed (at `6fc25d7b`, which repaired a cap1b count v3-mapfix had left red; master made the identical repair, so the rebase dropped it as empty) | **1 494 passed / 0 failed** |
| `v3_gates.sh` | 9/9 | **9/9** |
| `KF_DEVICE=kf3 fast_suite.sh … 180` (raw client and fast guest rebuilt at the revision) | 30/30 | **30/30** |
| CUDA ladder, guest (`cup2` `cup3` `cup8` `cup8bench`) | 8/8 PASS, as the baseline binary (8/8); cup8bench N=2048 1964.9 vs 1966.1 GFLOPS baseline, host 1978–2212 | **4/4 PASS** |
| re-recorded workloads | 23/23 PASS, divergences as §6.1 | **8/8 PASS** (`deviceQuery` `blocksync` `cup2` `torch_correct` `llama_gen` `clpeak` `vkpeak` `nvidia_smi`) |

### 6.3 At the rebased tip, the fixed divergences stay gone

`g5`, kf3 `393012fd`, against the same bare-metal captures: `0x2080a026` / `0x2080a084` absent from
every userspace census; `0x20800a70`, `0x00801814`, `0x2080a026`, `0x2080a084` (and master's
`0x20800a34`) absent from the kernel ledger; `deviceQuery` **1695 MHz**; `blocksync` 3/3; guest ioctl
record counts equal to bare metal for `cup2` 403, `deviceQuery` 422, `torch_correct` 839,
`llama_gen` 748, `blocksync` 415; the boot's heartbeat `sysmembars=158 root_unsets=12`.

⊘ Master moved on to `e05ff74d` (v3-gpcmask) while this ran. The branch merges onto it with no
conflict, and the merge commit's `kf-*` crate tests pass **1 507 / 0** (measured on `vrf`); the
hardware lanes above were not re-run on that merge.

Evidence (every capture, compressed, with the verification logs):
`traces/v3_refusal_audit/ga102_vrf/`. The userspace matrix re-derives from it offline (its
README; reproduces the 39 rows of §6.1).

## 7. Class C — kept refused, the list

Telemetry and log-only. Each is either a status the guest ignores with no state left different, or
a field `nvidia-smi` prints as `N/A` / `ERR!`.

| control | who calls it | host status | our status | class | fixed? | evidence |
|---|---|---|---|---|---|---|
| `0x20810108` (NV2081 BINAPI, 992 B) | libcuda init | NV_OK, **nothing written** | `0x56` | C (semantics unknown) | no | 19/19 CUDA workloads pass; an all-zero request answered with nothing written may be a set — not answerable without knowing |
| `0x20810107` (NV2081 BINAPI, 2 B) | Vulkan / EGL init | NV_OK, nothing written | `0x56` | C (semantics unknown) | no | EGL, Vulkan pass |
| `0x2080a0d1` (GSS-legacy, 2 024 B, ~1 Hz) | Vulkan UMD perf sampling | NV_OK, changing samples | `0x56` | C | no | dynamic data: a realize-time replay would be a false sample; vkpeak within 1 % of host |
| `0x20808165` (GSS-legacy) | libnvidia-encode / NVDEC | NV_OK 6×, `0x57` 4× | `0x56` 2 of 3 | C | no | NVENC/NVDEC frame counts equal to host |
| `0x20802a12` `CE_IS_DECOMP_LCE_ENABLED` (kernel) | `GET_ENGINES(_V2)`, ~12 per CUDA process | (physical) | `0x56` → `bDecompEnabled = FALSE` | C | no | FALSE is GA10x's truth (no decompression LCEs; `kceMapPceLceForDecomp_GB100` is GB100-family only) |
| `0x20800a9e` / `0x20800a9c` / `0x20800a1e` unregister client-shadow / replayable fault buffer / access-counter buffer (kernel) | nvidia-uvm when the last VA space releases the GPU | (physical) | `0x56` → `LEVEL_ERROR … proceeding` | C | no (trivially servable: kf3 holds no fault-buffer state) | the next register succeeds (every later process in a boot passes) |
| `0x20800afe` / `0x20800aff` RUSD init / data poll (kernel) + `0x00de0001` `REQUEST_DATA_POLL` (user) | guest RM init; NVML | NV_OK | `0x56` | C | no | RUSD holds telemetry only (clocks, utilisation, power, PMA/BAR1 free); readers see `lastModifiedTimestamp = 0` and fall back; `crate::sweep` rows argue the refusal (no publisher) |
| init-time static info the sweep already triages: `0x20800a87` NVLINK, `0x20800a4b` display IP, `0x20800a80` SLI boost sync, `0x20802a0f` PCE config, `0x2080017e` VMMU segment, `0x20800a30` PPC masks, `0x20800a2e` ROP info, `0x20800a3f` / `0x20800a38` FECS trace (kernel, once per GPU life) | `gpuStatePreInit` / `kgraphicsLoadStaticInfo` / … | (physical) | `0x56` | C (argued per row) | no | `crates/kf-rm/src/sweep.rs` (`AmputationIntended` / `RefusalIsInvisible`, with ogkm citations) |
| `0x2080013f` `GPU_GET_OEM_BOARD_INFO` | `gpuStatePostLoad` + nvidia-smi | NV_OK | `0x56` | C | no | only readers are serial-number log lines (`kernel_rc.c:265-269`) |
| nvidia-smi telemetry: `0x00800294` brand caps, `0x20800513` / `0x20808513` thermal, `0x2080852a` `0x2080852e` `0x20808536` `0x20808546` `0x20809004` `0x20809038` `0x2080a080` `0x2080a097` `0x2080a0a7` `0x2080a0f2` `0x2080a612` `0x2080a618` `0x2080a63c` (GSS-legacy), `0x20810110` / `0x20810111` (BINAPI), `0x2080014b` / `0x20800156` / `0x20800157` / `0x20801357` InfoROM, `0x20802068` P-state, `0x20802087` video perfmon, `0x20800102` GPU_GET_INFO_V2 (1 of 35 indices), `0x208001a4` chip details, `0x20801813` / `0x20801819` PEX counters, `0x20801829` / `0x20801830` PCIe atomics caps, `0x20803083` NVLink platform, `0x20803401` / `0x20803404` ECC, `0x20801322` / `0x20801344` offlined pages / remapped rows; allocs `0x0073` (display common), `0x208f` (subdevice diag), `0x90e7` (InfoROM) | nvidia-smi / NVML (`gpu_burn` via NVML) | mostly NV_OK; ECC status `0x2080012f` is `0x56` on bare metal too (not a divergence) | `0x56` | C | no | nvidia-smi exits 0 and prints the fields it can; telemetry the guest has no truthful source for, or none it may be shown |

## 8. What this audit does not cover

- Only GA102 was measured. The classes are per control, but a family whose libraries call a
  control GA10x's do not (the SM-issue-rate controls on Blackwell) needs its own run.
- Only statuses are compared. A control answered NV_OK with the **wrong body** (the `#203` class)
  is invisible here; `nvdiff.py diff` compares bodies for one program at a time.
- Guest-kernel paths no workload exercised (MIG, profilers, debugger, suspend/resume) were not
  reached, so their refusals are not in the ledger.
