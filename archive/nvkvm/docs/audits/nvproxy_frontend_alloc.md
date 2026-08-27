# nvproxy 575-ABI frontend-ioctl + RM_ALLOC class allowlists

Read-only audit. Source of truth: gVisor nvproxy at
`gvisor/pkg/sentry/devices/nvproxy/version.go` and the ABI constants at
`gvisor/pkg/abi/nvgpu/frontend.go` + `classes.go`. Target ABI is
**v575_51_02** (closest to our running 575.51.03).

## How the 575 ABI is built (replay the chain)

nvproxy builds each `driverABI` incrementally. The base map lives in the
`v535_104_05` builder (`version.go:164`). The chain to 575 is:

```
v535_104_05 (base, version.go:164)
  -> v535_113_01 = v535_104_05 (alias, :793)
  -> v545_23_06 (:805)      alloc-class param-struct swaps only; +NV_MEMORY_EXPORT
  -> v550_40_07 (:844)      +NV_ESC_WAIT_OPEN_COMPLETE (FE); +NV_IMEX_SESSION,
                            NV_MEMORY_FABRIC_IMPORTED_REF, NVENC_SW_SESSION,
                            NV_MEMORY_MAPPER (alloc)
  -> v550_54_14 (:896)      uvm only
  -> v550_90_07 (:912)      control only
  -> v555_42_02 (:930)      NV_MEMORY_MAPPER param swap; control delete
  -> v560_28_03 (:945)      +NVCDB0/NVCDD1/NVCDFA, BLACKWELL_CHANNEL_GPFIFO_A,
                            BLACKWELL_DMA_COPY_A, BLACKWELL_A, BLACKWELL_COMPUTE_A,
                            BLACKWELL_INLINE_TO_MEMORY_A (alloc)
  -> v565_57_01 (:978)      control only
  -> v570_86_15 (:990)      channel param swap (rmAllocChannelV570); +BLACKWELL_DMA_COPY_B,
                            BLACKWELL_CHANNEL_GPFIFO_B, BLACKWELL_B, BLACKWELL_COMPUTE_B,
                            BLACKWELL_USERMODE_A, NVCFB7_VIDEO_ENCODER (alloc)
  -> v570_124_06 = (alias, :1028)
  -> v570_133_20 = (alias, :1029)
  -> v575_51_02 (:1037)     CONTROL COMMANDS ONLY (DRAM-encryption v575 rename +
                            THERMAL_SYSTEM_EXECUTE_V2). No FE-ioctl or alloc-class change.
```

**Conclusion:** the 575 FE-ioctl set = base set + `NV_ESC_WAIT_OPEN_COMPLETE`.
The 575 alloc-class set = base set + every class added through `v570_86_15`.
575 itself adds nothing to either map (it only touches `controlCmd`). The
NVOS46/FERMI_VASPACE param-struct V580 swaps and `NVCEB7/NVD1B7` classes are
**580+** (`version.go:1057-1078`) and are NOT in 575.

---

## A) FRONTEND IOCTLS (default-deny allowlist)

These are IOC_NR values only. nvproxy ignores IOC_TYPE; the driver's canonical
type is `'F'` (`frontend.go:25`). `NV_IOCTL_BASE = 200 = 0xC8`
(`frontend.go:31`), so the symbolic `NV_IOCTL_BASE + N` entries resolve to the
hex below. Every entry is registered in `v535_104_05.frontendIoctl`
(`version.go:168-191`) except `NV_ESC_WAIT_OPEN_COMPLETE` (added at
`version.go:846`).

| NR hex | NR dec | NV_ESC name | meaning | handler (special vs passthrough) |
|--------|--------|-------------|---------|----------------------------------|
| 0xC8 | 200 | NV_ESC_CARD_INFO | query attached card info | `frontendIoctlBytes` (raw byte copy; v.go:169) |
| 0xC9 | 201 | NV_ESC_REGISTER_FD | bind a /dev/nvidia* fd to ctl fd | **special** `frontendRegisterFD` (fd translate; :179) |
| 0xCE | 206 | NV_ESC_ALLOC_OS_EVENT | create OS event object on an fd | **special** `frontendIoctlHasFD` (embedded fd; :180) |
| 0xCF | 207 | NV_ESC_FREE_OS_EVENT | free OS event object | **special** `frontendIoctlHasFD` (embedded fd; :181) |
| 0xD2 | 210 | NV_ESC_CHECK_VERSION_STR | RM API version handshake | passthrough `frontendIoctlSimpleNoStatus` (:170) |
| 0xD4 | 212 | NV_ESC_ATTACH_GPUS_TO_FD | attach GPU-ID array to fd | `frontendIoctlBytes` (NvU32 array; :171) |
| 0xD6 | 214 | NV_ESC_SYS_PARAMS | set system params (large page size) | passthrough `frontendIoctlSimpleNoStatus` (:172) |
| 0xD7 | 215 | NV_ESC_NUMA_INFO | NUMA topology query | **special** `rmNumaInfo` (nvproxy mostly ignores; :182) |
| 0xDA | 218 | NV_ESC_WAIT_OPEN_COMPLETE | block until device open finishes | passthrough `frontendIoctlSimple` (added v550; :846) |
| 0x27 | 39 | NV_ESC_RM_ALLOC_MEMORY | alloc + map sysmem (carries fd) | **special** `rmAllocMemory` (NVOS02-with-fd; :184) |
| 0x29 | 41 | NV_ESC_RM_FREE | free an RM object handle | **special** `rmFree` (handle untrack; :185) |
| 0x2A | 42 | NV_ESC_RM_CONTROL | RM control (sub-cmd multiplexed) | **special** `rmControl` (per-cmd allowlist + ptr xlate; :186) |
| 0x2B | 43 | NV_ESC_RM_ALLOC | alloc RM object of an hClass | **special** `rmAlloc` (per-class dispatch; :187) |
| 0x34 | 52 | NV_ESC_RM_DUP_OBJECT | dup object handle across clients | **special** `rmDupObject` (handle xlate; :173) |
| 0x35 | 53 | NV_ESC_RM_SHARE | adjust object share policy | passthrough `frontendIoctlSimple[NVOS57]` (:174) |
| 0x41 | 65 | NV_ESC_RM_IDLE_CHANNELS | idle channels on a device | **special** `rmIdleChannels` (CapGraphics; :188) |
| 0x4A | 74 | NV_ESC_RM_VID_HEAP_CONTROL | vidmem heap alloc/free (NVOS32) | **special** `rmVidHeapControl` (:189) |
| 0x4E | 78 | NV_ESC_RM_MAP_MEMORY | map memory to userspace (carries fd) | **special** `rmMapMemory` (NVOS33-with-fd; :190) |
| 0x4F | 79 | NV_ESC_RM_UNMAP_MEMORY | unmap previously-mapped memory | passthrough `frontendIoctlSimple[NVOS34]` (:175) |
| 0x54 | 84 | NV_ESC_RM_ALLOC_CONTEXT_DMA2 | alloc context DMA (NVOS39) | **special** `rmAllocContextDMA2` (CapGraphics; :183) |
| 0x57 | 87 | NV_ESC_RM_MAP_MEMORY_DMA | map memory into a GPU VA (NVOS46) | passthrough `frontendIoctlSimple[NVOS46]` (:176) |
| 0x58 | 88 | NV_ESC_RM_UNMAP_MEMORY_DMA | unmap a GPU VA (NVOS47) | passthrough `frontendIoctlSimple[NVOS47_V550]` (:177) |
| 0x5E | 94 | NV_ESC_RM_UPDATE_DEVICE_MAPPING_INFO | refresh device mapping (NVOS56) | passthrough `frontendIoctlSimple[NVOS56]` (:178) |

**Copy-pasteable C array (allowed frontend IOC_NRs, gate on `_IOC_TYPE=='F'`):**

```c
/* nvproxy v575_51_02 frontend-ioctl allowlist (IOC_NR only). */
static const uint8_t nvkvm_fe_nr_allowlist[] = {
    0x27, /* NV_ESC_RM_ALLOC_MEMORY */
    0x29, /* NV_ESC_RM_FREE */
    0x2a, /* NV_ESC_RM_CONTROL */
    0x2b, /* NV_ESC_RM_ALLOC */
    0x34, /* NV_ESC_RM_DUP_OBJECT */
    0x35, /* NV_ESC_RM_SHARE */
    0x41, /* NV_ESC_RM_IDLE_CHANNELS */
    0x4a, /* NV_ESC_RM_VID_HEAP_CONTROL */
    0x4e, /* NV_ESC_RM_MAP_MEMORY */
    0x4f, /* NV_ESC_RM_UNMAP_MEMORY */
    0x54, /* NV_ESC_RM_ALLOC_CONTEXT_DMA2 */
    0x57, /* NV_ESC_RM_MAP_MEMORY_DMA */
    0x58, /* NV_ESC_RM_UNMAP_MEMORY_DMA */
    0x5e, /* NV_ESC_RM_UPDATE_DEVICE_MAPPING_INFO */
    0xc8, /* NV_ESC_CARD_INFO          (200) */
    0xc9, /* NV_ESC_REGISTER_FD        (201) */
    0xce, /* NV_ESC_ALLOC_OS_EVENT     (206) */
    0xcf, /* NV_ESC_FREE_OS_EVENT      (207) */
    0xd2, /* NV_ESC_CHECK_VERSION_STR  (210) */
    0xd4, /* NV_ESC_ATTACH_GPUS_TO_FD  (212) */
    0xd6, /* NV_ESC_SYS_PARAMS         (214) */
    0xd7, /* NV_ESC_NUMA_INFO          (215) */
    0xda, /* NV_ESC_WAIT_OPEN_COMPLETE (218) */
};
```

### Frontend IOC_NRs nvproxy DEFAULT-DENIES (present in headers, NOT registered)
- `0x70` **NV_ESC_EXPORT_TO_DMABUF_FD** — nvproxy never registers it (only
  appears in our `nvgpu.h:732`, not in nvproxy's `frontend.go`). DENY.
- `NV_IOCTL_BASE+N` values with no entry above (e.g. +2..+5, +8, +9, +11, +13,
  +16, +17) — DENY.

---

## B) RM_ALLOC CLASSES (`hClass` allowlist for NV_ESC_RM_ALLOC = 0x2b)

Every class nvproxy permits for the 575 ABI, keyed by `nvgpu.ClassID`. Hex from
`gvisor/pkg/abi/nvgpu/classes.go`. Registration line is in `version.go` (base
map `:406-474`, plus the chain additions noted). "param handler" = whether the
class has a dedicated alloc-param handler (anything other than plain
`rmAllocSimple[...]`/`rmAllocNoParams` is **special**: it does pointer/fd/handle
translation).

| hClass | class name | meaning | param handler |
|--------|-----------|---------|---------------|
| 0x00000000 | NV01_ROOT | root client handle | **special** `rmAllocRootClient` (:406) |
| 0x00000001 | NV01_ROOT_NON_PRIV | root client (non-priv) | **special** `rmAllocRootClient` (:407) |
| 0x00000002 | NV01_CONTEXT_DMA | context DMA object | **special** `rmAllocContextDMA` (CapVideo; :408) |
| 0x0000003e | NV01_MEMORY_SYSTEM | sysmem allocation | `rmAllocSimple[NV_MEMORY_ALLOCATION_PARAMS_V545]` (:813) |
| 0x00000040 | NV01_MEMORY_LOCAL_USER | vidmem allocation | `rmAllocSimple[..._V545]` (:814) |
| 0x00000041 | NV01_ROOT_CLIENT | root client handle | **special** `rmAllocRootClient` (:411) |
| 0x00000070 | NV01_MEMORY_VIRTUAL | virtual memory range | **special** `rmAllocMemoryVirtual` (:413) |
| 0x00000079 | NV01_EVENT_OS_EVENT | OS event (carries fd) | **special** `rmAllocEventOSEvent` (fd xlate; :412) |
| 0x00000080 | NV01_DEVICE_0 | device object | `rmAllocSimple[NV0080_ALLOC_PARAMETERS]` (:414) |
| 0x000000da | NV_SEMAPHORE_SURFACE | semaphore surface | `rmAllocSimple[NV_SEMAPHORE_SURFACE_ALLOC_PARAMETERS]` (CapGraphics; :415) |
| 0x000000de | RM_USER_SHARED_DATA | shared RUSD page | `rmAllocSimple[NV00DE_ALLOC_PARAMETERS_V545]` (:810) |
| 0x000000e0 | NV_MEMORY_EXPORT | fabric memory export | `rmAllocSimple[NV00E0_ALLOCATION_PARAMETERS]` (CapFabric; :811) |
| 0x000000f1 | NV_IMEX_SESSION | IMEX fabric session | **special** `rmAllocIMEXSession` (CapFabric; :862) |
| 0x000000f8 | NV_MEMORY_FABRIC | fabric memory | `rmAllocSimple[NV00F8_ALLOCATION_PARAMETERS]` (:417) |
| 0x000000fb | NV_MEMORY_FABRIC_IMPORTED_REF | imported fabric ref | `rmAllocSimple[NV00FB_ALLOCATION_PARAMETERS]` (CapFabric; :863) |
| 0x000000fd | NV_MEMORY_MULTICAST_FABRIC | multicast fabric mem | `rmAllocSimple[NV00FD_ALLOCATION_PARAMETERS_V545]` (:812) |
| 0x000000fe | NV_MEMORY_MAPPER | memory mapper | `rmAllocSimple[NV_MEMORY_MAPPER_ALLOCATION_PARAMS_V555]` (CapVideo; :932) |
| 0x00002080 | NV20_SUBDEVICE_0 | subdevice object | `rmAllocSimple[NV2080_ALLOC_PARAMETERS]` (:420) |
| 0x00002081 | NV2081_BINAPI | binary-API subdevice | `rmAllocSimple[NV2081_ALLOC_PARAMETERS]` (:421) |
| 0x0000208f | NV20_SUBDEVICE_DIAG | subdevice diag | `rmAllocNoParams` (:474) |
| 0x0000503b | NV50_P2P | peer-to-peer object | `rmAllocSimple[NV503B_ALLOC_PARAMETERS]` (:423) |
| 0x0000503c | NV50_THIRD_PARTY_P2P | third-party P2P | `rmAllocSimple[NV503C_ALLOC_PARAMETERS]` (:424) |
| 0x000050a0 | NV50_MEMORY_VIRTUAL | virtual memory | `rmAllocSimple[..._V545]` (:815) |
| 0x00000073 | NV04_DISPLAY_COMMON | display common | `rmAllocNoParams` (CapGraphics; :473) |
| 0x000083de | GT200_DEBUGGER | SM debugger session | **special** `rmAllocSMDebuggerSession` (:427) |
| 0x0000902d | FERMI_TWOD_A | 2D engine class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :428) |
| 0x00009067 | FERMI_CONTEXT_SHARE_A | context share | **special** `rmAllocContextShare` (:429) |
| 0x00009072 | GF100_DISP_SW | display sw class | `rmAllocSimple[NV9072_ALLOCATION_PARAMETERS]` (CapGraphics; :430) |
| 0x00009096 | GF100_ZBC_CLEAR | ZBC clear | `rmAllocNoParams` (CapGraphics; :431) |
| 0x000090cc | GF100_PROFILER | profiler | `rmAllocNoParams` (CapProfiling; :425) |
| 0x000090e6 | GF100_SUBDEVICE_MASTER | subdevice master | `rmAllocNoParams` (:470) |
| 0x000090e7 | GF100_SUBDEVICE_INFOROM | inforom subdevice | `rmAllocNoParams` (CapGraphics; :432) |
| 0x000090f1 | FERMI_VASPACE_A | GPU VA space | `rmAllocSimple[NV_VASPACE_ALLOCATION_PARAMETERS]` (:433) |
| 0x0000a06c | KEPLER_CHANNEL_GROUP_A | channel group (TSG) | **special** `rmAllocChannelGroup` (:434) |
| 0x0000a0bc | NVENC_SW_SESSION | NVENC sw session | `rmAllocSimple[NVA0BC_ALLOC_PARAMETERS]` (CapVideo; :864) |
| 0x0000a140 | KEPLER_INLINE_TO_MEMORY_B | inline-to-memory | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :435) |
| 0x0000b2cc | MAXWELL_PROFILER_DEVICE | profiler device | `rmAllocSimple[NVB2CC_ALLOC_PARAMETERS]` (CapProfiling; :426) |
| 0x0000b8b0 | NVB8B0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :438) |
| 0x0000b8d1 | NVB8D1_VIDEO_NVJPG | NVJPG | `rmAllocSimple[NV_NVJPG_ALLOCATION_PARAMETERS]` (CapVideo; :446) |
| 0x0000b8fa | NVB8FA_VIDEO_OFA | OFA | `rmAllocSimple[NV_OFA_ALLOCATION_PARAMETERS_V545]` (CapVideo; :816) |
| 0x0000c361 | VOLTA_USERMODE_A | usermode submission | `rmAllocNoParams` (CapGraphics\|CapVideo; :436) |
| 0x0000c461 | TURING_USERMODE_A | usermode submission | `rmAllocNoParams` (:471) |
| 0x0000c46f | TURING_CHANNEL_GPFIFO_A | GPFIFO channel | **special** `rmAllocChannelV570` (:996) |
| 0x0000c4b0 | NVC4B0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :439) |
| 0x0000c4b7 | NVC4B7_VIDEO_ENCODER | video encoder | `rmAllocSimple[NV_MSENC_ALLOCATION_PARAMETERS]` (CapVideo; :443) |
| 0x0000c4d1 | NVC4D1_VIDEO_NVJPG | NVJPG | `rmAllocSimple[NV_NVJPG_ALLOCATION_PARAMETERS]` (CapVideo; :447) |
| 0x0000c56f | AMPERE_CHANNEL_GPFIFO_A | GPFIFO channel | **special** `rmAllocChannelV570` (:997) |
| 0x0000c597 | TURING_A | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :455) |
| 0x0000c5b5 | TURING_DMA_COPY_A | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:459) |
| 0x0000c5c0 | TURING_COMPUTE_A | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:463) |
| 0x0000c661 | HOPPER_USERMODE_A | usermode submission | `rmAllocSimple[NV_HOPPER_USERMODE_A_PARAMS]` (:469) |
| 0x0000c697 | AMPERE_A | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :456) |
| 0x0000c6b0 | NVC6B0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :440) |
| 0x0000c6b5 | AMPERE_DMA_COPY_A | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:460) |
| 0x0000c6c0 | AMPERE_COMPUTE_A | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:464) |
| 0x0000c6fa | NVC6FA_VIDEO_OFA | OFA | `rmAllocSimple[NV_OFA_ALLOCATION_PARAMETERS_V545]` (CapVideo; :817) |
| 0x0000c761 | BLACKWELL_USERMODE_A | usermode submission | `rmAllocSimple[NV_HOPPER_USERMODE_A_PARAMS]` (:1004) |
| 0x0000c7b0 | NVC7B0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :441) |
| 0x0000c7b5 | AMPERE_DMA_COPY_B | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:461) |
| 0x0000c7b7 | NVC7B7_VIDEO_ENCODER | video encoder | `rmAllocSimple[NV_MSENC_ALLOCATION_PARAMETERS]` (CapVideo; :444) |
| 0x0000c7c0 | AMPERE_COMPUTE_B | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:465) |
| 0x0000c7fa | NVC7FA_VIDEO_OFA | OFA | `rmAllocSimple[NV_OFA_ALLOCATION_PARAMETERS_V545]` (CapVideo; :818) |
| 0x0000c86f | HOPPER_CHANNEL_GPFIFO_A | GPFIFO channel | **special** `rmAllocChannelV570` (:998) |
| 0x0000c8b5 | HOPPER_DMA_COPY_A | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:462) |
| 0x0000c96f | BLACKWELL_CHANNEL_GPFIFO_A | GPFIFO channel | **special** `rmAllocChannelV570` (:1000) |
| 0x0000c997 | ADA_A | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :457) |
| 0x0000c9b0 | NVC9B0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :442) |
| 0x0000c9b5 | BLACKWELL_DMA_COPY_A | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:951) |
| 0x0000c9b7 | NVC9B7_VIDEO_ENCODER | video encoder | `rmAllocSimple[NV_MSENC_ALLOCATION_PARAMETERS]` (CapVideo; :445) |
| 0x0000c9c0 | ADA_COMPUTE_A | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:466) |
| 0x0000c9d1 | NVC9D1_VIDEO_NVJPG | NVJPG | `rmAllocSimple[NV_NVJPG_ALLOCATION_PARAMETERS]` (CapVideo; :448) |
| 0x0000c9fa | NVC9FA_VIDEO_OFA | OFA | `rmAllocSimple[NV_OFA_ALLOCATION_PARAMETERS_V545]` (CapVideo; :819) |
| 0x0000ca6f | BLACKWELL_CHANNEL_GPFIFO_B | GPFIFO channel | **special** `rmAllocChannelV570` (:1001) |
| 0x0000cab5 | BLACKWELL_DMA_COPY_B | copy engine | `rmAllocSimple[NVB0B5_ALLOCATION_PARAMETERS]` (:999) |
| 0x0000cb33 | NV_CONFIDENTIAL_COMPUTE | CC object | `rmAllocSimple[NV_CONFIDENTIAL_COMPUTE_ALLOC_PARAMS]` (:467) |
| 0x0000cb97 | HOPPER_A | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :458) |
| 0x0000cba2 | HOPPER_SEC2_WORK_LAUNCH_A | SEC2 work launch | `rmAllocNoParams` (:472) |
| 0x0000cbc0 | HOPPER_COMPUTE_A | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:468) |
| 0x0000cd40 | BLACKWELL_INLINE_TO_MEMORY_A | inline-to-memory | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :954) |
| 0x0000cd97 | BLACKWELL_A | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :952) |
| 0x0000cdb0 | NVCDB0_VIDEO_DECODER | video decoder | `rmAllocSimple[NV_BSP_ALLOCATION_PARAMETERS]` (CapVideo; :947) |
| 0x0000cdc0 | BLACKWELL_COMPUTE_A | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:953) |
| 0x0000cdd1 | NVCDD1_VIDEO_NVJPG | NVJPG | `rmAllocSimple[NV_NVJPG_ALLOCATION_PARAMETERS]` (CapVideo; :948) |
| 0x0000cdfa | NVCDFA_VIDEO_OFA | OFA | `rmAllocSimple[NV_OFA_ALLOCATION_PARAMETERS_V545]` (CapVideo; :949) |
| 0x0000ce97 | BLACKWELL_B | graphics class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (CapGraphics; :1002) |
| 0x0000cec0 | BLACKWELL_COMPUTE_B | compute class | `rmAllocSimple[NV_GR_ALLOCATION_PARAMETERS]` (:1003) |
| 0x0000cfb7 | NVCFB7_VIDEO_ENCODER | video encoder | `rmAllocSimple[NV_MSENC_ALLOCATION_PARAMETERS]` (CapVideo; :1005) |

**Copy-pasteable C array (allowed hClass for NV_ESC_RM_ALLOC, v575_51_02):**

```c
/* nvproxy v575_51_02 RM_ALLOC hClass allowlist. */
static const uint32_t nvkvm_alloc_class_allowlist[] = {
    0x00000000u, /* NV01_ROOT */
    0x00000001u, /* NV01_ROOT_NON_PRIV */
    0x00000002u, /* NV01_CONTEXT_DMA */
    0x0000003eu, /* NV01_MEMORY_SYSTEM */
    0x00000040u, /* NV01_MEMORY_LOCAL_USER */
    0x00000041u, /* NV01_ROOT_CLIENT */
    0x00000070u, /* NV01_MEMORY_VIRTUAL */
    0x00000073u, /* NV04_DISPLAY_COMMON */
    0x00000079u, /* NV01_EVENT_OS_EVENT */
    0x00000080u, /* NV01_DEVICE_0 */
    0x000000dau, /* NV_SEMAPHORE_SURFACE */
    0x000000deu, /* RM_USER_SHARED_DATA */
    0x000000e0u, /* NV_MEMORY_EXPORT */
    0x000000f1u, /* NV_IMEX_SESSION */
    0x000000f8u, /* NV_MEMORY_FABRIC */
    0x000000fbu, /* NV_MEMORY_FABRIC_IMPORTED_REF */
    0x000000fdu, /* NV_MEMORY_MULTICAST_FABRIC */
    0x000000feu, /* NV_MEMORY_MAPPER */
    0x00002080u, /* NV20_SUBDEVICE_0 */
    0x00002081u, /* NV2081_BINAPI */
    0x0000208fu, /* NV20_SUBDEVICE_DIAG */
    0x0000503bu, /* NV50_P2P */
    0x0000503cu, /* NV50_THIRD_PARTY_P2P */
    0x000050a0u, /* NV50_MEMORY_VIRTUAL */
    0x000083deu, /* GT200_DEBUGGER */
    0x0000902du, /* FERMI_TWOD_A */
    0x00009067u, /* FERMI_CONTEXT_SHARE_A */
    0x00009072u, /* GF100_DISP_SW */
    0x00009096u, /* GF100_ZBC_CLEAR */
    0x000090ccu, /* GF100_PROFILER */
    0x000090e6u, /* GF100_SUBDEVICE_MASTER */
    0x000090e7u, /* GF100_SUBDEVICE_INFOROM */
    0x000090f1u, /* FERMI_VASPACE_A */
    0x0000a06cu, /* KEPLER_CHANNEL_GROUP_A */
    0x0000a0bcu, /* NVENC_SW_SESSION */
    0x0000a140u, /* KEPLER_INLINE_TO_MEMORY_B */
    0x0000b2ccu, /* MAXWELL_PROFILER_DEVICE */
    0x0000b8b0u, /* NVB8B0_VIDEO_DECODER */
    0x0000b8d1u, /* NVB8D1_VIDEO_NVJPG */
    0x0000b8fau, /* NVB8FA_VIDEO_OFA */
    0x0000c361u, /* VOLTA_USERMODE_A */
    0x0000c461u, /* TURING_USERMODE_A */
    0x0000c46fu, /* TURING_CHANNEL_GPFIFO_A */
    0x0000c4b0u, /* NVC4B0_VIDEO_DECODER */
    0x0000c4b7u, /* NVC4B7_VIDEO_ENCODER */
    0x0000c4d1u, /* NVC4D1_VIDEO_NVJPG */
    0x0000c56fu, /* AMPERE_CHANNEL_GPFIFO_A */
    0x0000c597u, /* TURING_A */
    0x0000c5b5u, /* TURING_DMA_COPY_A */
    0x0000c5c0u, /* TURING_COMPUTE_A */
    0x0000c661u, /* HOPPER_USERMODE_A */
    0x0000c697u, /* AMPERE_A */
    0x0000c6b0u, /* NVC6B0_VIDEO_DECODER */
    0x0000c6b5u, /* AMPERE_DMA_COPY_A */
    0x0000c6c0u, /* AMPERE_COMPUTE_A */
    0x0000c6fau, /* NVC6FA_VIDEO_OFA */
    0x0000c761u, /* BLACKWELL_USERMODE_A */
    0x0000c7b0u, /* NVC7B0_VIDEO_DECODER */
    0x0000c7b5u, /* AMPERE_DMA_COPY_B */
    0x0000c7b7u, /* NVC7B7_VIDEO_ENCODER */
    0x0000c7c0u, /* AMPERE_COMPUTE_B */
    0x0000c7fau, /* NVC7FA_VIDEO_OFA */
    0x0000c86fu, /* HOPPER_CHANNEL_GPFIFO_A */
    0x0000c8b5u, /* HOPPER_DMA_COPY_A */
    0x0000c96fu, /* BLACKWELL_CHANNEL_GPFIFO_A */
    0x0000c997u, /* ADA_A */
    0x0000c9b0u, /* NVC9B0_VIDEO_DECODER */
    0x0000c9b5u, /* BLACKWELL_DMA_COPY_A */
    0x0000c9b7u, /* NVC9B7_VIDEO_ENCODER */
    0x0000c9c0u, /* ADA_COMPUTE_A */
    0x0000c9d1u, /* NVC9D1_VIDEO_NVJPG */
    0x0000c9fau, /* NVC9FA_VIDEO_OFA */
    0x0000ca6fu, /* BLACKWELL_CHANNEL_GPFIFO_B */
    0x0000cab5u, /* BLACKWELL_DMA_COPY_B */
    0x0000cb33u, /* NV_CONFIDENTIAL_COMPUTE */
    0x0000cb97u, /* HOPPER_A */
    0x0000cba2u, /* HOPPER_SEC2_WORK_LAUNCH_A */
    0x0000cbc0u, /* HOPPER_COMPUTE_A */
    0x0000cd40u, /* BLACKWELL_INLINE_TO_MEMORY_A */
    0x0000cd97u, /* BLACKWELL_A */
    0x0000cdb0u, /* NVCDB0_VIDEO_DECODER */
    0x0000cdc0u, /* BLACKWELL_COMPUTE_A */
    0x0000cdd1u, /* NVCDD1_VIDEO_NVJPG */
    0x0000cdfau, /* NVCDFA_VIDEO_OFA */
    0x0000ce97u, /* BLACKWELL_B */
    0x0000cec0u, /* BLACKWELL_COMPUTE_B */
    0x0000cfb7u, /* NVCFB7_VIDEO_ENCODER */
};
```

### Classes nvproxy DEFAULT-DENIES in 575 (declared in classes.go but NOT registered)
- `0x00000005` **NV01_EVENT** (only the OS-event variant 0x79 is allowed)
- `0x0000003f` **NV01_MEMORY_LOCAL_PRIVILEGED**
- `0x00000042` **NV_MEMORY_EXTENDED_USER**
- `0x00000071` **NV01_MEMORY_SYSTEM_OS_DESCRIPTOR**
- `0x0000ceb7` **NVCEB7_VIDEO_ENCODER** — 580+ only (`version.go:1061`)
- `0x0000d1b7` **NVD1B7_VIDEO_ENCODER** — 580+ only (`version.go:1062`)
- `0xffffffff` **NV_MEMORY_VIRTUAL_SYSMEM_DYNAMIC_HVASPACE** (sentinel)
- Any class not in the array above — DENY.

---

## Cross-check vs nvkvm (present / missing)

### Where the gates live
- **QEMU `src/qemu/nvkvm_isolate_handlers.c`** is the host/cross-VM trust
  boundary. Today it enforces a **control-command** default-deny allowlist
  (`nvkvm_ctrl_cmd_allowed`, `:502`, backed by `nvkvm_ctrl_allowlist.h`) and a
  per-VM **hClient** allowlist (`:479-524`). It has **NO frontend-IOC_NR
  allowlist and NO RM_ALLOC hClass allowlist** — there is only a TEMP empirical
  capture (`fprintf "FE_NR …"` / `"ALLOC hClass …"`, `:812-823`) explicitly left
  in to seed exactly these two tables. **This is the gap this spec fills.**
- **Guest module `src/guest/nvkvm_main.c`** is untrusted; its
  `NV_ESC_RM_ALLOC` `switch (alloc->h_class)` blocks (`:1039` nvos21, `:1124`
  nvos64) are a **param-sizing** table, not a security allowlist. An unknown
  class there just yields `ap_size==0` (no aux copy) and is still forwarded.
- **ABI header `src/abi/nvgpu.h`** declares all 23 NV_ESC numbers (`:708-732`).

### A) Frontend ioctls — present vs missing on each side
nvkvm `nvgpu.h` defines all 22 of nvproxy's 575 FE-ioctls (matching hex), PLUS
`NV_ESC_EXPORT_TO_DMABUF_FD` (0x70) which nvproxy default-denies — nvkvm should
NOT add 0x70 to its allowlist. The forwarder forwards by NR with no FE-NR
allowlist, so every NR (including ones nvproxy denies) currently passes the FE
gate. **Action: add `nvkvm_fe_nr_allowlist[]` (above), gated on
`_IOC_TYPE=='F'`, default-deny.**

### B) Alloc classes — present vs missing on each side
The guest sizing `switch` handles these classes (subset, for param sizing only):
`NV01_DEVICE_0, NV20_SUBDEVICE_0, RM_USER_SHARED_DATA, FERMI_VASPACE_A,
NV50_MEMORY_VIRTUAL, NV01_MEMORY_LOCAL_USER, NV01_MEMORY_SYSTEM,
KEPLER_CHANNEL_GROUP_A, FERMI_CONTEXT_SHARE_A, TURING/AMPERE/HOPPER_CHANNEL_GPFIFO_A,
NV01_EVENT_OS_EVENT, VOLTA/TURING/AMPERE_A/AMPERE_B/HOPPER/BLACKWELL DMA_COPY,
GT200_DEBUGGER, and the VOLTA/TURING/AMPERE/ADA/HOPPER/BLACKWELL
COMPUTE+graphics families`.

- Classes nvproxy allows that nvkvm's guest table does NOT size (but still
  forwards): `NV01_ROOT*` family, `NV01_CONTEXT_DMA`, `NV01_MEMORY_VIRTUAL`,
  `NV_SEMAPHORE_SURFACE`, `NV_MEMORY_EXPORT/FABRIC/FABRIC_IMPORTED_REF/
  MULTICAST_FABRIC/MAPPER`, `NV_IMEX_SESSION`, `NV2081_BINAPI`,
  `NV20_SUBDEVICE_DIAG`, `NV50_P2P/THIRD_PARTY_P2P`, `NV04_DISPLAY_COMMON`,
  `FERMI_TWOD_A`, `GF100_DISP_SW/ZBC_CLEAR/PROFILER/SUBDEVICE_MASTER/INFOROM`,
  `KEPLER_INLINE_TO_MEMORY_B`, `MAXWELL_PROFILER_DEVICE`, `NVENC_SW_SESSION`,
  all `*_VIDEO_DECODER/ENCODER/NVJPG/OFA`, `*_USERMODE_A`,
  `HOPPER_SEC2_WORK_LAUNCH_A`, `NV_CONFIDENTIAL_COMPUTE`. These are fine to
  forward (no params or fixed-size) but should still be **explicitly allowed**
  by the new hClass allowlist so unknown classes are denied.
- nvkvm guest references some symbols nvproxy keys differently —
  `VOLTA_DMA_COPY_A`, `VOLTA_A/B`, `VOLTA_COMPUTE_A/B`, `AMPERE_B` — confirm
  these resolve to in-allowlist hex before relying on them; nvproxy's 575 map
  has no `VOLTA_DMA_COPY_A`/`VOLTA_A`/`AMPERE_B` graphics entry (Volta graphics
  predates the proxy's compute-only surface), so those guest cases are dead for
  the proxied workload.
- **Action: add `nvkvm_alloc_class_allowlist[]` (above) to QEMU, gated on
  `_IOC_TYPE=='F' && _IOC_NR==0x2b`, read hClass at param offset 12 (the
  existing code already does `memcpy(&hClass, param_buf+12, 4)` at `:798/:903`),
  default-deny.** Combine with the existing per-VM hClient gate.

### Net
QEMU currently has the control-cmd + hClient allowlists but is **missing both
the frontend-IOC_NR allowlist and the RM_ALLOC hClass allowlist**. Both arrays
above are ready to drop into a new `nvkvm_frontend_alloc_allowlist.h` alongside
`nvkvm_ctrl_allowlist.h`, completing nvproxy parity for the default-deny posture.
