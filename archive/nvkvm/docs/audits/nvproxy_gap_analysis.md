# nvproxy → nvkvm Gap Analysis

Audit date: 2026-05-29. Reference: gVisor `pkg/sentry/devices/nvproxy/` (driver ABI
table v535_104_05 base + incremental adds through v575). Our impl: guest module
`src/guest/nvkvm_main.c` + `src/qemu/nvkvm_dispatch.c` + `src/qemu/nvkvm_isolate_handlers.c`.

## How the two designs differ (read this first)

nvproxy is **default-deny everywhere**: it maintains explicit allowlist tables for
frontend ioctls, RM control commands, alloc classes, and UVM ioctls. Anything not in
a table is rejected (ENOTTY/EINVAL).

nvkvm is **default-deny only for UVM** (`nvkvm_uvm_schema[]` in
`nvkvm_isolate_handlers.c`). For frontend `'F'`-type ioctls, RM control commands, and
RM alloc classes, nvkvm is effectively **default-allow**: it forwards whatever the
guest sends after (a) per-VM hClient ownership checks and (b) DUP_OBJECT cross-VM
defense, plus a *size/sanitizer* table that translates embedded pointers for the
specific ops it recognizes. Ops the sanitizer doesn't recognize are still forwarded
raw.

Consequently, "gaps" fall into two distinct buckets:

- **Correctness gaps** — ops nvproxy *translates/handles specially* (embedded pointer
  lists, embedded FDs, per-class alloc-param sizing, RegOps, etc.) that nvkvm forwards
  raw or sizes incorrectly. These can silently corrupt data or fail.
- **Security gaps** — ops nvproxy *refuses* but nvkvm forwards. Because nvkvm is
  default-allow on frontend/control/class, nvkvm currently forwards a *superset* of
  what nvproxy allows. For a multi-tenant security story, nvproxy's allowlists are the
  model; nvkvm has none for these three categories.

## Summary counts

| Category | nvproxy supports | nvkvm handles specially | Correctness gaps | Security posture |
|---|---:|---:|---:|---|
| Frontend ioctls (NV_ESC_*) | 23 | 23 (all, via size table) | ~2 special-handlers missing | default-allow (no allowlist) |
| RM control commands | ~185 | 6 list-bearing + generic forward | ~10 special-handlers missing | default-allow (no allowlist) |
| Alloc classes (hClass) | ~90 | ~22 sized in table; rest forwarded | compute/gfx/video param-sizing | default-allow (no allowlist) |
| UVM ioctls | 33 | 30 (schema allowlist) | 0 | default-deny (good) |
| mmap handlers | 2 (frontend + uvm) | 2 (both) | window/BAR allocator limits | n/a |
| procfs / sysfs | 2 files | 0 | both missing | n/a |

### Top ~10 highest-priority gaps to address now

1. **RM control-command allowlist is absent** (security, High) — nvproxy enumerates
   ~185 specific `NVxxxx_CTRL_CMD_*`; nvkvm forwards any control cmd on an owned
   client. Without an allowlist a guest can reach profiling/HWPM/regops/debug control
   paths nvproxy gates behind capabilities.
2. **`NV2080_CTRL_CMD_GPU_EXEC_REG_OPS` (0x20800122)** — special handler in nvproxy
   (`ctrlGpuExecRegOps`) that bounds-checks an embedded reg-op array. nvkvm forwards
   raw. (correctness + security, High)
3. **`ctrlIoctlHasInfoList` coverage** — nvproxy translates embedded info-list pointers
   for `BUS_GET_INFO` (0x20801802), `BIOS_GET_INFO` (0x20800802), `FB_GET_INFO`
   (0x20801301), `GR_GET_INFO` (0x20801201/0x801104/0x801204…), `GET_SURFACE_INFO`
   (0x410110). nvkvm handles these 6 — verify list-pointer translation matches; any
   *other* GET_INFO-style cmd forwarded raw will corrupt. (correctness, High)
4. **Compute/graphics alloc-param sizing absent in guest table** — `NV_GR_ALLOCATION_PARAMETERS`
   classes (TURING/AMPERE/ADA/HOPPER `_A`, `_COMPUTE_A/B`) are NOT in the
   `nvkvm_main.c` alloc-params size switch. Works today only because libcuda sets
   `alloc_parms_size`; breaks for any caller leaving size=0. (correctness, High)
5. **`ctrlClientSystemGetP2PCaps` / `_V550` (0x127)** — nvproxy has a dedicated handler;
   needed by NCCL/multi-GPU P2P. nvkvm forwards raw. (correctness, Med-High for NCCL)
6. **`NV0000_CTRL_CMD_OS_UNIX_*_FROM_FD / TO_FD` (0x3d05/06/08/0b/0c)** — embedded-FD
   export/import object handlers (`ctrlHasFrontendFD`). nvkvm has no FD translation for
   these; required for CUDA IPC / `cudaIpcGetMemHandle`. (correctness, Med-High if IPC needed)
7. **UVM `MAP_DYNAMIC_PARALLELISM_REGION` (65), `MIGRATE_RANGE_GROUP` (53),
   read-duplication (44/45)** — present in nvkvm schema with `min_size 0` (size not
   validated). Tighten sizes; functionally present. (correctness/hardening, Med)
8. **procfs `/proc/driver/nvidia/params`** — nvproxy synthesizes it; some tools
   (`nvidia-smi`, libnvidia-ml) read it. nvkvm exposes nothing under
   `/proc/driver/nvidia/`. (compat, Med)
9. **`ctrlSubdevFIFODisableChannels` (0x2080110b)** & **`ctrlDevFIFOGetChannelList`
   (0x80170d)** — nvproxy special-cases these (channel-handle list translation). nvkvm
   forwards raw → wrong handles. (correctness, Med)
10. **`NV2080_CTRL_CMD_GPU_GET_PID_INFO` / `GET_PIDS` (0x2080018e / 0x2080018d)** —
    nvkvm has partial support (see MEMORY note: entry ABI 72B; per-proc MEMORY blocked).
    nvproxy forwards via `rmControlSimple`. Finish ns-translation parity. (compat, Med)

---

## 1. Frontend ioctls (NV_ESC_*)  —  base NV_IOCTL_BASE = 200 (0xC8)

nvproxy-supported set and our coverage. **Every NV_ESC_* nvproxy registers is present
in our `nvkvm_dispatch.c` size table** — so there are no *missing* frontend ioctls. The
gaps here are missing *special handlers* (pointer/FD translation) for two, plus the
absence of an allowlist.

| NV_ESC_* | NR (dec/hex) | Meaning | nvkvm | Gap / importance |
|---|---|---|---|---|
| NV_ESC_CARD_INFO | 200 / 0xc8 | enumerate GPU cards | sized | OK |
| NV_ESC_REGISTER_FD | 201 / 0xc9 | bind ctl fd to device fd | special (frontendRegisterFD) | OK |
| NV_ESC_ALLOC_OS_EVENT | 206 / 0xce | create OS event w/ embedded fd | special (HasFD) | OK |
| NV_ESC_FREE_OS_EVENT | 207 / 0xcf | free OS event | special (HasFD) | OK |
| NV_ESC_CHECK_VERSION_STR | 210 / 0xd2 | RM version handshake | sized | OK |
| NV_ESC_ATTACH_GPUS_TO_FD | 212 / 0xd4 | attach GPU-ID array to fd | sized (raw NvU32 array) | OK |
| NV_ESC_SYS_PARAMS | 214 / 0xd6 | system page-size params | sized | OK |
| NV_ESC_NUMA_INFO | 215 / 0xd7 | NUMA node info (rmNumaInfo) | sized | **Med**: nvproxy uses a dedicated `rmNumaInfo` handler that rewrites node masks; nvkvm forwards raw. Fine for non-NUMA hosts, wrong on multi-socket. |
| NV_ESC_WAIT_OPEN_COMPLETE | 218 / 0xda | wait for async dev open (v550+) | sized | OK |
| NV_ESC_RM_ALLOC_MEMORY | 0x27 | legacy mem alloc w/ embedded fd | special (rmAllocMemory) | OK |
| NV_ESC_RM_FREE | 0x29 | free object handle | sized | OK |
| NV_ESC_RM_CONTROL | 0x2a | RM control dispatch | special | OK (see §2) |
| NV_ESC_RM_ALLOC | 0x2b | alloc object by class | special | OK (see §3) |
| NV_ESC_RM_DUP_OBJECT | 0x34 | dup handle across clients | special (rmDupObject) | OK; nvkvm adds cross-VM gate |
| NV_ESC_RM_SHARE | 0x35 | share object (NVOS57) | sized | OK |
| NV_ESC_RM_IDLE_CHANNELS | 0x41 | idle channels (CapGraphics) | sized | Low |
| NV_ESC_RM_VID_HEAP_CONTROL | 0x4a | vidheap alloc (NVOS32) | sized | OK |
| NV_ESC_RM_MAP_MEMORY | 0x4e | map RM mem → VA, embedded fd | special (rmMapMemory) | OK |
| NV_ESC_RM_UNMAP_MEMORY | 0x4f | unmap (NVOS34) | sized | OK |
| NV_ESC_RM_ALLOC_CONTEXT_DMA2 | 0x54 | ctx DMA alloc (CapGraphics) | sized (raw) | **Low-Med**: nvproxy uses `rmAllocContextDMA2`; graphics-only. |
| NV_ESC_RM_MAP_MEMORY_DMA | 0x57 | map into GPU VA space (NVOS46) | sized | OK |
| NV_ESC_RM_UNMAP_MEMORY_DMA | 0x58 | unmap GPU VA (NVOS47) | sized | OK |
| NV_ESC_RM_UPDATE_DEVICE_MAPPING_INFO | 0x5e | update mapping (NVOS56) | special (forged NV_OK) | OK |

> nvkvm extras NOT in nvproxy: `NV_ESC_EXPORT_TO_DMABUF_FD` (dispatch.c:138). nvproxy
> has no dmabuf path at all — this is a deliberate nvkvm-only feature, not a gap.

**Frontend security gap (High):** nvproxy's frontend table *is* the allowlist; nvkvm
has no equivalent — it accepts any `'F'`-type NR with a known size and a valid hClient.
Adding an explicit frontend allowlist mirroring this table closes the surface.

---

## 2. RM control commands (NVxxxx_CTRL_CMD_*)

nvproxy registers ~185 control commands. The overwhelming majority use the generic
`rmControlSimple` (copy params in/out, no embedded pointers) — nvkvm forwards those
correctly by default. The gaps are: (a) **no allowlist** (security), and (b) the
**handful that need special translation** which nvkvm must replicate.

### 2a. Control commands needing SPECIAL handlers in nvproxy (correctness-critical)

| Control cmd | Code | nvproxy handler | nvkvm | Importance |
|---|---|---|---|---|
| NV2080_CTRL_CMD_GPU_EXEC_REG_OPS | 0x20800122 | ctrlGpuExecRegOps (bounds reg-op array) | raw fwd | **High** — embedded array; raw forward is OOB-prone; CapProfiling in nvproxy |
| NV2080_CTRL_CMD_BUS_GET_INFO | 0x20801802 | ctrlIoctlHasInfoList | handled (list) | High — verify parity |
| NV2080_CTRL_CMD_BIOS_GET_INFO | 0x20800802 | ctrlIoctlHasInfoList | handled (list) | Med |
| NV2080_CTRL_CMD_FB_GET_INFO | 0x20801301 | ctrlIoctlHasInfoList | handled (list) | High |
| NV2080_CTRL_CMD_GR_GET_INFO | 0x20801201 | ctrlIoctlHasInfoList | handled (list) | High |
| NV0080_CTRL_CMD_GR_GET_INFO | 0x801104 | ctrlIoctlHasInfoList | handled (list) | Med |
| NV0041_CTRL_CMD_GET_SURFACE_INFO | 0x410110 | ctrlIoctlHasInfoList | handled (list) | Med |
| NV2080_CTRL_CMD_GPU_GET_ENGINES | 0x20800123 | ctrlGetNvU32List (embedded list) | raw fwd | **Med** — list ptr not translated |
| NV0080_CTRL_CMD_GPU_GET_CLASSLIST | 0x800201 | ctrlGetNvU32List | raw fwd | **Med** — class enumeration; CUDA uses CLASSLIST_V2 (rmControlSimple, OK) |
| NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS | 0x2080110b | ctrlSubdevFIFODisableChannels | raw fwd | **Med** — embedded channel-handle list |
| NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST | 0x80170d | ctrlDevFIFOGetChannelList | raw fwd | **Med** — channel-handle list translation |
| NV0080_CTRL_CMD_{GR,FB,FIFO,MSENC}_GET_CAPS | 0x801102/801301/801701/801b01 | ctrlDevGetCaps (embedded caps buf ptr) | raw fwd | **Med** — caps-table ptr; gfx mostly |
| NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS | 0x127 | ctrlClientSystemGetP2PCaps(_V550) | raw fwd | **Med-High** — NCCL/peer; embedded fields |
| NV0000_CTRL_CMD_GPU_GET_ID_INFO | 0x202 | ctrlGpuGetIDInfo | raw fwd | Med |
| NV0000_CTRL_CMD_SYSTEM_GET_BUILD_VERSION | 0x101 | ctrlClientSystemGetBuildVersion (embedded strings) | handled | OK (in guest table) |
| NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD | 0x3d05 | ctrlHasFrontendFD | no FD xlat | **Med-High** — CUDA IPC export |
| NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD | 0x3d06 | ctrlHasFrontendFD | no FD xlat | **Med-High** — CUDA IPC import |
| NV0000_CTRL_CMD_OS_UNIX_GET_EXPORT_OBJECT_INFO | 0x3d08 | ctrlHasFrontendFD(_V545) | no FD xlat | Med |
| NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECTS_TO_FD | 0x3d0b | ctrlHasFrontendFD | no FD xlat | Med |
| NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECTS_FROM_FD | 0x3d0c | ctrlHasFrontendFD | no FD xlat | Med |
| NV00FD_CTRL_CMD_ATTACH_GPU | 0xfd0104 | ctrlMemoryMulticastFabricAttachGPU | raw fwd | Low (multi-node fabric) |
| NV503C_CTRL_CMD_REGISTER_VA_SPACE | 0x503c0102 | ctrlRegisterVASpace | raw fwd | Low (3rd-party P2P) |

### 2b. Control commands nvproxy gates by capability that nvkvm forwards unconditionally

nvproxy tags many control cmds with `CapProfiling` / `CapGraphics` / `CapVideo` /
`CapFabricIMEXManagement`. nvkvm has no capability concept, so it forwards all of them
to any guest client. Highest-risk classes for a multi-tenant box:

| Family | Examples (code) | nvproxy cap | Why it matters (security, Med-High) |
|---|---|---|---|
| HWPM / perfmon (NVB0CC_*) | RESERVE_HWPM_LEGACY 0xb0cc0101, ALLOC_PMA_STREAM 0xb0cc0105, EXEC_REG_OPS 0xb0cc010a, BIND_PM_RESOURCES 0xb0cc0107, … (18 cmds) | CapProfiling | Hardware performance-counter / reg access; side-channel + stability risk |
| Debug (NV83DE_*) | DEBUG_READ_ALL_SM_ERROR_STATES 0x83de030c, SET_EXCEPTION_MASK 0x83de0309, CLEAR 0x83de0310 | compUtil | SM debugger surface |
| RegOps (NV2080 EXEC_REG_OPS) | 0x20800122 | CapProfiling | direct register poke |
| GR profiling | GR_GET_SM_TO_GPC_TPC_MAPPINGS 0x2080120f, FB_GET_FS_INFO 0x20801346 | CapProfiling | topology leak |
| Power (NV90CC_*) | POWER_REQUEST/RELEASE_FEATURES 0x90cc0301/2 | CapProfiling | power-state control |
| Fabric/IMEX (NV00F1/00FB/00E0/00FD remote) | GET_FABRIC_EVENTS 0xf10001, VALIDATE 0xfb0101, IMPORT_MEM 0xe00102, ATTACH_REMOTE_GPU 0xfd0106 | CapFabricIMEXManagement | cross-node memory import |

> Action: introduce a control-command allowlist mirroring nvproxy's table, and map
> nvproxy's cap tags to a per-VM capability set (default = compute-only, i.e. nvproxy's
> `compUtil`). This is the single biggest lever for "secure multi-tenant" parity.

### 2c. Control commands present in nvgpu ABI but added only in later driver ABIs

These are registered by nvproxy for newer drivers (≥555/565/570/575). nvkvm forwards
them anyway (default-allow), so they are *not* correctness gaps for 575, but note them
for the allowlist: `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2` (0x20800513),
`FB_QUERY_DRAM_ENCRYPTION_*` (V575 variants), `NVLINK_GET_PLATFORM_INFO` (0x20803083),
`BUS_GET_PCIE_CPL_ATOMICS_CAPS` (0x20801830), `GPU_GET_RECOVERY_ACTION` (0x208001b2),
`NVB0CC_CTRL_CMD_RESERVE_CCU_PROF` (0xb0cc0119), `NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE`
(0xcb33010c), `NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL` (0xda0006).

---

## 3. Alloc classes (hClass)

nvproxy registers ~90 classes (base + later ABIs incl. Blackwell). nvkvm forwards any
class; correctness depends on the guest sizing the alloc-params buffer. The guest
`nvkvm_main.c` size table covers ~22 classes; classes with explicit `alloc_parms_size`
work regardless; `rmAllocNoParams` classes work raw.

### 3a. Classes nvproxy sizes but nvkvm's guest table does NOT (correctness if size=0)

| Class | ID | nvproxy param struct | Importance |
|---|---|---|---|
| TURING_COMPUTE_A | 0xc5c0 | NV_GR_ALLOCATION_PARAMETERS | **High** — compute object; on cuLaunchKernel path |
| AMPERE_COMPUTE_A / _B | 0xc6c0 / 0xc7c0 | NV_GR_ALLOCATION_PARAMETERS | **High** |
| ADA_COMPUTE_A | 0xc9c0 | NV_GR_ALLOCATION_PARAMETERS | **High** |
| HOPPER_COMPUTE_A | 0xcbc0 | NV_GR_ALLOCATION_PARAMETERS | **High** |
| TURING_A / AMPERE_A / ADA_A / HOPPER_A | 0xc597/c697/c997/cb97 | NV_GR_ALLOCATION_PARAMETERS | Med (graphics) |
| FERMI_TWOD_A | 0x902d | NV_GR_ALLOCATION_PARAMETERS | Low (gfx) |
| KEPLER_INLINE_TO_MEMORY_B | 0xa140 | NV_GR_ALLOCATION_PARAMETERS | Low |
| NV50_P2P | 0x503b | NV503B_ALLOC_PARAMETERS | Med (NCCL P2P) |
| NV50_THIRD_PARTY_P2P | 0x503c | NV503C_ALLOC_PARAMETERS | Low |
| NV_SEMAPHORE_SURFACE | 0xda | NV_SEMAPHORE_SURFACE_ALLOC_PARAMETERS | Med (sync) |
| NV_MEMORY_FABRIC | 0xf8 | NV00F8_ALLOCATION_PARAMETERS | Low (fabric) |
| NV_MEMORY_MULTICAST_FABRIC | 0xfd | NV00FD_ALLOCATION_PARAMETERS(_V545/590) | Low |
| NV_MEMORY_MAPPER | 0xfe | NV_MEMORY_MAPPER_ALLOCATION_PARAMS(_V550/555) | Low |
| MAXWELL_PROFILER_DEVICE | 0xb2cc | NVB2CC_ALLOC_PARAMETERS | Low (profiling) |
| NV2081_BINAPI | 0x2081 | NV2081_ALLOC_PARAMETERS | Low |
| GF100_DISP_SW | 0x9072 | NV9072_ALLOCATION_PARAMETERS | Low (display) |
| NVxxB0_VIDEO_DECODER | b8b0/c4b0/c6b0/c7b0/c9b0 | NV_BSP_ALLOCATION_PARAMETERS | Low (NVDEC) |
| NVxxB7_VIDEO_ENCODER | c4b7/c7b7/c9b7 | NV_MSENC_ALLOCATION_PARAMETERS | Low (NVENC) |
| NVxxD1_VIDEO_NVJPG | b8d1/c4d1/c9d1 | NV_NVJPG_ALLOCATION_PARAMETERS | Low |
| NVxxFA_VIDEO_OFA | b8fa/c6fa/c7fa/c9fa | NV_OFA_ALLOCATION_PARAMETERS(_V545) | Low |
| NV_CONFIDENTIAL_COMPUTE | 0xcb33 | NV_CONFIDENTIAL_COMPUTE_ALLOC_PARAMS | Low (CC) |
| HOPPER_USERMODE_A | 0xc661 | NV_HOPPER_USERMODE_A_PARAMS | Med — usermode doorbell; nvkvm relies on explicit size |
| NV01_MEMORY_VIRTUAL | 0x70 | rmAllocMemoryVirtual (special) | Med (gfx/video) |
| FERMI_CONTEXT_SHARE_A | 0x9067 | rmAllocContextShare (special) | in table ✓ |
| KEPLER_CHANNEL_GROUP_A | 0xa06c | rmAllocChannelGroup (special) | in table ✓ |
| TURING/AMPERE/HOPPER_CHANNEL_GPFIFO_A | c46f/c56f/c86f | rmAllocChannel(_V570) (special) | in table ✓ |
| GT200_DEBUGGER | 0x83de | rmAllocSMDebuggerSession (special) | in table ✓ |
| NV01_EVENT_OS_EVENT | 0x79 | rmAllocEventOSEvent (embedded fd) | in table ✓ |
| NV01_ROOT* / NV01_ROOT_CLIENT | 0x0/1/41 | rmAllocRootClient (special) | handled in qemu gate ✓ |

> Note: in practice libcuda passes `alloc_parms_size` for compute classes, so cuInit→
> cuLaunchKernel works today (per MEMORY: matmul passes). The **High** ratings flag
> latent fragility — any client (cuDNN/cuGraph/graphics/Vulkan) that leaves size=0
> will silently get zeroed params. Adding the NV_GR/compute entries to the guest table
> removes the dependency on caller behavior.

### 3b. Classes nvproxy registers as `rmAllocNoParams` (forward raw — no gap)

GF100_PROFILER 0x90cc, GF100_ZBC_CLEAR 0x9096, GF100_SUBDEVICE_INFOROM 0x90e7,
VOLTA_USERMODE_A 0xc361, TURING_USERMODE_A 0xc461, GF100_SUBDEVICE_MASTER 0x90e6,
HOPPER_SEC2_WORK_LAUNCH_A 0xcba2, NV04_DISPLAY_COMMON 0x73, NV20_SUBDEVICE_DIAG 0x208f.
These need no param translation; nvkvm raw-forward is correct.

### 3c. Blackwell + IMEX classes (later ABIs, not needed for 575)

BLACKWELL_* (compute/gpfifo/dma/usermode/inline) 0xcd**/0xc9b5/0xca**, NV_IMEX_SESSION,
NV_MEMORY_EXPORT 0xe0, NV_MEMORY_FABRIC_IMPORTED_REF 0xfb, NVENC_SW_SESSION. Not gaps
for the 575 target; add when bumping ABI.

---

## 4. UVM ioctls

nvproxy registers 33 UVM ioctls (base table, line 192-224). nvkvm's
`nvkvm_uvm_schema[]` is a **default-deny allowlist** — good design parity. Diff:

### Present in nvproxy, MISSING from nvkvm schema (= nvkvm denies them)

| UVM ioctl | # | Meaning | Importance |
|---|---|---|---|
| UVM_MIGRATE | 51 | migrate VA range between processors | **present** in schema (size 48) ✓ |
| UVM_TOOLS_READ_PROCESS_MEMORY | 62 | read another process's UVM mem | **Deliberately denied** (cross-proc read) — keep denied. Low/never |
| UVM_TOOLS_WRITE_PROCESS_MEMORY | 63 | write another process's UVM mem | **Deliberately denied** — keep denied. Low/never |

> nvproxy itself does **not** register UVM 62/63 either (they are absent from its base
> uvmIoctl map), so nvkvm and nvproxy agree: these are unsupported by both. No gap.

Comparing entry-by-entry, **nvkvm's schema covers every UVM ioctl nvproxy registers**
(INITIALIZE, DEINITIALIZE, CREATE/DESTROY_RANGE_GROUP, REGISTER/UNREGISTER_GPU_VASPACE,
REGISTER/UNREGISTER_CHANNEL, ENABLE/DISABLE_PEER_ACCESS, SET_RANGE_GROUP,
MAP_EXTERNAL_ALLOCATION, FREE, REGISTER/UNREGISTER_GPU, PAGEABLE_MEM_ACCESS(_ON_GPU),
SET/UNSET_PREFERRED_LOCATION, ENABLE/DISABLE_READ_DUPLICATION, SET/UNSET_ACCESSED_BY,
MIGRATE, MIGRATE_RANGE_GROUP, MAP_DYNAMIC_PARALLELISM_REGION, UNMAP_EXTERNAL,
ALLOC_SEMAPHORE_POOL, VALIDATE_VA_RANGE, CREATE_EXTERNAL_RANGE, MM_INITIALIZE).

**No missing UVM ioctls.** Remaining UVM work is hardening (several schema entries use
`min_size 0`, i.e. size not validated: 44/45/53/65/66/68's second-arg). Importance: Med.

---

## 5. mmap handlers

nvproxy implements two mmap mappables:

| Handler | File | Purpose | nvkvm |
|---|---|---|---|
| frontend mmap (`/dev/nvidia*`, `/dev/nvidiactl`) | frontend_mmap.go | maps BAR / RM memory cookies into the app | nvkvm: `nvkvm_mmap_host.c` double-mmap + KVM memslot. **Present.** |
| uvm mmap (`/dev/nvidia-uvm`) | uvm_mmap.go | UVM managed-memory mappings | nvkvm: handled in QEMU UVM path. **Present.** |

Both categories are implemented. Gaps are *capacity/architecture*, not missing
handlers, and are already tracked in MEMORY:

- Single shared bump-allocator mmap window → concurrent-process collisions
  (multiproc_collision_blocker). Importance: **High** for multi-tenant, but it's an
  nvkvm-internal scaling issue, not an nvproxy op we fail to implement.
- PCI-BAR reservation TODO (gpa_window_design). Med.
- nvproxy validates mmap length/offset against the RM mmap_context cookie
  (`frontend_mmap.go` mmapMu/mmapLength); confirm nvkvm enforces equivalent
  length/offset bounds on the host mmap to avoid mapping beyond the cookie. Med (security).

No nvproxy mmap *handler* is missing.

---

## 6. procfs / sysfs / device-file behaviors

nvproxy synthesizes a small procfs tree (`procfs.go`):

| Path | Content | nvkvm | Importance |
|---|---|---|---|
| `/proc/driver/nvidia/params` | driver param string (`nvp.procDriverNvidiaParams`) | **MISSING** | **Med** — read by nvidia-smi/NVML and some CUDA init paths |
| `/proc/driver/nvidia/capabilities/fabric-imex-mgmt` | IMEX cap device minor | **MISSING** | Low — fabric/IMEX only |

nvproxy device-file set: `/dev/nvidiactl`, `/dev/nvidia#`, `/dev/nvidia-uvm`,
`/dev/nvidia-uvm-tools`, and (cap) `/dev/nvidia-caps/*`. nvkvm creates `nvidiactl`,
`nvidia%d`, `nvidia-uvm`, and a uvm-tools node (nvkvm_main.c device_create calls) —
parity on the core nodes. `/dev/nvidia-caps/*` (capability FS) is **not** modeled;
needed only for cgroup/IMEX device-access control — Low for single-tenant compute.

> Action (Med): synthesize `/proc/driver/nvidia/params` in the guest module (it can be
> a static/forwarded string) so NVML-based tooling that stats it doesn't degrade.

---

## Appendix: ops both sides intentionally DON'T support

- UVM_TOOLS_READ/WRITE_PROCESS_MEMORY (62/63): absent in nvproxy *and* explicitly
  denied by nvkvm — agreement, keep denied (cross-process memory access).
- Display/Vulkan graphics classes & video-codec classes: nvproxy supports them behind
  CapGraphics/CapVideo; nvkvm forwards but doesn't size their params. Not on the CUDA
  critical path — Low unless the project adds graphics/codec use cases.
- dmabuf export (`NV_ESC_EXPORT_TO_DMABUF_FD`): nvkvm-only extension; nvproxy has no
  dmabuf path. Not a gap.

## Bottom line

nvkvm's **functional** coverage of the CUDA critical path is complete (all frontend
ioctls present; UVM allowlist complete; compute alloc + channels + mmap implemented —
matmul/nvidia-smi proven). The real gaps are:

1. **Security model**: nvkvm is default-allow on frontend ioctls, control commands, and
   alloc classes. nvproxy's per-category allowlists + capability tags are the parity
   target — this is the work that backs a "secure multi-tenant VM" claim.
2. **Latent correctness**: ~10 control commands and the NV_GR/compute alloc classes
   that nvproxy translates specially are forwarded raw / sized only when the caller
   provides a size. Add the missing special handlers + guest alloc-param table entries.
3. **Compat polish**: `/proc/driver/nvidia/params`, OS_UNIX export/import FDs (CUDA
   IPC), P2P caps handler (NCCL), GET_PID_INFO ns-parity.
