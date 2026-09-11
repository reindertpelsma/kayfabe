# The implementation surface — everything kayfabe must answer

**STATUS: LIVE, 2026-09-11.** Target: **GA106 (RTX 3060), driver 580.159.04, open kernel
module.** Five independent source surveys, assembled. Every status below was traced to a
`match` arm, a dispatch table entry or an NVOC export array — never to a `#define`, a comment
or a string literal.

## ⚠ Read these five caveats before using any number here

1. **`research_clones/ogkm` is 610.43.02, NOT 580.** The version kayfabe targets is a separate
   checkout, `research_clones/ogkm-580.159.04`. The RPC and control surveys used the 580 tree;
   the PRAMIN and BAR1/BAR2 findings elsewhere in `docs/design/` were read from 610 and
   re-verified against 580 only for their load-bearing claims.
2. **The `ROUTE_TO_PHYSICAL` (0x40) predicate is NOT the whole answer.** It is complete for the
   *open* module. Kayfabe serves **10 control ids that appear in no ogkm header at all** —
   closed-driver GSS-legacy and cudart-init ids, confirmed present in a real GA106 CUDA trace.
   A predicate derived from open source is structurally blind to them, and kayfabe must support
   both drivers.
3. **ABSENT is not silent.** An unhandled RPC or control is answered `NV_ERR_NOT_SUPPORTED`
   (`0x56`) with a zeroed body and a row in the unserviced ledger. "90 % absent" means
   *unimplemented and recorded*, not *unsafe* and not *ignored*.
4. **A green test can hold a wall in place.** Several gaps below are invisible because our boots
   have never produced the traffic that would expose them. They are marked.
5. Counts are of **distinct ids**, not calls.

## The headline: the reachable surface is far smaller than the ABI

| surface | ABI defines | reachable on our path | kayfabe answers |
|---|---|---|---|
| GSP RPC functions | 230 | **15** | 12 |
| RM controls (physical-routed) | 753 (+84 by other transports) | 837 total | 64 |
| BAR0 registers | 16 MiB aperture | — | 69 discrete + 2 windows |
| Channel classes (guest-facing) | 35 examined | — | 7 served, 8 stub |
| Pushbuffer methods | 202 across 4 classes | — | 23 served, 5 partial |
| Isolate verbs | — | — | 12 plans, 9 live |

★ **The GSP function surface collapses from 230 to 15** because 83 ids have no sender anywhere
in the driver, 25 are literally empty macros, and **103 are vGPU-only transports a GSP-offload
guest sends as ordinary controls over function 76 instead**. Three real-hardware captures on
three boards (GA106/GA102/AD102, two driver versions, `n_dropped=0`) agree exactly on the
vocabulary, and 127 of our own boot ledgers recorded only two ids ever unanswered.

## The gaps worth acting on, ranked

| # | gap | why it matters | status |
|---|---|---|---|
| 1 | **Working-set gate is vacuous** — an `Untracked` VA reaches a real copy engine; the only production caller passes `&[]` | cross-process isolation, the owner's stated priority | live, census added w477, **not measured** |
| 2 | **UVM kernel channel completes unconditionally** — `⊘ NO REFRESH, NO PUBLISH`, release written anyway | synchronization point (3) has no ordering guarantee; the code says so itself | live, named in-tree |
| 3 | **Silent write-back truncation** — short host reply leaves the guest's own request bytes in the tail, returns `NV_OK` | the house failure: completion without correctness | live, no counter |
| 4 | **`CONTINUATION_RECORD` (fn 71) unreachable** in the shipping chain | real hardware sends 2 per boot; a fragmented control would be truncated then refused | latent, 0 in 127 ledgers |
| 5 | **MMU fault buffers absent** (`0xB83000..0xB8311C`) | UVM polls GET/PUT directly out of a BAR0 mapping | largest register gap |
| 6 | **`VOLTA_USERMODE_A` (`0xc361`) refused** — permitted but no params arm | UVM hardcodes it pre-Hopper; would break device create | unconfirmed on a boot |
| 7 | **`0x110094` unimplemented** — `FALCON_DEBUGINFO`, the CrashCat wayfinder | reads 0; was ~99 % of BAR0 reads in the C artifact | measured unclaimed |
| 8 | **`0x20802209` served but unreachable** on the open driver | dead code, or it masks a divergence | one confirmed scoping bug |

## Distinguishability: ten ways a guest can tell an emulated channel from a real one

The sharpest is that **our ring cursor never advances** while completions land — a state no
real host FIFO produces. The others: the four-word timestamp is a host CPU clock in a different
timebase; a one-word payload is zero-extended into an eight-byte slot; the completion interrupt
is raised before its leaf bit is latched, inverting hardware's order; the interrupt ignores the
guest's enable masks; a channel without a BIND gets no vector at all; the os-event wake is a
broadcast to every registration rather than the owner; the non-deferred arm runs inline on the
vCPU against its own declared contract; bytes never reach real device memory; and a refused
doorbell produces no error notifier where hardware would write a robust-channel code.

## What the tables below are

- **§A** — all 231 GSP RPC function rows.
- **§B** — all 753 physical-routed control rows.
- **§C** — the 84 controls that reach GSP by other transports.
- BAR0 registers, channel methods and the isolate surface are in the sections after those.

---

## §A — GSP RPC functions (231 rows)

| # | RPC function name | kayfabe status | evidence (file:line) | notes |
|---|---|---|---|---|
| 0 | `NOP` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 1 | `SET_GUEST_SYSTEM_INFO` (RM) | **SERVED** | `KF:crates/kayfabe-gsp/src/rpc.rs:213` classify; `KF:crates/kayfabe-device/src/guestsysinfo.rs:112-118` answers a **negotiated** RPC version via `encode_set_guest_system_info_reply`; `KF:crates/kayfabe-device/src/inittables.rs:1395` latches the guest `NV_VERSION_STRING` | Version handshake. Real content both directions. |
| 2 | `ALLOC_ROOT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:inc/kernel/vgpu/rpc.h:198-221`, the `ALLOC_SHARE_DEVICE` macro, which issues `ALLOC_ROOT` first at `OG580:rpc.c:3363`); Only writer is inside `rpcAllocShareDevice_v03_00` (`OG580:rpc.c:3363`), itself vGPU-only. |
| 3 | `ALLOC_DEVICE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 4 | `ALLOC_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | `NV_RM_RPC_ALLOC_MEMORY` (`OG580:rpc.h:113-152`) has no `IS_FW_CLIENT` branch, but every caller is gated: `os_desc_mem.c:180` `IS_VIRTUAL`, `video_mem.c:963` `!IS_GSP_CLIENT`, `egm_mem.c`. 0 in all three real traces. |
| 5 | `ALLOC_CTX_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 6 | `ALLOC_CHANNEL_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:rpc.h:245-270`) |
| 7 | `MAP_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 8 | `BIND_CTX_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 9 | `ALLOC_OBJECT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:rpc.h:271-290`) |
| 10 | `FREE` (RM) | **SERVED** | `rpc.rs:215`; claimed at `KF:crates/kayfabe-rmrpc/src/policy.rs:1872` (`OBJECT_VERBS`); decoded `KF:crates/kayfabe-rmrpc/src/lib.rs:1955` (`translate_free`) → `RmEvent::Free` applied to the object graph | Also observed by `KF:crates/kayfabe-device/src/osevent.rs:489`. |
| 11 | `LOG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_LOG` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:63` |
| 12 | `ALLOC_VIDMEM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 13 | `UNMAP_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 14 | `MAP_MEMORY_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ⚠ `NV_RM_RPC_MAP_MEMORY_DMA` has **no** `IS_FW_CLIENT` branch — reachable in principle, gated on `pMemory->bRpcAlloc`, which `video_mem.c:963-994` sets only when `!IS_GSP_CLIENT`. `[measured w388]` **ZERO** across seven CUDA API families; 0 in all three real traces. |
| 15 | `UNMAP_MEMORY_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | Same gate as fn 14 (`OG580:src/nvidia/src/kernel/mem_mgr/virtual_mem.c:1863`). |
| 16 | `GET_EDID` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 17 | `ALLOC_DISP_CHANNEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 18 | `ALLOC_DISP_OBJECT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 19 | `ALLOC_SUBDEVICE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:rpc.h:367-392`) |
| 20 | `ALLOC_DYNAMIC_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 21 | `DUP_OBJECT` (RM) | **SERVED** | `rpc.rs:216`; `policy.rs:1895` (`OBJECT_VERBS`); `lib.rs:1918` (`translate_dup`) → `RmEvent::Dup`, binds dst to the source resource id | Added on a measurement (`policy.rs:1873-1894`): UVM dups libcuda's `FERMI_VASPACE_A`. |
| 22 | `IDLE_CHANNELS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ⚠ **Reachable on a GSP client**: `NV_RM_RPC_IDLE_CHANNELS` (`OG580:rpc.h:185-197`) has no `IS_FW_CLIENT` branch and `kfifoIdleChannelsPerDevice` is unconditionally `_KERNEL` (`OG580:src/nvidia/generated/g_kernel_fifo_nvoc.h:1090`). Never observed in any trace or boot. |
| 23 | `ALLOC_EVENT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:rpc.h:337-366`) |
| 24 | `SEND_EVENT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 25 | `REMAPPER_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 26 | `DMA_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 27 | `DMA_FILL_PTE_MEM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 28 | `MANAGE_HW_RESOURCE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 29 | `BIND_ARBITRARY_CTX_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 30 | `CREATE_FB_SEGMENT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 31 | `DESTROY_FB_SEGMENT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 32 | `ALLOC_SHARE_DEVICE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: the macro takes the physical-RMAPI branch under `IS_FW_CLIENT` (`OG580:rpc.h:198-221`) |
| 33 | `DEFERRED_API_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 34 | `REMOVE_DEFERRED_API` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 35 | `SIM_ESCAPE_READ` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 36 | `SIM_ESCAPE_WRITE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 37 | `SIM_MANAGE_DISPLAY_CONTEXT_DMA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 38 | `FREE_VIDMEM_VIRT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 39 | `PERF_GET_PSTATE_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 40 | `PERF_GET_PERFMON_SAMPLE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 41 | `PERF_GET_VIRTUAL_PSTATE_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 42 | `PERF_GET_LEVEL_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_PERF_GET_LEVEL_INFO` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:152` |
| 43 | `MAP_SEMA_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 44 | `UNMAP_SEMA_MEMORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 45 | `SET_SURFACE_PROPERTIES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_SET_SURFACE_PROPERTIES` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:80` |
| 46 | `CLEANUP_SURFACE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_CLEANUP_SURFACE` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:88` |
| 47 | `UNLOADING_GUEST_DRIVER` (RM) | **STUB** | `rpc.rs:217`; `KF:crates/kayfabe-device/src/inert.rs:116-121` + `:124-136` | Answers `NV_OK` with an **empty body** (zero-filled to the request length). `Disposition::ReplyRequired` (`rpc.rs:321`) — an unanswered fn 47 hangs `rmmod`. |
| 48 | `TDR_SET_TIMEOUT_STATE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 49 | `SWITCH_TO_VGA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_SWITCH_TO_VGA` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:96` |
| 50 | `GPU_EXEC_REG_OPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only at the caller: `subdevice_ctrl_gpu_regops.c:163` gates the whole RPC on `IS_VIRTUAL(pGpu)`. |
| 51 | `GET_STATIC_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 52 | `ALLOC_VIRTMEM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 53 | `UPDATE_PDE_2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 54 | `SET_PAGE_DIRECTORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control; On a GSP client this is control `0x00801813` over fn 76, which **is** served (`KF:crates/kayfabe-device/src/setpagedir.rs:377-386`). |
| 55 | `GET_STATIC_PSTATE_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 56 | `TRANSLATE_GUEST_GPU_PTES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 57 | `RESERVED_57` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 58 | `RESET_CURRENT_GR_CONTEXT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 59 | `SET_SEMA_MEM_VALIDATION_STATE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 60 | `GET_ENGINE_UTILIZATION` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_GET_ENGINE_UTILIZATION` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:163` |
| 61 | `UPDATE_GPU_PDES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 62 | `GET_ENCODER_CAPACITY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_GET_ENCODER_CAPACITY` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:120` |
| 63 | `VGPU_PF_REG_READ32` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 64 | `SET_GUEST_SYSTEM_INFO_EXT` (RM) | **STUB** | `rpc.rs:214`; `KF:crates/kayfabe-device/src/guestsysinfo.rs:122-126` | vGPU only: `NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:71`; Answers `NV_OK`, **empty body**, no decode. ⊘ Dead on our target: the macro is `/* VGPU only */` (`OG580:src/nvidia/inc/kernel/vgpu/rpc_vgpu.h:71-78`, `GPU_GET_VGPU_RPC`) and 0 occurrences in all three real-hardware traces. |
| 65 | `GET_GSP_STATIC_INFO` (GSP) | **SERVED** | `rpc.rs:218`; `KF:crates/kayfabe-device/src/staticinfo.rs:184-227` — size-checked, then a full `GspStaticConfigInfo` body | ⚠ **Constructed** from the chip/driver tables, not queried from the host GPU. `gpuNameString` is empty unless `KAYFABE_GPU_NAME` is set (`KF:crates/kayfabe-device/src/lib.rs:1290-1310`) — the known `nvidia-smi Name: ERR!` defect. |
| 66 | `RMFS_INIT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 67 | `RMFS_CLOSE_QUEUE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 68 | `RMFS_CLEANUP` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 69 | `RMFS_TEST` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 70 | `UPDATE_BAR_PDE` (RM) | **SERVED** | `rpc.rs:224`; `KF:crates/kayfabe-device/src/bar2.rs:300-320` — decodes and latches the BAR2 root PDE via `self.log.publish` | Real effect: the BAR2 aperture is unrooted until this arrives. Answers `NV_OK` empty body; malformed → `NV_ERR_INVALID_ARGUMENT`. |
| 71 | `CONTINUATION_RECORD` (RM) | **ABSENT** | Classified at `rpc.rs:220` but **no production link claims it**: `ObjectPolicy::respond_serviced` returns early for `RmControl` (`policy.rs:3468-3470`) and then gates on `OBJECT_VERBS` (`policy.rs:3472`), so `Reassembler::accept` (`KF:crates/kayfabe-rmrpc/src/reasm.rs:250`) is only reachable through `Bridge::deliver` (`policy.rs:1345`) for fn 10/21/103. Falls through to `NV_ERR_NOT_SUPPORTED` at `KF:crates/kayfabe-gsp/src/boot.rs:1762-1768` | ★ **A real latent gap.** Real GA106/GA102/AD102 traces each carry 2 `CONTINUATION_RECORD`s; our boots have never produced one (0 in 127 committed ledgers), so nothing has gone red. `lib.rs:1133` refuses a lone one as `ContinuationWithoutHead` by design. |
| 72 | `GSP_SET_SYSTEM_INFO` (RM) | **STUB** | `rpc.rs:221`; `Disposition::NoReply` at `rpc.rs:318-320`; `boot.rs:1726-1741` returns without replying | Correct per protocol (`_issueRpcAsync`, no waiter). ⊘ The payload (`systemInfo`) is **never decoded** — `lib.rs:1105` maps it to `Translation::Inert` and no crate reads it. |
| 73 | `SET_REGISTRY` (RM) | **STUB** | `rpc.rs:222`; `rpc.rs:318-320`; `boot.rs:1726-1741` | Same: accepted, not answered, **registry table discarded**. `lib.rs:1106` → `Translation::Inert`. |
| 74 | `GSP_INIT_POST_OBJGPU` (GSP) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 75 | `SUBDEV_EVENT_SET_NOTIFICATION` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 76 | `GSP_RM_CONTROL` (GSP) | **SERVED** | `rpc.rs:225`. Partial, by command id: `KF:crates/kayfabe-device/src/inittables.rs:1366+` (table controls), `KF:crates/kayfabe-rmrpc/src/policy.rs:1914+` (`OBJECT_CONTROLS`) dispatched at `policy.rs:2257-2302`, `KF:crates/kayfabe-device/src/setpagedir.rs:377-386` (`0x00801813`), `KF:crates/kayfabe-device/src/bar2.rs`/`gvaspub.rs`/`faultbuffer.rs` observers. Everything unclaimed → `NV_ERR_NOT_SUPPORTED` + the ledger at `KF:crates/kayfabe-device/src/unserviced.rs:309-322` | The whole control plane rides here. `[measured, traces/w297_cup3]` **52** distinct control ids answered, **40** distinct refused, and that boot still reached `CUP3_VAL=43`. |
| 77 | `GET_STATIC_INFO2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 78 | `DUMP_PROTOBUF_COMPONENT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ⚠ **Reachable on a GSP client** and explicitly so: `OG580:src/nvidia/src/kernel/diagnostics/journal.c:1216-1220` guards on `IS_GSP_CLIENT(pGpu)`. Crash-dump path only (`nvdebugdump`/NOCAT); never seen in a normal boot. |
| 79 | `UNSET_PAGE_DIRECTORY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control; Peer of fn 54; a GSP client sends the equivalent control over fn 76. |
| 80 | `GET_CONSOLIDATED_STATIC_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 81 | `GMMU_REGISTER_FAULT_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 82 | `GMMU_UNREGISTER_FAULT_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 83 | `GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 84 | `GMMU_UNREGISTER_CLIENT_SHADOW_FAULT_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 85 | `CTRL_SET_VGPU_FB_USAGE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 86 | `CTRL_NVFBC_SW_SESSION_UPDATE_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 87 | `CTRL_NVENC_SW_SESSION_UPDATE_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 88 | `CTRL_RESET_CHANNEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 89 | `CTRL_RESET_ISOLATED_CHANNEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 90 | `CTRL_GPU_HANDLE_VF_PRI_FAULT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 91 | `CTRL_CLK_GET_EXTENDED_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 92 | `CTRL_PERF_BOOST` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 93 | `CTRL_PERF_VPSTATES_GET_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 94 | `CTRL_GET_ZBC_CLEAR_TABLE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 95 | `CTRL_SET_ZBC_COLOR_CLEAR` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 96 | `CTRL_SET_ZBC_DEPTH_CLEAR` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 97 | `CTRL_GPFIFO_SCHEDULE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 98 | `CTRL_SET_TIMESLICE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 99 | `CTRL_PREEMPT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 100 | `CTRL_FIFO_DISABLE_CHANNELS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 101 | `CTRL_SET_TSG_INTERLEAVE_LEVEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 102 | `CTRL_SET_CHANNEL_INTERLEAVE_LEVEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 103 | `GSP_RM_ALLOC` (GSP) | **SERVED** | `rpc.rs:226`; `policy.rs:1871` (`OBJECT_VERBS`); `lib.rs:1094`→`lib.rs:1229` (`translate_alloc`) → `RmEvent::Alloc`; reply built by `RpcCommand::reply_alloc` (`boot.rs:1757-1760`) | Reads `wire_body()` not `payload` — fn 103's declared `length` stops where `params[]` begins (`KF:crates/kayfabe-gsp/src/rpc.rs:378-430`). |
| 104 | `CTRL_GET_P2P_CAPS_V2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 105 | `CTRL_CIPHER_AES_ENCRYPT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 106 | `CTRL_CIPHER_SESSION_KEY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 107 | `CTRL_CIPHER_SESSION_KEY_STATUS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 108 | `CTRL_DBG_CLEAR_ALL_SM_ERROR_STATES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 109 | `CTRL_DBG_READ_ALL_SM_ERROR_STATES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 110 | `CTRL_DBG_SET_EXCEPTION_MASK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 111 | `CTRL_GPU_PROMOTE_CTX` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 112 | `CTRL_GR_CTXSW_PREEMPTION_BIND` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 113 | `CTRL_GR_SET_CTXSW_PREEMPTION_MODE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 114 | `CTRL_GR_CTXSW_ZCULL_BIND` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 115 | `CTRL_GPU_INITIALIZE_CTX` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 116 | `CTRL_VASPACE_COPY_SERVER_RESERVED_PDES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 117 | `CTRL_FIFO_CLEAR_FAULTED_BIT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 118 | `CTRL_GET_LATEST_ECC_ADDRESSES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 119 | `CTRL_MC_SERVICE_INTERRUPTS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 120 | `CTRL_DMA_SET_DEFAULT_VASPACE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 121 | `CTRL_GET_CE_PCE_MASK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 122 | `CTRL_GET_ZBC_CLEAR_TABLE_ENTRY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 123 | `CTRL_GET_NVLINK_PEER_ID_MASK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 124 | `CTRL_GET_NVLINK_STATUS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 125 | `CTRL_GET_P2P_CAPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 126 | `CTRL_GET_P2P_CAPS_MATRIX` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 127 | `RESERVED_0` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 128 | `CTRL_RESERVE_PM_AREA_SMPC` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 129 | `CTRL_RESERVE_HWPM_LEGACY` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 130 | `CTRL_B0CC_EXEC_REG_OPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 131 | `CTRL_BIND_PM_RESOURCES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 132 | `CTRL_DBG_SUSPEND_CONTEXT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 133 | `CTRL_DBG_RESUME_CONTEXT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 134 | `CTRL_DBG_EXEC_REG_OPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 135 | `CTRL_DBG_SET_MODE_MMU_DEBUG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 136 | `CTRL_DBG_READ_SINGLE_SM_ERROR_STATE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 137 | `CTRL_DBG_CLEAR_SINGLE_SM_ERROR_STATE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 138 | `CTRL_DBG_SET_MODE_ERRBAR_DEBUG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 139 | `CTRL_DBG_SET_NEXT_STOP_TRIGGER_TYPE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 140 | `CTRL_ALLOC_PMA_STREAM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 141 | `CTRL_PMA_STREAM_UPDATE_GET_PUT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 142 | `CTRL_FB_GET_INFO_V2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 143 | `CTRL_FIFO_SET_CHANNEL_PROPERTIES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 144 | `CTRL_GR_GET_CTX_BUFFER_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 145 | `CTRL_KGR_GET_CTX_BUFFER_PTES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 146 | `CTRL_GPU_EVICT_CTX` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 147 | `CTRL_FB_GET_FS_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 148 | `CTRL_GRMGR_GET_GR_FS_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 149 | `CTRL_STOP_CHANNEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 150 | `CTRL_GR_PC_SAMPLING_MODE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 151 | `CTRL_PERF_RATED_TDP_GET_STATUS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ★ **Dead stub.** `rpcCtrlPerfRatedTdpGetStatus_v1A_1F` is defined (`OG580:rpc.c:6694`) and `rpcCtrlPerfRatedTdpGetStatus_HAL` has **zero call sites** — it is not a `rpcDmaControl_wrapper` case and no macro reaches it. Unreachable on **every** driver personality. |
| 152 | `CTRL_PERF_RATED_TDP_SET_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 153 | `CTRL_FREE_PMA_STREAM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 154 | `CTRL_TIMER_SET_GR_TICK_FREQ` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 155 | `CTRL_FIFO_SETUP_VF_ZOMBIE_SUBCTX_PDB` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 156 | `GET_CONSOLIDATED_GR_STATIC_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_GET_CONSOLIDATED_GR_STATIC_INFO` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:112` |
| 157 | `CTRL_DBG_SET_SINGLE_SM_SINGLE_STEP` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 158 | `CTRL_GR_GET_TPC_PARTITION_MODE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 159 | `CTRL_GR_SET_TPC_PARTITION_MODE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 160 | `UVM_PAGING_CHANNEL_ALLOCATE` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 161 | `UVM_PAGING_CHANNEL_DESTROY` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 162 | `UVM_PAGING_CHANNEL_MAP` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 163 | `UVM_PAGING_CHANNEL_UNMAP` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 164 | `UVM_PAGING_CHANNEL_PUSH_STREAM` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 165 | `UVM_PAGING_CHANNEL_SET_HANDLES` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 166 | `UVM_METHOD_STREAM_GUEST_PAGES_OPERATION` (UVM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 167 | `CTRL_INTERNAL_QUIESCE_PMA_CHANNEL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 168 | `DCE_RM_INIT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | DCE (Tegra display) client only — `NV_RM_RPC_DCE_RM_INIT` is called from `OG580:src/nvidia/src/kernel/gpu/dce_client/dce_client_rpc.c:138`. Not a GSP path. |
| 169 | `REGISTER_VIRTUAL_EVENT_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 170 | `CTRL_EVENT_BUFFER_UPDATE_GET` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 171 | `GET_PLCABLE_ADDRESS_KIND` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 172 | `CTRL_PERF_LIMITS_SET_STATUS_V2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 173 | `CTRL_INTERNAL_SRIOV_PROMOTE_PMA_STREAM` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 174 | `CTRL_GET_MMU_DEBUG_MODE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 175 | `CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 176 | `CTRL_FLCN_GET_CTX_BUFFER_SIZE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 177 | `CTRL_FLCN_GET_CTX_BUFFER_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 178 | `DISABLE_CHANNELS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 179 | `CTRL_FABRIC_MEMORY_DESCRIBE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 180 | `CTRL_FABRIC_MEM_STATS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 181 | `SAVE_HIBERNATION_DATA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_SAVE_HIBERNATION_DATA` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:128` |
| 182 | `RESTORE_HIBERNATION_DATA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_RESTORE_HIBERNATION_DATA` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:136` |
| 183 | `CTRL_INTERNAL_MEMSYS_SET_ZBC_REFERENCED` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 184 | `CTRL_EXEC_PARTITIONS_CREATE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 185 | `CTRL_EXEC_PARTITIONS_DELETE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 186 | `CTRL_GPFIFO_GET_WORK_SUBMIT_TOKEN` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 187 | `CTRL_GPFIFO_SET_WORK_SUBMIT_TOKEN_NOTIF_INDEX` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 188 | `PMA_SCRUBBER_SHARED_BUFFER_GUEST_PAGES_OPERATION` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no-op macro** in `OG580:inc/kernel/vgpu/rpc_vgpu.h:37-57` (or `rpc.h:422-427,486`) — the open driver cannot send it at all |
| 189 | `CTRL_MASTER_GET_VIRTUAL_FUNCTION_ERROR_CONT_INTR_MASK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 190 | `RESERVED_190` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 191 | `CTRL_SUBDEVICE_GET_P2P_CAPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 192 | `CTRL_BUS_SET_P2P_MAPPING` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 193 | `CTRL_BUS_UNSET_P2P_MAPPING` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 194 | `CTRL_FLA_SETUP_INSTANCE_MEM_BLOCK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 195 | `CTRL_GPU_MIGRATABLE_OPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 196 | `CTRL_GET_TOTAL_HS_CREDITS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 197 | `CTRL_GET_HS_CREDITS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 198 | `CTRL_SET_HS_CREDITS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 199 | `CTRL_PM_AREA_PC_SAMPLER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 200 | `INVALIDATE_TLB` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ⊘ vGPU only: `bDoVgpuRpc` requires `GPU_GET_VGPU(pGpu)` **and** a vGPU config cap bit (`OG580:src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm107.c:145-156`). Corroborates the campaign measurement that fn-200 count is **0** on the Mode-2 compute path. |
| 201 | `CTRL_GPU_QUERY_ECC_STATUS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **deprecated** in the enum; **no sender anywhere in `OG580/src`** |
| 202 | `ECC_NOTIFIER_WRITE_ACK` (RM) | **STUB** | `rpc.rs:223`; `rpc.rs:318-320`; `boot.rs:1726-1741` | Accepted, not answered, discarded. `lib.rs:1114` → `Translation::Inert`. |
| 203 | `CTRL_DBG_GET_MODE_MMU_DEBUG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 204 | `RM_API_CONTROL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_RM_API_CONTROL` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:171` |
| 205 | `CTRL_CMD_INTERNAL_GPU_START_FABRIC_PROBE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 206 | `CTRL_NVLINK_GET_INBAND_RECEIVED_DATA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | Sent only from `OG580:src/nvidia/src/kernel/vgpu/vgpu_events.c:372,386` — the vGPU event loop. |
| 207 | `GET_STATIC_DATA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_GET_STATIC_DATA` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:104` |
| 208 | `RESERVED_208` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **no sender anywhere in `OG580/src`** |
| 209 | `CTRL_GPU_GET_INFO_V2` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 210 | `GET_BRAND_CAPS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 211 | `CTRL_CMD_NVLINK_INBAND_SEND_DATA` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 212 | `UPDATE_GPM_GUEST_BUFFER_INFO` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_UPDATE_GPM_GUEST_BUFFER_INFO` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:184` |
| 213 | `CTRL_CMD_INTERNAL_CONTROL_GSP_TRACE` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 214 | `CTRL_SET_ZBC_STENCIL_CLEAR` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 215 | `CTRL_SUBDEVICE_GET_VGPU_HEAP_STATS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 216 | `CTRL_SUBDEVICE_GET_LIBOS_HEAP_STATS` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 217 | `CTRL_DBG_SET_MODE_MMU_GCC_DEBUG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 218 | `CTRL_DBG_GET_MODE_MMU_GCC_DEBUG` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 219 | `CTRL_RESERVE_HES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 220 | `CTRL_RELEASE_HES` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 221 | `CTRL_RESERVE_CCU_PROF` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 222 | `CTRL_RELEASE_CCU_PROF` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 223 | `SETUP_HIBERNATION_BUFFER` (RM) | **ABSENT** | **⟨FALLBACK⟩** | vGPU only: `NV_RM_RPC_SETUP_HIBERNATION_BUFFER` lives in `OG580:inc/kernel/vgpu/rpc_vgpu.h:144` |
| 224 | `CTRL_CMD_GET_CHIPLET_HS_CREDIT_POOL` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 225 | `CTRL_CMD_GET_HS_CREDITS_MAPPING` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 226 | `CTRL_EXEC_PARTITIONS_EXPORT` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 227 | `CTRL_CMD_INTERNAL_GPU_CHECK_CTS_ID_VALID` (RM) | **ABSENT** | **⟨FALLBACK⟩** | **vGPU/SR-IOV only** — `rpcDmaControl_wrapper` case (`OG580:rpc.c:4513-4851`), bypassed by `IS_FW_CLIENT` (`OG580:rpc.h:230-233`); a GSP client sends it as a fn-76 control |
| 228 | `INIT_GSP_TRACE_CRASH_BUFFER` (RM) | **STUB** | `rpc.rs:219`; `KF:crates/kayfabe-device/src/inert.rs:116-121` + `:124-136` | Answers `NV_OK` empty body. ⊘ The guest's declared crash-buffer physical address is deliberately **not recorded** (`inert.rs:90-93`). |
| 229 | `CTRL_GPU_SET_MIGRATION_BLOCK` (RM) | **ABSENT** | **⟨FALLBACK⟩** | ⚠ **610-only — this id does not exist in `OG580` at all** (`OG580` ends at `NUM_FUNCTIONS = 229`). No sender anywhere in `OG580/src`; in `OG610` it is a `rpcDmaControl_wrapper` case, i.e. vGPU-only. |
| 230 | `NUM_FUNCTIONS` (RM) | **— (sentinel)** | `OG610:src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:240` | Sentinel, not a function. ⚠ In `OG580` this name is **229** and `CTRL_GPU_SET_MIGRATION_BLOCK` does not exist. |

## §B — physical-routed RM controls (753 rows)

| id | symbolic name | cls | kf | evidence |
|---|---|---|---|---|
|`0x00730101`|`NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2`|Disp|T|disp_objs:3441 f=0x48|
|`0x00730102`|`NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS`|Disp|-|disp_objs:3456 f=0x4a|
|`0x00730104`|`NV0073_CTRL_CMD_SYSTEM_GET_SCANLINE`|Disp|-|disp_objs:3471 f=0x48|
|`0x00730107`|`NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED`|Disp|-|disp_objs:3516 f=0x82004a|
|`0x00730108`|`NV0073_CTRL_CMD_SYSTEM_GET_CONNECT_STATE`|Disp|-|disp_objs:3531 f=0x848|
|`0x00730109`|`NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_CONFIG`|Disp|-|disp_objs:3546 f=0x44|
|`0x0073010a`|`NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_STATE`|Disp|-|disp_objs:3561 f=0x48|
|`0x0073010b`|`NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP`|Disp|-|disp_objs:3576 f=0x48|
|`0x0073010c`|`NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE`|Disp|-|disp_objs:3591 f=0x48|
|`0x00730115`|`NV0073_CTRL_CMD_SYSTEM_GET_ACPI_ID_MAP`|Disp|-|disp_objs:3606 f=0x44|
|`0x00730116`|`NV0073_CTRL_CMD_SYSTEM_GET_INTERNAL_DISPLAYS`|Disp|-|disp_objs:3621 f=0x82004a|
|`0x00730118`|`NV0073_CTRL_CMD_SYSTEM_VALIDATE_SRM`|Disp|-|disp_objs:3651 f=0x44|
|`0x00730119`|`NV0073_CTRL_CMD_SYSTEM_GET_SRM_STATUS`|Disp|-|disp_objs:3666 f=0x44|
|`0x0073011b`|`NV0073_CTRL_CMD_SYSTEM_HDCP_REVOCATION_CHECK`|Disp|-|disp_objs:3681 f=0x44|
|`0x0073011c`|`NV0073_CTRL_CMD_SYSTEM_UPDATE_SRM`|Disp|-|disp_objs:3696 f=0x44|
|`0x0073011d`|`NV0073_CTRL_CMD_SYSTEM_GET_CONNECTOR_TABLE`|Disp|-|disp_objs:3711 f=0x44|
|`0x0073011e`|`NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS`|Disp|-|disp_objs:3726 f=0x44|
|`0x0073012c`|`NV0073_CTRL_CMD_SYSTEM_VRR_DISPLAY_INFO`|Disp|-|disp_objs:3756 f=0x44|
|`0x0073012e`|`NV0073_CTRL_CMD_SYSTEM_CLEAR_ELV_BLOCK`|Disp|-|disp_objs:3786 f=0x40|
|`0x0073012f`|`NV0073_CTRL_CMD_SYSTEM_ARM_LIGHTWEIGHT_SUPERVISOR`|Disp|-|disp_objs:3801 f=0x44|
|`0x00730134`|`NV0073_CTRL_CMD_SYSTEM_CONFIG_VRR_PSTATE_SWITCH`|Disp|-|disp_objs:3816 f=0x44|
|`0x0073013d`|`NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX`|Disp|-|disp_objs:3831 f=0x40|
|`0x00730144`|`NV0073_CTRL_CMD_SYSTEM_GET_HOTPLUG_EVENT_CONFIG`|Disp|-|disp_objs:3861 f=0x44|
|`0x00730145`|`NV0073_CTRL_CMD_SYSTEM_SET_HOTPLUG_EVENT_CONFIG`|Disp|-|disp_objs:3876 f=0x44|
|`0x0073014a`|`NV0073_CTRL_CMD_SYSTEM_RECORD_CHANNEL_REGS`|Disp|-|disp_objs:3891 f=0x40|
|`0x0073014b`|`NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT`|Disp|-|disp_objs:3906 f=0x40|
|`0x0073014c`|`NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_SR_SUPPORT`|Disp|-|disp_objs:3921 f=0x40|
|`0x00730157`|`NV0073_CTRL_CMD_SYSTEM_INTERNAL_ALLOCATE_DISPLAY_BANDWIDTH`|Disp|-|disp_objs:3951 f=0xc4|
|`0x00730159`|`NV0073_CTRL_CMD_SYSTEM_NOTIFY_DRR_MSCG_WAR`|Disp|-|disp_objs:3966 f=0x48|
|`0x00730211`|`NV0073_CTRL_CMD_SPECIFIC_GET_I2C_PORTID`|Disp|-|disp_objs:4011 f=0x48|
|`0x00730240`|`NV0073_CTRL_CMD_SPECIFIC_GET_TYPE`|Disp|-|disp_objs:4026 f=0x820046|
|`0x00730243`|`NV0073_CTRL_CMD_SPECIFIC_FAKE_DEVICE`|Disp|-|disp_objs:4041 f=0x44|
|`0x00730245`|`NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2`|Disp|-|disp_objs:4056 f=0x44|
|`0x00730246`|`NV0073_CTRL_CMD_SPECIFIC_SET_EDID_V2`|Disp|-|disp_objs:4071 f=0x44|
|`0x00730250`|`NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA`|Disp|-|disp_objs:4086 f=0x48|
|`0x00730260`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_REPEATER_INFO`|Disp|-|disp_objs:4101 f=0x44|
|`0x00730273`|`NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_ENABLE`|Disp|-|disp_objs:4116 f=0x44|
|`0x00730274`|`NV0073_CTRL_CMD_SPECIFIC_CTRL_HDMI`|Disp|-|disp_objs:4131 f=0x44|
|`0x00730275`|`NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_AUDIO_MUTESTREAM`|Disp|-|disp_objs:4146 f=0x44|
|`0x00730280`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_STATE`|Disp|-|disp_objs:4161 f=0x44|
|`0x00730281`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDCP_DIAGNOSTICS`|Disp|-|disp_objs:4176 f=0x44|
|`0x00730282`|`NV0073_CTRL_CMD_SPECIFIC_HDCP_CTRL`|Disp|-|disp_objs:4191 f=0x44|
|`0x00730285`|`NV0073_CTRL_CMD_SPECIFIC_GET_ACPI_DOD_DISPLAY_PORT_ATTACHMENT`|Disp|-|disp_objs:4221 f=0x44|
|`0x00730287`|`NV0073_CTRL_CMD_SPECIFIC_GET_ALL_HEAD_MASK`|Disp|-|disp_objs:4236 f=0x44|
|`0x00730288`|`NV0073_CTRL_CMD_SPECIFIC_SET_OD_PACKET`|Disp|-|disp_objs:4251 f=0x44|
|`0x00730289`|`NV0073_CTRL_CMD_SPECIFIC_SET_OD_PACKET_CTRL`|Disp|-|disp_objs:4266 f=0x40|
|`0x0073028a`|`NV0073_CTRL_CMD_SPECIFIC_GET_PCLK_LIMIT`|Disp|-|disp_objs:4281 f=0x40|
|`0x0073028b`|`NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO`|Disp|-|disp_objs:4296 f=0x46|
|`0x0073028d`|`NV0073_CTRL_CMD_SPECIFIC_HDCP_KSVLIST_VALIDATE`|Disp|-|disp_objs:4311 f=0x44|
|`0x0073028e`|`NV0073_CTRL_CMD_SPECIFIC_HDCP_UPDATE`|Disp|-|disp_objs:4326 f=0x44|
|`0x00730291`|`NV0073_CTRL_CMD_SPECIFIC_GET_BACKLIGHT_BRIGHTNESS`|Disp|-|disp_objs:4341 f=0x40|
|`0x00730292`|`NV0073_CTRL_CMD_SPECIFIC_SET_BACKLIGHT_BRIGHTNESS`|Disp|-|disp_objs:4356 f=0x40|
|`0x00730293`|`NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_SINK_CAPS`|Disp|-|disp_objs:4371 f=0x44|
|`0x00730295`|`NV0073_CTRL_CMD_SPECIFIC_SET_MONITOR_POWER`|Disp|-|disp_objs:4386 f=0x40|
|`0x0073029a`|`NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_FRL_CONFIG`|Disp|-|disp_objs:4401 f=0x44|
|`0x0073029b`|`NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_FRL_FLUSH_MODE`|Disp|-|disp_objs:4416 f=0x44|
|`0x007302a0`|`NV0073_CTRL_CMD_SPECIFIC_GET_REGIONAL_CRCS`|Disp|-|disp_objs:4431 f=0x44|
|`0x007302a1`|`NV0073_CTRL_CMD_SPECIFIC_APPLY_EDID_OVERRIDE_V2`|Disp|-|disp_objs:4446 f=0x40|
|`0x007302a2`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_GPU_CAPS`|Disp|-|disp_objs:4461 f=0x40|
|`0x007302a4`|`NV0073_CTRL_CMD_SPECIFIC_DISPLAY_CHANGE`|Disp|-|disp_objs:4476 f=0x40|
|`0x007302a6`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_SCDC_DATA`|Disp|-|disp_objs:4491 f=0x44|
|`0x007302a7`|`NV0073_CTRL_CMD_SPECIFIC_IS_DIRECTMODE_DISPLAY`|Disp|-|disp_objs:4506 f=0x40|
|`0x007302a8`|`NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_FRL_CAPACITY_COMPUTATION`|Disp|-|disp_objs:4521 f=0x44|
|`0x007302a9`|`NV0073_CTRL_CMD_SPECIFIC_SET_SHARED_GENERIC_PACKET`|Disp|-|disp_objs:4536 f=0x44|
|`0x007302aa`|`NV0073_CTRL_CMD_SPECIFIC_ACQUIRE_SHARED_GENERIC_PACKET`|Disp|-|disp_objs:4551 f=0x44|
|`0x007302ab`|`NV0073_CTRL_CMD_SPECIFIC_RELEASE_SHARED_GENERIC_PACKET`|Disp|-|disp_objs:4566 f=0x44|
|`0x007302ac`|`NV0073_CTRL_CMD_SPECIFIC_DISP_I2C_READ_WRITE`|Disp|-|disp_objs:4581 f=0x44|
|`0x007302ad`|`NV0073_CTRL_CMD_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT`|Disp|-|disp_objs:4596 f=0x48|
|`0x007302ae`|`NV0073_CTRL_CMD_SPECIFIC_DEFAULT_ADAPTIVESYNC_DISPLAY`|Disp|-|disp_objs:4611 f=0x40|
|`0x00730401`|`NV0073_CTRL_CMD_INTERNAL_GET_HOTPLUG_UNPLUG_STATE`|Disp|-|disp_objs:4626 f=0xc0|
|`0x00730404`|`NV0073_CTRL_CMD_INTERNAL_DFP_GET_DISP_MUX_STATUS`|Disp|-|disp_objs:4641 f=0xc0|
|`0x00730460`|`NV0073_CTRL_CMD_INTERNAL_DFP_SWITCH_DISP_MUX`|Disp|-|disp_objs:4656 f=0xc0|
|`0x00730502`|`NV0073_CTRL_CMD_FRL_CONFIG_MACRO_PAD`|Disp|-|disp_objs:4671 f=0x44|
|`0x00731140`|`NV0073_CTRL_CMD_DFP_GET_INFO`|Disp|-|disp_objs:4686 f=0x4a|
|`0x00731142`|`NV0073_CTRL_CMD_DFP_GET_DISPLAYPORT_DONGLE_INFO`|Disp|-|disp_objs:4701 f=0x48|
|`0x00731144`|`NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS`|Disp|-|disp_objs:4716 f=0x44|
|`0x0073114c`|`NV0073_CTRL_CMD_DFP_GET_SPREAD_SPECTRUM`|Disp|-|disp_objs:4731 f=0x44|
|`0x0073114e`|`NV0073_CTRL_CMD_DFP_UPDATE_DYNAMIC_DFP_CACHE`|Disp|-|disp_objs:4746 f=0x44|
|`0x00731150`|`NV0073_CTRL_CMD_DFP_SET_AUDIO_ENABLE`|Disp|-|disp_objs:4761 f=0x40|
|`0x00731152`|`NV0073_CTRL_CMD_DFP_ASSIGN_SOR`|Disp|-|disp_objs:4776 f=0x44|
|`0x00731153`|`NV0073_CTRL_CMD_DFP_GET_PADLINK_MASK`|Disp|-|disp_objs:4791 f=0x44|
|`0x00731154`|`NV0073_CTRL_CMD_DFP_GET_LCD_GPIO_PIN_NUM`|Disp|-|disp_objs:4806 f=0x44|
|`0x00731156`|`NV0073_CTRL_CMD_DFP_CONFIG_TWO_HEAD_ONE_OR`|Disp|-|disp_objs:4821 f=0x44|
|`0x00731157`|`NV0073_CTRL_CMD_DFP_DSC_CRC_CONTROL`|Disp|-|disp_objs:4836 f=0x44|
|`0x00731158`|`NV0073_CTRL_CMD_DFP_INIT_MUX_DATA`|Disp|-|disp_objs:4851 f=0x40|
|`0x00731161`|`NV0073_CTRL_CMD_DFP_RUN_PRE_DISP_MUX_OPERATIONS`|Disp|-|disp_objs:4881 f=0x40|
|`0x00731162`|`NV0073_CTRL_CMD_DFP_RUN_POST_DISP_MUX_OPERATIONS`|Disp|-|disp_objs:4896 f=0x40|
|`0x00731166`|`NV0073_CTRL_CMD_DFP_GET_DSI_MODE_TIMING`|Disp|-|disp_objs:4926 f=0x44|
|`0x00731172`|`NV0073_CTRL_CMD_DFP_GET_FIXED_MODE_TIMING`|Disp|-|disp_objs:4941 f=0x46|
|`0x00731174`|`NV0073_CTRL_CMD_DFP_ENTER_DISPLAY_POWER_GATING`|Disp|-|disp_objs:4956 f=0x40|
|`0x00731175`|`NV0073_CTRL_CMD_DFP_EXIT_DISPLAY_POWER_GATING`|Disp|-|disp_objs:4971 f=0x40|
|`0x00731176`|`NV0073_CTRL_CMD_DFP_EDP_DRIVER_UNLOAD`|Disp|-|disp_objs:4986 f=0x4a|
|`0x00731177`|`NV0073_CTRL_CMD_SYSTEM_SET_REGION_RAM_RECTANGLES`|Disp|-|disp_objs:5001 f=0x44|
|`0x00731178`|`NV0073_CTRL_CMD_SYSTEM_CONFIGURE_SAFETY_INTERRUPTS`|Disp|-|disp_objs:5016 f=0x44|
|`0x00731180`|`NV0073_CTRL_CMD_DFP_GET_DISP_PHY_INFO`|Disp|-|disp_objs:5031 f=0x44|
|`0x00731341`|`NV0073_CTRL_CMD_DP_AUXCH_CTRL`|Disp|-|disp_objs:5046 f=0x844|
|`0x00731343`|`NV0073_CTRL_CMD_DP_CTRL`|Disp|-|disp_objs:5061 f=0x844|
|`0x00731345`|`NV0073_CTRL_CMD_DP_GET_LANE_DATA`|Disp|-|disp_objs:5076 f=0x44|
|`0x00731346`|`NV0073_CTRL_CMD_DP_SET_LANE_DATA`|Disp|-|disp_objs:5091 f=0x44|
|`0x00731347`|`NV0073_CTRL_CMD_DP_SET_TESTPATTERN`|Disp|-|disp_objs:5106 f=0x44|
|`0x00731348`|`NV0073_CTRL_CMD_DP_GET_TESTPATTERN`|Disp|-|disp_objs:5121 f=0x44|
|`0x00731351`|`NV0073_CTRL_CMD_DP_SET_PREEMPHASIS_DRIVECURRENT_POSTCURSOR2_DATA`|Disp|-|disp_objs:5136 f=0x44|
|`0x00731352`|`NV0073_CTRL_CMD_DP_GET_PREEMPHASIS_DRIVECURRENT_POSTCURSOR2_DATA`|Disp|-|disp_objs:5151 f=0x44|
|`0x00731356`|`NV0073_CTRL_CMD_DP_MAIN_LINK_CTRL`|Disp|-|disp_objs:5166 f=0x44|
|`0x00731359`|`NV0073_CTRL_CMD_DP_SET_AUDIO_MUTESTREAM`|Disp|-|disp_objs:5181 f=0x44|
|`0x0073135a`|`NV0073_CTRL_CMD_DP_ASSR_CTRL`|Disp|-|disp_objs:5196 f=0x44|
|`0x0073135b`|`NV0073_CTRL_CMD_DP_TOPOLOGY_ALLOCATE_DISPLAYID`|Disp|-|disp_objs:5211 f=0x44|
|`0x0073135c`|`NV0073_CTRL_CMD_DP_TOPOLOGY_FREE_DISPLAYID`|Disp|-|disp_objs:5226 f=0x44|
|`0x00731360`|`NV0073_CTRL_CMD_DP_GET_LINK_CONFIG`|Disp|-|disp_objs:5241 f=0x44|
|`0x00731361`|`NV0073_CTRL_CMD_DP_GET_EDP_DATA`|Disp|-|disp_objs:5256 f=0x44|
|`0x00731362`|`NV0073_CTRL_CMD_DP_CONFIG_STREAM`|Disp|-|disp_objs:5271 f=0x44|
|`0x00731363`|`NV0073_CTRL_CMD_DP_SET_RATE_GOV`|Disp|-|disp_objs:5286 f=0x44|
|`0x00731365`|`NV0073_CTRL_CMD_DP_SET_MANUAL_DISPLAYPORT`|Disp|-|disp_objs:5301 f=0x44|
|`0x00731366`|`NV0073_CTRL_CMD_DP_SET_ECF`|Disp|-|disp_objs:5316 f=0x44|
|`0x00731367`|`NV0073_CTRL_CMD_DP_SEND_ACT`|Disp|-|disp_objs:5331 f=0x44|
|`0x00731369`|`NV0073_CTRL_CMD_DP_GET_CAPS`|Disp|-|disp_objs:5346 f=0x820046|
|`0x0073136c`|`NV0073_CTRL_CMD_DP_CONFIG_RAD_SCRATCH_REG`|Disp|-|disp_objs:5376 f=0x44|
|`0x0073136e`|`NV0073_CTRL_CMD_DP_CONFIG_SINGLE_HEAD_MULTI_STREAM`|Disp|-|disp_objs:5391 f=0x44|
|`0x0073136f`|`NV0073_CTRL_CMD_DP_SET_TRIGGER_SELECT`|Disp|-|disp_objs:5406 f=0x44|
|`0x00731370`|`NV0073_CTRL_CMD_DP_SET_TRIGGER_ALL`|Disp|-|disp_objs:5421 f=0x44|
|`0x00731373`|`NV0073_CTRL_CMD_DP_GET_AUXLOGGER_BUFFER_DATA`|Disp|-|disp_objs:5451 f=0x44|
|`0x00731377`|`NV0073_CTRL_CMD_DP_CONFIG_INDEXED_LINK_RATES`|Disp|-|disp_objs:5466 f=0x44|
|`0x00731378`|`NV0073_CTRL_CMD_DP_SET_STEREO_MSA_PROPERTIES`|Disp|-|disp_objs:5481 f=0x44|
|`0x0073137a`|`NV0073_CTRL_CMD_DP_CONFIGURE_FEC`|Disp|-|disp_objs:5496 f=0x44|
|`0x0073137b`|`NV0073_CTRL_CMD_DP_CONFIG_MACRO_PAD`|Disp|-|disp_objs:5511 f=0x44|
|`0x0073137c`|`NV0073_CTRL_CMD_DP_AUXCH_I2C_TRANSFER_CTRL`|Disp|-|disp_objs:5526 f=0x40|
|`0x0073137d`|`NV0073_CTRL_CMD_DP_ENABLE_VRR`|Disp|-|disp_objs:5541 f=0x844|
|`0x0073137e`|`NV0073_CTRL_CMD_DP_GET_GENERIC_INFOFRAME`|Disp|-|disp_objs:5556 f=0x44|
|`0x0073137f`|`NV0073_CTRL_CMD_DP_GET_MSA_ATTRIBUTES`|Disp|-|disp_objs:5571 f=0x44|
|`0x00731380`|`NV0073_CTRL_CMD_DP_AUXCH_OD_CTRL`|Disp|-|disp_objs:5586 f=0x44|
|`0x00731381`|`NV0073_CTRL_CMD_DP_SET_MSA_PROPERTIES_V2`|Disp|-|disp_objs:5601 f=0x44|
|`0x00731383`|`NV0073_CTRL_CMD_DP2X_LINK_TRAINING_CTRL`|Disp|-|disp_objs:5616 f=0x844|
|`0x00731384`|`NV0073_CTRL_CMD_DP2X_GET_LANE_DATA`|Disp|-|disp_objs:5631 f=0x44|
|`0x00731385`|`NV0073_CTRL_CMD_DP2X_SET_LANE_DATA`|Disp|-|disp_objs:5646 f=0x44|
|`0x00731386`|`NV0073_CTRL_CMD_DP_AUXCH_VBL_CTRL`|Disp|-|disp_objs:5661 f=0x40|
|`0x00731387`|`NV0073_CTRL_CMD_DP_SET_LEVEL_INFO_TABLE_DATA`|Disp|-|disp_objs:5676 f=0x44|
|`0x00731388`|`NV0073_CTRL_CMD_DP_GET_LEVEL_INFO_TABLE_DATA`|Disp|-|disp_objs:5691 f=0x44|
|`0x00731389`|`NV0073_CTRL_CMD_DP2X_SET_LEVEL_INFO_TABLE_DATA`|Disp|-|disp_objs:5706 f=0x44|
|`0x0073138a`|`NV0073_CTRL_CMD_DP2X_GET_LEVEL_INFO_TABLE_DATA`|Disp|-|disp_objs:5721 f=0x44|
|`0x0073138d`|`NV0073_CTRL_CMD_DP_GET_CABLEID_INFO_FROM_MACRO`|Disp|-|disp_objs:5751 f=0x44|
|`0x0073138f`|`NV0073_CTRL_CMD_DP_NOTIFY_LT`|Disp|-|disp_objs:5766 f=0x40|
|`0x00731602`|`NV0073_CTRL_CMD_PSR_GET_SR_PANEL_INFO`|Disp|-|disp_objs:5781 f=0x44|
|`0x00800105`|`NV0080_CTRL_CMD_BIF_ASPM_CYA_UPDATE`|Dev|-|device:498 f=0x40|
|`0x00800294`|`NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS`|Dev|T|device:708 f=0x40049|
|`0x00800296`|`NV0080_CTRL_GPU_SET_VGPU_VF_BAR1_SIZE`|Dev|-|device:723 f=0x44|
|`0x00801107`|`NV0080_CTRL_CMD_GR_GET_TPC_PARTITION_MODE`|Dev|-|device:798 f=0x10248|
|`0x00801108`|`NV0080_CTRL_CMD_GR_SET_TPC_PARTITION_MODE`|Dev|-|device:813 f=0x10248|
|`0x00801306`|`NV0080_CTRL_CMD_FB_GET_COMPBIT_STORE_INFO`|Dev|-|device:873 f=0x48|
|`0x00801707`|`NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES`|Dev|T|device:963 f=0x50148|
|`0x0080170e`|`NV0080_CTRL_CMD_FIFO_GET_LATENCY_BUFFER_SIZE`|Dev|-|device:993 f=0x50048|
|`0x0080170f`|`NV0080_CTRL_CMD_FIFO_SET_CHANNEL_PROPERTIES`|Dev|-|device:1008 f=0x10248|
|`0x00801711`|`NV0080_CTRL_CMD_FIFO_STOP_RUNLIST`|Dev|-|device:1023 f=0x10244|
|`0x00801712`|`NV0080_CTRL_CMD_FIFO_START_RUNLIST`|Dev|-|device:1038 f=0x10244|
|`0x00801805`|`NV0080_CTRL_CMD_DMA_FLUSH`|Dev|-|device:1098 f=0x50048|
|`0x00801b02`|`NV0080_CTRL_CMD_NVENC_GET_CAPS_V2`|Dev|-|device:1263 f=0x50148|
|`0x00801c02`|`NV0080_CTRL_CMD_NVDEC_GET_CAPS_V2`|Dev|T|device:1278 f=0x50148|
|`0x00801f02`|`NV0080_CTRL_CMD_NVJPG_GET_CAPS_V2`|Dev|-|device:1323 f=0x50048|
|`0x00802004`|`NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_DISABLE`|Dev|S|device:1338 f=0xc0|
|`0x00802009`|`NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_SET_CONTROL`|Dev|S|device:1353 f=0x1d8|
|`0x00802046`|`NV0080_CTRL_CMD_INTERNAL_KGR_INIT_BUG4208224_WAR`|Dev|-|device:1383 f=0x1d0|
|`0x20800112`|`NV2080_CTRL_CMD_GPU_SET_POWER`|Sub|-|subdevice:4042 f=0x48|
|`0x2080012b`|`NV2080_CTRL_CMD_GPU_PROMOTE_CTX`|Sub|S|subdevice:4147 f=0x10244|
|`0x2080012c`|`NV2080_CTRL_CMD_GPU_EVICT_CTX`|Sub|-|subdevice:4162 f=0x1c240|
|`0x2080012d`|`NV2080_CTRL_CMD_GPU_INITIALIZE_CTX`|Sub|-|subdevice:4177 f=0x14244|
|`0x2080012f`|`NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS`|Sub|T|subdevice:4192 f=0x50158|
|`0x20800133`|`NV2080_CTRL_CMD_GPU_QUERY_ECC_CONFIGURATION`|Sub|T|subdevice:4237 f=0x40048|
|`0x20800134`|`NV2080_CTRL_CMD_GPU_SET_ECC_CONFIGURATION`|Sub|-|subdevice:4252 f=0x40044|
|`0x20800136`|`NV2080_CTRL_CMD_GPU_RESET_ECC_ERROR_STATUS`|Sub|-|subdevice:4267 f=0x40044|
|`0x2080013f`|`NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO`|Sub|T|subdevice:4327 f=0x448|
|`0x2080014b`|`NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION`|Sub|T|subdevice:4417 f=0x10048|
|`0x2080014d`|`NV2080_CTRL_CMD_GPU_GET_IP_VERSION`|Sub|-|subdevice:4447 f=0x48|
|`0x20800153`|`NV2080_CTRL_CMD_GPU_QUERY_ILLUM_SUPPORT`|Sub|-|subdevice:4462 f=0x40048|
|`0x20800154`|`NV2080_CTRL_CMD_GPU_GET_ILLUM`|Sub|-|subdevice:4477 f=0x48|
|`0x20800156`|`NV2080_CTRL_CMD_GPU_GET_INFOROM_IMAGE_VERSION`|Sub|T|subdevice:4507 f=0x448|
|`0x20800157`|`NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT`|Sub|T|subdevice:4522 f=0x48|
|`0x2080015f`|`NV2080_CTRL_CMD_GPU_QUERY_SCRUBBER_STATUS`|Sub|-|subdevice:4567 f=0x40048|
|`0x20800160`|`NV2080_CTRL_CMD_GPU_GET_VPR_CAPS`|Sub|-|subdevice:4582 f=0x48|
|`0x20800169`|`NV2080_CTRL_CMD_GPU_GET_OEM_INFO`|Sub|-|subdevice:4627 f=0x448|
|`0x2080016b`|`NV2080_CTRL_CMD_GPU_GET_VPR_INFO`|Sub|-|subdevice:4642 f=0x48|
|`0x20800173`|`NV2080_CTRL_CMD_GPU_QUERY_FUNCTION_STATUS`|Sub|-|subdevice:4732 f=0x48|
|`0x20800177`|`NV2080_CTRL_CMD_GPU_REPORT_NON_REPLAYABLE_FAULT`|Sub|R|subdevice:4777 f=0x40040|
|`0x2080017e`|`NV2080_CTRL_CMD_GPU_GET_VMMU_SEGMENT_SIZE`|Sub|-|subdevice:4852 f=0x10448|
|`0x20800181`|`NV2080_CTRL_CMD_GPU_GET_PARTITION_CAPACITY`|Sub|-|subdevice:4867 f=0x40048|
|`0x20800192`|`NV2080_CTRL_CMD_GPU_HANDLE_VF_PRI_FAULT`|Sub|-|subdevice:4987 f=0x10248|
|`0x208001a0`|`NV2080_CTRL_CMD_GET_P2P_CAPS`|Sub|-|subdevice:5107 f=0x50048|
|`0x208001a4`|`NV2080_CTRL_CMD_GPU_GET_CHIP_DETAILS`|Sub|-|subdevice:5152 f=0x40448|
|`0x208001a9`|`NV2080_CTRL_CMD_GPU_MARK_DEVICE_FOR_RESET`|Sub|-|subdevice:5182 f=0x100048|
|`0x208001aa`|`NV2080_CTRL_CMD_GPU_UNMARK_DEVICE_FOR_RESET`|Sub|-|subdevice:5197 f=0x100048|
|`0x208001ab`|`NV2080_CTRL_CMD_GPU_GET_RESET_STATUS`|Sub|-|subdevice:5212 f=0x158|
|`0x208001ac`|`NV2080_CTRL_CMD_GPU_MARK_DEVICE_FOR_DRAIN_AND_RESET`|Sub|-|subdevice:5227 f=0x100048|
|`0x208001ad`|`NV2080_CTRL_CMD_GPU_UNMARK_DEVICE_FOR_DRAIN_AND_RESET`|Sub|-|subdevice:5242 f=0x100048|
|`0x208001ae`|`NV2080_CTRL_CMD_GPU_GET_DRAIN_AND_RESET_STATUS`|Sub|-|subdevice:5257 f=0x48|
|`0x208001af`|`NV2080_CTRL_GPU_GET_NVENC_SW_SESSION_INFO_V2`|Sub|-|subdevice:5272 f=0x40048|
|`0x208001b0`|`NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO`|Sub|S|subdevice:5287 f=0x10048|
|`0x208001e3`|`NV2080_CTRL_CMD_INTERNAL_CONTROL_GSP_TRACE`|Sub|-|subdevice:5332 f=0x102d0|
|`0x208001e4`|`NV2080_CTRL_GPU_GET_FIPS_STATUS`|Sub|-|subdevice:5347 f=0x44|
|`0x208001e6`|`NV2080_CTRL_CMD_GPU_GET_FIRST_ASYNC_CE_IDX`|Sub|-|subdevice:5377 f=0x10448|
|`0x208001f2`|`NV2080_CTRL_CMD_GSP_CRYPTO_CONTROL`|Sub|-|subdevice:5512 f=0x100044|
|`0x20800601`|`NV2080_CTRL_CMD_I2C_READ_BUFFER`|Sub|-|subdevice:5782 f=0x48|
|`0x20800602`|`NV2080_CTRL_CMD_I2C_WRITE_BUFFER`|Sub|-|subdevice:5797 f=0x48|
|`0x20800603`|`NV2080_CTRL_CMD_I2C_READ_REG`|Sub|-|subdevice:5812 f=0x48|
|`0x20800604`|`NV2080_CTRL_CMD_I2C_WRITE_REG`|Sub|-|subdevice:5827 f=0x48|
|`0x20800809`|`NV2080_CTRL_CMD_BIOS_GET_POST_TIME`|Sub|-|subdevice:5857 f=0x40048|
|`0x2080080b`|`NV2080_CTRL_CMD_BIOS_GET_UEFI_SUPPORT`|Sub|-|subdevice:5872 f=0x48|
|`0x2080080e`|`NV2080_CTRL_CMD_BIOS_GET_NBSI_V2`|Sub|-|subdevice:5887 f=0x48|
|`0x20800810`|`NV2080_CTRL_CMD_BIOS_GET_INFO_V2`|Sub|-|subdevice:5902 f=0x60048|
|`0x20800a00`|`NV2080_CTRL_CMD_INTERNAL_BIF_GET_DATA`|Sub|-|subdevice:5917 f=0xc0|
|`0x20800a01`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO`|Sub|-|subdevice:5932 f=0xc0|
|`0x20800a14`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_GET_RASTER_SYNC_DECODE_MODE`|Sub|-|subdevice:5947 f=0xc0|
|`0x20800a1c`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG`|Sub|S|subdevice:5962 f=0x5c0c0|
|`0x20800a1d`|`NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER`|Sub|S|subdevice:5977 f=0x400c0|
|`0x20800a1e`|`NV2080_CTRL_CMD_INTERNAL_UVM_UNREGISTER_ACCESS_CNTR_BUFFER`|Sub|-|subdevice:5992 f=0x400c0|
|`0x20800a1f`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS`|Sub|S|subdevice:6007 f=0x100c0|
|`0x20800a21`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_ENABLE_NVLINK_PEER`|Sub|-|subdevice:6022 f=0xc1|
|`0x20800a22`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER`|Sub|S|subdevice:6037 f=0x1c0c0|
|`0x20800a24`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_CORE_CALLBACK`|Sub|-|subdevice:6052 f=0xd0|
|`0x20800a25`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_UPDATE_REMOTE_LOCAL_SID`|Sub|-|subdevice:6067 f=0xd0|
|`0x20800a26`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS`|Sub|S|subdevice:6082 f=0x1c0c0|
|`0x20800a29`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_ALI_ENABLED`|Sub|-|subdevice:6112 f=0xc0|
|`0x20800a2a`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO`|Sub|S|subdevice:6127 f=0x1c0c0|
|`0x20800a2c`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ZCULL_INFO`|Sub|-|subdevice:6142 f=0x1c0c0|
|`0x20800a2e`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_ROP_INFO`|Sub|-|subdevice:6157 f=0x1c0c0|
|`0x20800a30`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PPC_MASKS`|Sub|-|subdevice:6172 f=0x1c0c0|
|`0x20800a32`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO`|Sub|S|subdevice:6187 f=0x1c1c0|
|`0x20800a34`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER`|Sub|-|subdevice:6202 f=0x1c0c0|
|`0x20800a36`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO`|Sub|S|subdevice:6217 f=0x404c0|
|`0x20800a37`|`NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_HW_ENABLE`|Sub|-|subdevice:6232 f=0x400c0|
|`0x20800a38`|`NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_HW_ENABLE`|Sub|-|subdevice:6247 f=0x400c0|
|`0x20800a39`|`NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_RD_OFFSET`|Sub|-|subdevice:6262 f=0x400c0|
|`0x20800a3a`|`NV2080_CTRL_CMD_INTERNAL_GR_SET_FECS_TRACE_WR_OFFSET`|Sub|-|subdevice:6277 f=0x400c0|
|`0x20800a3b`|`NV2080_CTRL_CMD_INTERNAL_GR_GET_FECS_TRACE_RD_OFFSET`|Sub|-|subdevice:6292 f=0x400c0|
|`0x20800a3d`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE`|Sub|S|subdevice:6307 f=0x1c0c0|
|`0x20800a3e`|`UNCLEAR-U1`|Sub|-|subdevice:6322 f=0x10040|
|`0x20800a3f`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_TRACE_DEFINES`|Sub|-|subdevice:6337 f=0x1c0c0|
|`0x20800a40`|`NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE`|Sub|S|subdevice:6352 f=0x1c4c0|
|`0x20800a41`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP`|Sub|S|subdevice:6367 f=0x4c0|
|`0x20800a42`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_UPDATE_HSHUB_MUX`|Sub|-|subdevice:6382 f=0xd0|
|`0x20800a44`|`NV2080_CTRL_CMD_INTERNAL_KMIGMGR_PROMOTE_GPU_INSTANCE_MEM_RANGE`|Sub|-|subdevice:6397 f=0xc0|
|`0x20800a45`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_DEPENDENCY_CHECK`|Sub|-|subdevice:6412 f=0xc0|
|`0x20800a46`|`NV2080_CTRL_CMD_INTERNAL_GPU_CHECK_CTS_ID_VALID`|Sub|-|subdevice:6427 f=0x2c0|
|`0x20800a48`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES`|Sub|S|subdevice:6442 f=0x1c0c0|
|`0x20800a49`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM`|Sub|-|subdevice:6457 f=0xc0|
|`0x20800a4a`|`NV2080_CTRL_CMD_INTERNAL_RECOVER_ALL_COMPUTE_CONTEXTS`|Sub|-|subdevice:6472 f=0xc0|
|`0x20800a4b`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`|Sub|-|subdevice:6487 f=0xc0|
|`0x20800a4c`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE`|Sub|S|subdevice:6502 f=0xc0|
|`0x20800a4d`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_SETUP_RG_LINE_INTR`|Sub|-|subdevice:6517 f=0xc0|
|`0x20800a4e`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_PRE_SETUP_NVLINK_PEER`|Sub|-|subdevice:6532 f=0xd0|
|`0x20800a50`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_POST_SETUP_NVLINK_PEER`|Sub|-|subdevice:6547 f=0xd0|
|`0x20800a51`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_PARTITIONABLE_MEM`|Sub|-|subdevice:6562 f=0xc0|
|`0x20800a53`|`NV2080_CTRL_CMD_INTERNAL_FIFO_PROMOTE_RUNLIST_BUFFERS`|Sub|-|subdevice:6577 f=0xc0|
|`0x20800a54`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_SET_IMP_INIT_INFO`|Sub|-|subdevice:6592 f=0xc0|
|`0x20800a55`|`NV2080_CTRL_CMD_INTERNAL_GET_EGPU_BRIDGE_INFO`|Sub|-|subdevice:6607 f=0xc0|
|`0x20800a56`|`NV2080_CTRL_CMD_INTERNAL_LOG_OOB_XID`|Sub|-|subdevice:6622 f=0xc0|
|`0x20800a57`|`NV2080_CTRL_CMD_INTERNAL_VMMU_GET_SPA_FOR_GPA_ENTRIES`|Sub|-|subdevice:6637 f=0xc0|
|`0x20800a58`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER`|Sub|-|subdevice:6652 f=0xc0|
|`0x20800a59`|`NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO`|Sub|S|subdevice:6667 f=0x400c0|
|`0x20800a5b`|`NV2080_CTRL_CMD_INTERNAL_FB_GET_HEAP_RESERVATION_SIZE`|Sub|-|subdevice:6682 f=0xc0|
|`0x20800a5c`|`NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE`|Sub|S|subdevice:6697 f=0xc0|
|`0x20800a5d`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES`|Sub|-|subdevice:6712 f=0xc0|
|`0x20800a5f`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_REMOVE_NVLINK_MAPPING`|Sub|-|subdevice:6727 f=0xd0|
|`0x20800a61`|`NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS`|Sub|S|subdevice:6742 f=0x1d8|
|`0x20800a62`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_SAVE_RESTORE_HSHUB_STATE`|Sub|-|subdevice:6757 f=0xd0|
|`0x20800a63`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KMIGMGR_GET_PROFILES`|Sub|-|subdevice:6772 f=0xc0|
|`0x20800a64`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_PROGRAM_BUFFERREADY`|Sub|-|subdevice:6787 f=0xd0|
|`0x20800a65`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KMIGMGR_GET_PARTITIONABLE_ENGINES`|Sub|-|subdevice:6802 f=0xc0|
|`0x20800a66`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KMIGMGR_GET_SWIZZ_ID_FB_MEM_PAGE_RANGES`|Sub|-|subdevice:6817 f=0xc0|
|`0x20800a67`|`NV2080_CTRL_CMD_INTERNAL_KMEMSYS_GET_MIG_MEMORY_CONFIG`|Sub|-|subdevice:6832 f=0xc0|
|`0x20800a6a`|`NV2080_CTRL_CMD_INTERNAL_RC_WATCHDOG_TIMEOUT`|Sub|-|subdevice:6862 f=0xc0|
|`0x20800a6b`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_MIG_MEMORY_PARTITION_TABLE`|Sub|-|subdevice:6877 f=0xc0|
|`0x20800a6c`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT`|Sub|S|subdevice:6892 f=0xc0|
|`0x20800a6d`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_FLUSH_L2_ALL_RAMS_AND_CACHES`|Sub|-|subdevice:6907 f=0xc0|
|`0x20800a6e`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_DISABLE_NVLINK_PEERS`|Sub|-|subdevice:6922 f=0xc0|
|`0x20800a6f`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_PROGRAM_RAW_COMPRESSION_MODE`|Sub|-|subdevice:6937 f=0xc0|
|`0x20800a70`|`NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR`|Sub|-|subdevice:6952 f=0xc0|
|`0x20800a71`|`NV2080_CTRL_CMD_INTERNAL_BUS_SETUP_P2P_MAILBOX_LOCAL`|Sub|-|subdevice:6967 f=0xc0|
|`0x20800a72`|`NV2080_CTRL_CMD_INTERNAL_BUS_SETUP_P2P_MAILBOX_REMOTE`|Sub|-|subdevice:6982 f=0xc0|
|`0x20800a73`|`NV2080_CTRL_CMD_INTERNAL_BUS_DESTROY_P2P_MAILBOX`|Sub|-|subdevice:6997 f=0xc0|
|`0x20800a74`|`NV2080_CTRL_CMD_INTERNAL_BUS_CREATE_C2C_PEER_MAPPING`|Sub|-|subdevice:7012 f=0xc0|
|`0x20800a75`|`NV2080_CTRL_CMD_INTERNAL_BUS_REMOVE_C2C_PEER_MAPPING`|Sub|-|subdevice:7027 f=0xc0|
|`0x20800a76`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE`|Sub|-|subdevice:7042 f=0xc0|
|`0x20800a77`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_UNIX_CONSOLE`|Sub|-|subdevice:7057 f=0xc0|
|`0x20800a78`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_UPDATE_CURRENT_CONFIG`|Sub|-|subdevice:7072 f=0xd0|
|`0x20800a79`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_GET_MAX_HSHUBS_PER_SHIM`|Sub|-|subdevice:7087 f=0xc0|
|`0x20800a7a`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_HFRP_INFO`|Sub|-|subdevice:7102 f=0xc8|
|`0x20800a7b`|`NV2080_CTRL_CMD_INTERNAL_PMGR_UNSET_DYNAMIC_BOOST_LIMIT`|Sub|-|subdevice:7117 f=0xc0|
|`0x20800a7d`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_UPDATE_PEER_LINK_MASK`|Sub|-|subdevice:7132 f=0xc1|
|`0x20800a7e`|`NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_SET_CONTROL`|Sub|-|subdevice:7147 f=0xc0|
|`0x20800a7f`|`NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_SET_LIMITS`|Sub|-|subdevice:7162 f=0xc0|
|`0x20800a80`|`NV2080_CTRL_CMD_INTERNAL_PERF_GPU_BOOST_SYNC_GET_INFO`|Sub|-|subdevice:7177 f=0xc0|
|`0x20800a81`|`NV2080_CTRL_CMD_INTERNAL_PERF_GET_AUX_POWER_STATE`|Sub|-|subdevice:7192 f=0xc0|
|`0x20800a82`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_UPDATE_LINK_CONNECTION`|Sub|-|subdevice:7207 f=0xc1|
|`0x20800a83`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_ENABLE_LINKS_POST_TOPOLOGY`|Sub|-|subdevice:7222 f=0xd0|
|`0x20800a84`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_PRE_LINK_TRAIN_ALI`|Sub|-|subdevice:7237 f=0xc0|
|`0x20800a85`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_LINK_MASK_POST_RX_DET`|Sub|-|subdevice:7252 f=0x100d0|
|`0x20800a86`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_LINK_TRAIN_ALI`|Sub|-|subdevice:7267 f=0xc0|
|`0x20800a87`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_NVLINK_DEVICE_INFO`|Sub|-|subdevice:7282 f=0xc0|
|`0x20800a88`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_PEER_CONN_CONFIG`|Sub|-|subdevice:7297 f=0xc0|
|`0x20800a8a`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_GET_HSHUB_ID_FOR_LINKS`|Sub|-|subdevice:7312 f=0xc0|
|`0x20800a8b`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_GET_NUM_UNITS`|Sub|-|subdevice:7327 f=0xc0|
|`0x20800a8c`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_NEXT_HSHUB_ID`|Sub|-|subdevice:7342 f=0xc0|
|`0x20800a8d`|`NV2080_CTRL_CMD_INTERNAL_HSHUB_EGM_CONFIG`|Sub|-|subdevice:7357 f=0xc0|
|`0x20800a8e`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_IOCTRL_DEVICE_INFO`|Sub|-|subdevice:7372 f=0xc0|
|`0x20800a8f`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_PROGRAM_LINK_SPEED`|Sub|-|subdevice:7387 f=0xc0|
|`0x20800a90`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_ARE_LINKS_TRAINED`|Sub|-|subdevice:7402 f=0xd0|
|`0x20800a91`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_RESET_LINKS`|Sub|-|subdevice:7417 f=0xc0|
|`0x20800a92`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_DISABLE_DL_INTERRUPTS`|Sub|-|subdevice:7432 f=0xc0|
|`0x20800a93`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_LINK_AND_CLOCK_INFO`|Sub|-|subdevice:7447 f=0xd0|
|`0x20800a94`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_SETUP_NVLINK_SYSMEM`|Sub|-|subdevice:7462 f=0xc0|
|`0x20800a95`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_PROCESS_FORCED_CONFIGS`|Sub|-|subdevice:7477 f=0xc0|
|`0x20800a96`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_SYNC_NVLINK_SHUTDOWN_PROPS`|Sub|-|subdevice:7492 f=0xc0|
|`0x20800a97`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_ENABLE_SYSMEM_NVLINK_ATS`|Sub|-|subdevice:7507 f=0xc0|
|`0x20800a98`|`NV2080_CTRL_CMD_INTERNAL_PERF_PERFMON_CLIENT_RESERVATION_CHECK`|Sub|-|subdevice:7522 f=0xc8|
|`0x20800a99`|`NV2080_CTRL_CMD_INTERNAL_PERF_PERFMON_CLIENT_RESERVATION_SET`|Sub|-|subdevice:7537 f=0xc8|
|`0x20800a9a`|`NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X`|Sub|-|subdevice:7552 f=0x100c8|
|`0x20800a9b`|`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`|Sub|S|subdevice:7567 f=0x400c0|
|`0x20800a9c`|`NV2080_CTRL_CMD_INTERNAL_GMMU_UNREGISTER_FAULT_BUFFER`|Sub|-|subdevice:7582 f=0x400c0|
|`0x20800a9d`|`NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER`|Sub|S|subdevice:7597 f=0xc0|
|`0x20800a9e`|`NV2080_CTRL_CMD_INTERNAL_GMMU_UNREGISTER_CLIENT_SHADOW_FAULT_BUFFER`|Sub|-|subdevice:7612 f=0xc0|
|`0x20800a9f`|`NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`|Sub|S|subdevice:7627 f=0xc0|
|`0x20800aa0`|`NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_3X`|Sub|-|subdevice:7642 f=0x100c8|
|`0x20800aa1`|`NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_CLEAR_3X`|Sub|-|subdevice:7657 f=0x100c8|
|`0x20800aab`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_HSHUB_GET_SYSMEM_NVLINK_MASK`|Sub|-|subdevice:7702 f=0xd0|
|`0x20800aac`|`NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO`|Sub|S|subdevice:7717 f=0x400c0|
|`0x20800aad`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_ENABLE_COMPUTE_PEER_ADDR`|Sub|-|subdevice:7732 f=0xc0|
|`0x20800aae`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_GET_SET_NVSWITCH_FABRIC_ADDR`|Sub|-|subdevice:7747 f=0xc0|
|`0x20800ab0`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_CLEAN_LTC_PROBE_FILTER`|Sub|-|subdevice:7762 f=0xc0|
|`0x20800ab1`|`NV2080_CTRL_CMD_INTERNAL_PERF_CF_CONTROLLERS_SET_MAX_VGPU_VM_COUNT`|Sub|-|subdevice:7777 f=0xc0|
|`0x20800ab2`|`NV2080_CTRL_CMD_INTERNAL_CCU_GET_SAMPLE_INFO`|Sub|-|subdevice:7792 f=0x400c0|
|`0x20800ab3`|`NV2080_CTRL_CMD_INTERNAL_CCU_MAP`|Sub|-|subdevice:7807 f=0xc0|
|`0x20800ab4`|`NV2080_CTRL_CMD_INTERNAL_CCU_UNMAP`|Sub|-|subdevice:7822 f=0xc0|
|`0x20800ab5`|`NV2080_CTRL_CMD_INTERNAL_SET_P2P_CAPS`|Sub|-|subdevice:7837 f=0xc0|
|`0x20800ab6`|`NV2080_CTRL_CMD_INTERNAL_REMOVE_P2P_CAPS`|Sub|-|subdevice:7852 f=0xc0|
|`0x20800ab8`|`NV2080_CTRL_CMD_INTERNAL_GET_PCIE_P2P_CAPS`|Sub|-|subdevice:7867 f=0xc0|
|`0x20800ab9`|`NV2080_CTRL_CMD_INTERNAL_BIF_SET_PCIE_RO`|Sub|-|subdevice:7882 f=0xc0|
|`0x20800aba`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KMIGMGR_GET_COMPUTE_PROFILES`|Sub|-|subdevice:7897 f=0xc0|
|`0x20800abd`|`NV2080_CTRL_CMD_INTERNAL_CCU_SET_STREAM_STATE`|Sub|-|subdevice:7912 f=0xc0|
|`0x20800abe`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_ATTACH_AND_INIT`|Sub|-|subdevice:7927 f=0xc0|
|`0x20800abf`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_OPTIMIZE_TIMING_PARAMETERS`|Sub|-|subdevice:7942 f=0xc0|
|`0x20800ac0`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_GET_DISPLAY_IDS`|Sub|-|subdevice:7957 f=0xc0|
|`0x20800ac1`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_SET_STREO_SYNC`|Sub|-|subdevice:7972 f=0xc0|
|`0x20800ac2`|`NV2080_CTRL_CMD_INTERNAL_FBSR_INIT`|Sub|-|subdevice:7987 f=0xc0|
|`0x20800ac3`|`NV2080_CTRL_CMD_INTERNAL_FIFO_TOGGLE_ACTIVE_CHANNEL_SCHEDULING`|Sub|-|subdevice:8002 f=0xc0|
|`0x20800ac4`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_GET_VERTICAL_ACTIVE_LINES`|Sub|-|subdevice:8017 f=0xc0|
|`0x20800ac5`|`NV2080_CTRL_CMD_INTERNAL_MEMMGR_GET_VGPU_CONFIG_HOST_RESERVED_FB`|Sub|-|subdevice:8032 f=0xc8|
|`0x20800ac6`|`NV2080_CTRL_CMD_INTERNAL_INIT_BRIGHTC_STATE_LOAD`|Sub|-|subdevice:8047 f=0xc0|
|`0x20800ac7`|`NV2080_CTRL_INTERNAL_NVLINK_GET_NUM_ACTIVE_LINK_PER_IOCTRL`|Sub|-|subdevice:8062 f=0x100c0|
|`0x20800ac8`|`NV2080_CTRL_INTERNAL_NVLINK_GET_TOTAL_NUM_LINK_PER_IOCTRL`|Sub|-|subdevice:8077 f=0x100c0|
|`0x20800ac9`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_IS_DISPLAYID_VALID`|Sub|-|subdevice:8092 f=0xc0|
|`0x20800aca`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_SET_OR_RESTORE_RASTER_SYNC`|Sub|-|subdevice:8107 f=0xc0|
|`0x20800acb`|`NV2080_CTRL_CMD_INTERNAL_SMBPBI_PFM_REQ_HNDLR_CAP_UPDATE`|Sub|-|subdevice:8122 f=0xc0|
|`0x20800acc`|`NV2080_CTRL_CMD_INTERNAL_PMGR_PFM_REQ_HNDLR_STATE_LOAD_SYNC`|Sub|-|subdevice:8137 f=0xc0|
|`0x20800acd`|`NV2080_CTRL_CMD_INTERNAL_THERM_PFM_REQ_HNDLR_STATE_INIT_SYNC`|Sub|-|subdevice:8152 f=0xc0|
|`0x20800ace`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_GET_PM1_STATE`|Sub|-|subdevice:8167 f=0xc0|
|`0x20800acf`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_SET_PM1_STATE`|Sub|-|subdevice:8182 f=0xc0|
|`0x20800ad0`|`NV2080_CTRL_CMD_INTERNAL_PMGR_PFM_REQ_HNDLR_UPDATE_EDPP_LIMIT`|Sub|-|subdevice:8197 f=0xc0|
|`0x20800ad1`|`NV2080_CTRL_CMD_INTERNAL_THERM_PFM_REQ_HNDLR_UPDATE_TGPU_LIMIT`|Sub|-|subdevice:8212 f=0xc0|
|`0x20800ad2`|`NV2080_CTRL_CMD_INTERNAL_PMGR_PFM_REQ_HNDLR_CONFIGURE_TGP_MODE`|Sub|-|subdevice:8227 f=0xc0|
|`0x20800ad3`|`NV2080_CTRL_CMD_INTERNAL_PMGR_PFM_REQ_HNDLR_CONFIGURE_TURBO_V2`|Sub|-|subdevice:8242 f=0xc0|
|`0x20800ad4`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_GET_VPSTATE_INFO`|Sub|-|subdevice:8257 f=0xc0|
|`0x20800ad5`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_GET_VPSTATE_MAPPING`|Sub|-|subdevice:8272 f=0xc0|
|`0x20800ad6`|`NV2080_CTRL_CMD_INTERNAL_PERF_PFM_REQ_HNDLR_SET_VPSTATE`|Sub|-|subdevice:8287 f=0xc0|
|`0x20800ad8`|`NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_SECURE_CHANNELS`|Sub|-|subdevice:8302 f=0xc8|
|`0x20800ad9`|`NV2080_CTRL_INTERNAL_SPDM_PARTITION`|Sub|-|subdevice:8317 f=0xc0|
|`0x20800adb`|`NV2080_CTRL_CMD_INTERNAL_BIF_DISABLE_SYSTEM_MEMORY_ACCESS`|Sub|-|subdevice:8347 f=0xc0|
|`0x20800adc`|`NV2080_CTRL_CMD_INTERNAL_DISP_PINSETS_TO_LOCKPINS`|Sub|-|subdevice:8362 f=0xc0|
|`0x20800add`|`NV2080_CTRL_CMD_INTERNAL_DETECT_HS_VIDEO_BRIDGE`|Sub|-|subdevice:8377 f=0xc0|
|`0x20800ade`|`NV2080_CTRL_CMD_INTERNAL_DISP_SET_SLI_LINK_GPIO_SW_CONTROL`|Sub|-|subdevice:8392 f=0xc0|
|`0x20800adf`|`NV2080_CTRL_CMD_INTERNAL_SET_STATIC_EDID_DATA`|Sub|-|subdevice:8407 f=0xc0|
|`0x20800ae1`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_DERIVE_SWL_KEYS`|Sub|-|subdevice:8422 f=0xc0|
|`0x20800ae2`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_DERIVE_LCE_KEYS`|Sub|-|subdevice:8437 f=0xc0|
|`0x20800ae5`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_ROTATE_KEYS`|Sub|-|subdevice:8452 f=0xc0|
|`0x20800ae6`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_RC_CHANNELS_FOR_KEY_ROTATION`|Sub|-|subdevice:8467 f=0xc0|
|`0x20800ae7`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_SET_GPU_STATE`|Sub|-|subdevice:8482 f=0xc0|
|`0x20800ae8`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_SET_SECURITY_POLICY`|Sub|-|subdevice:8497 f=0xc0|
|`0x20800ae9`|`NV2080_CTRL_INTERNAL_GPU_CLIENT_LOW_POWER_MODE_ENTER`|Sub|-|subdevice:8512 f=0xc8|
|`0x20800aea`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_POST_FATAL_ERROR_RECOVERY`|Sub|-|subdevice:8527 f=0x100c0|
|`0x20800aeb`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_GSP_RM_FREE_HEAP`|Sub|-|subdevice:8542 f=0xc0|
|`0x20800aec`|`NV2080_CTRL_CMD_INTERNAL_GPU_SET_ILLUM`|Sub|-|subdevice:8557 f=0xc8|
|`0x20800aed`|`NV2080_CTRL_CMD_INTERNAL_GSYNC_APPLY_STEREO_PIN_ALWAYS_HI_WAR`|Sub|-|subdevice:8572 f=0xc0|
|`0x20800aee`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_PF_BAR1_SPA`|Sub|-|subdevice:8587 f=0xc0|
|`0x20800af0`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_ACPI_SUBSYSTEM_ACTIVATED`|Sub|-|subdevice:8602 f=0xc0|
|`0x20800af1`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_MODESET`|Sub|-|subdevice:8617 f=0xc0|
|`0x20800af2`|`NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_MODESET`|Sub|-|subdevice:8632 f=0xc0|
|`0x20800af3`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO`|Sub|S|subdevice:8647 f=0xc0|
|`0x20800afa`|`NV2080_CTRL_CMD_INTERNAL_MEMMGR_MEMORY_TRANSFER_WITH_GSP`|Sub|-|subdevice:8662 f=0xc0|
|`0x20800afb`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_LOCAL_ATS_CONFIG`|Sub|-|subdevice:8677 f=0xc0|
|`0x20800afc`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_PEER_ATS_CONFIG`|Sub|-|subdevice:8692 f=0xc0|
|`0x20800afd`|`NV2080_CTRL_CMD_INTERNAL_PMGR_PFM_REQ_HNDLR_GET_EDPP_LIMIT_INFO`|Sub|-|subdevice:8707 f=0xc0|
|`0x20800afe`|`NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA`|Sub|-|subdevice:8722 f=0xc0|
|`0x20800aff`|`NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL`|Sub|-|subdevice:8737 f=0xc0|
|`0x20800b01`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_REPLAY_SUPPRESSED_ERRORS`|Sub|-|subdevice:8752 f=0xd0|
|`0x20800b03`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_RATE_MODIFIER_V2`|Sub|-|subdevice:8767 f=0x1c0c0|
|`0x20800b05`|`NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_SM_ISSUE_THROTTLE_CTRL`|Sub|-|subdevice:8782 f=0x1c0c0|
|`0x20800b07`|`NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_ROTATE_PER_CHANNEL_KEYS`|Sub|-|subdevice:8797 f=0xc0|
|`0x20800b08`|`NV2080_CTRL_CMD_INTERNAL_NVLINK_RC_USER_MODE_CHANNELS`|Sub|-|subdevice:8812 f=0xc0|
|`0x20800b12`|`NV2080_CTRL_CMD_INTERNAL_UCODE_INSTRUMENTATION_GET_STATE`|Sub|-|subdevice:8827 f=0xc0|
|`0x20800b13`|`NV2080_CTRL_CMD_INTERNAL_UCODE_INSTRUMENTATION_SET_STATE`|Sub|-|subdevice:8842 f=0xc0|
|`0x20800b14`|`NV2080_CTRL_CMD_INTERNAL_UCODE_INSTRUMENTATION_GET_DATA`|Sub|-|subdevice:8857 f=0xc0|
|`0x20801037`|`NV2080_CTRL_CMD_CLK_PMUMON_CLK_DOMAINS_GET_SAMPLES`|Sub|-|subdevice:8872 f=0x40048|
|`0x20801102`|`NV2080_CTRL_CMD_SET_GPFIFO`|Sub|-|subdevice:8887 f=0x48|
|`0x2080110e`|`NV2080_CTRL_CMD_FIFO_OBJSCHED_SW_GET_LOG`|Sub|-|subdevice:8977 f=0x48|
|`0x20801110`|`NV2080_CTRL_CMD_FIFO_CONFIG_CTXSW_TIMEOUT`|Sub|-|subdevice:8992 f=0x44|
|`0x20801112`|`NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE`|Sub|S|subdevice:9007 f=0x5c040|
|`0x20801113`|`NV2080_CTRL_CMD_FIFO_CLEAR_FAULTED_BIT`|Sub|-|subdevice:9022 f=0x244|
|`0x20801115`|`NV2080_CTRL_CMD_FIFO_RUNLIST_SET_SCHED_POLICY`|Sub|-|subdevice:9037 f=0x68|
|`0x20801117`|`NV2080_CTRL_CMD_FIFO_DISABLE_USERMODE_CHANNELS`|Sub|-|subdevice:9067 f=0x40040|
|`0x20801118`|`NV2080_CTRL_CMD_FIFO_SETUP_VF_ZOMBIE_SUBCTX_PDB`|Sub|-|subdevice:9082 f=0x10248|
|`0x20801120`|`NV2080_CTRL_CMD_FIFO_OBJSCHED_GET_STATE`|Sub|-|subdevice:9157 f=0x48|
|`0x20801121`|`NV2080_CTRL_CMD_FIFO_OBJSCHED_SET_STATE`|Sub|-|subdevice:9172 f=0x48|
|`0x20801122`|`NV2080_CTRL_CMD_FIFO_OBJSCHED_GET_CAPS`|Sub|-|subdevice:9187 f=0x40048|
|`0x20801205`|`NV2080_CTRL_CMD_GR_CTXSW_ZCULL_MODE`|Sub|-|subdevice:9247 f=0x10248|
|`0x20801208`|`NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND`|Sub|T|subdevice:9292 f=0x90348|
|`0x20801209`|`NV2080_CTRL_CMD_GR_CTXSW_PM_BIND`|Sub|-|subdevice:9307 f=0x10048|
|`0x2080120a`|`NV2080_CTRL_CMD_GR_SET_GPC_TILE_MAP`|Sub|-|subdevice:9322 f=0x48|
|`0x2080120e`|`NV2080_CTRL_CMD_GR_CTXSW_SMPC_MODE`|Sub|-|subdevice:9337 f=0x48|
|`0x20801210`|`NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`|Sub|S|subdevice:9367 f=0x10348|
|`0x20801211`|`NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND`|Sub|-|subdevice:9382 f=0x10248|
|`0x20801212`|`NV2080_CTRL_CMD_GR_PC_SAMPLING_MODE`|Sub|-|subdevice:9397 f=0x10248|
|`0x20801215`|`NV2080_CTRL_CMD_GR_GET_CTXSW_STATS`|Sub|-|subdevice:9427 f=0x48|
|`0x2080121c`|`NV2080_CTRL_CMD_GR_GET_CURRENT_RESIDENT_CHANNEL`|Sub|-|subdevice:9487 f=0x48|
|`0x2080121f`|`NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE`|Sub|-|subdevice:9517 f=0x40|
|`0x20801220`|`NV2080_CTRL_CMD_GR_GFX_POOL_INITIALIZE`|Sub|-|subdevice:9532 f=0x40|
|`0x20801221`|`NV2080_CTRL_CMD_GR_GFX_POOL_ADD_SLOTS`|Sub|-|subdevice:9547 f=0x40|
|`0x20801222`|`NV2080_CTRL_CMD_GR_GFX_POOL_REMOVE_SLOTS`|Sub|-|subdevice:9562 f=0x40|
|`0x2080122c`|`NV2080_CTRL_CMD_GR_SET_TPC_PARTITION_MODE`|Sub|-|subdevice:9637 f=0x48|
|`0x20801235`|`NV2080_CTRL_CMD_GR_GET_CTXSW_MODES`|Sub|-|subdevice:9742 f=0x48|
|`0x20801236`|`NV2080_CTRL_CMD_GR_GET_GPC_TILE_MAP`|Sub|-|subdevice:9757 f=0x48|
|`0x2080123a`|`NV2080_CTRL_CMD_GR_CTXSW_SETUP_BIND`|Sub|-|subdevice:9817 f=0x48|
|`0x2080123e`|`NV2080_CTRL_CMD_GR_TEST_CTXSW_ERROR_LOGS`|Sub|-|subdevice:9862 f=0x100048|
|`0x2080130c`|`NV2080_CTRL_CMD_FB_GET_CALIBRATION_LOCK_FAILED`|Sub|-|subdevice:9892 f=0x48|
|`0x20801315`|`NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO`|Sub|T|subdevice:9952 f=0x40148|
|`0x20801322`|`NV2080_CTRL_CMD_FB_GET_OFFLINED_PAGES`|Sub|-|subdevice:9982 f=0x50048|
|`0x20801328`|`NV2080_CTRL_CMD_FB_GET_LTC_INFO_FOR_FBP`|Sub|-|subdevice:9997 f=0x50158|
|`0x2080132a`|`NV2080_CTRL_CMD_FB_COMPBITCOPY_GET_COMPBITS`|Sub|-|subdevice:10012 f=0xc8|
|`0x20801337`|`NV2080_CTRL_CMD_FB_CBC_OP`|Sub|-|subdevice:10042 f=0x44|
|`0x20801338`|`NV2080_CTRL_CMD_FB_GET_CTAGS_FOR_CBC_EVICTION`|Sub|-|subdevice:10057 f=0x44|
|`0x2080133b`|`NV2080_CTRL_CMD_FB_SETUP_VPR_REGION`|Sub|-|subdevice:10072 f=0x40|
|`0x2080133d`|`NV2080_CTRL_CMD_FB_GET_COMPBITCOPY_CONSTRUCT_INFO`|Sub|-|subdevice:10102 f=0x44|
|`0x2080133e`|`NV2080_CTRL_CMD_FB_SET_RRD`|Sub|-|subdevice:10117 f=0x44|
|`0x2080133f`|`NV2080_CTRL_CMD_FB_SET_READ_LIMIT`|Sub|-|subdevice:10132 f=0x44|
|`0x20801340`|`NV2080_CTRL_CMD_FB_SET_WRITE_LIMIT`|Sub|-|subdevice:10147 f=0x44|
|`0x20801341`|`NV2080_CTRL_CMD_FB_PATCH_PBR_FOR_MINING`|Sub|-|subdevice:10162 f=0x44|
|`0x20801344`|`NV2080_CTRL_CMD_FB_GET_REMAPPED_ROWS`|Sub|-|subdevice:10192 f=0x58|
|`0x20801346`|`NV2080_CTRL_CMD_FB_GET_FS_INFO`|Sub|-|subdevice:10207 f=0x10248|
|`0x20801347`|`NV2080_CTRL_CMD_FB_GET_ROW_REMAPPER_HISTOGRAM`|Sub|-|subdevice:10222 f=0x58|
|`0x20801348`|`NV2080_CTRL_CMD_FB_GET_DYNAMIC_OFFLINED_PAGES`|Sub|-|subdevice:10237 f=0x50048|
|`0x20801355`|`NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_PENDING_CONFIGURATION`|Sub|-|subdevice:10327 f=0x40048|
|`0x20801356`|`NV2080_CTRL_CMD_FB_SET_DRAM_ENCRYPTION_CONFIGURATION`|Sub|-|subdevice:10342 f=0x40044|
|`0x20801357`|`NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT`|Sub|T|subdevice:10357 f=0x48|
|`0x20801358`|`NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS`|Sub|T|subdevice:10372 f=0x40048|
|`0x20801360`|`NV2080_CTRL_CMD_FB_GET_CARVEOUT_REGION_INFO`|Sub|-|subdevice:10387 f=0x148|
|`0x20801361`|`NV2080_CTRL_CMD_FB_GET_REMAPPED_BANKS`|Sub|-|subdevice:10402 f=0x58|
|`0x20801364`|`NV2080_CTRL_CMD_FB_GET_WPR_REGION_INFO`|Sub|-|subdevice:10447 f=0x554|
|`0x2080170d`|`NV2080_CTRL_CMD_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS`|Sub|-|subdevice:10522 f=0x10048|
|`0x2080170e`|`NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE`|Sub|-|subdevice:10537 f=0x10048|
|`0x2080170f`|`NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP`|Sub|-|subdevice:10552 f=0x48|
|`0x20801804`|`NV2080_CTRL_CMD_BUS_SET_PCIE_LINK_WIDTH`|Sub|-|subdevice:10597 f=0x44|
|`0x20801805`|`NV2080_CTRL_CMD_BUS_SET_PCIE_SPEED`|Sub|-|subdevice:10612 f=0x44|
|`0x20801812`|`NV2080_CTRL_CMD_BUS_SERVICE_GPU_MULTIFUNC_STATE`|Sub|-|subdevice:10627 f=0x48|
|`0x20801813`|`NV2080_CTRL_CMD_BUS_GET_PEX_COUNTERS`|Sub|-|subdevice:10642 f=0x48|
|`0x20801814`|`NV2080_CTRL_CMD_BUS_CLEAR_PEX_COUNTERS`|Sub|-|subdevice:10657 f=0x48|
|`0x20801815`|`NV2080_CTRL_CMD_BUS_FREEZE_PEX_COUNTERS`|Sub|-|subdevice:10672 f=0x48|
|`0x20801816`|`NV2080_CTRL_CMD_BUS_GET_PEX_LANE_COUNTERS`|Sub|-|subdevice:10687 f=0x48|
|`0x20801817`|`NV2080_CTRL_CMD_BUS_GET_PCIE_LTR_LATENCY`|Sub|-|subdevice:10702 f=0x48|
|`0x20801818`|`NV2080_CTRL_CMD_BUS_SET_PCIE_LTR_LATENCY`|Sub|-|subdevice:10717 f=0x48|
|`0x20801819`|`NV2080_CTRL_CMD_BUS_GET_PEX_UTIL_COUNTERS`|Sub|-|subdevice:10732 f=0x48|
|`0x20801820`|`NV2080_CTRL_CMD_BUS_CLEAR_PEX_UTIL_COUNTERS`|Sub|-|subdevice:10747 f=0x48|
|`0x20801824`|`NV2080_CTRL_CMD_BUS_CONTROL_PUBLIC_ASPM_BITS`|Sub|-|subdevice:10777 f=0x48|
|`0x20801826`|`NV2080_CTRL_CMD_BUS_SET_EOM_PARAMETERS`|Sub|-|subdevice:10807 f=0x48|
|`0x20801827`|`NV2080_CTRL_CMD_BUS_GET_UPHY_DLN_CFG_SPACE`|Sub|-|subdevice:10822 f=0x48|
|`0x20801828`|`NV2080_CTRL_CMD_BUS_GET_EOM_STATUS`|Sub|-|subdevice:10837 f=0x48|
|`0x20801829`|`NV2080_CTRL_CMD_BUS_GET_PCIE_REQ_ATOMICS_CAPS`|Sub|-|subdevice:10852 f=0x40048|
|`0x2080182a`|`NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS`|Sub|S|subdevice:10867 f=0x40048|
|`0x2080182b`|`NV2080_CTRL_CMD_BUS_GET_C2C_INFO`|Sub|S|subdevice:10882 f=0x50048|
|`0x2080182d`|`NV2080_CTRL_CMD_BUS_GET_C2C_ERR_INFO`|Sub|-|subdevice:10912 f=0x48|
|`0x2080182e`|`NV2080_CTRL_CMD_BUS_SET_P2P_MAPPING`|Sub|-|subdevice:10927 f=0x50040|
|`0x2080182f`|`NV2080_CTRL_CMD_BUS_UNSET_P2P_MAPPING`|Sub|-|subdevice:10942 f=0x50040|
|`0x20801830`|`NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS`|Sub|-|subdevice:10957 f=0x40448|
|`0x20801831`|`NV2080_CTRL_CMD_BUS_GET_C2C_LPWR_STATS`|Sub|-|subdevice:10972 f=0x40048|
|`0x20801832`|`NV2080_CTRL_CMD_BUS_SET_C2C_LPWR_STATE_VOTE`|Sub|-|subdevice:10987 f=0x40048|
|`0x20801836`|`NV2080_CTRL_CMD_BUS_SET_C2C_LPWR_IDLE_THRESHOLD`|Sub|-|subdevice:11002 f=0x40048|
|`0x20801837`|`NV2080_CTRL_CMD_BUS_GET_C2C_PACKET_COUNTERS`|Sub|-|subdevice:11017 f=0x48|
|`0x2080200b`|`NV2080_CTRL_CMD_PERF_GET_LEVEL_INFO_V2`|Sub|S|subdevice:11047 f=0x50048|
|`0x2080205a`|`NV2080_CTRL_CMD_PERF_GET_POWERSTATE`|Sub|-|subdevice:11062 f=0x40048|
|`0x2080205d`|`NV2080_CTRL_CMD_PERF_NOTIFY_VIDEOEVENT`|Sub|-|subdevice:11092 f=0x50048|
|`0x20802068`|`NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE`|Sub|T|subdevice:11107 f=0x50048|
|`0x20802069`|`NV2080_CTRL_CMD_PERF_GET_TEGRA_PERFMON_SAMPLE`|Sub|-|subdevice:11122 f=0x48|
|`0x2080206e`|`NV2080_CTRL_CMD_PERF_RATED_TDP_GET_CONTROL`|Sub|-|subdevice:11137 f=0x4a|
|`0x20802087`|`NV2080_CTRL_CMD_PERF_GET_VID_ENG_PERFMON_SAMPLE`|Sub|-|subdevice:11167 f=0x40048|
|`0x20802207`|`NV2080_CTRL_CMD_RC_SET_CLEAN_ERROR_HISTORY`|Sub|-|subdevice:11257 f=0x44|
|`0x2080220d`|`NV2080_CTRL_CMD_SET_RC_RECOVERY`|Sub|-|subdevice:11332 f=0x40154|
|`0x2080220e`|`NV2080_CTRL_CMD_GET_RC_RECOVERY`|Sub|-|subdevice:11347 f=0x40154|
|`0x20802211`|`NV2080_CTRL_CMD_SET_RC_INFO`|Sub|-|subdevice:11377 f=0x44|
|`0x20802212`|`NV2080_CTRL_CMD_GET_RC_INFO`|Sub|-|subdevice:11392 f=0x44|
|`0x20802214`|`NV2080_CTRL_CMD_SET_RC_WATCHDOG_INFO`|Sub|-|subdevice:11422 f=0x44|
|`0x20802300`|`NV2080_CTRL_CMD_INTERNAL_GPIO_PROGRAM_DIRECTION`|Sub|-|subdevice:11437 f=0xc0|
|`0x20802301`|`NV2080_CTRL_CMD_INTERNAL_GPIO_PROGRAM_OUTPUT`|Sub|-|subdevice:11452 f=0xc0|
|`0x20802302`|`NV2080_CTRL_CMD_INTERNAL_GPIO_READ_INPUT`|Sub|-|subdevice:11467 f=0xc0|
|`0x20802303`|`NV2080_CTRL_CMD_INTERNAL_GPIO_ACTIVATE_HW_FUNCTION`|Sub|-|subdevice:11482 f=0xc0|
|`0x20802609`|`NV2080_CTRL_CMD_PMGR_GET_MODULE_INFO`|Sub|-|subdevice:11602 f=0x158|
|`0x20802801`|`NV2080_CTRL_CMD_LPWR_DIFR_CTRL`|Sub|-|subdevice:11647 f=0x44|
|`0x20802802`|`NV2080_CTRL_CMD_LPWR_DIFR_PREFETCH_RESPONSE`|Sub|-|subdevice:11662 f=0x40|
|`0x20802a02`|`NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK`|Sub|S|subdevice:11692 f=0x30349|
|`0x20802a06`|`NV2080_CTRL_CMD_CE_UPDATE_CLASS_DB`|Sub|-|subdevice:11737 f=0xc0|
|`0x20802a07`|`NV2080_CTRL_CMD_CE_GET_PHYSICAL_CAPS`|Sub|S|subdevice:11752 f=0x101d0|
|`0x20802a08`|`NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE`|Sub|S|subdevice:11767 f=0x1c040|
|`0x20802a09`|`NV2080_CTRL_CMD_CE_GET_HUB_PCE_MASK`|Sub|-|subdevice:11782 f=0x4c0|
|`0x20802a0b`|`NV2080_CTRL_CMD_CE_GET_ALL_PHYSICAL_CAPS`|Sub|S|subdevice:11812 f=0x101d0|
|`0x20802a0c`|`NV2080_CTRL_CMD_CE_GET_LCE_SHIM_INFO`|Sub|-|subdevice:11827 f=0x145|
|`0x20802a0e`|`NV2080_CTRL_CMD_CE_GET_HUB_PCE_MASK_V2`|Sub|-|subdevice:11857 f=0x44|
|`0x20802a0f`|`NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_TYPE`|Sub|-|subdevice:11872 f=0x100c0|
|`0x20802a11`|`NV2080_CTRL_CMD_CE_GET_DECOMP_LCE_MASK`|Sub|-|subdevice:11887 f=0x154|
|`0x20802a12`|`NV2080_CTRL_CMD_CE_IS_DECOMP_LCE_ENABLED`|Sub|-|subdevice:11902 f=0x154|
|`0x20802a13`|`NV2080_CTRL_CMD_INTERNAL_CE_GET_PCE_CONFIG_FOR_LCE_MIG_GPU_INSTANCE`|Sub|-|subdevice:11917 f=0x102c0|
|`0x20802a14`|`NV2080_CTRL_CE_UPDATE_PCE_LCE_MIG_MAPPINGS`|Sub|-|subdevice:11932 f=0x100c0|
|`0x20803003`|`NV2080_CTRL_CMD_NVLINK_GET_ERR_INFO`|Sub|-|subdevice:11977 f=0x48|
|`0x20803004`|`NV2080_CTRL_CMD_NVLINK_GET_COUNTERS`|Sub|-|subdevice:11992 f=0x48|
|`0x20803005`|`NV2080_CTRL_CMD_NVLINK_CLEAR_COUNTERS`|Sub|-|subdevice:12007 f=0x44|
|`0x20803009`|`NV2080_CTRL_CMD_NVLINK_GET_LINK_FATAL_ERROR_COUNTS`|Sub|-|subdevice:12022 f=0x48|
|`0x2080300c`|`NV2080_CTRL_CMD_NVLINK_SETUP_EOM`|Sub|-|subdevice:12037 f=0x40|
|`0x2080300e`|`NV2080_CTRL_CMD_NVLINK_GET_POWER_STATE`|Sub|-|subdevice:12052 f=0x48|
|`0x20803011`|`NV2080_CTRL_CMD_NVLINK_GET_LINK_FOM_VALUES`|Sub|-|subdevice:12067 f=0x44|
|`0x20803014`|`NV2080_CTRL_CMD_NVLINK_GET_NVLINK_ECC_ERRORS`|Sub|-|subdevice:12082 f=0x44|
|`0x20803015`|`NV2080_CTRL_CMD_NVLINK_READ_TP_COUNTERS`|Sub|-|subdevice:12097 f=0x48|
|`0x20803018`|`NV2080_CTRL_CMD_NVLINK_GET_LP_COUNTERS`|Sub|-|subdevice:12112 f=0x48|
|`0x20803023`|`NV2080_CTRL_CMD_NVLINK_SET_LOOPBACK_MODE`|Sub|-|subdevice:12127 f=0x40|
|`0x20803028`|`NV2080_CTRL_CMD_NVLINK_GET_REFRESH_COUNTERS`|Sub|-|subdevice:12142 f=0x40|
|`0x20803029`|`NV2080_CTRL_CMD_NVLINK_CLEAR_REFRESH_COUNTERS`|Sub|-|subdevice:12157 f=0x40|
|`0x20803038`|`NV2080_CTRL_CMD_NVLINK_GET_SET_NVSWITCH_FLA_ADDR`|Sub|-|subdevice:12172 f=0x40|
|`0x20803039`|`NV2080_CTRL_CMD_NVLINK_SYNC_LINK_MASKS_AND_VBIOS_INFO`|Sub|-|subdevice:12187 f=0x10041|
|`0x2080303a`|`NV2080_CTRL_CMD_NVLINK_ENABLE_LINKS`|Sub|-|subdevice:12202 f=0x40|
|`0x2080303b`|`NV2080_CTRL_CMD_NVLINK_PROCESS_INIT_DISABLED_LINKS`|Sub|-|subdevice:12217 f=0x40|
|`0x2080303c`|`NV2080_CTRL_CMD_NVLINK_EOM_CONTROL`|Sub|-|subdevice:12232 f=0x40|
|`0x2080303e`|`NV2080_CTRL_CMD_NVLINK_SET_L1_THRESHOLD`|Sub|-|subdevice:12247 f=0x44|
|`0x2080303f`|`NV2080_CTRL_CMD_NVLINK_GET_L1_THRESHOLD`|Sub|-|subdevice:12262 f=0x48|
|`0x20803040`|`NV2080_CTRL_CMD_NVLINK_INBAND_SEND_DATA`|Sub|-|subdevice:12277 f=0x10250|
|`0x20803041`|`NV2080_CTRL_CMD_NVLINK_IS_GPU_DEGRADED`|Sub|-|subdevice:12292 f=0x44|
|`0x20803042`|`NV2080_CTRL_CMD_NVLINK_DIRECT_CONNECT_CHECK`|Sub|-|subdevice:12307 f=0x48|
|`0x20803043`|`NV2080_CTRL_CMD_NVLINK_POST_FAULT_UP`|Sub|-|subdevice:12322 f=0x40|
|`0x20803044`|`NV2080_CTRL_CMD_NVLINK_GET_PORT_EVENTS`|Sub|-|subdevice:12337 f=0x44|
|`0x20803045`|`NV2080_CTRL_CMD_NVLINK_CYCLE_LINK`|Sub|-|subdevice:12352 f=0x44|
|`0x20803046`|`NV2080_CTRL_CMD_NVLINK_IS_REDUCED_CONFIG`|Sub|-|subdevice:12367 f=0x40|
|`0x20803050`|`NV2080_CTRL_CMD_NVLINK_GET_COUNTERS_V2`|Sub|-|subdevice:12382 f=0x48|
|`0x20803051`|`NV2080_CTRL_CMD_NVLINK_CLEAR_COUNTERS_V2`|Sub|-|subdevice:12397 f=0x44|
|`0x20803052`|`NV2080_CTRL_CMD_NVLINK_CLEAR_LP_COUNTERS`|Sub|-|subdevice:12412 f=0x44|
|`0x20803081`|`NV2080_CTRL_CMD_NVLINK_SET_HW_ERROR_INJECT`|Sub|-|subdevice:12442 f=0x44|
|`0x20803082`|`NV2080_CTRL_CMD_NVLINK_GET_HW_ERROR_INJECT`|Sub|-|subdevice:12457 f=0x48|
|`0x20803083`|`NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO`|Sub|R|subdevice:12472 f=0x48|
|`0x20803089`|`NV2080_CTRL_CMD_NVLINK_INJECT_SW_ERROR`|Sub|-|subdevice:12547 f=0x44|
|`0x2080308a`|`NV2080_CTRL_CMD_NVLINK_POST_LAZY_ERROR_RECOVERY`|Sub|-|subdevice:12562 f=0x40|
|`0x2080308b`|`NV2080_CTRL_CMD_NVLINK_GET_NVLE_ENCRYPT_EN_INFO`|Sub|-|subdevice:12577 f=0x44|
|`0x2080308c`|`NV2080_CTRL_NVLINK_UPDATE_NVLE_TOPOLOGY`|Sub|-|subdevice:12592 f=0x44|
|`0x2080308d`|`NV2080_CTRL_NVLINK_GET_UPDATE_NVLE_LIDS`|Sub|-|subdevice:12607 f=0x44|
|`0x2080308e`|`NV2080_CTRL_CMD_NVLINK_CONFIGURE_L1_TOGGLE`|Sub|-|subdevice:12622 f=0x44|
|`0x2080308f`|`NV2080_CTRL_CMD_NVLINK_GET_L1_TOGGLE`|Sub|-|subdevice:12637 f=0x48|
|`0x20803091`|`NV2080_CTRL_NVLINK_GET_FIRMWARE_VERSION_INFO`|Sub|-|subdevice:12652 f=0x48|
|`0x20803092`|`NV2080_CTRL_CMD_NVLINK_SET_NVLE_ENABLED_STATE`|Sub|-|subdevice:12667 f=0x40|
|`0x20803095`|`NV2080_CTRL_CMD_NVLINK_PRM_ACCESS`|Sub|-|subdevice:12682 f=0x44|
|`0x2080309a`|`NV2080_CTRL_CMD_NVLINK_SAVE_NODE_HOSTNAME`|Sub|-|subdevice:12697 f=0x44|
|`0x2080309b`|`NV2080_CTRL_CMD_NVLINK_GET_SAVED_NODE_HOSTNAME`|Sub|-|subdevice:12712 f=0x48|
|`0x2080309c`|`NV2080_CTRL_CMD_NVLINK_UPDATE_CLID`|Sub|-|subdevice:12727 f=0x40|
|`0x2080309d`|`NV2080_CTRL_CMD_NVLINK_LOCK_REMAP_TABLE_AND_MSE`|Sub|-|subdevice:12742 f=0x54|
|`0x2080309e`|`NV2080_CTRL_CMD_NVLINK_GET_REMAP_TABLE_INFO`|Sub|-|subdevice:12757 f=0x54|
|`0x2080309f`|`NV2080_CTRL_CMD_NVLINK_GET_NVLE_PKT_COUNTERS`|Sub|-|subdevice:12772 f=0x54|
|`0x208030a1`|`NV2080_CTRL_CMD_NVLINK_GET_REMAP_TABLE_INFO_V2`|Sub|-|subdevice:12802 f=0x54|
|`0x208030a2`|`NV2080_CTRL_NVLINK_GET_UPDATE_NVLE_LIDS_V2`|Sub|-|subdevice:12817 f=0x44|
|`0x20803101`|`NV2080_CTRL_CMD_FLCN_GET_DMEM_USAGE`|Sub|-|subdevice:12832 f=0x48|
|`0x20803118`|`NV2080_CTRL_CMD_FLCN_GET_ENGINE_ARCH`|Sub|-|subdevice:12847 f=0x48|
|`0x20803120`|`NV2080_CTRL_CMD_FLCN_USTREAMER_QUEUE_INFO`|Sub|-|subdevice:12862 f=0x48|
|`0x20803122`|`NV2080_CTRL_CMD_FLCN_USTREAMER_CONTROL_GET`|Sub|-|subdevice:12877 f=0x48|
|`0x20803123`|`NV2080_CTRL_CMD_FLCN_USTREAMER_CONTROL_SET`|Sub|-|subdevice:12892 f=0x44|
|`0x20803400`|`NV2080_CTRL_CMD_ECC_GET_CLIENT_EXPOSED_COUNTERS`|Sub|-|subdevice:12937 f=0x48|
|`0x20803401`|`NV2080_CTRL_CMD_ECC_GET_VOLATILE_COUNTS`|Sub|-|subdevice:12952 f=0x48|
|`0x20803402`|`NV2080_CTRL_CMD_ECC_GET_SRAM_UNIQUE_UNCORR_COUNTS`|Sub|-|subdevice:12967 f=0x44|
|`0x20803403`|`NV2080_CTRL_CMD_ECC_INJECT_ERROR`|Sub|-|subdevice:12982 f=0x44|
|`0x20803404`|`NV2080_CTRL_CMD_ECC_GET_REPAIR_STATUS`|Sub|-|subdevice:12997 f=0x48|
|`0x20803405`|`NV2080_CTRL_CMD_ECC_INJECTION_SUPPORTED`|Sub|-|subdevice:13012 f=0x44|
|`0x20803406`|`NV2080_CTRL_CMD_ECC_GET_UNREPAIRABLE_MEMORY_FLAG`|Sub|-|subdevice:13027 f=0x48|
|`0x20803502`|`NV2080_CTRL_CMD_FLA_SETUP_INSTANCE_MEM_BLOCK`|Sub|-|subdevice:13057 f=0x10244|
|`0x20803601`|`NV2080_CTRL_CMD_GSP_GET_FEATURES`|Sub|S|subdevice:13102 f=0x40549|
|`0x20803602`|`NV2080_CTRL_CMD_GSP_GET_RM_HEAP_STATS`|Sub|-|subdevice:13117 f=0x48|
|`0x20803604`|`NV2080_CTRL_CMD_GSP_GET_LIBOS_HEAP_STATS`|Sub|-|subdevice:13147 f=0x248|
|`0x20803606`|`NV2080_CTRL_CMD_GSP_CORE_TEST`|Sub|-|subdevice:13177 f=0x100044|
|`0x20803801`|`NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO`|Sub|S|subdevice:13192 f=0x10248|
|`0x20804001`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_BOOTLOAD_GSP_VGPU_PLUGIN_TASK`|Sub|-|subdevice:13297 f=0xc0|
|`0x20804002`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_SHUTDOWN_GSP_VGPU_PLUGIN_TASK`|Sub|-|subdevice:13312 f=0xc0|
|`0x20804003`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_PGPU_ADD_VGPU_TYPE`|Sub|-|subdevice:13327 f=0xc0|
|`0x20804004`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_ENUMERATE_VGPU_PER_PGPU`|Sub|-|subdevice:13342 f=0xc0|
|`0x20804005`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_CLEAR_GUEST_VM_INFO`|Sub|-|subdevice:13357 f=0xc0|
|`0x20804006`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_GET_VGPU_FB_USAGE`|Sub|-|subdevice:13372 f=0xc0|
|`0x20804007`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_SET_VGPU_ENCODER_CAPACITY`|Sub|-|subdevice:13387 f=0x1d0|
|`0x20804008`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_VGPU_PLUGIN_CLEANUP`|Sub|-|subdevice:13402 f=0xc0|
|`0x20804009`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_GET_PGPU_FS_ENCODING`|Sub|-|subdevice:13417 f=0xc0|
|`0x2080400a`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_GET_PGPU_MIGRATION_SUPPORT`|Sub|-|subdevice:13432 f=0xc0|
|`0x2080400b`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_SET_VGPU_MGR_CONFIG`|Sub|-|subdevice:13447 f=0xc0|
|`0x2080400c`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_FREE_STATES`|Sub|-|subdevice:13462 f=0xc0|
|`0x2080400d`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_GET_FRAME_RATE_LIMITER_STATUS`|Sub|-|subdevice:13477 f=0xc0|
|`0x2080400e`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_SET_VGPU_HETEROGENEOUS_MODE`|Sub|-|subdevice:13492 f=0xc0|
|`0x2080400f`|`NV2080_CTRL_CMD_VGPU_MGR_INTERNAL_SET_VGPU_MIG_TIMESLICE_MODE`|Sub|-|subdevice:13507 f=0xc0|
|`0x20804101`|`NV2080_CTRL_CMD_HSHUB_GET_AVAILABLE_MASK`|Sub|-|subdevice:13522 f=0x158|
|`0x20804102`|`NV2080_CTRL_CMD_HSHUB_SET_EC_THROTTLE_MODE`|Sub|-|subdevice:13537 f=0x158|
|`0x208081f3`|`NV2080_CTRL_CMD_GPU_IS_RESET_COUPLED`|Sub|R|subdevice:13552 f=0x48|
|`0x208081f4`|`NV2080_CTRL_CMD_GPU_GET_DIELET_INFO`|Sub|R|subdevice:13567 f=0x48|
|`0x208081f5`|`NV2080_CTRL_CMD_GPU_GET_UNIT_FS_INFO_FROM_DIELET`|Sub|R|subdevice:13582 f=0x48|
|`0x2080a7d7`|`NV2080_CTRL_CMD_INTERNAL_GCX_ENTRY_PREREQUISITE`|Sub|R|subdevice:13597 f=0xc0|
|`0x208f0401`|`NV208F_CTRL_CMD_FIFO_CHECK_ENGINE_CONTEXT`|Diag|-|subdevice_diag:265 f=0x48|
|`0x208f0506`|`NV208F_CTRL_CMD_FB_CTRL_GPU_CACHE`|Diag|-|subdevice_diag:295 f=0x48|
|`0x208f050e`|`NV208F_CTRL_CMD_FB_ECC_SET_KILL_PTR`|Diag|-|subdevice_diag:310 f=0x48|
|`0x208f0510`|`NV208F_CTRL_CMD_FB_ECC_INJECTION_SUPPORTED`|Diag|-|subdevice_diag:325 f=0x48|
|`0x208f0511`|`NV208F_CTRL_CMD_FB_ECC_SET_WRITE_KILL`|Diag|-|subdevice_diag:340 f=0x48|
|`0x208f0515`|`NV208F_CTRL_CMD_FB_CLEAR_REMAPPED_ROWS`|Diag|-|subdevice_diag:355 f=0x44|
|`0x208f051b`|`NV208F_CTRL_CMD_FB_CLEAR_REMAPPED_BANKS`|Diag|-|subdevice_diag:370 f=0x44|
|`0x208f0701`|`NV208F_CTRL_CMD_BIF_PBI_WRITE_COMMAND`|Diag|-|subdevice_diag:385 f=0x48|
|`0x208f0702`|`NV208F_CTRL_CMD_BIF_CONFIG_REG_READ`|Diag|-|subdevice_diag:400 f=0x48|
|`0x208f0703`|`NV208F_CTRL_CMD_BIF_CONFIG_REG_WRITE`|Diag|-|subdevice_diag:415 f=0x48|
|`0x208f0704`|`NV208F_CTRL_CMD_BIF_INFO`|Diag|-|subdevice_diag:430 f=0x48|
|`0x208f1101`|`NV208F_CTRL_CMD_GPU_GET_RAM_SVOP_VALUES`|Diag|-|subdevice_diag:445 f=0x48|
|`0x208f1102`|`NV208F_CTRL_CMD_GPU_SET_RAM_SVOP_VALUES`|Diag|-|subdevice_diag:460 f=0x48|
|`0x208f1105`|`NV208F_CTRL_CMD_GPU_VERIFY_INFOROM`|Diag|T|subdevice_diag:475 f=0x48|
|`0x208f1206`|`NV208F_CTRL_CMD_GR_INJECT_CTXSW_UCODE_ERROR`|Diag|-|subdevice_diag:490 f=0x100048|
|`0x402c0101`|`NV402C_CTRL_CMD_I2C_GET_PORT_INFO`|I2c|-|i2c_api:168 f=0x48|
|`0x402c0102`|`NV402C_CTRL_CMD_I2C_INDEXED`|I2c|-|i2c_api:183 f=0x48|
|`0x402c0103`|`NV402C_CTRL_CMD_I2C_GET_PORT_SPEED`|I2c|-|i2c_api:198 f=0x48|
|`0x402c0104`|`NV402C_CTRL_CMD_I2C_TABLE_GET_DEV_INFO`|I2c|-|i2c_api:213 f=0x48|
|`0x402c0105`|`NV402C_CTRL_CMD_I2C_TRANSACTION`|I2c|-|i2c_api:228 f=0x48|
|`0x506f0106`|`NV506F_CTRL_CMD_INTERNAL_RESET_ISOLATED_CHANNEL`|Chan|-|kernel_channel:433 f=0xc8|
|`0x50700115`|`NV5070_CTRL_CMD_GET_PINSET_COUNT`|NvDisp|-|disp_objs:853 f=0x48|
|`0x50700116`|`NV5070_CTRL_CMD_GET_PINSET_PEER`|NvDisp|-|disp_objs:868 f=0x48|
|`0x50700117`|`NV5070_CTRL_CMD_SET_RMFREE_FLAGS`|NvDisp|-|disp_objs:883 f=0x40|
|`0x50700118`|`NV5070_CTRL_CMD_IMP_SET_GET_PARAMETER`|NvDisp|-|disp_objs:898 f=0x48|
|`0x50700119`|`NV5070_CTRL_CMD_SET_MEMPOOL_WAR_FOR_BLIT_TEARING`|NvDisp|-|disp_objs:913 f=0x48|
|`0x50700202`|`NV5070_CTRL_CMD_GET_RG_STATUS`|NvDisp|-|disp_objs:928 f=0x44|
|`0x50700203`|`NV5070_CTRL_CMD_GET_RG_UNDERFLOW_PROP`|NvDisp|-|disp_objs:943 f=0x48|
|`0x50700204`|`NV5070_CTRL_CMD_SET_RG_UNDERFLOW_PROP`|NvDisp|-|disp_objs:958 f=0x48|
|`0x50700205`|`NV5070_CTRL_CMD_GET_RG_FLIPLOCK_PROP`|NvDisp|-|disp_objs:973 f=0x48|
|`0x50700206`|`NV5070_CTRL_CMD_SET_RG_FLIPLOCK_PROP`|NvDisp|-|disp_objs:988 f=0x40|
|`0x5070020b`|`NV5070_CTRL_CMD_GET_PINSET_LOCKPINS`|NvDisp|-|disp_objs:1018 f=0x48|
|`0x5070020c`|`NV5070_CTRL_CMD_GET_RG_SCAN_LINE`|NvDisp|-|disp_objs:1033 f=0x48|
|`0x5070020d`|`NV5070_CTRL_CMD_GET_FRAMELOCK_HEADER_LOCKPINS`|NvDisp|-|disp_objs:1048 f=0x40|
|`0x50700422`|`NV5070_CTRL_CMD_GET_SOR_OP_MODE`|NvDisp|-|disp_objs:1063 f=0x44|
|`0x50700423`|`NV5070_CTRL_CMD_SET_SOR_OP_MODE`|NvDisp|-|disp_objs:1078 f=0x44|
|`0x50700457`|`NV5070_CTRL_CMD_SET_SOR_FLUSH_MODE`|NvDisp|-|disp_objs:1093 f=0x44|
|`0x50700709`|`NV5070_CTRL_CMD_SYSTEM_GET_CAPS_V2`|NvDisp|-|disp_objs:1108 f=0x48|
|`0x50800101`|`NV5080_CTRL_CMD_DEFERRED_API`|DefApi|-|deferred_api:207 f=0x50048|
|`0x50800102`|`NV5080_CTRL_CMD_REMOVE_API`|DefApi|-|deferred_api:222 f=0x50048|
|`0x50800104`|`NV5080_CTRL_CMD_DEFERRED_API_INTERNAL`|DefApi|-|deferred_api:252 f=0x500c8|
|`0x83de022d`|`NV83DE_CTRL_CMD_DEBUG_SET_NON_PREEMPTABLE_DEBUGGER_CONTEXT`|SmDbg|-|kernel_sm_debugger_session:593 f=0x48|
|`0x83de022e`|`NV83DE_CTRL_CMD_DEBUG_SET_RECOVERY_SUPPRESSION_TIMEOUT`|SmDbg|-|kernel_sm_debugger_session:608 f=0x48|
|`0x83de0301`|`NV83DE_CTRL_CMD_SM_DEBUG_MODE_ENABLE`|SmDbg|-|kernel_sm_debugger_session:623 f=0x48|
|`0x83de0302`|`NV83DE_CTRL_CMD_SM_DEBUG_MODE_DISABLE`|SmDbg|-|kernel_sm_debugger_session:638 f=0x48|
|`0x83de0307`|`NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG`|SmDbg|R|kernel_sm_debugger_session:653 f=0x10248|
|`0x83de0308`|`NV83DE_CTRL_CMD_DEBUG_GET_MODE_MMU_DEBUG`|SmDbg|-|kernel_sm_debugger_session:668 f=0x10248|
|`0x83de0309`|`NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK`|SmDbg|S|kernel_sm_debugger_session:683 f=0x10248|
|`0x83de030b`|`NV83DE_CTRL_CMD_DEBUG_READ_SINGLE_SM_ERROR_STATE`|SmDbg|-|kernel_sm_debugger_session:698 f=0x10248|
|`0x83de030c`|`NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES`|SmDbg|T|kernel_sm_debugger_session:713 f=0x50048|
|`0x83de030f`|`NV83DE_CTRL_CMD_DEBUG_CLEAR_SINGLE_SM_ERROR_STATE`|SmDbg|-|kernel_sm_debugger_session:728 f=0x10248|
|`0x83de0310`|`NV83DE_CTRL_CMD_DEBUG_CLEAR_ALL_SM_ERROR_STATES`|SmDbg|T|kernel_sm_debugger_session:743 f=0x50048|
|`0x83de0313`|`NV83DE_CTRL_CMD_DEBUG_SET_NEXT_STOP_TRIGGER_TYPE`|SmDbg|-|kernel_sm_debugger_session:758 f=0x10248|
|`0x83de0314`|`NV83DE_CTRL_CMD_DEBUG_SET_SINGLE_STEP_INTERRUPT_HANDLING`|SmDbg|-|kernel_sm_debugger_session:773 f=0x48|
|`0x83de0317`|`NV83DE_CTRL_CMD_DEBUG_SUSPEND_CONTEXT`|SmDbg|R|kernel_sm_debugger_session:818 f=0x10248|
|`0x83de0318`|`NV83DE_CTRL_CMD_DEBUG_RESUME_CONTEXT`|SmDbg|R|kernel_sm_debugger_session:833 f=0x10248|
|`0x83de031f`|`NV83DE_CTRL_CMD_DEBUG_SET_MODE_ERRBAR_DEBUG`|SmDbg|-|kernel_sm_debugger_session:908 f=0x10248|
|`0x83de0320`|`NV83DE_CTRL_CMD_DEBUG_GET_MODE_ERRBAR_DEBUG`|SmDbg|-|kernel_sm_debugger_session:923 f=0x48|
|`0x83de0321`|`NV83DE_CTRL_CMD_DEBUG_SET_SINGLE_SM_SINGLE_STEP`|SmDbg|-|kernel_sm_debugger_session:938 f=0x10248|
|`0x83de0322`|`NV83DE_CTRL_CMD_DEBUG_SET_SINGLE_SM_STOP_TRIGGER`|SmDbg|-|kernel_sm_debugger_session:953 f=0x48|
|`0x83de0323`|`NV83DE_CTRL_CMD_DEBUG_SET_SINGLE_SM_RUN_TRIGGER`|SmDbg|-|kernel_sm_debugger_session:968 f=0x48|
|`0x83de0324`|`NV83DE_CTRL_CMD_DEBUG_SET_SINGLE_SM_SKIP_IDLE_WARP_DETECT`|SmDbg|-|kernel_sm_debugger_session:983 f=0x48|
|`0x83de0325`|`NV83DE_CTRL_CMD_DEBUG_GET_SINGLE_SM_DEBUGGER_STATUS`|SmDbg|-|kernel_sm_debugger_session:998 f=0x48|
|`0x83de0329`|`NV83DE_CTRL_CMD_DEBUG_SET_DROP_DEFERRED_RC`|SmDbg|-|kernel_sm_debugger_session:1058 f=0x48|
|`0x83de032a`|`NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_GCC_DEBUG`|SmDbg|-|kernel_sm_debugger_session:1073 f=0x248|
|`0x83de032b`|`NV83DE_CTRL_CMD_DEBUG_GET_MODE_MMU_GCC_DEBUG`|SmDbg|-|kernel_sm_debugger_session:1088 f=0x248|
|`0x90670102`|`NV9067_CTRL_CMD_SET_TPC_PARTITION_TABLE`|CtxShare|-|kernel_ctxshare:396 f=0x40|
|`0x90670201`|`NV9067_CTRL_CMD_GET_CWD_WATERMARK`|CtxShare|-|kernel_ctxshare:411 f=0x40|
|`0x90670202`|`NV9067_CTRL_CMD_SET_CWD_WATERMARK`|CtxShare|-|kernel_ctxshare:426 f=0x40|
|`0x906f0105`|`NV906F_CTRL_CMD_GET_DEFER_RC_STATE`|Chan|-|kernel_channel:478 f=0x48|
|`0x906f0106`|`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`|Chan|S|kernel_channel:493 f=0x10048|
|`0x90720101`|`NV9072_CTRL_CMD_NOTIFY_ON_VBLANK`|DispSw0|-|dispsw:189 f=0x48|
|`0x90740101`|`NV9074_CTRL_CMD_FLUSH`|TimedSema|-|timed_sema:201 f=0x48|
|`0x90740103`|`NV9074_CTRL_CMD_RELEASE`|TimedSema|-|timed_sema:231 f=0x48|
|`0x90960101`|`NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR`|Zbc|T|zbc_api:180 f=0x10248|
|`0x90960102`|`NV9096_CTRL_CMD_SET_ZBC_DEPTH_CLEAR`|Zbc|-|zbc_api:195 f=0x10248|
|`0x90960103`|`NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE`|Zbc|-|zbc_api:210 f=0x10248|
|`0x90960104`|`NV9096_CTRL_CMD_SET_ZBC_CLEAR_TABLE`|Zbc|-|zbc_api:225 f=0x48|
|`0x90960105`|`NV9096_CTRL_CMD_SET_ZBC_STENCIL_CLEAR`|Zbc|-|zbc_api:240 f=0x10248|
|`0x90960106`|`NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE`|Zbc|T|zbc_api:255 f=0x50048|
|`0x90960107`|`NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_ENTRY`|Zbc|T|zbc_api:270 f=0x10248|
|`0x90cc0101`|`NV90CC_CTRL_CMD_HWPM_RESERVE`|Prof|-|profiler_v1:168 f=0x48|
|`0x90cc0102`|`NV90CC_CTRL_CMD_HWPM_RELEASE`|Prof|-|profiler_v1:183 f=0x48|
|`0x90cc0103`|`NV90CC_CTRL_CMD_HWPM_GET_RESERVATION_INFO`|Prof|-|profiler_v1:198 f=0x48|
|`0x90cc0301`|`NV90CC_CTRL_CMD_POWER_REQUEST_FEATURES`|Prof|-|profiler_v1:213 f=0x48|
|`0x90cc0302`|`NV90CC_CTRL_CMD_POWER_RELEASE_FEATURES`|Prof|-|profiler_v1:228 f=0x48|
|`0x90e60101`|`NV90E6_CTRL_CMD_MASTER_GET_ERROR_INTR_OFFSET_MASK`|GenEng|-|generic_engine:168 f=0x48|
|`0x90e60102`|`NV90E6_CTRL_CMD_MASTER_GET_VIRTUAL_FUNCTION_ERROR_CONT_INTR_MASK`|GenEng|T|generic_engine:183 f=0x10248|
|`0x90e70102`|`NV90E7_CTRL_CMD_BBX_GET_TIME_DATA`|GenEng|-|generic_engine:198 f=0x58|
|`0x90e70113`|`NV90E7_CTRL_CMD_BBX_GET_LAST_FLUSH_TIME`|GenEng|-|generic_engine:213 f=0x58|
|`0x90e70119`|`NV90E7_CTRL_CMD_BBX_IS_NVM_FLUSH_ENABLED`|GenEng|-|generic_engine:228 f=0x58|
|`0xa06c0105`|`NVA06C_CTRL_CMD_PREEMPT`|ChanGrp|S|kernel_channel_group_api:375 f=0x10248|
|`0xa06c0109`|`NVA06C_CTRL_CMD_PROGRAM_VIDMEM_PROMOTE`|ChanGrp|-|kernel_channel_group_api:420 f=0x48|
|`0xa06c010a`|`NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS`|ChanGrp|S|kernel_channel_group_api:435 f=0x14240|
|`0xa06c0110`|`NVA06C_CTRL_CMD_MAKE_REALTIME`|ChanGrp|-|kernel_channel_group_api:450 f=0x48|
|`0xa06c0201`|`NVA06C_CTRL_CMD_INTERNAL_GPFIFO_SCHEDULE`|ChanGrp|-|kernel_channel_group_api:465 f=0x100c8|
|`0xa06c0202`|`NVA06C_CTRL_CMD_INTERNAL_SET_TIMESLICE`|ChanGrp|-|kernel_channel_group_api:480 f=0x100c8|
|`0xa06f0111`|`NVA06F_CTRL_CMD_RESTART_RUNLIST`|Chan|-|kernel_channel:568 f=0x48|
|`0xb06f010b`|`NVB06F_CTRL_CMD_GET_ENGINE_CTX_SIZE`|Chan|-|kernel_channel:613 f=0x10048|
|`0xb06f010c`|`NVB06F_CTRL_CMD_GET_ENGINE_CTX_DATA`|Chan|-|kernel_channel:628 f=0x10048|
|`0xb06f010d`|`NVB06F_CTRL_CMD_MIGRATE_ENGINE_CTX_DATA`|Chan|-|kernel_channel:643 f=0x14044|
|`0xb06f010e`|`NVB06F_CTRL_CMD_GET_ENGINE_CTX_STATE`|Chan|-|kernel_channel:658 f=0x10048|
|`0xb06f010f`|`NVB06F_CTRL_CMD_GET_CHANNEL_HW_STATE`|Chan|-|kernel_channel:673 f=0x10048|
|`0xb06f0110`|`NVB06F_CTRL_CMD_SET_CHANNEL_HW_STATE`|Chan|-|kernel_channel:688 f=0x14044|
|`0xb06f0111`|`NVB06F_CTRL_CMD_SAVE_ENGINE_CTX_DATA`|Chan|-|kernel_channel:703 f=0x10048|
|`0xb06f0112`|`NVB06F_CTRL_CMD_RESTORE_ENGINE_CTX_DATA`|Chan|-|kernel_channel:718 f=0x14044|
|`0xb0cc0102`|`NVB0CC_CTRL_CMD_RELEASE_HWPM_LEGACY`|ProfBase|-|profiler_v2:357 f=0x48|
|`0xb0cc0103`|`NVB0CC_CTRL_CMD_RESERVE_PM_AREA_SMPC`|ProfBase|-|profiler_v2:372 f=0x10248|
|`0xb0cc0104`|`NVB0CC_CTRL_CMD_RELEASE_PM_AREA_SMPC`|ProfBase|-|profiler_v2:387 f=0x48|
|`0xb0cc0109`|`NVB0CC_CTRL_CMD_PMA_STREAM_UPDATE_GET_PUT`|ProfBase|-|profiler_v2:462 f=0x50048|
|`0xb0cc010a`|`NVB0CC_CTRL_CMD_EXEC_REG_OPS`|ProfBase|R|profiler_v2:477 f=0x10248|
|`0xb0cc010b`|`NVB0CC_CTRL_CMD_RESERVE_PM_AREA_PC_SAMPLER`|ProfBase|-|profiler_v2:492 f=0x10248|
|`0xb0cc010c`|`NVB0CC_CTRL_CMD_RELEASE_PM_AREA_PC_SAMPLER`|ProfBase|-|profiler_v2:507 f=0x10248|
|`0xb0cc010d`|`NVB0CC_CTRL_CMD_GET_TOTAL_HS_CREDITS`|ProfBase|-|profiler_v2:522 f=0x10248|
|`0xb0cc010e`|`NVB0CC_CTRL_CMD_SET_HS_CREDITS`|ProfBase|-|profiler_v2:537 f=0x10248|
|`0xb0cc010f`|`NVB0CC_CTRL_CMD_GET_HS_CREDITS`|ProfBase|-|profiler_v2:552 f=0x10248|
|`0xb0cc0113`|`NVB0CC_CTRL_CMD_RESERVE_HES`|ProfBase|-|profiler_v2:567 f=0x10248|
|`0xb0cc0114`|`NVB0CC_CTRL_CMD_RELEASE_HES`|ProfBase|-|profiler_v2:582 f=0x10248|
|`0xb0cc0115`|`NVB0CC_CTRL_CMD_GET_CHIPLET_HS_CREDIT_POOL`|ProfBase|-|profiler_v2:597 f=0x10248|
|`0xb0cc0116`|`NVB0CC_CTRL_CMD_GET_HS_CREDITS_MAPPING`|ProfBase|-|profiler_v2:612 f=0x10248|
|`0xb0cc0117`|`NVB0CC_CTRL_CMD_DISABLE_DYNAMIC_MMA_BOOST`|ProfBase|-|profiler_v2:627 f=0x48|
|`0xb0cc0118`|`NVB0CC_CTRL_CMD_GET_DYNAMIC_MMA_BOOST_STATUS`|ProfBase|-|profiler_v2:642 f=0x48|
|`0xb0cc0119`|`NVB0CC_CTRL_CMD_RESERVE_CCU_PROF`|ProfBase|-|profiler_v2:657 f=0x10248|
|`0xb0cc011a`|`NVB0CC_CTRL_CMD_RELEASE_CCU_PROF`|ProfBase|-|profiler_v2:672 f=0x10248|
|`0xb0cc0201`|`NVB0CC_CTRL_CMD_INTERNAL_QUIESCE_PMA_CHANNEL`|ProfBase|-|profiler_v2:687 f=0x10048|
|`0xb0cc0203`|`NVB0CC_CTRL_CMD_INTERNAL_PERMISSIONS_INIT`|ProfBase|-|profiler_v2:717 f=0x48|
|`0xb0cc0204`|`NVB0CC_CTRL_CMD_INTERNAL_ALLOC_PMA_STREAM`|ProfBase|-|profiler_v2:732 f=0x500c8|
|`0xb0cc0206`|`NVB0CC_CTRL_CMD_INTERNAL_FREE_PMA_STREAM`|ProfBase|-|profiler_v2:747 f=0x500c8|
|`0xb0cc0207`|`NVB0CC_CTRL_CMD_INTERNAL_GET_MAX_PMAS`|ProfBase|-|profiler_v2:762 f=0x500c8|
|`0xb0cc0208`|`NVB0CC_CTRL_CMD_INTERNAL_BIND_PM_RESOURCES`|ProfBase|-|profiler_v2:777 f=0x100c8|
|`0xb0cc0209`|`NVB0CC_CTRL_CMD_INTERNAL_UNBIND_PM_RESOURCES`|ProfBase|-|profiler_v2:792 f=0xc8|
|`0xb0cc020a`|`NVB0CC_CTRL_CMD_INTERNAL_RESERVE_HWPM_LEGACY`|ProfBase|-|profiler_v2:807 f=0x100c8|
|`0xb0cc0301`|`NVB0CC_CTRL_CMD_POWER_REQUEST_FEATURES`|ProfBase|-|profiler_v2:822 f=0x40048|
|`0xb0cc0302`|`NVB0CC_CTRL_CMD_POWER_RELEASE_FEATURES`|ProfBase|-|profiler_v2:837 f=0x40048|
|`0xc3650108`|`NVC365_CTRL_CMD_ACCESS_CNTR_BUFFER_RESET_COUNTERS`|AccCntr|-|access_cntr_buffer:316 f=0x40|
|`0xc36f0109`|`NVC36F_CTRL_CMD_GPFIFO_UPDATE_FAULT_METHOD_BUFFER`|Chan|-|kernel_channel:748 f=0x10244|
|`0xc36f0301`|`NVC36F_CTRL_CMD_INTERNAL_GPFIFO_GET_WORK_SUBMIT_TOKEN`|Chan|-|kernel_channel:778 f=0xc0|
|`0xc3700101`|`NVC370_CTRL_CMD_IDLE_CHANNEL`|NvDisp|-|disp_objs:1123 f=0x48|
|`0xc3700102`|`NVC370_CTRL_CMD_SET_ACCL`|NvDisp|-|disp_objs:1138 f=0x48|
|`0xc3700103`|`NVC370_CTRL_CMD_GET_ACCL`|NvDisp|-|disp_objs:1153 f=0x48|
|`0xc3700104`|`NVC370_CTRL_CMD_GET_CHANNEL_INFO`|NvDisp|-|disp_objs:1168 f=0x40|
|`0xc3700201`|`NVC370_CTRL_CMD_GET_LOCKPINS_CAPS`|NvDisp|-|disp_objs:1198 f=0x48|
|`0xc3700401`|`NVC370_CTRL_CMD_SET_SOR_FLUSH_MODE`|NvDisp|-|disp_objs:1213 f=0x44|
|`0xc3700602`|`NVC370_CTRL_CMD_SET_FORCE_MODESWITCH_FLAGS_OVERRIDES`|NvDisp|-|disp_objs:1228 f=0x48|
|`0xc3720101`|`NVC372_CTRL_CMD_IS_MODE_POSSIBLE`|DispSw|-|disp_objs:1870 f=0x48|
|`0xc3720102`|`NVC372_CTRL_CMD_IS_MODE_POSSIBLE_OR_SETTINGS`|DispSw|-|disp_objs:1885 f=0x48|
|`0xc3720104`|`NVC372_CTRL_CMD_GET_ACTIVE_VIEWPORT_POINT_IN`|DispSw|-|disp_objs:1900 f=0x49|
|`0xc7630103`|`NVC763_CTRL_CMD_VIDMEM_ACCESS_BIT_DUMP`|VidAccBit|-|vidmem_access_bit_buffer:157 f=0x10048|

## §C — controls reaching GSP by other transports (84 rows)

| id | symbolic name | kernel flags | transport to GSP (file:line) | wire? | kf |
|---|---|---|---|---|---|
|`0x0000013c`|`NV0000_CTRL_CMD_SYSTEM_SYNC_EXTERNAL_FABRIC_MGMT`|`0x4`|PhysRmApi core/system.c:857||ABSENT|
|`0x00730151`|`NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/disp/kern_disp.c:723|**Y**|ABSENT|
|`0x00801809`|`NV0080_CTRL_CMD_DMA_GET_PDE_INFO`|`0x10008`|RPC-pass gpu/mem_mgr/dma.c:799 (handler deviceCtrlCmdDmaGetPdeInfo_IMPL)||ABSENT|
|`0x0080180c`|`NV0080_CTRL_CMD_DMA_INVALIDATE_TLB`|`0x10008`|RPC-pass gpu/mem_mgr/dma.c:988 (handler deviceCtrlCmdDmaInvalidateTLB_IMPL)||ABSENT|
|`0x0080180e`|`NV0080_CTRL_CMD_DMA_SET_VA_SPACE_SIZE`|`0x10008`|RPC-pass gpu/mem_mgr/dma.c:400 (handler deviceCtrlCmdDmaSetVASpaceSize_IMPL)||ABSENT|
|`0x00801812`|`NV0080_CTRL_DMA_SET_DEFAULT_VASPACE`|`0x1c000`|RPC-pass gpu/mem_mgr/dma.c:844 (handler deviceCtrlCmdDmaSetDefaultVASpace_IMPL)||ABSENT|
|`0x00801813`|`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`|`0x14004`|RPC-lit gpu/mem_mgr/dma.c:445||SERVED|
|`0x00801814`|`NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY`|`0x14004`|RPC-lit gpu/mem_mgr/dma.c:543||STUB|
|`0x0080200a`|`NV0080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED`|`0x10008`|RPC-pass gpu/mem_sys/kern_mem_sys_ctrl.c:1409 (helper _kmemsysSetZbcReferenced, caller deviceCtrlCmdFbSetZbcReferenced_IMPL:1424)||ABSENT|
|`0x00900105`|`NV0090_CTRL_CMD_GET_MMU_DEBUG_MODE`|`0x10008`|RPC-lit gpu/mmu/arch/volta/kern_gmmu_gv100.c:2039||ABSENT|
|`0x00900107`|`NV0090_CTRL_CMD_PROGRAM_VIDMEM_PROMOTE`|`0x8`|RPC-pass gpu/gr/kernel_graphics_context.c:3464 (handler kgrctxCtrlProgramVidmemPromote_IMPL)||ABSENT|
|`0x00f80102`|`NV00F8_CTRL_CMD_DESCRIBE`|`0x10318`|RPC-lit mem_mgr/mem_fabric.c:432||ABSENT|
|`0x20800102`|`NV2080_CTRL_CMD_GPU_GET_INFO_V2`|`0x30118`|RPC-lit gpu/subdevice/subdevice_ctrl_gpu_kernel.c:628||SERVED|
|`0x20800122`|`NV2080_CTRL_CMD_GPU_EXEC_REG_OPS`|`0x10118`|RPC-pass gpu/subdevice/subdevice_ctrl_gpu_regops.c:171 (handler subdeviceCtrlCmdGpuExecRegOps_cmn)||REFUSED|
|`0x20800198`|`NV2080_CTRL_CMD_GPU_VALIDATE_MEM_MAP_REQUEST`|`0x110`|PhysRmApi gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2975||ABSENT|
|`0x208001e8`|`NV2080_CTRL_CMD_GPU_RPC_GSP_TEST`|`0x100108`|RPC-pass gpu/subdevice/subdevice_ctrl_gpu_kernel.c:3872 (handler subdeviceCtrlCmdGpuRpcGspTest_IMPL)||ABSENT|
|`0x208001f4`|`NV2080_CTRL_CMD_INTERNAL_GPU_GET_FABRIC_PROBE_INFO`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:601||ABSENT|
|`0x208001f5`|`NV2080_CTRL_CMD_INTERNAL_GPU_START_FABRIC_PROBE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:1435||ABSENT|
|`0x208001f6`|`NV2080_CTRL_CMD_INTERNAL_GPU_STOP_FABRIC_PROBE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:1482||ABSENT|
|`0x208001f7`|`NV2080_CTRL_CMD_INTERNAL_GPU_SUSPEND_FABRIC_PROBE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:1285||ABSENT|
|`0x208001f8`|`NV2080_CTRL_CMD_INTERNAL_GPU_RESUME_FABRIC_PROBE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:1345||ABSENT|
|`0x208001f9`|`NV2080_CTRL_CMD_INTERNAL_GPU_INVALIDATE_FABRIC_PROBE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gpu_fabric_probe.c:1313||ABSENT|
|`0x20800301`|`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION`|`0x10118`|PhysRmApi gpu/subdevice/subdevice_ctrl_event_kernel.c:114|**Y**|SERVED|
|`0x20800406`|`NV2080_CTRL_CMD_TIMER_GET_GPU_CPU_TIME_CORRELATION_INFO`|`0x108`|PhysRmApi gpu/subdevice/subdevice_ctrl_timer_kernel.c:338||STUB|
|`0x20800a28`|`NV2080_CTRL_CMD_KGR_GET_CTX_BUFFER_PTES`|`0x8000`|RPC-pass gpu/gr/kernel_graphics.c:4339 (handler subdeviceCtrlCmdKGrGetCtxBufferPtes_IMPL)||ABSENT|
|`0x20800a69`|`NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED`|`0x10008`|RPC-pass gpu/mem_sys/kern_mem_sys_ctrl.c:1409 (helper _kmemsysSetZbcReferenced, caller subdeviceCtrlCmdFbSetZbcReferenced_IMPL:1434)||ABSENT|
|`0x20800a89`|`NV2080_CTRL_CMD_INTERNAL_SEND_CMC_LIBOS_BUFFER_INFO`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/gsplite/kernel_gsplite.c:253||ABSENT|
|`0x20800aa2`|`NV2080_CTRL_CMD_INTERNAL_STATIC_GRMGR_GET_SKYLINE_INFO`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/subdevice/subdevice_ctrl_gpu_smc.c:65||ABSENT|
|`0x20800aa3`|`NV2080_CTRL_CMD_INTERNAL_MIGMGR_SET_PARTITIONING_MODE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/mig_mgr/kernel_mig_manager.c:1019||ABSENT|
|`0x20800aa5`|`NV2080_CTRL_CMD_INTERNAL_MIGMGR_SET_GPU_INSTANCES`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/mig_mgr/kernel_mig_manager.c:1148||ABSENT|
|`0x20800aa6`|`NV2080_CTRL_CMD_INTERNAL_MIGMGR_GET_GPU_INSTANCES`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/mig_mgr/kernel_mig_manager.c:7964||ABSENT|
|`0x20800aa8`|`NV2080_CTRL_CMD_INTERNAL_MIGMGR_EXPORT_GPU_INSTANCE`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/mig_mgr/kernel_mig_manager.c:4285||ABSENT|
|`0x2080110b`|`NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS`|`0x10108`|RPC-pass gpu/fifo/kernel_fifo_ctrl.c:731 (handler subdeviceCtrlCmdFifoDisableChannels_IMPL)||STUB|
|`0x2080110c`|`NV2080_CTRL_CMD_FIFO_GET_CHANNEL_MEM_INFO`|`0x10004`|RPC-lit gpu/fifo/kernel_channel.c:2476||ABSENT|
|`0x20801207`|`NV2080_CTRL_CMD_GR_CTXSW_PM_MODE`|`0x118`|PhysRmApi gpu/gr/kernel_graphics.c:3940||ABSENT|
|`0x20801218`|`NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_SIZE`|`0x18`|RPC-pass gpu/gr/kernel_graphics.c:4150 (handler subdeviceCtrlCmdKGrGetCtxBufferSize_IMPL)||STUB|
|`0x20801219`|`NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO`|`0x8000`|RPC-pass gpu/gr/kernel_graphics.c:4257 (handler subdeviceCtrlCmdKGrGetCtxBufferInfo_IMPL)||STUB|
|`0x2080123b`|`NV2080_CTRL_CMD_GR_GET_TPC_RECONFIG_MASK`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/subdevice/subdevice_ctrl_gpu_kernel.c:1927||ABSENT|
|`0x20801303`|`NV2080_CTRL_CMD_FB_GET_INFO_V2`|`0x10118`|RPC-lit gpu/mem_mgr/arch/maxwell/mem_mgr_gm107.c:666||SERVED|
|`0x20801702`|`NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`|`0x10118`|RPC-pass gpu/intr/intr.c:219 (handler subdeviceCtrlCmdMcServiceInterrupts_IMPL)||SERVED|
|`0x20801803`|`NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO`|`0x10518`|PhysRmApi gpu/bus/kern_bus.c:602|**Y**|SERVED|
|`0x20801823`|`NV2080_CTRL_CMD_BUS_GET_INFO_V2`|`0x10118`|PhysRmApi gpu/bus/kern_bus.c:1109||SERVED|
|`0x2080205b`|`NV2080_CTRL_CMD_PERF_SET_POWERSTATE`|`0x8`|PhysRmApi gpu/perf/kern_perf_pwr.c:238||ABSENT|
|`0x2080206f`|`NV2080_CTRL_CMD_PERF_RATED_TDP_SET_CONTROL`|`0x10008`|RPC-pass gpu/perf/kern_perf_pwr.c:103 (handler subdeviceCtrlCmdPerfRatedTdpSetControl_KERNEL)||ABSENT|
|`0x20802092`|`NV2080_CTRL_CMD_PERF_SET_AUX_POWER_STATE`|`0x8`|PhysRmApi gpu/perf/kern_perf_pwr.c:188||ABSENT|
|`0x20802093`|`NV2080_CTRL_CMD_PERF_RESERVE_PERFMON_HW`|`0x18`|PhysRmApi gpu/perf/kern_perf_pm.c:270||ABSENT|
|`0x20802096`|`NV2080_CTRL_CMD_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2`|`0x50008`|PhysRmApi gpu/perf/kern_perf_ctrl.c:112||ABSENT|
|`0x20802205`|`NV2080_CTRL_CMD_RC_GET_ERROR_COUNT`|`0x8`|PhysRmApi gpu/rc/kernel_rc_ctrl.c:123||ABSENT|
|`0x20802213`|`NV2080_CTRL_CMD_RC_GET_ERROR_V2`|`0x8`|PhysRmApi gpu/rc/kernel_rc_ctrl.c:156||ABSENT|
|`0x20802502`|`NV2080_CTRL_CMD_DMA_INVALIDATE_TLB`|`0x10008`|RPC-pass gpu/mem_mgr/dma.c:885 (handler subdeviceCtrlCmdDmaInvalidateTLB_IMPL)||ABSENT|
|`0x20802a0d`|`NV2080_CTRL_CMD_CE_UPDATE_PCE_LCE_MAPPINGS_V2`|`0x4`|PhysRmApi gpu/ce/kernel_ce.c:814|**Y**|ABSENT|
|`0x20803002`|`NV2080_CTRL_CMD_NVLINK_GET_NVLINK_STATUS`|`0x10108`|RPC-pass gpu/nvlink/common_nvlinkapi.c:757 (handler subdeviceCtrlCmdBusGetNvlinkStatus_IMPL)||STUB|
|`0x20803125`|`NV2080_CTRL_CMD_FLCN_GET_CTX_BUFFER_SIZE`|`0x8`|RPC-pass gpu/falcon/kernel_falcon_ctrl.c:188 (handler subdeviceCtrlCmdFlcnGetCtxBufferSize_IMPL)||STUB|
|`0x20803501`|`NV2080_CTRL_CMD_FLA_RANGE`|`0x10008`|RPC-lit gpu/bus/arch/ampere/kern_bus_ga100.c:256||ABSENT|
|`0x20803504`|`NV2080_CTRL_CMD_FLA_GET_FABRIC_MEM_STATS`|`0x10108`|RPC-pass gpu/subdevice/subdevice_ctrl_fla.c:299 (handler subdeviceCtrlCmdFlaGetFabricMemStats_IMPL)||ABSENT|
|`0x20803605`|`NV2080_CTRL_CMD_GSP_GDMA_FUZZ_TEST`|`0x100004`|RPC-lit gpu/subdevice/subdevice_ctrl_gpu_kernel.c:4393||ABSENT|
|`0x20808513`|`NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL`|`NO-KERNEL-EXPORT`|PhysRmApi gpu/subdevice/subdevice_ctrl_gpu_kernel.c:4081||REFUSED(rule)|
|`0x208f0403`|`NV208F_CTRL_CMD_FIFO_GET_CHANNEL_STATE`|`0x10008`|PhysRmApi gpu/fifo/kernel_channel.c:4107||ABSENT|
|`0x506f0105`|`NV506F_CTRL_CMD_RESET_ISOLATED_CHANNEL`|`0x10008`|PhysRmApi gpu/rc/kernel_rc_callback.c:198||ABSENT|
|`0x906f0101`|`NV906F_CTRL_GET_CLASS_ENGINEID`|`0x10008`|RPC-pass gpu/fifo/kernel_channel.c:2853 (handler kchannelCtrlCmdGetClassEngineid_IMPL)||STUB|
|`0x906f0102`|`NV906F_CTRL_CMD_RESET_CHANNEL`|`0x10008`|PhysRmApi gpu/rc/kernel_rc_watchdog_callback.c:88||STUB|
|`0x90f10102`|`NV90F1_CTRL_CMD_VASPACE_GET_PAGE_LEVEL_INFO`|`0x18000`|RPC-pass mem_mgr/gpu_vaspace.c:4044 (handler vaspaceapiCtrlCmdVaspaceGetPageLevelInfo_IMPL)||ABSENT|
|`0x90f10103`|`NV90F1_CTRL_CMD_VASPACE_RESERVE_ENTRIES`|`0x18000`|RPC-pass mem_mgr/gpu_vaspace.c:4117 (handler vaspaceapiCtrlCmdVaspaceReserveEntries_IMPL)||ABSENT|
|`0x90f10104`|`NV90F1_CTRL_CMD_VASPACE_RELEASE_ENTRIES`|`0x18000`|RPC-pass mem_mgr/gpu_vaspace.c:4170 (handler vaspaceapiCtrlCmdVaspaceReleaseEntries_IMPL)||ABSENT|
|`0x90f10106`|`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`|`0x14004`|RPC-pass mem_mgr/gpu_vaspace.c:4250 (handler gvaspaceCopyServerReservedPdes_IMPL)|**Y**|SERVED|
|`0xa06c0101`|`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`|`0x10008`|RPC-pass gpu/fifo/kernel_channel_group_api.c:1184 (handler kchangrpapiCtrlCmdGpFifoSchedule_IMPL)||SERVED|
|`0xa06c0103`|`NVA06C_CTRL_CMD_SET_TIMESLICE`|`0x10008`|RPC-pass gpu/fifo/kernel_channel_group_api.c:1329 (handler kchangrpapiCtrlCmdSetTimeslice_IMPL)||SERVED|
|`0xa06c0107`|`NVA06C_CTRL_CMD_SET_INTERLEAVE_LEVEL`|`0x10028`|RPC-pass gpu/fifo/kernel_channel_group_api.c:1406 (handler kchangrpapiCtrlCmdSetInterleaveLevel_IMPL)||ABSENT|
|`0xa06f0103`|`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`|`0x10008`|RPC-lit gpu/fifo/kernel_channel.c:3020|**Y**|SERVED|
|`0xa06f0104`|`NVA06F_CTRL_CMD_BIND`|`0x10008`|RPC-lit gpu/fifo/kernel_channel.c:2776|**Y**|SERVED|
|`0xa06f0109`|`NVA06F_CTRL_CMD_SET_INTERLEAVE_LEVEL`|`0x10028`|RPC-pass gpu/fifo/kernel_channel.c:3150 (handler kchannelCtrlCmdSetInterleaveLevel_IMPL)||ABSENT|
|`0xa06f0112`|`NVA06F_CTRL_CMD_STOP_CHANNEL`|`0x10008`|RPC-pass gpu/fifo/kernel_channel.c:1846 (handler kchannelCtrlCmdStopChannel_IMPL)||ABSENT|
|`0xa0800204`|`NVA080_CTRL_CMD_SET_FB_USAGE`|`NO-KERNEL-EXPORT`|RPC-lit vgpu/rpc.c:10625||ABSENT|
|`0xa0bc0101`|`NVA0BC_CTRL_CMD_NVENC_SW_SESSION_UPDATE_INFO`|`0x8`|RPC-lit gpu/nvenc/nvencsession.c:408||ABSENT|
|`0xa0bd0101`|`NVA0BD_CTRL_CMD_NVFBC_SW_SESSION_UPDATE_INFO`|`0x10008`|RPC-pass disp/nvfbc_session.c:332 (handler nvfbcsessionCtrlCmdNvFBCSwSessionUpdateInfo_IMPL)||ABSENT|
|`0xb0cc0105`|`NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM`|`0x10008`|RPC-lit gpu/hwpm/profiler_v2/kern_profiler_v2_ctrl.c:381||REFUSED|
|`0xb0cc0106`|`NVB0CC_CTRL_CMD_FREE_PMA_STREAM`|`0x10008`|RPC-lit gpu/hwpm/profiler_v2/kern_profiler_v2_ctrl.c:523||ABSENT|
|`0xc36f0108`|`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN`|`0x10008`|RPC-pass gpu/fifo/kernel_channel.c:3208 (handler kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL)||STUB|
|`0xc36f010a`|`NVC36F_CTRL_CMD_GPFIFO_SET_WORK_SUBMIT_TOKEN_NOTIF_INDEX`|`0x8`|RPC-pass gpu/fifo/kernel_channel.c:3277 (handler kchannelCtrlCmdGpfifoSetWorkSubmitTokenNotifIndex_IMPL)||STUB|
|`0xc56f010b`|`NVC56F_CTRL_CMD_GET_KMB`|`0x8`|PhysRmApi gpu/conf_compute/ccsl.c:493||STUB|
|`0xc56f010c`|`NVC56F_CTRL_ROTATE_SECURE_CHANNEL_IV`|`0x8`|PhysRmApi gpu/fifo/kernel_channel.c:4381||ABSENT|
|`0xc6370101`|`NVC637_CTRL_CMD_EXEC_PARTITIONS_CREATE`|`0x10008`|RPC-pass gpu/mig_mgr/gpu_instance_subscription.c:445 (handler gisubscriptionCtrlCmdExecPartitionsCreate_IMPL)||ABSENT|
|`0xc6370102`|`NVC637_CTRL_CMD_EXEC_PARTITIONS_DELETE`|`0x10008`|PhysRmApi gpu/mig_mgr/kernel_mig_manager.c:1125||ABSENT|
|`0xc6370105`|`NVC637_CTRL_CMD_EXEC_PARTITIONS_EXPORT`|`0x8`|PhysRmApi gpu/mig_mgr/gpu_instance_subscription.c:543||ABSENT|

---

## §D — BAR0 registers

The complete dispatch is two functions: `RegPlane::read_inner` (`plane.rs:3395-3466`) and
`RegPlane::write` (`plane.rs:3987-4172`). Anything not matched is **unclaimed**: reads return
`0`, writes are dropped, and the pair is sampled into the `UNCLAIMED-CENSUS`.

**Implemented: 69 discrete dwords + 2 byte-addressed windows (2 MiB), across 8 decode arms.**

| group | count | what |
|---|---|---|
| chip-constant boot registers | 7 | `PMC_BOOT_0/1/42`, PTIMER PLM, XVE link caps, usable-FB-size, access-counter buffer size |
| PTIMER (VF aliases `0xBB0080/84`) | 2 | free-running ns counter; **writes refused by name** |
| `NV_PBUS_BAR0_WINDOW` (`0x001700`) | 1 | the PRAMIN window latch; returns the guest's raw word verbatim |
| CPU interrupt tree (`0xB81000..0xB81640`) | 28 | 8 leaves, 8 enable-set, 8 enable-clear, top, top-en-set, top-en-clear, leaf-trigger |
| MMU invalidate (`0xB830A0/A4/B0`) | 3 | PDB lo/hi latch + trigger |
| usermode doorbell (`0xBB0090`) | 1 | write-only; rings a channel with no plane lock held |
| GSP plane | 27 | 9 PGSP falcon + 8 queue heads + 3 RISC-V + 3 SEC2 + 2 PGC6 + 2 PFB WPR2 |
| `NV_PROM_DATA` window | 1 MiB | synthesized VBIOS image |
| `NV_PRAMIN` window | 1 MiB | framebuffer through the BAR0 window latch |

### The hot offsets, identified

⊘ **`0xB830B0` is `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE`, not `NV_PFB_PRI_MMU_INVALIDATE`.**
That symbol is real but is `0x00100CBC`, a different register in a different unit; Turing and
later abandoned it for the VF aperture. This tree used the wrong name everywhere until w476.

| offset | name | unit | use | kayfabe |
|---|---|---|---|---|
| `0x110C00` | `NV_PGSP_QUEUE_HEAD(0)` | PGSP | **the GSP RPC submit doorbell** — post and return, the driver waits on the response queue | all 8 indices decoded; write services the command ring |
| `0x110094` | `NV_PFALCON_FALCON_DEBUGINFO` | PGSP | **CrashCat wayfinder level 0** — how RM discovers a GSP crash | ⊘ **not implemented**, reads 0 |
| `0xB830B0` | `..._PRIV_MMU_INVALIDATE` | VF PRIV | write fires a TLB invalidate, then the driver **spin-polls** the same register | write latches and arms a publication; read answers bit 31 while outstanding, lock-free |
| `0xB830A0/A4` | `..._MMU_INVALIDATE_PDB` / `_UPPER_PDB` | VF PRIV | PDB address low+aperture / high | write-latch only; no read arm |
| `0xB81208/0xB81210` | `..._CPU_INTR_LEAF_EN_SET(2)` / `(4)` | VF PRIV | leaf 2 = replayable-fault + access-counter vectors; leaf 4 = CPU doorbell, GSP, display | served |
| `0xB81408/0xB81410` | `..._CPU_INTR_LEAF_EN_CLEAR(2)` / `(4)` | VF PRIV | the disable side | served |
| `0xB81608/0xB81610` | `..._CPU_INTR_TOP_EN_SET(0)` / `_CLEAR(0)` | VF PRIV | GA10x-specific HAL | served |
| `0xBB0090` | `NV_VIRTUAL_FUNCTION_DOORBELL` | usermode | work-submit token write | served |

⚠ The `0x208/0x408/0x608` pattern is **not** a stride-`0x200` array of per-runlist blocks. It is
four different arrays inside the interrupt tree, and the last pair is a *different register
pair* from the first two. The symmetry is coincidental.

### Units absent entirely

PFIFO / runlist / CHRAM (zero registers — all submission goes through the one doorbell), MMU
fault buffers and access-counter buffers, PGRAPH, PDISP, NV_FUSE, PFSP, LTC/FBPA, the MSI-X
table, `PRIV_BAR1_BLOCK`/`BAR2_BLOCK`, `PRIV_DOORBELL`, `ERR_CONT`.

---

## §E — Channel classes and pushbuffer methods

### Classes, guest-facing (35 examined)

**SERVED 7** · **STUB 8** · **REFUSED 14** (permitted, no params arm) · **ABSENT 6** (not on
the allowlist).

Served: `NV01_ROOT/_CLIENT`, `NV01_DEVICE_0`, `FERMI_VASPACE_A`, `KEPLER_CHANNEL_GROUP_A`,
`FERMI_CONTEXT_SHARE_A`, `AMPERE_CHANNEL_GPFIFO_A` (`0xc56f`, the only channel class), and
`AMPERE_DMA_COPY_B` (`0xc7b5`, **the only class whose methods are both decoded and executed**).

⚠ `AMPERE_COMPUTE_B` (`0xc7c0`) is **stub at alloc, methods observed only**. Graphics is
passthrough by design, so `[measured, boot w270]` **97.7 % of a `cuCtxCreate` pushbuffer
decodes as opaque** — that is the design, not a defect, but it is the number to quote.

⚠ The `0xc7b5`/`0xc6b5` split is load-bearing and kayfabe has it right: `0xc6b5` is GA100,
`0xc7b5` is GA10x, and GA106 lists only the latter.

### Methods

| class | addresses | served | partial | refused | absent |
|---|---|---|---|---|---|
| `NVC56F` host/FIFO | 22 | 7 | 3 | 0 | 12 |
| `NVC7B5` copy engine | 43 | 14 | 2 | 0 | 27 |
| `NVC7C0` compute | 130 | **0 on the execution path** | — | 0 | ~110 |
| `NVC076` UVM SW | 7 | 2 | 0 | **5** (fault methods refuse the whole submission) | 0 |

`LAUNCH_DMA` field coverage: **7 of 17 fields read**. The two consequential omissions are
`SEMAPHORE_REDUCTION*` (a reduction decodes as a plain release writing the literal payload
instead of incrementing) and `DISABLE_PLC` (which UVM sets on *every* GA10x launch).

### The UVM path

★ **There is no non-pushbuffer TLB-invalidate path in UVM.** Every invalidate is `MEM_OP_A..D`;
the physical-invalidate HAL on Ampere is an unsupported assert stub. Served, and it drives a
real per-PDB refresh.

⚠ A **targeted** invalidate is over-approximated to the whole VA space — the target address and
invalidation size are decoded into a variable that is then discarded. Correctness-safe,
cost-unsafe.

⚠ UVM waits by **CPU polling, never interrupts**, and asserts that a payload never jumps more
than 2 Mi or exceeds the queued value. A payload written out of order **panics UVM**.

⚠ On GA106 the UVM ring and its producer cursor live in **framebuffer**, not sysmem.

---

## §F — The isolate forwarding surface

Three layers: **12 plans** (what a locked phase emits), **24 backend verbs** (the abstract RM
surface), **25 wire messages**. The single door to any host RM verb is `Worker::execute`, which
asserts lock-freedom and an off-trap witness.

| layer | total | production-live | orphaned or disarmed |
|---|---|---|---|
| plans | 12 | 9 | `Publish` (no caller), `PublishVidmem` (dead arm), `SubdeviceControl` (env-gated off) |
| backend verbs | 24 | 17 | `alloc`, `alloc_sysmem`, `alloc_vidmem`, `export_surface`, `export_backing`, `export_device_view`; plus `fb_read`, called in production against a host impl that always returns not-implemented |
| wire requests | 25 | 21 | 5 dead, of which `ExportDeviceView` has **zero senders including tests** |

### Controls: forwarded vs authored

Only **four** control ids are forwarded at all, and **three ship disabled**. The live one is
`NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`, forwarded because reading the record destroys it, so it
must be relayed one-ask-one-issue and never cached. Seven controls are **authored** — we build
our own host call and the guest never names it — of which three are live (`GPFIFO_SCHEDULE`,
`BIND`, `GET_WORK_SUBMIT_TOKEN`) and four are diagnostic-only.

### Guest values that reach a host ioctl

Structurally neutralised first: client handles never cross, object handles never cross (the
guest handle is a routing key; the host twin is sent), user pointers in controls are zeroed and
re-pointed, cross-isolate handle confusion is caught centrally, and ring/USERD addresses must
have been minted by that isolate's own join.

What does reach the host, in order of exposure:

1. **Engine-object alloc params, fully verbatim.** The class is allowlisted to 3 ids on GA10x;
   the struct contents are inspected by nothing. The largest such surface in the tree.
2. **CE operand addresses and lengths** from the guest's pushbuffer, including the `Untracked`
   case (gap #1 above).
3. **A guest-chosen semaphore release address and payload**, written by a real engine.
4. **The guest's raw engine-type number**, verbatim, selecting the host runlist.
5. **Guest VAs as fixed mapping parameters** — this is address identity and is the design; it
   is checked, and a mismatch fails the verb rather than silently placing elsewhere.
