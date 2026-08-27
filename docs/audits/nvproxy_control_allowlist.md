# nvproxy RM control-command allowlist (reference for nvkvm task #76)

Read-only extraction of the **exact** set of `NVxxxx_CTRL_CMD_*` control commands
that gVisor's nvproxy registers/allows, with numeric hex, handler type
(passthrough vs special), and capability tag. Source tree:
`/workspace/nvidia-gpu-passthrough/gvisor/pkg/sentry/devices/nvproxy/`.

This is the **effective set for driver ABI `v575_51_02`** (the closest nvproxy ABI
to our target 575.51.03), computed by replaying the version chain
`535_104_05 → 545_23_06 → 550_40_07 → 555_42_02 → 560 → 565 → 570_86_15 →
570_124/133 → 575_51_02`. 200 control commands total.

## How nvproxy dispatches control ioctls (default-deny)

`frontend.go:rmControl()` (lines 756-816) is the entry for `NV_ESC_RM_CONTROL`:

1. **Two forced-passthrough bypass rules** (checked *before* the allowlist map):
   - `cmd & RM_GSS_LEGACY_MASK != 0` → `rmControlSimple` (legacy GSS/GSP control;
     params can't contain app pointers). `RM_GSS_LEGACY_MASK = 0x00008000`
     (`pkg/abi/nvgpu/ctrl.go:23`).
   - `(cmd >> 16) & 0xffff == NV2081_BINAPI (0x2081)` → `rmControlSimple`
     (binary-API class forwards everything to GSP). (`classes.go:69`)
2. Otherwise look up `abi.controlCmd[cmd]` (`frontend.go:804`). The map is the
   allowlist defined in `version.go:225-404` plus per-version deltas.
3. **Default-deny:** if the cmd is absent (nil handler) it returns
   `errUndefinedHandler` and writes `Status = NV_ERR_NOT_SUPPORTED` back to the
   caller (`frontend.go:805-813`). Missing required capability →
   `errMissingCapability` (`handlers.go:88-95`).

The registration table type is `controlCmdHandler{handler, capSet}`
(`handlers.go:68-96`); each entry = `ctrlHandler(<handlerFn>, <capSet>)`.

## Capability tags (`nvconf/caps.go`, `seccomp_filters.go:31`)

| Tag in this doc | nvproxy expression | meaning |
|---|---|---|
| compUtil | `nvconf.CapCompute \| nvconf.CapUtility` | baseline CUDA-compute (default-enabled set) |
| graphics | `nvconf.CapGraphics` | graphics/GL |
| video | `nvconf.CapVideo` | NVENC/NVDEC/NVJPG/OFA |
| profiling | `nvconf.CapProfiling` | HW perf counters (Nsight); **privileged** |
| fabric | `nvconf.CapFabricIMEXManagement` | NVLink/fabric IMEX mgmt; **privileged** |

A handler runs only if `capSet & capsEnabled != 0` (any-of). Tags like
`compUtil|graphics` mean either set unlocks it.

## Handler types (what "special" does)

| handler | file:func | what it does to the params |
|---|---|---|
| `rmControlSimple` | frontend.go:818 | **passthrough**: copy `ParamsSize` bytes in, invoke, copy out. Enforces `ParamsSize <= RMAPI_PARAM_COPY_MAX_PARAMS_SIZE (1 MiB)` and null-ptr/0-size consistency. |
| `ctrlHasFrontendFD[T]` | frontend.go:846 | **FD translation**: rewrites an embedded guest fd field to the host `/dev/nvidiactl` fd, invoke, restore. Used by OS_UNIX export/import-object cmds. |
| `ctrlMemoryMulticastFabricAttachGPU` | frontend.go:879 | **FD translation** of `DevDescriptor` (input-only, no copy-out). |
| `ctrlIoctlHasInfoList[T]` | frontend_unsafe.go:84 | **embedded-array translation + bounds check**: copies in the `NVXXXX_CTRL_XXX_INFO` list, bounds-checks count via `rmapiParamsSizeCheck`, swaps pointer to host buffer, invoke, copy out. |
| `ctrlGetNvU32List` | frontend.go:969 | **embedded NvU32[] translation + bounds check** (`NumElems * 4`). |
| `ctrlDevGetCaps` | frontend.go:990 | **embedded caps-table translation**: bounds-checks `CapsTblSize`, allocates host buffer (skip-copyin/zero), invoke, copy out. |
| `ctrlGpuExecRegOps` | frontend_unsafe.go:151 | **embedded reg-op array translation + bounds check** (`RegOpCount * sizeof(NV2080_CTRL_GPU_REG_OP)`). profiling-gated. |
| `ctrlDevFIFOGetChannelList` | frontend.go:208(unsafe) | **embedded channel-handle/id array translation**. |
| `ctrlSubdevFIFODisableChannels` | frontend.go:1036 | **null-pointer assertion**: requires `PRunlistPreemptEvent == 0` (rejects non-null), then passthrough. |
| `ctrlGpuGetIDInfo` | frontend.go:1061 | **pointer scrub**: forces `SzName = 0` (driver ignores it) then passthrough. |
| `ctrlClientSystemGetBuildVersion` | frontend.go:927 | **multi-string buffer translation** of driver/version/title bufs. |
| `ctrlClientSystemGetP2PCaps` / `...V550` | frontend_unsafe.go:275/305 | **embedded p2p caps array translation** (V550 = newer struct layout). |
| `ctrlRegisterVASpace` | frontend.go:1008 | **handle dependency tracking**: validates HClient, on success records `objAddDep(HObject, HVASpace)` for object-graph teardown. |

> For nvkvm's C allowlist, **every entry below is allowed**; the
> "special" handlers are the ones requiring extra in-VMM sanitization (embedded
> pointer/array/fd translation or bounds checks). Anything `rmControlSimple` is a
> plain size-bounded passthrough.

---

## NV0000 — root client / system (31 cmds)

`version.go` lines 226-244, 373-374, 384-389, 394, 808, 848-849, 859.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x101 | NV0000_CTRL_CMD_SYSTEM_GET_BUILD_VERSION | ctrlClientSystemGetBuildVersion (string bufs) | compUtil |
| 0x102 | NV0000_CTRL_CMD_SYSTEM_GET_CPU_INFO | rmControlSimple | compUtil\|graphics |
| 0x127 | NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS | ctrlClientSystemGetP2PCapsV550 (array) | compUtil |
| 0x12b | NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS_V2 | rmControlSimple | compUtil |
| 0x136 | NV0000_CTRL_CMD_SYSTEM_GET_FABRIC_STATUS | rmControlSimple | compUtil |
| 0x13a | NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS_MATRIX | rmControlSimple | compUtil |
| 0x1f0 | NV0000_CTRL_CMD_SYSTEM_GET_FEATURES | rmControlSimple | compUtil |
| 0x201 | NV0000_CTRL_CMD_GPU_GET_ATTACHED_IDS | rmControlSimple | compUtil |
| 0x202 | NV0000_CTRL_CMD_GPU_GET_ID_INFO | ctrlGpuGetIDInfo (scrub SzName) | compUtil |
| 0x204 | NV0000_CTRL_CMD_GPU_GET_DEVICE_IDS | rmControlSimple | compUtil\|graphics |
| 0x205 | NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2 | rmControlSimple | compUtil |
| 0x214 | NV0000_CTRL_CMD_GPU_GET_PROBED_IDS | rmControlSimple | compUtil |
| 0x215 | NV0000_CTRL_CMD_GPU_ATTACH_IDS | rmControlSimple | compUtil |
| 0x216 | NV0000_CTRL_CMD_GPU_DETACH_IDS | rmControlSimple | compUtil |
| 0x21b | NV0000_CTRL_CMD_GPU_GET_PCI_INFO | rmControlSimple | compUtil |
| 0x275 | NV0000_CTRL_CMD_GPU_GET_UUID_FROM_GPU_ID | rmControlSimple | compUtil\|graphics |
| 0x279 | NV0000_CTRL_CMD_GPU_QUERY_DRAIN_STATE | rmControlSimple | compUtil |
| 0x27b | NV0000_CTRL_CMD_GPU_GET_MEMOP_ENABLE | rmControlSimple | compUtil |
| 0x288 | NV0000_CTRL_CMD_GPU_GET_ACTIVE_DEVICE_IDS | rmControlSimple | compUtil |
| 0x289 | NV0000_CTRL_CMD_GPU_ASYNC_ATTACH_ID | rmControlSimple | compUtil |
| 0x290 | NV0000_CTRL_CMD_GPU_WAIT_ATTACH_ID | rmControlSimple | compUtil |
| 0x301 | NV0000_CTRL_CMD_GSYNC_GET_ATTACHED_IDS | rmControlSimple | graphics |
| 0x3d05 | NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD | ctrlHasFrontendFD (fd xlat) | compUtil |
| 0x3d06 | NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD | ctrlHasFrontendFD (fd xlat) | compUtil |
| 0x3d08 | NV0000_CTRL_CMD_OS_UNIX_GET_EXPORT_OBJECT_INFO | ctrlHasFrontendFD (fd xlat, V545 struct) | compUtil |
| 0x3d0b | NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECTS_TO_FD | ctrlHasFrontendFD (fd xlat) | compUtil |
| 0x3d0c | NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECTS_FROM_FD | ctrlHasFrontendFD (fd xlat) | compUtil |
| 0xa04 | NV0000_CTRL_CMD_SYNC_GPU_BOOST_GROUP_INFO | rmControlSimple | compUtil |
| 0xb02 | NV0000_CTRL_CMD_GPUACCT_GET_ACCOUNTING_STATE | rmControlSimple | graphics |
| 0xd01 | NV0000_CTRL_CMD_CLIENT_GET_ADDR_SPACE_TYPE | rmControlSimple | compUtil |
| 0xd04 | NV0000_CTRL_CMD_CLIENT_SET_INHERITED_SHARE_POLICY | rmControlSimple | compUtil |

```c
/* NV0000 hex */
0x101,0x102,0x127,0x12b,0x136,0x13a,0x1f0,0x201,0x202,0x204,0x205,0x214,0x215,
0x216,0x21b,0x275,0x279,0x27b,0x288,0x289,0x290,0x301,0x3d05,0x3d06,0x3d08,
0x3d0b,0x3d0c,0xa04,0xb02,0xd01,0xd04,
```

## NV0041 — memory/surface (1 cmd)

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x410110 | NV0041_CTRL_CMD_GET_SURFACE_INFO | ctrlIoctlHasInfoList (array+bounds) | compUtil |

```c
0x410110,
```

## NV0080 — device (21 cmds)

`version.go` lines 245-256, 375-383, 850.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x800201 | NV0080_CTRL_CMD_GPU_GET_CLASSLIST | ctrlGetNvU32List (array+bounds) | compUtil |
| 0x800280 | NV0080_CTRL_CMD_GPU_GET_NUM_SUBDEVICES | rmControlSimple | compUtil |
| 0x800288 | NV0080_CTRL_CMD_GPU_QUERY_SW_STATE_PERSISTENCE | rmControlSimple | compUtil |
| 0x800289 | NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE | rmControlSimple | compUtil |
| 0x800292 | NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 | rmControlSimple | compUtil |
| 0x801102 | NV0080_CTRL_CMD_GR_GET_CAPS | ctrlDevGetCaps (caps-tbl) | graphics |
| 0x801104 | NV0080_CTRL_CMD_GR_GET_INFO | ctrlIoctlHasInfoList (array+bounds) | graphics |
| 0x801109 | NV0080_CTRL_CMD_GR_GET_CAPS_V2 | rmControlSimple | graphics\|video |
| 0x801301 | NV0080_CTRL_CMD_FB_GET_CAPS | ctrlDevGetCaps (caps-tbl) | graphics |
| 0x801307 | NV0080_CTRL_CMD_FB_GET_CAPS_V2 | rmControlSimple | compUtil |
| 0x801402 | NV0080_CTRL_CMD_HOST_GET_CAPS_V2 | rmControlSimple | compUtil |
| 0x801701 | NV0080_CTRL_CMD_FIFO_GET_CAPS | ctrlDevGetCaps (caps-tbl) | graphics |
| 0x801707 | NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES | rmControlSimple | graphics |
| 0x80170d | NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST | ctrlDevFIFOGetChannelList (array xlat) | compUtil |
| 0x801713 | NV0080_CTRL_CMD_FIFO_GET_CAPS_V2 | rmControlSimple | video |
| 0x801806 | NV0080_CTRL_CMD_DMA_ADV_SCHED_GET_VA_CAPS | rmControlSimple | compUtil\|graphics |
| 0x80180d | NV0080_CTRL_CMD_DMA_GET_CAPS | rmControlSimple | compUtil\|graphics |
| 0x801909 | NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL | rmControlSimple | compUtil |
| 0x801b01 | NV0080_CTRL_CMD_MSENC_GET_CAPS | ctrlDevGetCaps (caps-tbl) | graphics\|video |
| 0x801c02 | NV0080_CTRL_CMD_BSP_GET_CAPS_V2 | rmControlSimple | graphics\|video |
| 0x801f02 | NV0080_CTRL_CMD_NVJPG_GET_CAPS_V2 | rmControlSimple | video |

```c
/* NV0080 hex */
0x800201,0x800280,0x800288,0x800289,0x800292,0x801102,0x801104,0x801109,
0x801301,0x801307,0x801402,0x801701,0x801707,0x80170d,0x801713,0x801806,
0x80180d,0x801909,0x801b01,0x801c02,0x801f02,
```

## NV00DE — RM user shared data (1) · NV00E0 — fabric import (1) · NV00F1 — fabric events (3)

`version.go` lines 809, 851, 852-854.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0xde0001 | NV00DE_CTRL_CMD_REQUEST_DATA_POLL | rmControlSimple | compUtil |
| 0xe00102 | NV00E0_CTRL_CMD_IMPORT_MEM | rmControlSimple | **fabric** |
| 0xf10001 | NV00F1_CTRL_CMD_GET_FABRIC_EVENTS | rmControlSimple | **fabric** |
| 0xf10002 | NV00F1_CTRL_CMD_FINISH_MEM_UNIMPORT | rmControlSimple | **fabric** |
| 0xf10003 | NV00F1_CTRL_CMD_DISABLE_IMPORTERS | rmControlSimple | **fabric** |

```c
0xde0001,0xe00102,0xf10001,0xf10002,0xf10003,
```

## NV00F8 — fabric memory (2) · NV00FB — IMEX validate (1) · NV00FD — multicast fabric (6)

`version.go` lines 258-262, 391, 855-857.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0xf80102 | NV00F8_CTRL_CMD_DESCRIBE | rmControlSimple | **fabric** |
| 0xf80103 | NV00F8_CTRL_CMD_ATTACH_MEM | rmControlSimple | compUtil |
| 0xfb0101 | NV00FB_CTRL_CMD_VALIDATE | rmControlSimple | **fabric** |
| 0xfd0101 | NV00FD_CTRL_CMD_GET_INFO | rmControlSimple | compUtil |
| 0xfd0102 | NV00FD_CTRL_CMD_ATTACH_MEM | rmControlSimple | compUtil |
| 0xfd0104 | NV00FD_CTRL_CMD_ATTACH_GPU | ctrlMemoryMulticastFabricAttachGPU (fd xlat) | compUtil |
| 0xfd0105 | NV00FD_CTRL_CMD_DETACH_MEM | rmControlSimple | compUtil |
| 0xfd0106 | NV00FD_CTRL_CMD_ATTACH_REMOTE_GPU | rmControlSimple | **fabric** |
| 0xfd0107 | NV00FD_CTRL_CMD_SET_FAILURE | rmControlSimple | **fabric** |

```c
0xf80102,0xf80103,0xfb0101,0xfd0101,0xfd0102,0xfd0104,0xfd0105,0xfd0106,0xfd0107,
```

## NV2080 — subdevice (85 cmds)

`version.go` lines 263-331, 393, 395-402, 858, 956-957, 980, 993-995, 1041-1043
(575 deltas: DRAM-encryption support/status renamed to `_V575` codes, THERMAL_SYSTEM_EXECUTE_V2 added).

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x20800102 | NV2080_CTRL_CMD_GPU_GET_INFO_V2 | rmControlSimple | compUtil |
| 0x20800110 | NV2080_CTRL_CMD_GPU_GET_NAME_STRING | rmControlSimple | compUtil |
| 0x20800111 | NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING | rmControlSimple | compUtil |
| 0x20800119 | NV2080_CTRL_CMD_GPU_GET_SIMULATION_INFO | rmControlSimple | compUtil |
| 0x20800122 | NV2080_CTRL_CMD_GPU_EXEC_REG_OPS | ctrlGpuExecRegOps (regop array+bounds) | **profiling** |
| 0x20800123 | NV2080_CTRL_CMD_GPU_GET_ENGINES | ctrlGetNvU32List (array+bounds) | graphics |
| 0x2080012f | NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS | rmControlSimple | compUtil |
| 0x20800131 | NV2080_CTRL_CMD_GPU_QUERY_COMPUTE_MODE_RULES | rmControlSimple | compUtil |
| 0x20800133 | NV2080_CTRL_CMD_GPU_QUERY_ECC_CONFIGURATION | rmControlSimple | compUtil |
| 0x2080013f | NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO | rmControlSimple | compUtil |
| 0x20800142 | NV2080_CTRL_CMD_GPU_GET_ID | rmControlSimple | compUtil\|graphics |
| 0x20800145 | NV2080_CTRL_CMD_GPU_ACQUIRE_COMPUTE_MODE_RESERVATION | rmControlSimple | compUtil |
| 0x20800146 | NV2080_CTRL_CMD_GPU_RELEASE_COMPUTE_MODE_RESERVATION | rmControlSimple | compUtil |
| 0x20800147 | NV2080_CTRL_CMD_GPU_GET_ENGINE_PARTNERLIST | rmControlSimple | graphics |
| 0x2080014a | NV2080_CTRL_CMD_GPU_GET_GID_INFO | rmControlSimple | compUtil |
| 0x2080014b | NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION | rmControlSimple | compUtil |
| 0x20800156 | NV2080_CTRL_CMD_GPU_GET_INFOROM_IMAGE_VERSION | rmControlSimple | compUtil |
| 0x20800157 | NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT | rmControlSimple | compUtil |
| 0x2080016c | NV2080_CTRL_CMD_GPU_GET_ENCODER_CAPACITY | rmControlSimple | video |
| 0x2080016d | NV2080_CTRL_CMD_GPU_GET_NVENC_SW_SESSION_STATS | rmControlSimple | graphics |
| 0x20800170 | NV2080_CTRL_CMD_GPU_GET_ENGINES_V2 | rmControlSimple | compUtil |
| 0x2080017b | NV2080_CTRL_CMD_GPU_GET_NVFBC_SW_SESSION_STATS | rmControlSimple | graphics |
| 0x2080018b | NV2080_CTRL_CMD_GPU_GET_ACTIVE_PARTITION_IDS | rmControlSimple | compUtil |
| 0x2080018d | NV2080_CTRL_CMD_GPU_GET_PIDS | rmControlSimple | compUtil |
| 0x2080018e | NV2080_CTRL_CMD_GPU_GET_PID_INFO | rmControlSimple | compUtil |
| 0x20800195 | NV2080_CTRL_CMD_GPU_GET_COMPUTE_POLICY_CONFIG | rmControlSimple | compUtil |
| 0x208001a3 | NV2080_CTRL_CMD_GET_GPU_FABRIC_PROBE_INFO | rmControlSimple | compUtil |
| 0x208001a4 | NV2080_CTRL_CMD_GPU_GET_CHIP_DETAILS | rmControlSimple | graphics |
| 0x208001b2 | NV2080_CTRL_CMD_GPU_GET_RECOVERY_ACTION | rmControlSimple | graphics |
| 0x20800301 | NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION | rmControlSimple | compUtil |
| 0x20800403 | NV2080_CTRL_CMD_TIMER_GET_TIME | rmControlSimple | compUtil\|graphics |
| 0x20800406 | NV2080_CTRL_CMD_TIMER_GET_GPU_CPU_TIME_CORRELATION_INFO | rmControlSimple | compUtil |
| 0x20800407 | NV2080_CTRL_CMD_TIMER_SET_GR_TICK_FREQ | rmControlSimple | compUtil |
| 0x20800513 | NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2 | rmControlSimple | compUtil |
| 0x20800802 | NV2080_CTRL_CMD_BIOS_GET_INFO | ctrlIoctlHasInfoList (array+bounds) | compUtil |
| 0x2080110b | NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS | ctrlSubdevFIFODisableChannels (null-ptr assert) | compUtil |
| 0x20801201 | NV2080_CTRL_CMD_GR_GET_INFO | ctrlIoctlHasInfoList[GR_GET_INFO] (array+bounds) | compUtil |
| 0x20801206 | NV2080_CTRL_CMD_GR_GET_ZCULL_INFO | rmControlSimple | graphics |
| 0x20801208 | NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND | rmControlSimple | graphics |
| 0x2080120f | NV2080_CTRL_CMD_GR_GET_SM_TO_GPC_TPC_MAPPINGS | rmControlSimple | **profiling** |
| 0x20801210 | NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE | rmControlSimple | compUtil |
| 0x20801218 | NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_SIZE | rmControlSimple | compUtil |
| 0x2080121b | NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER | rmControlSimple | compUtil |
| 0x20801227 | NV2080_CTRL_CMD_GR_GET_CAPS_V2 | rmControlSimple | compUtil |
| 0x2080122a | NV2080_CTRL_CMD_GR_GET_GPC_MASK | rmControlSimple | compUtil |
| 0x2080122b | NV2080_CTRL_CMD_GR_GET_TPC_MASK | rmControlSimple | compUtil |
| 0x20801230 | NV2080_CTRL_CMD_GR_GET_SM_ISSUE_RATE_MODIFIER | rmControlSimple | compUtil |
| 0x20801301 | NV2080_CTRL_CMD_FB_GET_INFO | ctrlIoctlHasInfoList (array+bounds) | graphics |
| 0x20801303 | NV2080_CTRL_CMD_FB_GET_INFO_V2 | rmControlSimple | compUtil |
| 0x2080130e | NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE | rmControlSimple | **profiling** |
| 0x20801315 | NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO | rmControlSimple | graphics |
| 0x20801320 | NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO | rmControlSimple | graphics |
| 0x20801322 | NV2080_CTRL_CMD_FB_GET_OFFLINED_PAGES | rmControlSimple | graphics |
| 0x20801346 | NV2080_CTRL_CMD_FB_GET_FS_INFO | rmControlSimple | **profiling** |
| 0x20801352 | NV2080_CTRL_CMD_FB_GET_SEMAPHORE_SURFACE_LAYOUT | rmControlSimple | graphics |
| 0x20801355 | NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_PENDING_CONFIGURATION | rmControlSimple | graphics |
| 0x20801357 | NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575 | rmControlSimple | compUtil |
| 0x20801358 | NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575 | rmControlSimple | compUtil |
| 0x20801701 | NV2080_CTRL_CMD_MC_GET_ARCH_INFO | rmControlSimple | compUtil |
| 0x20801702 | NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS | rmControlSimple | compUtil |
| 0x20801801 | NV2080_CTRL_CMD_BUS_GET_PCI_INFO | rmControlSimple | compUtil |
| 0x20801802 | NV2080_CTRL_CMD_BUS_GET_INFO | ctrlIoctlHasInfoList (array+bounds) | video |
| 0x20801803 | NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO | rmControlSimple | compUtil |
| 0x20801813 | NV2080_CTRL_CMD_BUS_GET_PEX_COUNTERS | rmControlSimple | graphics |
| 0x20801819 | NV2080_CTRL_CMD_BUS_GET_PEX_UTIL_COUNTERS | rmControlSimple | graphics |
| 0x20801823 | NV2080_CTRL_CMD_BUS_GET_INFO_V2 | rmControlSimple | compUtil |
| 0x20801829 | NV2080_CTRL_CMD_BUS_GET_PCIE_REQ_ATOMICS_CAPS | rmControlSimple | graphics |
| 0x2080182a | NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS | rmControlSimple | compUtil |
| 0x2080182b | NV2080_CTRL_CMD_BUS_GET_C2C_INFO | rmControlSimple | compUtil |
| 0x20801830 | NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS | rmControlSimple | graphics |
| 0x2080200a | NV2080_CTRL_CMD_PERF_BOOST | rmControlSimple | compUtil |
| 0x20802068 | NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE | rmControlSimple | compUtil |
| 0x20802209 | NV2080_CTRL_CMD_RC_GET_WATCHDOG_INFO | rmControlSimple | compUtil |
| 0x2080220c | NV2080_CTRL_CMD_RC_RELEASE_WATCHDOG_REQUESTS | rmControlSimple | compUtil |
| 0x20802210 | NV2080_CTRL_CMD_RC_SOFT_DISABLE_WATCHDOG | rmControlSimple | compUtil |
| 0x20802a02 | NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK | rmControlSimple | graphics |
| 0x20802a03 | NV2080_CTRL_CMD_CE_GET_CAPS_V2 | rmControlSimple | compUtil\|graphics |
| 0x20802a0a | NV2080_CTRL_CMD_CE_GET_ALL_CAPS | rmControlSimple | compUtil |
| 0x20803001 | NV2080_CTRL_CMD_NVLINK_GET_NVLINK_CAPS | rmControlSimple | compUtil |
| 0x20803002 | NV2080_CTRL_CMD_NVLINK_GET_NVLINK_STATUS | rmControlSimple | compUtil |
| 0x20803083 | NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO | rmControlSimple | **fabric** |
| 0x20803125 | NV2080_CTRL_CMD_FLCN_GET_CTX_BUFFER_SIZE | rmControlSimple | compUtil |
| 0x20803601 | NV2080_CTRL_CMD_GSP_GET_FEATURES | rmControlSimple | compUtil |
| 0x20803801 | NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO | rmControlSimple | compUtil |
| 0x20803d07 | NV2080_CTRL_CMD_OS_UNIX_VIDMEM_PERSISTENCE_STATUS | rmControlSimple | graphics |

```c
/* NV2080 hex */
0x20800102,0x20800110,0x20800111,0x20800119,0x20800122,0x20800123,0x2080012f,
0x20800131,0x20800133,0x2080013f,0x20800142,0x20800145,0x20800146,0x20800147,
0x2080014a,0x2080014b,0x20800156,0x20800157,0x2080016c,0x2080016d,0x20800170,
0x2080017b,0x2080018b,0x2080018d,0x2080018e,0x20800195,0x208001a3,0x208001a4,
0x208001b2,0x20800301,0x20800403,0x20800406,0x20800407,0x20800513,0x20800802,
0x2080110b,0x20801201,0x20801206,0x20801208,0x2080120f,0x20801210,0x20801218,
0x2080121b,0x20801227,0x2080122a,0x2080122b,0x20801230,0x20801301,0x20801303,
0x2080130e,0x20801315,0x20801320,0x20801322,0x20801346,0x20801352,0x20801355,
0x20801357,0x20801358,0x20801701,0x20801702,0x20801801,0x20801802,0x20801803,
0x20801813,0x20801819,0x20801823,0x20801829,0x2080182a,0x2080182b,0x20801830,
0x2080200a,0x20802068,0x20802209,0x2080220c,0x20802210,0x20802a02,0x20802a03,
0x20802a0a,0x20803001,0x20803002,0x20803083,0x20803125,0x20803601,0x20803801,
0x20803d07,
```

## NV208F — diag GPU (1)

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x208f1105 | NV208F_CTRL_CMD_GPU_VERIFY_INFOROM | rmControlSimple | compUtil |

```c
0x208f1105,
```

## NV503C — third-party P2P (3)

`version.go` lines 332-333, 392.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x503c0102 | NV503C_CTRL_CMD_REGISTER_VA_SPACE | ctrlRegisterVASpace (handle dep tracking) | compUtil |
| 0x503c0104 | NV503C_CTRL_CMD_REGISTER_VIDMEM | rmControlSimple | compUtil |
| 0x503c0105 | NV503C_CTRL_CMD_UNREGISTER_VIDMEM | rmControlSimple | compUtil |

```c
0x503c0102,0x503c0104,0x503c0105,
```

## NV83DE — SM debug (3)

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x83de0309 | NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK | rmControlSimple | compUtil |
| 0x83de030c | NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES | rmControlSimple | compUtil |
| 0x83de0310 | NV83DE_CTRL_CMD_DEBUG_CLEAR_ALL_SM_ERROR_STATES | rmControlSimple | compUtil |

```c
0x83de0309,0x83de030c,0x83de0310,
```

## NV906F — GPFIFO (Kepler) (2) · NVA06C / NVA06F — channel-group / GPFIFO (6) · NVC36F / NVC56F — GPFIFO (3)

`version.go` lines 337-338, 360-362, 366-372; NVC36F_CTRL_GET_CLASS_ENGINEID is
**deleted** at v555 (line 933), so it is **NOT** in the 575 set.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x906f0101 | NV906F_CTRL_GET_CLASS_ENGINEID | rmControlSimple | compUtil |
| 0x906f0102 | NV906F_CTRL_CMD_RESET_CHANNEL | rmControlSimple | compUtil |
| 0xa06c0101 | NVA06C_CTRL_CMD_GPFIFO_SCHEDULE | rmControlSimple | compUtil |
| 0xa06c0103 | NVA06C_CTRL_CMD_SET_TIMESLICE | rmControlSimple | compUtil |
| 0xa06c0104 | NVA06C_CTRL_CMD_GET_TIMESLICE | rmControlSimple | **profiling** |
| 0xa06c0105 | NVA06C_CTRL_CMD_PREEMPT | rmControlSimple | compUtil |
| 0xa06f0103 | NVA06F_CTRL_CMD_GPFIFO_SCHEDULE | rmControlSimple | compUtil |
| 0xa06f0104 | NVA06F_CTRL_CMD_BIND | rmControlSimple | graphics\|video |
| 0xc36f0108 | NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN | rmControlSimple | compUtil |
| 0xc36f010a | NVC36F_CTRL_CMD_GPFIFO_SET_WORK_SUBMIT_TOKEN_NOTIF_INDEX | rmControlSimple | graphics\|video |
| 0xc56f010b | NVC56F_CTRL_CMD_GET_KMB | rmControlSimple | compUtil |

```c
/* fifo / channel */
0x906f0101,0x906f0102,0xa06c0101,0xa06c0103,0xa06c0104,0xa06c0105,
0xa06f0103,0xa06f0104,0xc36f0108,0xc36f010a,0xc56f010b,
```

## NV9096 — ZBC (3) · NV90CC — power profiling (2) · NV90E6 — master fault (1)

`version.go` lines 339-342, 359, 403.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x90960101 | NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR | rmControlSimple | graphics |
| 0x90960106 | NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE | rmControlSimple | graphics |
| 0x90960107 | NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_ENTRY | rmControlSimple | graphics |
| 0x90cc0301 | NV90CC_CTRL_CMD_POWER_REQUEST_FEATURES | rmControlSimple | **profiling** |
| 0x90cc0302 | NV90CC_CTRL_CMD_POWER_RELEASE_FEATURES | rmControlSimple | **profiling** |
| 0x90e60102 | NV90E6_CTRL_CMD_MASTER_GET_VIRTUAL_FUNCTION_ERROR_CONT_INTR_MASK | rmControlSimple | compUtil |

```c
0x90960101,0x90960106,0x90960107,0x90cc0301,0x90cc0302,0x90e60102,
```

## NVB0CC — profiler/HWPM (17, all profiling-gated)

`version.go` lines 343-358, 992.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0xb0cc0101 | NVB0CC_CTRL_CMD_RESERVE_HWPM_LEGACY | rmControlSimple | **profiling** |
| 0xb0cc0102 | NVB0CC_CTRL_CMD_RELEASE_HWPM_LEGACY | rmControlSimple | **profiling** |
| 0xb0cc0103 | NVB0CC_CTRL_CMD_RESERVE_PM_AREA_SMPC | rmControlSimple | **profiling** |
| 0xb0cc0104 | NVB0CC_CTRL_CMD_RELEASE_PM_AREA_SMPC | rmControlSimple | **profiling** |
| 0xb0cc0105 | NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM | rmControlSimple | **profiling** |
| 0xb0cc0106 | NVB0CC_CTRL_CMD_FREE_PMA_STREAM | rmControlSimple | **profiling** |
| 0xb0cc0107 | NVB0CC_CTRL_CMD_BIND_PM_RESOURCES | rmControlSimple | **profiling** |
| 0xb0cc0108 | NVB0CC_CTRL_CMD_UNBIND_PM_RESOURCES | rmControlSimple | **profiling** |
| 0xb0cc0109 | NVB0CC_CTRL_CMD_PMA_STREAM_UPDATE_GET_PUT | rmControlSimple | **profiling** |
| 0xb0cc010a | NVB0CC_CTRL_CMD_EXEC_REG_OPS | rmControlSimple | **profiling** |
| 0xb0cc010b | NVB0CC_CTRL_CMD_RESERVE_PM_AREA_PC_SAMPLER | rmControlSimple | **profiling** |
| 0xb0cc010c | NVB0CC_CTRL_CMD_RELEASE_PM_AREA_PC_SAMPLER | rmControlSimple | **profiling** |
| 0xb0cc010d | NVB0CC_CTRL_CMD_GET_TOTAL_HS_CREDITS | rmControlSimple | **profiling** |
| 0xb0cc010e | NVB0CC_CTRL_CMD_SET_HS_CREDITS | rmControlSimple | **profiling** |
| 0xb0cc0119 | NVB0CC_CTRL_CMD_RESERVE_CCU_PROF | rmControlSimple | **profiling** |
| 0xb0cc0301 | NVB0CC_CTRL_CMD_POWER_REQUEST_FEATURES | rmControlSimple | **profiling** |
| 0xb0cc0302 | NVB0CC_CTRL_CMD_POWER_RELEASE_FEATURES | rmControlSimple | **profiling** |

```c
/* NVB0CC — all require CapProfiling */
0xb0cc0101,0xb0cc0102,0xb0cc0103,0xb0cc0104,0xb0cc0105,0xb0cc0106,0xb0cc0107,
0xb0cc0108,0xb0cc0109,0xb0cc010a,0xb0cc010b,0xb0cc010c,0xb0cc010d,0xb0cc010e,
0xb0cc0119,0xb0cc0301,0xb0cc0302,
```

## NV_CONF_COMPUTE (cb33xx) — confidential compute (4)

`version.go` lines 363-365, 914.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0xcb330101 | NV_CONF_COMPUTE_CTRL_CMD_SYSTEM_GET_CAPABILITIES | rmControlSimple | compUtil |
| 0xcb330104 | NV_CONF_COMPUTE_CTRL_CMD_SYSTEM_GET_GPUS_STATE | rmControlSimple | compUtil |
| 0xcb33010b | NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_NUM_SECURE_CHANNELS | rmControlSimple | compUtil |
| 0xcb33010c | NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE | rmControlSimple | compUtil |

```c
0xcb330101,0xcb330104,0xcb33010b,0xcb33010c,
```

## NV_SEMAPHORE_SURFACE (da00xx) (2)

`version.go` lines 257, 955.

| hex | cmd | handler | cap |
|---|---|---|---|
| 0xda0002 | NV_SEMAPHORE_SURFACE_CTRL_CMD_BIND_CHANNEL | rmControlSimple | graphics |
| 0xda0006 | NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL | rmControlSimple | graphics |

```c
0xda0002,0xda0006,
```

## UNKNOWN (1)

| hex | cmd | handler | cap |
|---|---|---|---|
| 0x80028b | UNKNOWN_CONTROL_COMMAND_80028B | rmControlSimple | compUtil |

> nvproxy comment: "unknown, paramsSize == 1" — empirically observed in CUDA
> traffic, class 0x0080. Include it for CUDA compat.

```c
0x80028b,
```

---

## What nvproxy explicitly does NOT allow (deny list)

nvproxy is **default-deny**: any control cmd not in the table above returns
`NV_ERR_NOT_SUPPORTED`. Notable commonly-seen commands that are deliberately
absent (deny these unless/until proven needed):

- **`NVC36F_CTRL_GET_CLASS_ENGINEID` (0xc36f0109)** — was allowed pre-555, then
  explicitly `delete`d at v555 (`version.go:933`). Deny on 575.
- **Anything in the `NV2080_CTRL_CMD_PERF_*` family except** PERF_BOOST (0x2080200a)
  and PERF_GET_CURRENT_PSTATE (0x20802068) — e.g. arbitrary pstate/clock setting
  is not registered.
- **`NV0000_CTRL_CMD_GPU_DETACH_IDS` mutation aside**, no host-wide admin/driver
  control (no `NV0000_CTRL_CMD_GPU_SET_*`, no NVLINK train/inject, no GSP debug
  dumps) is registered.
- **Profiling-class commands (NVB0CC, NV90CC, GPU_EXEC_REG_OPS, GR_GET_SM_TO_GPC_TPC,
  FB_FLUSH_GPU_CACHE, FB_GET_FS_INFO, GET_TIMESLICE)** are present but gated behind
  `CapProfiling`, which is **off by default** — so in the default container/VM
  posture they are effectively denied. For multi-tenant nvkvm, treat the entire
  `**profiling**` and `**fabric**` columns as "deny by default, opt-in only".
- The generic frontend ioctls `NV_ESC_RM_*` (alloc/map/free/etc.) are a separate
  allowlist (`version.go:166-223`, `frontendIoctl` map) — out of scope for this
  control-cmd table but note they too are default-deny.

## Recommended nvkvm allowlist posture

1. Start from the **compUtil-only** subset (drop every row tagged graphics / video
   / profiling / fabric). That is the minimal CUDA-compute surface and matches
   nvproxy's `DefaultDriverCaps = CapCompute | CapUtility`.
2. For each allowed cmd, replicate the **special handler's** sanitization:
   - fd-translation cmds (0x3d05/06/08/0b/0c, 0xfd0104) — must remap embedded fds.
   - array/info-list cmds (ctrlIoctlHasInfoList, ctrlGetNvU32List, ctrlDevGetCaps,
     ctrlGpuExecRegOps, ctrlDevFIFOGetChannelList) — must bounds-check the
     embedded count against `rmapiParamsSizeCheck` and translate the pointer.
   - 0x2080110b — assert the runlist-preempt-event pointer is NULL.
   - 0x202 — zero `SzName`. 0x503c0102 — track HObject→HVASpace dependency.
3. Keep the two forced-passthrough rules (`RM_GSS_LEGACY_MASK 0x8000`,
   class==NV2081 0x2081) **only** if you trust GSP-routed params (no app pointers).
4. Enforce `ParamsSize <= 1 MiB` (`RMAPI_PARAM_COPY_MAX_PARAMS_SIZE`) on every cmd.

## Source citations

- Allowlist map base: `pkg/sentry/devices/nvproxy/version.go:225-404`
- Per-version deltas (545→575): `version.go:806-1055` (575 block at 1037-1055)
- Dispatch + default-deny: `frontend.go:756-816`
- Handler registration types: `handlers.go:68-126`
- Handler bodies: `frontend.go:818-1080`, `frontend_unsafe.go:61-340`
- Capability defs: `nvconf/caps.go:24-69`, `seccomp_filters.go:31`
- Hex constants: `pkg/abi/nvgpu/ctrl.go`, `pkg/abi/nvgpu/classes.go`
- Masks/limits: `RM_GSS_LEGACY_MASK=0x8000` ctrl.go:23, `NV2081_BINAPI=0x2081`
  classes.go:69, `RMAPI_PARAM_COPY_MAX_PARAMS_SIZE=1MiB` ctrl.go:31
