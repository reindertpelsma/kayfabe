# nvkvm multi-driver-version ABI support — implementation spec

Date: 2026-05-30
Author: ABI-profile design pass
Scope: design + concrete code-site map. Read-only against nvkvm src/; all claims
verified against the bundled gVisor nvproxy
(`gvisor/pkg/sentry/devices/nvproxy/version.go`,
`gvisor/pkg/sentry/devices/nvproxy/nvconf/version.go`,
`gvisor/pkg/abi/nvgpu/{uvm,frontend,classes,ctrl,nvgpu}.go`).

This builds on `docs/audits/version_abi_support.md` (the survey). That doc
established *that* nvkvm is single-version and *where* it pins 575. This doc
picks the concrete target versions, enumerates the exact field/size deltas
between them (cross-checked against nvproxy structs), maps each delta to the
nvkvm source site that must consult a profile instead of a constant, and
proposes the `nvkvm_abi_profile` struct shape.

---

## 0. Background: how nvproxy keys ABI on version

nvproxy keeps **every** struct variant compiled in simultaneously (e.g. both
`NV_CHANNEL_ALLOC_PARAMS` and `NV_CHANNEL_ALLOC_PARAMS_V570`) and selects which
one a given driver uses via a per-version `driverABI` built by **inheritance**:
each version constructor calls its parent and mutates four maps
(`version.go:100-107`):

```go
type driverABI struct {
    frontendIoctl   map[uint32]frontendIoctlHandler   // keyed by IOC_NR(cmd)
    uvmIoctl        map[uint32]uvmIoctlHandler         // keyed by cmd
    controlCmd      map[uint32]controlCmdHandler       // keyed by NVOS54.Cmd
    allocationClass map[nvgpu.ClassID]allocationClassHandler // keyed by NVOS64.HClass
}
```

Registration + exact lookup: `addDriverABI(major,minor,patch, sumX86, sumARM,
cons)` (`version.go:149`) into `var abis map[nvconf.DriverVersion]…`
(`version.go:142`); runtime selection is an **exact map lookup** keyed on the
parsed `{major,minor,patch}` — no range/nearest matching (see
`nvproxy.go:Register`, summarised in `version_abi_support.md` §2d).

nvkvm should adopt the same principle: a small set of *exact* supported
versions, each mapped to a vetted `nvkvm_abi_profile`, and refuse everything
else.

---

## 1. The three target versions

### 1a. nvproxy's registered OPEN-driver families (the menu)

From `addDriverABI` calls in `version.go`:

| Family | NVIDIA role | Registered patch versions (`addDriverABI`) |
|---|---|---|
| **535.x** | data-center **LTSB** | 535.129.03, .183.06, .247.01, .261.03, .274.02, .288.01 (`version.go:797-802`) |
| 545/555/560/565 | intermediate "main-branch" | constructors only — **not** registered patches |
| **550.x** | prior production | 550.90.12 only (`version.go:927`); 550.40.07/.54.14/.90.07 are constructors carrying the V550 deltas |
| **570.x** | production | 570.124.06, .133.20, .172.08, .195.03 (`version.go:1028-1034`) |
| **575.x** | nvkvm's current target | **none registered**; only 575.51.02 exists as an intermediate constructor (`version.go:1037`). 575.51.03 (nvkvm's exact build) is *not* in nvproxy. |
| **580.x** | latest production / next-LTSB candidate | 580.65.06, .105.08, .126.09, .126.20 (`version.go:1057-1085`) |
| 590.x | bleeding edge | 590.48.01 (`version.go:1105`) |

### 1b. Chosen 3 targets — **535.274.02, 570.133.20, 580.65.06**

Justification against the three criteria (a) nvproxy table exists, (b)
buildable open-gpu-kernel-modules tag, (c) spans meaningful ABI deltas:

1. **535.274.02** — the **LTSB** family. Highest product value (data-center
   customers pin LTSB). nvproxy has it registered (`version.go:801`).
   open-gpu-kernel-modules has a `535.274.02` tag. Crucially it is the
   **pre-V550 / pre-V570 baseline**: it uses the *base* `NV_CHANNEL_ALLOC_PARAMS`
   (no `TPCConfigID`), the *base* `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` (1-entry
   per-GPU array, **not** the 256-entry/offset-9248 layout), base
   `UVM_MIGRATE`/`UVM_SET_PREFERRED_LOCATION`, and base
   `NV_MEMORY_ALLOCATION_PARAMS` (no `numa_node`). Targeting it exercises *every*
   high-risk struct switch — it is the largest delta vs nvkvm's current 575
   headers.

   - Note: the 535 baseline constructor is `v535_104_05`/`v535_113_01`
     (`version.go:164,793`); the registered patch 535.274.02 inherits it
     unchanged for struct layouts and only differs in checksums. So
     "535 profile" == the `v535_113_01` struct set.

2. **570.133.20** — the **production** family and the **closest sibling to
   nvkvm's actual 575.51.03 build**. nvproxy registers it (`version.go:1029`)
   and open-gpu-kernel-modules has a `570.133.20` tag. It is the branch point
   from which 575.51.02 inherits (`version.go:1038` `abi := v570_133_20()`).
   Its struct layout set (**V550 UVM** + **V570 channel** + **pre-580
   VASPACE/NVOS46** + **V545 memory**) is *identical to what nvkvm's headers
   already encode for 575*. This is the lowest-effort target and is the one that
   validates "575 == 570-layout + 3 control-cmd deltas" end-to-end.

3. **580.65.06** — the **latest production / next-LTSB candidate**. nvproxy
   registers it (`version.go:1057`) and open-gpu-kernel-modules has a
   `580.65.06` tag. It is the *only* family that introduces the **post-575**
   struct deltas: `NVOS46_PARAMETERS_V580` (RM_MAP_MEMORY_DMA grows
   8 bytes: `Flags2`+`KindOverride`) and `NV_VASPACE_ALLOCATION_PARAMETERS_V580`
   (FERMI_VASPACE grows 8 bytes: `Pasid`+pad). Targeting it forces the profile
   to drive forward as well as backward from the 575 baseline.

This trio brackets nvkvm's 575 build on both sides (535 below, 580 above) and
covers all five "silent-corruption" switches identified in
`version_abi_support.md` §4.3. **575.51.03 itself stays the build-default**;
it shares 570's layouts, so its profile == the 570 profile with the 575
control-cmd allowlist delta.

Deferred (low value / high churn): 545/555/560/565 (unqualified intermediates),
550.90.12 (superseded; its only struct deltas vs 570 are already covered by 570),
590 (bleeding edge; adds `UVM_FREE_V590`/`UVM_UNREGISTER_CHANNEL_V590` etc.).

---

## 2. Exact ABI deltas between 535 / 570 / 580

All sizes below are computed from the nvproxy Go structs (which carry
`structs.HostLayout` and so match the C ABI). Primitive sizes:
`Handle`=`uint32`=4, `NvUUID`=`[16]uint8`=16 (`nvgpu.go:97`),
`NV_MAX_DEVICES`=32, `NV_MAX_SUBDEVICES`=8 (`nvgpu.go:59-60`), so
`UVM_MAX_GPUS`=32 and `UVM_MAX_GPUS_V2`=256 (`uvm.go:887-888`).
`UvmGpuMappingAttributes` = `NvUUID`(16) + 5×`uint32`(20) = **36 bytes**
(`uvm.go:892-900`).

### 2a. UVM_MAP_EXTERNAL_ALLOCATION

| | 535 (base) | 570 / 575 / 580 (V550) |
|---|---|---|
| nvproxy struct | `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` (`uvm.go:298-309`) | `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS_V550` (`uvm.go:332-343`) |
| per-GPU array | `[UVM_MAX_GPUS]` = **1×36** | `[UVM_MAX_GPUS_V2]` = **256×36 = 9216** |
| layout | Base8, Length8, Offset8, PerGPU(36), count8, **RMCtrlFD@68**, HClient, HMemory, RMStatus | Base8, Length8, Offset8, PerGPU(9216), count8, **RMCtrlFD@9248**, HClient@9252, HMemory@9256, RMStatus@9260 |
| `sizeof` | **84** | **9264** |
| `RMCtrlFD` (embedded frontend fd) offset | **68** | **9248** |

The V550 switch happens at 550.54.14 (`version.go:899`:
`abi.uvmIoctl[UVM_MAP_EXTERNAL_ALLOCATION] = … UVM_MAP_EXTERNAL_ALLOCATION_PARAMS_V550`).
535 predates it → uses the 1-entry base.

### 2b. UVM_ALLOC_SEMAPHORE_POOL

| | 535 (base) | 570 / 575 / 580 (V550) |
|---|---|---|
| nvproxy struct | `UVM_ALLOC_SEMAPHORE_POOL_PARAMS` (`uvm.go:769-777`) | `…_PARAMS_V550` (`uvm.go:790-798`) |
| per-GPU array | `[UVM_MAX_GPUS]` = 1×36 | `[UVM_MAX_GPUS_V2]` = 256×36 = 9216 |
| layout | Base8, Length8, PerGPU(36), count8, RMStatus4, pad4 | Base8, Length8, PerGPU(9216), count8, RMStatus4, pad4 |
| `sizeof` | **68** | **9248** |

Switch at 550.54.14 (`version.go:898`). This is the literal **9248** all over
nvkvm.

### 2c. UVM_SET_PREFERRED_LOCATION and UVM_MIGRATE

| struct | 535 (base) | 570/575/580 (V550) |
|---|---|---|
| `UVM_SET_PREFERRED_LOCATION_PARAMS` | RequestedBase8, Length8, PreferredLocation(NvUUID 16), RMStatus4, pad4 → **40B** (`uvm.go:474-481`) | `…_V550`: adds `PreferredCPUNumaNode int32` *replacing* the pad → **40B** but byte 32 is now a NUMA node not pad (`uvm.go:494-501`) |
| `UVM_MIGRATE_PARAMS` | `CPUNumaNode uint32` (`uvm.go:611-625`) | `…_V550`: `CPUNumaNode int32` (signed), same size **88B** (`uvm.go:641-655`) |

Switch at 550.40.07 (`version.go:860-861`). **Same size, different
semantics/signedness** — low memory-safety risk, but the profile should still
record which variant so a future writeback of `CPUNumaNode` is interpreted
correctly. (nvkvm uses one layout today, `uvm.h:183-199`.)

### 2d. Channel alloc params (KEPLER/TURING/AMPERE/HOPPER `_CHANNEL_GPFIFO_A`)

| | 535/550/565 (base) | 570 / 575 / 580 (V570) |
|---|---|---|
| nvproxy struct | `NV_CHANNEL_ALLOC_PARAMS` (`classes.go:447-475`) | `NV_CHANNEL_ALLOC_PARAMS_V570` (`classes.go:481-486`) |
| extra tail | — | embeds base + `TPCConfigID uint32` + `uint32` pad → **+8 bytes** |
| `EngineType` | field at its base offset (the runlist-binding field) | **unchanged offset** (embedded base is first) |

Switch at 570.86.15 (`version.go:996-1001`): the TURING/AMPERE/HOPPER/BLACKWELL
GPFIFO classes move from `rmAllocChannel` (base) to `rmAllocChannelV570`. Note
`EngineType` (the field behind the historic runlist bug,
memory `gpfifo_schedule_runlist_bug`) stays at the same offset because V570
*appends*; the only risk is the trailing 8 bytes. **535 must use the base
struct (no `TPCConfigID`).**

### 2e. VASPACE alloc params (FERMI_VASPACE_A)

| | 535 / 570 / 575 (pre-580) | 580 (V580) |
|---|---|---|
| nvproxy struct | `NV_VASPACE_ALLOCATION_PARAMETERS` (`classes.go:371-381`) | `NV_VASPACE_ALLOCATION_PARAMETERS_V580` (`classes.go:387-392`) |
| layout | Index4, Flags4, VASize8, VAStartInternal8, VALimitInternal8, BigPageSize4, pad4, VABase8 → **48B** | base(48) + `Pasid uint32` + pad4 → **56B** |

Switch at 580.65.06 (`version.go:1060`). Only 580 differs; 535 and 570 share
the pre-580 layout. (This is the write-back struct behind
memory `cuctxcreate_401_diagnosis`.)

### 2f. NVOS46 (RM_MAP_MEMORY_DMA) and NVOS47 (RM_UNMAP_MEMORY_DMA)

| struct | 535 (base) | 570/575 | 580 |
|---|---|---|---|
| `NVOS46_PARAMETERS` (`frontend.go:625-638`) | 56B, Status@48 | 56B, Status@48 | `NVOS46_PARAMETERS_V580` (`frontend.go:654-669`): inserts `Flags2`+`KindOverride` after `Flags` → **64B**, DmaOffset@48, **Status@56** |
| `NVOS47_PARAMETERS` | base `NVOS47_PARAMETERS` (`frontend.go:684`) | `NVOS47_PARAMETERS_V550` (`frontend.go:711`) | same V550 |

- NVOS46 V580 switch: 580.65.06 (`version.go:1059`). **The Status offset moves
  48→56 at 580** — directly affects the stub status-offset switch (§3).
- NVOS47 V550 switch: 550.40.07 (`version.go:847`). 535 uses the base
  `NVOS47_PARAMETERS`; 570/575/580 use `_V550`. Both are 48B in nvkvm's reading
  (status@40), so this is layout-semantic, not a size change for the stub —
  **verify sizeof on the 535 SDK** before relying on equal size.

### 2g. NV_MEMORY_ALLOCATION_PARAMS (NV01_MEMORY_SYSTEM / LOCAL_USER / NV50_MEMORY_VIRTUAL)

| | 535 (base) | 570/575/580 (V545) |
|---|---|---|
| struct | `NV_MEMORY_ALLOCATION_PARAMS` | `NV_MEMORY_ALLOCATION_PARAMS_V545` (adds `numa_node`+pad) |
| switch | — | 545.23.06 (`version.go:813-815`) |

535 predates 545 → uses the base (no `numa_node`). 570/575/580 use V545.
nvkvm currently hardcodes V545 (`nvgpu.h:597`).

### 2h. NV00DE_ALLOC_PARAMETERS (RM_USER_SHARED_DATA)

| 535 (base) | 570/575/580 (V545) |
|---|---|
| base `NV00DE_ALLOC_PARAMETERS` | `NV00DE_ALLOC_PARAMETERS_V545` (single u64) — switch at 545.23.06 (`version.go:810`) |

### 2i. Core NVOS frontend structs that are version-STABLE across 535→580

These do **not** change across the three targets (sizes asserted in
`tests/abi_parity`); the profile may treat them as constants but the stub's
status-offset switch (§3) still bakes them per-NR:

- `NVOS21_PARAMETERS` (32B, `frontend.go:300`), `NVOS64_PARAMETERS` (48B,
  `frontend.go:788`), `NVOS55_PARAMETERS` (28B, status@24, `frontend.go:371`),
  `NVOS33_PARAMETERS`/`IoctlNVOS33ParametersWithFD` (56B, status@40 fd@48,
  `frontend.go:526,544`), `IoctlNVOS02ParametersWithFD` (56B, status@40 fd@48,
  `frontend.go:209`), `NVOS34_PARAMETERS` (`frontend.go:572`),
  `NVOS54_PARAMETERS` (`frontend.go:738`), `NVOS56_PARAMETERS`
  (`frontend.go:762`), `NVOS57_PARAMETERS` (`frontend.go:395`).
- The only frontend size that moves in this range is **NVOS46 at 580** (§2f).

### 2j. RM control-command allowlist deltas (compUtil surface)

Control cmds added/removed walking 535 → 570 → 580 (compute-relevant only;
graphics/video/profiling/fabric rows would gate on caps not version). Citing
the `abi.controlCmd[…]` mutations:

- **535 → 570** (cumulative through 545/550/555/560/565/570 constructors):
  adds `NV0000_CTRL_CMD_GPU_GET_ACTIVE_DEVICE_IDS`,
  `NV00DE_CTRL_CMD_REQUEST_DATA_POLL` (`version.go:808-809`);
  `NV0000_CTRL_CMD_GPU_ASYNC_ATTACH_ID`/`_WAIT_ATTACH_ID`,
  `NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL`,
  `NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE`, and replaces
  `NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS` handler with the V550 variant
  (`version.go:848-859`); `NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE`
  (`version.go:914`); **deletes** `NVC36F_CTRL_GET_CLASS_ENGINEID` at 555
  (`version.go:933`); adds DRAM-encryption query cmds at 570
  (`version.go:993-995`).
- **570 → 575** (the v575_51_02 delta, `version.go:1039-1043`): **deletes**
  `NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT` and `_STATUS`;
  **adds** their `_V575` variants + `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2`.
- **575 → 580** (`version.go:1063-1064`): adds
  `NV2080_CTRL_CMD_GPU_GET_SKYLINE_INFO`, `NV2080_CTRL_CMD_ECC_GET_REPAIR_STATUS`.

→ The allowlist must be **per-profile**, not one frozen 575 snapshot.

### 2k. Alloc-class list deltas

- **535 → 570** adds (cumulative): memory-export/fabric classes at 545
  (`NV_MEMORY_EXPORT`, `NV_MEMORY_MULTICAST_FABRIC` V545,
  `NV*_VIDEO_OFA` V545, `version.go:811-819`); IMEX/fabric +
  `NV_MEMORY_MAPPER` at 550 (`version.go:862-865`); Blackwell-gen 1 classes at
  560 (`BLACKWELL_CHANNEL_GPFIFO_A`, `BLACKWELL_DMA_COPY_A`, `BLACKWELL_A/_COMPUTE_A`,
  `version.go:950-954`); Blackwell-gen 2 + usermode at 570
  (`BLACKWELL_B`, `BLACKWELL_COMPUTE_B`, `BLACKWELL_DMA_COPY_B`,
  `BLACKWELL_CHANNEL_GPFIFO_B`, `BLACKWELL_USERMODE_A`, `version.go:999-1004`).
  Also at 570 the existing TURING/AMPERE/HOPPER GPFIFO classes switch to the
  **V570 channel handler** (§2d).
- **575 → 580** adds video encoders `NVCEB7_VIDEO_ENCODER`,
  `NVD1B7_VIDEO_ENCODER` (`version.go:1061-1062`) and re-keys `FERMI_VASPACE_A`
  to the **V580 vaspace handler** (§2e).

→ Class allowlist + per-class alloc-param size table must be per-profile.

---

## 3. Mapping each delta to the nvkvm code site

Each row: the delta from §2, the file:line where nvkvm currently hardcodes the
575 value, and what it must consult on the profile instead. (Line numbers
verified 2026-05-30; the survey doc’s numbers had drifted a few lines.)

### 3a. UVM map-ext / sem-pool array + offset-9248 (§2a, §2b)

| Site | Current hardcode | Profile field |
|---|---|---|
| `src/stub/nvkvm_stub.c:857` | `uvm_embedded_fd_off = 9248;` (V550) | `prof->uvm_map_ext_fd_off` (68 for 535, 9248 for 570/580) |
| `src/qemu/nvkvm_isolate_handlers.c:450` | UVM `min_size` row `UVM_MAP_EXTERNAL_ALLOCATION = 9264` | `prof->uvm_min_size[UVM_MAP_EXTERNAL_ALLOCATION]` (84 vs 9264) |
| `src/qemu/nvkvm_isolate_handlers.c:465` | `UVM_ALLOC_SEMAPHORE_POOL = 9248` | `prof->uvm_min_size[UVM_ALLOC_SEMAPHORE_POOL]` (68 vs 9248) |
| `src/qemu/nvkvm_isolate_handlers.c:1861` | SEM_POOL intent cap "9248" | `prof->uvm_min_size[UVM_ALLOC_SEMAPHORE_POOL]` |
| `src/stub/nvkvm_stub.c:1435` | SEM_POOL intent cap "9248" | profile id passed down → same field |
| `src/common/nvkvm_proto.h:513` | comment "9248 bytes; QEMU validates size exactly" | doc-only; QEMU validates via profile |
| `src/abi/uvm.h:162-168, 275-278` | `UVM_MAX_GPUS_V2 (32*8)`, both structs V550-only | add base (1-entry) variants `uvm_map_external_allocation_params_v535` / `uvm_alloc_semaphore_pool_params_v535`; keep both compiled in (nvproxy-style) |

### 3b. UVM SET_PREFERRED_LOCATION / MIGRATE (§2c)

| `src/abi/uvm.h:183-199` | single (V550) layout | add base variants; `prof->uvm_pref_loc_layout` / `uvm_migrate_layout` selector (semantics only; size equal). Stub status offsets for these are computed from `param_size`, so no stub switch change needed, but the guest writeback of `CPUNumaNode` must respect signedness per profile. |

### 3c. Channel alloc params V570 vs base (§2d)

| Site | Current | Profile field |
|---|---|---|
| `src/guest/nvkvm_main.c:1076-1080` | `TURING/AMPERE/HOPPER_CHANNEL_GPFIFO_A → sizeof(nv_channel_alloc_params_v570)` | `prof->chan_alloc_size` (base size for 535, V570 size = base+8 for 570/580) |
| `src/abi/nvgpu.h:562-592` | `nv_channel_alloc_params_v570` only | add base `nv_channel_alloc_params` (no `TPCConfigID`); profile selects |
| also `nvkvm_main.c:1122,1341` | the nvos64 RM_ALLOC size switches (same hClass cases) | same `prof->chan_alloc_size` |

### 3d. VASPACE pre-580 vs V580 (§2e)

| Site | Current | Profile field |
|---|---|---|
| `src/guest/nvkvm_main.c:1062-1063` | `FERMI_VASPACE_A → sizeof(nv_vaspace_allocation_parameters)` (48B) | `prof->vaspace_alloc_size` (48 for 535/570, 56 for 580) |
| `src/abi/nvgpu.h:505-514` | pre-580 struct only | add `nv_vaspace_allocation_parameters_v580` (+Pasid+pad); profile selects |
| nvos64 path `nvkvm_main.c:1122+` | same FERMI_VASPACE case | same field |

### 3e. NVOS46 status offset (§2f)

| Site | Current | Profile field |
|---|---|---|
| `src/stub/nvkvm_stub.c:1082` | `case 0x57: off = 48;` (NVOS46 status@48) | `prof->nvos_status_off[NR_0x57]` (48 for 535/570, **56** for 580) |
| `src/stub/nvkvm_stub.c:1071-1095` | the whole NR→status-offset switch baked "575 SDK" | drive from `prof->nvos_status_off[]` (only 0x57 differs in this trio, but the table makes the rest future-proof) |
| `src/abi/nvgpu.h:313-325` | `nvos46_parameters` 56B only | add `nvos46_parameters_v580` (64B); used by guest sanitizer + stub |
| `src/stub/nvkvm_stub.c:899-907` | frontend embedded-fd offsets (RM_MAP_MEMORY fd@48 etc.) | unaffected by 580 NVOS46 (RM_MAP_MEMORY is 0x4e, not the DMA 0x57); leave but verify per profile |

### 3f. NV_MEMORY_ALLOCATION_PARAMS / NV00DE V545 (§2g, §2h)

| Site | Current | Profile field |
|---|---|---|
| `src/guest/nvkvm_main.c:1059-1060` | `RM_USER_SHARED_DATA → nv00de_alloc_parameters_v545` | `prof->nv00de_alloc_size` (base for 535, V545 for 570/580) |
| `src/guest/nvkvm_main.c:1065-1068` | `NV50_MEMORY_VIRTUAL/LOCAL_USER/SYSTEM → nv_memory_allocation_params_v545` | `prof->mem_alloc_size` (base for 535, V545 for 570/580) |
| `src/abi/nvgpu.h:597-623` | V545 only | add base variants; profile selects |

### 3g. NV2080 PID_INFO stride (§ — not version-keyed in this trio)

| `src/qemu/nvkvm_isolate_handlers.c:561` (`NVKVM_PIDINFO_STRIDE 72u`) and the guest comment `src/guest/nvkvm_main.c:479-480` | 72 | `prof->pidinfo_stride`. **Verify**: nvproxy does not version `NV2080_CTRL_GPU_PID_INFO` across 535-580 in this tree, so 72 is likely stable — but route it through the profile anyway so a future delta is one line. |

### 3h. Allowlists (§2j, §2k)

| Site | Current | Profile field |
|---|---|---|
| `src/qemu/nvkvm_fe_alloc_allowlist.h:18+` (`nvkvm_fe_nr_allowlist[]`, alloc-class list) | one "575-ABI" snapshot | `prof->fe_nr_allow{,_n}`, `prof->alloc_class_allow{,_n}` — one array per profile (535/570/580) generated from `nvproxy.SupportedIoctlsNumbers(version)` |
| `src/qemu/nvkvm_ctrl_allowlist.h` | one "575-ABI" snapshot | `prof->ctrl_allow{,_n}` per profile |
| `src/qemu/nvkvm_isolate_handlers.c:439-469` | UVM `min_size` table baked at 575 sizes | `prof->uvm_min_size[]` per profile |

### 3i. Version parse + select (the entry point)

| Site | Current | Change |
|---|---|---|
| `src/qemu/virtio_nvgpu.c:933-937` | reads `ver.version_string` into `nv->driver_version`, never branched | parse into `{maj,min,patch}`; call `nvkvm_abi_select()`; **refuse** if unsupported (nvproxy-style exact match) |
| `src/qemu/virtio_nvgpu.c:958-959` | copies string into shm `ctrl->driver_version` | additionally write the selected **profile id** (small enum) into the shm control block so the guest module uses the identical profile |
| `src/guest/nvkvm_virtio.c` (driver_version readback) | reads string | read the profile id and resolve the same `nvkvm_abi_profile` guest-side |

---

## 4. Recommended `nvkvm_abi_profile` struct shape

A single source of truth in `src/abi/abi_profile.{h,c}`, selected by a small
enum keyed on the parsed `{major,minor,patch}` (mirrors nvproxy's `abis[ver]`).
The **profile id** (not the version string) crosses the shm boundary so QEMU
and the guest module agree.

```c
/* src/abi/abi_profile.h */
enum nvkvm_abi_id {
    NVKVM_ABI_535 = 0,   /* 535.x LTSB  — base channel, base UVM (1-entry),
                            base memory, pre-580 vaspace, base nvos46/nvos47 */
    NVKVM_ABI_570 = 1,   /* 570.x / 575.x — V570 channel, V550 UVM (9248),
                            V545 memory, pre-580 vaspace, 56B nvos46 */
    NVKVM_ABI_580 = 2,   /* 580.x — V570 channel, V550 UVM, V545 memory,
                            V580 vaspace (+Pasid), V580 nvos46 (64B) */
    NVKVM_ABI_COUNT
};

/* UVM ioctl count; index by the small UVM cmd number used in min_size today. */
#define NVKVM_NR_UVM 76

struct nvkvm_abi_profile {
    enum nvkvm_abi_id id;
    uint16_t major, minor, patch;          /* the exact pinned version */

    /* ---- UVM ---- */
    uint32_t uvm_map_ext_fd_off;           /* RMCtrlFD offset: 68 | 9248 */
    uint16_t uvm_min_size[NVKVM_NR_UVM];   /* per-cmd min param size (drives
                                              isolate_handlers + stub caps) */
    uint8_t  uvm_pref_loc_v550;            /* 0 base / 1 V550 (numa node) */
    uint8_t  uvm_migrate_v550;             /* 0 base / 1 V550 (signed numa) */

    /* ---- RM_ALLOC per-hClass param sizes (drives guest size tables) ---- */
    uint32_t chan_alloc_size;              /* GPFIFO: base | base+8 (V570) */
    uint32_t vaspace_alloc_size;           /* FERMI_VASPACE: 48 | 56 (V580) */
    uint32_t mem_alloc_size;               /* NV_MEMORY_ALLOCATION: base|V545 */
    uint32_t nv00de_alloc_size;            /* RM_USER_SHARED_DATA: base|V545 */
    /* (other hClass sizes are version-stable; keep as constants or here too) */

    /* ---- frontend NVOS status/fd offsets (drives stub switch) ---- */
    uint16_t nvos_status_off[256];         /* keyed by IOC_NR; only 0x57 (NVOS46)
                                              differs 48->56 at 580 in this trio */
    /* fe_embedded_fd_off table similarly if a future delta moves an fd */

    /* ---- misc strides ---- */
    uint32_t pidinfo_stride;               /* 72 (stable in 535-580) */

    /* ---- allowlists (QEMU trust boundary; pointers into per-profile arrays) */
    const uint8_t  *fe_nr_allow;       size_t fe_nr_allow_n;
    const uint32_t *ctrl_allow;        size_t ctrl_allow_n;
    const uint32_t *alloc_class_allow; size_t alloc_class_allow_n;
};

/* exact-match select; returns NULL on unsupported version (refuse to start). */
const struct nvkvm_abi_profile *nvkvm_abi_select(int major, int minor, int patch);
const struct nvkvm_abi_profile *nvkvm_abi_by_id(enum nvkvm_abi_id id);
```

Notes on the shape:

- **Sizes/offsets, not struct types, cross the boundary.** Both QEMU and the
  guest only ever need byte sizes and embedded-fd/status offsets at runtime;
  the C struct *variants* stay compiled-in headers (nvproxy keeps both variants
  too — §0). This keeps the shm contract just `enum nvkvm_abi_id`.
- **`uvm_min_size[]` and `nvos_status_off[]` are tables, not scalars**, because
  the existing code already switches on UVM cmd / IOC_NR; replacing a switch
  body with a table lookup is the minimal-diff refactor of
  `isolate_handlers.c:439-469` and `stub.c:1071-1095`.
- **Allowlists are pointers** so each profile references a statically-generated
  array (generate from `nvproxy.SupportedIoctlsNumbers(<version>)` per §2j/§2k);
  the QEMU gate stays default-deny.
- The default build target **575.51.03 maps to `NVKVM_ABI_570`** (identical
  layouts; the 3 control-cmd deltas of §2j only affect the allowlist array bound
  to that profile — if 575 vs 570 allowlists must differ, add a 4th id
  `NVKVM_ABI_575` that shares all size fields with 570 but points at the 575
  control allowlist).

### 4.1 Rollout order (cheapest first)

1. **Phase 0** — parse `virtio_nvgpu.c:933` version into `{maj,min,patch}`,
   add `nvkvm_abi_select`, **refuse** unknown versions. No struct work; removes
   the silent-skew footgun immediately.
2. **Phase 1 — `NVKVM_ABI_570`** (covers 570.133.20 *and* 575.51.03): wire all
   the size/offset/allowlist sites to the profile with today's 575 values. Pure
   refactor, behavior-preserving; proves the table machinery.
3. **Phase 2 — `NVKVM_ABI_580`**: add `nv_vaspace_…_v580` (56B),
   `nvos46_parameters_v580` (64B, status@56), bump `nvos_status_off[0x57]`,
   add 580 allowlist entries (§2j/§2k). Smallest forward delta.
4. **Phase 3 — `NVKVM_ABI_535`** (LTSB, largest delta): add base
   `nv_channel_alloc_params`, base `uvm_*` 1-entry structs (`uvm_map_ext_fd_off
   = 68`), base `nv_memory_allocation_params`, base `nv00de`, and the trimmed
   535 allowlists. Exercises all five high-risk switches.

### 4.2 Test gate

Extend `tests/abi_parity` to assert, per profile, `sizeof()` of every selected
variant against the nvproxy value for that version (cross-check via
`nvproxy.SupportedIoctls(version).{Frontend,Uvm,Allocation}Infos`), so layout
drift is caught at build time. Optionally record each version's `.run` SHA256
(nvproxy already carries them: 535.274.02 `3b4ef54f…`, 570.133.20 `1253d17b…`,
580.65.06 `04b10867…` — `version.go:801,1029,1057`) for a host-driver
bit-identity gate à la `ExpectedDriverChecksum` (`version.go:1195`).

---

## Appendix — verification index

- nvproxy mechanism: `version.go:100-107` (driverABI), `:149-159` (addDriverABI),
  `:1182-1242` (lookup APIs); `nvconf/version.go:26-91` (DriverVersion).
- Per-version deltas: `version.go:797-802` (535 patches), `:805-841` (545),
  `:844-894` (550.40.07 — NVOS47_V550, MIGRATE/PREF_LOC V550),
  `:896-910` (550.54.14 — SEM_POOL/MAP_EXT V550), `:990-1026` (570.86.15 —
  channel V570 + Blackwell), `:1028-1034` (570 patches),
  `:1037-1055` (575.51.02 control deltas), `:1057-1078` (580.65.06 — NVOS46_V580
  + VASPACE_V580), `:1080-1085` (580 patches).
- Struct bodies: `uvm.go:298-309/332-343` (MAP_EXT base/V550),
  `uvm.go:769-777/790-798` (SEM_POOL base/V550), `uvm.go:474-501` (PREF_LOC),
  `uvm.go:611-655` (MIGRATE), `uvm.go:887-900` (MAX_GPUS, mapping attrs),
  `nvgpu.go:59-60,97` (NV_MAX_DEVICES/SUBDEVICES, NvUUID),
  `classes.go:371-392` (VASPACE base/V580), `classes.go:447-486` (channel
  base/V570), `frontend.go:625-679` (NVOS46 base/V580),
  `frontend.go:684-731` (NVOS47 base/V550).
- nvkvm hardcode sites: `src/stub/nvkvm_stub.c:857,1071-1095,1435`;
  `src/qemu/nvkvm_isolate_handlers.c:450,465,561,1861`;
  `src/guest/nvkvm_main.c:1052-1104,1122,1341`;
  `src/abi/nvgpu.h:148-325,505-623`; `src/abi/uvm.h:162-199,275-278`;
  `src/qemu/virtio_nvgpu.c:933-937,958-959`;
  `src/qemu/nvkvm_fe_alloc_allowlist.h`, `src/qemu/nvkvm_ctrl_allowlist.h`.
