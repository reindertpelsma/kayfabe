**STATUS: RESEARCH, 2026-10-13.**

**DECISION (owner, 2026-10-13): kayfabe does not present itself as a vGPU, and may never.** Reasons (from this research and the discussion): a stock build refuses a VF in a hypervisor guest (needs a compile-time `GRID_BUILD` and an allowlisted PCI ID, so custom builds or NVIDIA's licensed guest drivers, which the project cannot obtain for Windows); a vGPU guest has only the displayless class (no real display); the real skew translation is in the closed host plugin, so it cannot be reused. The skew handling kayfabe needs is made proper in kayfabe itself: per-guest-branch tables generated from ogkm tags, the host-version matrix, the clean fallback outside the measured set, and a correct fn-1 negotiation checked by an oracle built from NVIDIA's own code (`claude/fn1-oracle-20261013`).
Revisit only if (a) a displayless profile for custom (`GRID_BUILD`) guests becomes worth its cost, or (b) a stock-driver path to NVIDIA's skew handling is found. Until then this document is a source of facts (negotiation semantics, VF detection, displayless class), not a plan.

# V3 vGPU / guest-host version skew — what ogkm 595.84 says, and what kayfabe presents

> Read-only source research. No GPU, QEMU or host was used. Question set from the coordinator, plus one
> addition from the owner (the vGPU code in ogkm counts as active, shipping guest-side code; separate a
> LINUX guest running an ogkm-built driver from a WINDOWS guest running NVIDIA's closed driver; state
> whether the vGPU guest sends the first-RPC version fields and what it expects back).

**Labels.** `MEASURED` = read in source, the file:line is given. `INFERRED` = my reading of what the
measured lines imply. `REPO` = a statement in the kayfabe tree at `origin/integration/windows-20261010`
(`8006168d4`), read, not re-run. **Nothing here was run.**

**Paths.** `ogkm-595:` = `/workspace/ogkm-tars/open-gpu-kernel-modules-595.84` (public ogkm 595.84 tree, the
only ogkm version used; no host reads were needed). ⊘ 2026-10-21: every citation below was written as a bare `ogkm` + colon
(CI's untagged-citation ratchet); it is now tagged `ogkm-595:`, the one tree this document names. The tag was applied
mechanically from that statement; the citations were not re-read. `kf:` = the kayfabe repo at the revision above.

---

## 0. Short answers

**Q1 — vGPU guest vs GSP-client guest.** *(MEASURED)* The RM decides "I am a vGPU guest" from **one BAR0
read**, `NV_PMC_BOOT_1[17:16] == 2` (`_VGPU_VF`), at HAL-selection time; that value picks
`RM_RUNTIME_VARIANT_VF`, which wins over the GSP-firmware policy and sets `isVirtual=1`, `isGspClient=0`.
There is no PCI-ID or config-space signature for the decision itself. A VF guest **does not load or boot
GSP**: `request_firmware` is forced off when (hypervisor CPUID signature seen) and (PMC_BOOT_1 says VF) and
(chip is VGX-firmware-capable), the `KernelGsp` HAL entries are stubs for VF, and the boot is replaced by
"set up three shared buffers, write a VF scratch register pair, ring a VF doorbell". The RPC messages then
travel in a 128 KiB buffer shared with a host-side **plugin**, not in a GSP message queue. The message
header, `OBJRPC`, `rpcWriteCommonHeader` and the per-RPC functions are the **same code** as the GSP-client
path; only the transport and the static-info call differ (`GET_STATIC_DATA` chunks, per-version
deserialisers, vs one `GET_GSP_STATIC_INFO` memcpy of a compile-time struct).

**Q2 — version skew.** *(MEASURED)* (a) **vGPU-guest-only**: a negotiated "VGX" RPC version
(`VGX_MAJOR/MINOR` = `0x2D.0x03` in 595.84) sent in `SET_GUEST_SYSTEM_INFO` (fn 1); the plugin answers in
the **same struct** and may return `rpc_result_private != OK` with its own pair; the guest retries once at
the host's pair if that is >= the guest's built-in minimum (`0x25.0x0E`) and otherwise logs "host VGX
version is older than the minimum ... Disabling vGPU"; the agreed pair then selects, through a table of
~190 version-range overrides, which per-version RPC serialiser and static-data deserialiser every later
message uses. (b) **also in an ordinary GSP-client driver**: the same `RmRpcSetGuestSystemInfo` runs after
GSP boots (`kernel_gsp.c:5060`), so the same handshake exists; and the one hard version check is
**driver `NV_VERSION_STRING` vs the `.fwversion` section of the GSP firmware ELF** (`kernel_gsp.c:5571-5624`),
mismatch -> `NV_ERR_INVALID_DATA`, GSP never boots. The GSP-client path has **no** tolerance for skew
beyond that: it requires driver == firmware, exactly.

**Q3 — display in vGPU.** *(MEASURED)* **No NV0073 display common in a VF guest.** `PDB_PROP_KDISP_IS_MISSING`
is hard-set TRUE for the VF variant, `kdispConstructEngine` returns `NV_ERR_NOT_SUPPORTED`, so there is no
`KernelDisplay`; `NV04_DISPLAY_COMMON` allocs return `NV_ERR_NOT_SUPPORTED`. What a vGPU guest gets
instead is the **`NVA083_GRID_DISPLAYLESS`** class (virtual heads, a max-resolution, a fixed EDID blob named
"NVIDIA VGX"), answered **locally** from numbers the plugin delivers inside `GET_STATIC_DATA`
(`VGPU_DISPLAYLESS_INFO`). The guest also does not own the console: it is "never primary VGA", and a
"switch to VGA" is an RPC to the host.

**Q4 — what kayfabe can reuse.** The concrete pieces are: the fn-1 request/reply structs and the
down-negotiation protocol (`rpc_result_private` + host pair in body + one retry); the
`rpc_iGrp_ipVersions` table (the guest-side map from agreed pair -> exact message layout); the
min/max-supported constants (`0x25.0x0E` internal; external `0x18.01 .. 0x1C.01`) and the internal->external
mapping table; the result-code space (`NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH =
NV_ERR_LIB_RM_VERSION_MISMATCH`); the VF control/response/message buffer ABI (`dev_vgpu.h`, `nv_sriov_defines.h`).
*Does it run in a stock driver?* The **shared** pieces (fn 1/fn 64, header, version-range HAL) **do** run
in an ordinary stock GSP-client driver. The **vGPU-only** pieces are *compiled into* an ogkm build
(`srcs.mk`, `RMCFG_MODULE_RPC=1`, VF HAL rows) but a stock build **refuses to bind a VF in a hypervisor
guest** (`rm_is_vgpu_supported_device`: needs `GRID_BUILD` at module-compile time and a PCI device+subsystem ID
on a 1124-entry allowlist; nothing in the tree sets `GRID_BUILD`). What the source cannot tell: licensing,
packaging, whether NVIDIA's shipped Linux guest driver is built with `GRID_BUILD`, and anything about the
closed Windows driver.

**Q5 — what kayfabe presents today.** *(REPO)* Fn-1 reply: `rpc_result=OK`, body = zeroed struct with only
the **vgx pair of the `guest-driver=` row** (`580.65.06` -> `0x2B.0x13`; measured per tag from ogkm). The
host driver (`595.91.07`, `0x2D.0x03` if it were speaking) is **never reported to the guest**; it is read by
`NV_ESC_CHECK_VERSION_STR` on the host's `/dev/nvidiactl` and used only to pick host-side encoders.
`NV2080_CTRL_CMD_GSP_GET_FEATURES.firmwareVersion` is the **guest's own `NV_VERSION_STRING` latched from
its fn 1** (`580.65.06` for a Linux 580.65.06 guest; the Windows guest's `580.88` is keyed as its Linux
twin `580.65.06`). There is no other firmware-version field in `GspStaticConfigInfo` at 595.84. On any
disagreement the device **refuses by name with `rpc_result = NV_ERR_NOT_SUPPORTED`** and cannot do the
protocol's own down-negotiation (it cannot make `rpc_result` and `rpc_result_private` differ).

---

## 1. Q1 evidence — how the RM decides, and the guest-side flow

### 1.1 The decision (MEASURED)

| Step | What happens | Source |
|---|---|---|
| a | Before the GPU object exists, RM reads `NV_PMC_BOOT_0/1/42` from BAR0 | `ogkm-595: src/nvidia/src/kernel/gpu_mgr/gpu_mgr.c:1228-1262` |
| b | `isVirtual = (NV_PMC_BOOT_1[17:16] == _VF)`; `_VF = 2`, `_PV = 1` | `gpu_mgr.c:993-1011` (`gpumgrGetGpuHalFactorOfVirtual`); field in `ogkm-595: src/common/inc/swref/published/nv_ref.h:145-148` |
| c | Runtime variant: `PLATFORM_GSP/DCE -> UCODE; else if isVirtual -> VF; else if isFwClient -> PF_KERNEL_ONLY; else PF_MONOLITHIC`. **VF is tested before the firmware policy**, so a VF never becomes a GSP client | `gpu_mgr.c:1279-1292` |
| d | `RmVariantHal: VF` row exists in the generated HAL, so the VF variant is real, selectable code, not a stub | `ogkm-595: src/nvidia/generated/g_chips2halspec_nvoc.c:190-203`, `g_chips2halspec_nvoc.h:107-124` |
| e | NVOC field `isVirtual = TRUE` for VF; `isGspClient = FALSE` for VF (and TRUE for PF_KERNEL_ONLY except Tegra display) | `ogkm-595: src/nvidia/generated/g_gpu_nvoc.c:560-585` |
| f | Macros: `IS_VIRTUAL(g) = g->isVirtual`; `IS_VIRTUAL_WITH_SRIOV(g)` from a **second** `PMC_BOOT_1` read in `gpuConstruct` and again in `gpuDetermineVirtualMode` (asserts both agree) | `ogkm-595: src/nvidia/generated/g_gpu_nvoc.h:5845-5857`; `ogkm-595: src/nvidia/src/kernel/gpu/gpu.c:378-392`, `:4342-4380` |
| g | `bVgpuGspPluginOffloadEnabled` for a VF = `PDB_PROP_GPU_VGPU_OFFLOAD_CAPABLE`, which is TRUE only for **AD10x, GH100, GB10x/GB20x** (Blackwell). Turing/Ampere VF guests have no supported path in 595.84 | `ogkm-595: src/nvidia/src/kernel/gpu/gpu_registry.c:139-142`; `g_gpu_nvoc.c:402-412` |
| h | The pre-GSP "legacy" vGPU is gone: `RMCFG_FEATURE_VIRTUALIZATION_LEGACY 0`; `initRpcInfrastructure_VGPU` returns `NV_ERR_NOT_SUPPORTED` when plugin offload is off | `ogkm-595: src/nvidia/generated/rmconfig.h:288`; `ogkm-595: src/nvidia/src/kernel/vgpu/rpc.c:1660-1685`, `:1494-1505` |

INFERRED: a hypervisor-hidden guest (CPUID signature absent) still gets `isVirtual=1` from step b, because
step b does not look at CPUID; but the firmware-request gate in 1.2 does. See *Unknowns U6*.

### 1.2 Who does NOT load GSP in a vGPU guest (MEASURED)

* `rm_set_rm_firmware_requested` is called per device at probe (`ogkm-595: kernel-open/nvidia/nv-pci.c:2101`). It
  sets `request_firmware = TRUE` and then **FALSE if `rm_get_is_gsp_capable_vgpu`** (`ogkm-595:
  src/nvidia/arch/nvalloc/unix/src/osapi.c:5036-5047`).
* `rm_get_is_gsp_capable_vgpu` = (`pHypervisor->bIsHVMGuest`) and (`PMC_BOOT_1 == VF`) and
  (`gpumgrIsVgxRmFirmwareCapableChip(PMC_BOOT_42)`) (`osapi.c:4995-5030`). The chip test is `arch >= GB100 ||
  arch == GH100 || arch == AD100` (`gpu_mgr.c:1071-1075`).
* `RmInitAdapter` fetches the firmware image only if `request_firmware` (`ogkm-595:
  src/nvidia/arch/nvalloc/unix/src/osinit.c:2062-2080`); no image -> no `kgspInitRm`, no
  `_kgspBootGspRm` (`ogkm-595: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4709`, `:4791`).
* `kgspConfigureFalcon` and the other `KernelGsp` HAL entries bind to a stub for the VF row (`ogkm-595:
  src/nvidia/generated/g_kernel_gsp_nvoc.c:587-770`, e.g. `kgspConfigureFalcon_f2d351` at `:589`).
* The boot of a *GSP-RM proxy* is skipped too: `if (!IS_GSP_CLIENT && !IS_VIRTUAL) gpuBootGspRmProxy`
  (`osinit.c:2131-2135`).
* Hypervisor detection is x86 CPUID leaf scan `0x40000000..0x40001000` for the signatures in the
  `HYPERVISOR_OPS` table (`KVMKVMKVM`, ...) (`ogkm-595: src/nvidia/src/kernel/virtualization/hypervisor/hypervisor.c:82-132,
  134-165`; `.../kvm/kvm.c:19-23`); for KVM `hypervisorPostDetection` always says "HVM guest"
  (`kvm.c:42-46`).

### 1.3 How the VF guest talks to the host (MEASURED)

Entry: `gpuPostConstruct` -> (if `IS_VIRTUAL`) `vgpuCreateObject` (`ogkm-595: src/nvidia/src/kernel/gpu/gpu.c:535-565`,
`ogkm-595: src/nvidia/src/kernel/vgpu/objvgpu.c:49-129`) -> `initRpcInfrastructure_VGPU` (`rpc.c:1660-1771`):

1. `initRpcObject` — the **same** `OBJRPC` constructor the GSP-client uses; it pre-sets the RPC version to the
   guest's own `VGX_*` (`ogkm-595: src/nvidia/src/kernel/rmapi/rpc_common.c:96-107`).
2. `vgpuGspSetupBuffers` (`rpc.c:1494-1612`) installs `_rpcSendMessage_VGPUGSP` / `_rpcRecvPoll_VGPUGSP`
   as the transport (`rpc.c:1514-1515`, `:1818-1830`) and allocates:
   * **control buffer** 4 KiB sysmem (guest writes, plugin reads), `VGPU_GSP_CTRL_BUF_V1`: `version`
     (guest writes `VGPU_GSP_CTRL_BUF_V2_VERSION = 2`, `rpc.c:1320`, `dev_vgpu.h:212`), `requestId`,
     `responseBuf`/`msgBuf`/`sharedMem`/`eventBuf`/`debugBuf` addresses, `getEventBuf`, `guestEccStatus`,
     `sysmemBitMapTablePfn`, `guestOsType`, `requestedGspCaps`, hibernation indices (`ogkm-595:
     src/nvidia/inc/kernel/vgpu/dev_vgpu.h:176-242`);
   * **response buffer** 4 KiB (plugin writes), `VGPU_GSP_RESPONSE_BUF_V1`: `responseId`, `putEventBuf`,
     `hostEccStatus`, `usmType`, `enabledGspCaps`, hibernation indices (`dev_vgpu.h:244-280`);
   * **message buffer** 128 KiB (FB via BAR2, or sysmem before BAR2 is up), `pRpc->largeRpcSize = 128K`
     (`rpc.c:1145-1180` `_setupGspMessageBuffer`, `:1541`);
   * event ring, shared memory, optional debug and hibernation buffers.
3. `_vgpuGspSetupCommunicationWithPlugin` (`rpc.c:1308-1445`; control-buffer `version = 2` at `:1320`) fills the control buffer (including `guestOsType`
   via `vgpuUpdateGuestOsType`, `rpc.c:216-244`, which encodes ARCH/OS/page size; `OS_LINUX` is the only OS the
   open code produces, but the field also defines `OS_WINDOWS10 = 3`, `dev_vgpu.h:45`) and then
   `_vgpuGspSendSetupRequest` (`rpc.c:1276-1291`): **write the control-buffer address into VF scratch
   registers, ring the setup doorbell, poll for `responseId == requestId`.**
4. Per RPC: `requestId = ++seq`, store fence, ring the message doorbell, poll `responseId`
   (`rpc.c:1250-1274`, `:1227-1248`). Timeout default 12.5 s (`dev_vgpu.h:324`).
5. Immediately after: `NV_RM_RPC_SET_GUEST_SYSTEM_INFO` (`rpc.c:1696-1722`), then
   `NV_RM_RPC_GET_STATIC_DATA` (`objvgpu.c:116`).

BAR0 registers touched (the VF's own BAR0; `virtualRegPhysOffset = 0` for a VF, `0xB80000` on a PF —
`ogkm-595: src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:94-100`, macro `GPU_VREG_WR32` in
`g_gpu_access_nvoc.h:256-259`):

| Register | Offset in VF BAR0 | Used for | Source |
|---|---|---|---|
| `NV_PMC_BOOT_0/1/42` | chip-global | HAL select, VF detection | `gpu_mgr.c:1228-1262` |
| `NV_VIRTUAL_FUNCTION_PRIV_MAILBOX_SCRATCH(2)` / `(3)` | `0x2108` / `0x210C` | control-buffer address LO / HI (`NV_VF_SCRATCH_REGISTER_GUEST_RPC_LO/HI = 2/3`) | `ogkm-595: src/nvidia/src/kernel/vgpu/arch/ampere/rpcga102.c:35-43`; `nv_sriov_defines.h:93-96`; `dev_vm.h` (ga100) `:132` |
| `NV_VIRTUAL_FUNCTION_PRIV_DOORBELL` | `0x2200` | value `NVBIT(0)` = setup request, `NVBIT(1)` = RPC message | `rpcga102.c:45-48`; `nv_sriov_defines.h:37-45`; `dev_vm.h` (ga100) `:126` |
| `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_*` | `0x1000-0x1640` | VF interrupt tree | `dev_vm.h` (tu102) `:31-66` |

GSP-client flow for contrast (MEASURED): `kgspInitRm` loads the ELF, checks `.fwversion`, builds radix-3 and
the WPR meta (`GSP_FW_WPR_META_MAGIC 0xdc3aae21371a60b3`, `REVISION 1`, `kernel_gsp_tu102.c:861-862`,
`gsp_fw_wpr_meta.h:211-212`), boots falcon/FSP/SEC2, and thereafter uses `NV_PGSP_QUEUE_HEAD(i)` as the
command-queue doorbell (`ogkm-595: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:327-338`;
send path `kernel_gsp.c:395-440`) with `_kgspRpcSendMessage`/`_kgspRpcRecvPoll` installed on the same
`OBJRPC` (`kernel_gsp.c:3014-3031`). So: **BAR0 footprint of a VF guest = `PMC_BOOT_*` + 2 scratch + 1
doorbell + the VF interrupt tree + normal `NV_VIRTUAL_FUNCTION_PRIV` register traffic; BAR0 footprint of a
GSP client = full falcon/FSP/SEC2/PGSP/queue register set.** (Whether a VF guest touches additional BAR0
registers outside these during engine init was not exhaustively traced - *Unknowns U4*.)

### 1.4 Static info: VF vs GSP client (MEASURED)

| | GSP client | vGPU guest |
|---|---|---|
| RPC | `GET_GSP_STATIC_INFO` (fn 65) | `GET_STATIC_DATA` (chunked by `offset`/`size`), then `GET_CONSOLIDATED_GR_STATIC_INFO` |
| Decode | one `portMemCopy(pSCI, sizeof(*pSCI), rpcInfo, sizeof(*rpcInfo))` of the compile-time `GspStaticConfigInfo` (`rpc.c:9962-9994`) - **no versioning at all** | `getStaticDataRpcBufferSize` + `copyPayloadToStaticData` through the **per-version** deserialisers `deserialize_VGPU_STATIC_DATA_v25_0E / v27_00 / v27_01 / v2A_07 / v2B_08 / v2D_00` (`ogkm-595: src/nvidia/src/kernel/vgpu/rpcstructurecopy.c:1372-1703`) |
| Call chosen by | fixed | `rpcGetStaticData` = `_v25_0E` for `0x250E0000-0x2700FFFF`, `_v27_01` for `0x27010000-0x2CFFFFFF`, `_v2D_00` for `>= 0x2D000000` (`ogkm-595: src/nvidia/generated/g_rpc_private.h:3246-3251`) |

---

## 2. Q2 evidence — version negotiation, skew handling, errors

### 2.1 Constants (MEASURED)

* `VGX_MAJOR_VERSION_NUMBER 0x2D`, `VGX_MINOR_VERSION_NUMBER 0x03` (`ogkm-595:
  src/nvidia/inc/kernel/vgpu/vgpu_version.h:33-34`). Wire packing `RPC_VERSION_FROM_VGX_VERSION(major, minor) =
  major<<24 | minor<<16` (`:28-31`). Named releases: `VGPU_17_0 = 0x25`, `18_0 = 0x29.0x0B`, `19_0 =
  0x2B.0x13` (`:36-40`).
* **Min supported internal host pair** `NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL = 0x25.0x0E`
  (`ogkm-595: src/nvidia/generated/g_rpc-structures.h:12822-12825`, included via `vgpu_version.h:94-97`).
* External (migration-compat) window: `MIN 0x18.01`, `MAX 0x1C.01` (`vgpu_version.h:110-117`), and the
  internal->external mapping table `NV_VGPU_GRIDSW_INTERNAL_TO_EXTERNAL_VERSION_MAPPING` (`:52-80`, e.g.
  `{{0x2D,0x00},{0x2D,0x03},{0x1C,0x01}}`). Registry keys `RM_SET_VGPU_VERSION_MIN/MAX` let a user narrow the
  window; the result is handed to the vGPU manager (`ogkm-595: src/nvidia/src/kernel/vgpu/objvgpu.c:206-234`;
  `kernel_vgpu_mgr.c:1953-1985`). These are **host-manager migration** knobs, not the guest handshake.
* Message envelope: `header_version = 3.0` (`rpc_headers.h:56-59`), `signature = 0x43505256`
  (`:61`); the guest writes both (`rpc_common.c:154-156`). `ogkm` never **reads** either back on the guest side
  (grep over `src/` finds writers only) - *INFERRED*: the plugin validates them, the guest does not.
* Fn 1 struct (`rpc_set_guest_system_info_v03_00`, 6 x `NvU32` then 3 x `char[0x100]`):
  `vgxVersionMajorNum, vgxVersionMinorNum, guestDriverVersionBufferLength, guestVersionBufferLength,
  guestTitleBufferLength, guestClNum, guestDriverVersion[256], guestVersion[256], guestTitle[256]`
  (`ogkm-595: src/nvidia/generated/g_rpc-structures.h:36-47`). Fn 64 `SET_GUEST_SYSTEM_INFO_EXT`:
  `v15_02 = {guestDriverBranch[256], domain, bus, device}`, `v25_1B` adds `gridBuildCsp` (`:51-66`),
  chosen by version range: `<= 0x251AFFFF -> v15_02`, `>= 0x251B0000 -> v25_1B` (`g_rpc_private.h:3432-3435`).

### 2.2 The handshake itself - `RmRpcSetGuestSystemInfo` (`ogkm-595: src/nvidia/src/kernel/vgpu/rpc.c:8836-9017`) (MEASURED)

1. Skip rule: on a multi-GPU box the second and later GPUs reuse the first GPU's agreed pair, **except** for
   GSP clients and for VGPU-GSP-offload guests, which always handshake (`rpc.c:8838-8880`).
2. Build the request: `guestDriverVersion = NV_VERSION_STRING`, `guestVersion = NV_BUILD_BRANCH_VERSION`,
   `guestTitle = NV_DISPLAY_DRIVER_TITLE` (each padded to 256), `guestClNum = NV_BUILD_CHANGELIST_NUM`,
   `vgx{Major,Minor}Num = VGX_{MAJOR,MINOR}_VERSION_NUMBER` (`rpc.c:8897-8943`).
3. `_issueRpcAndWait`. The envelope has **two** result fields: `rpc_result` and `rpc_result_private`
   (`g_rpc-message-header.h:41-52`). `_issueRpcAndWait` returns `rpc_result` verbatim if it is not
   `SUCCESS` (values `< 0xFF000000` are RM status codes, passed through; larger are mapped to
   `NV_ERR_GENERIC`) (`rpc.c:2008-2025`) - **a non-zero `rpc_result` aborts the handshake**.
4. If `rpc_result == OK` but `rpc_result_private != OK` (`rpc.c:8947-8949`): the guest takes
   `status = rpc_result_private`, and **if the pair now sitting in the message body differs from its own**:
   * and `RPC_VERSION(host pair) >= RPC_VERSION(0x25.0x0E)` -> prints "Guest VGX version is newer than the host
     VGX version ... Retrying with the VGX version requested by the host" and **re-issues the same buffer
     once** (the buffer now carries the host's pair) (`rpc.c:8953-8968`). The retried `status` is **not**
     re-checked against `rpc_result_private` (only `rpc_result` through `_issueRpcAndWait`);
   * else -> "The host version is too old. Minimum required host version is 0x25.0x0E",
     `NV_RM_RPC_LOG(... "Incompatible Guest/Host drivers: Host VGX version is older than the minimum version
     supported by the Guest. Disabling vGPU." ...)`, status stays the private status (`rpc.c:8969-8982`).
   * *Note (INFERRED):* the code never tests that the host pair is **below** the guest's, so any non-matching
     host pair >= min triggers the "retry at host's pair" branch; the semantics only make sense if the plugin
     returns a private error only when the host is older than the guest, and silently accepts an older guest
     otherwise.
5. On `status == OK`: if a pair is already stored (`rpcVgxVersion`, a process-global) and differs from this
   reply's -> `NV_ERR_INVALID_STATE` (`rpc.c:8987-8995`); else store the reply's pair, call
   `rpcSetIpVersion(RPC_VERSION_FROM_VGX_VERSION(reply pair))` (`rpc.c:9004-9006`) and send fn 64
   (`rpc.c:9008-9013`), whose status becomes the function's status.
6. **`rpcSetIpVersion`** (`rpc.c:982-996`) re-runs `rpc_iGrp_ipVersions_getInfo` / `_Wrapup`
   (`g_rpc_private.h:3045-3427`, 191 `IsIPVersionInRange` overrides): every versioned RPC function pointer
   (`rpcAllocMemory_v13_01`, `rpcGetStaticData_v2D_00`, `rpcAllocObject_v2C_01` ... and the structure-copy
   table `rpcstructurecopy_iGrp_ipVersions_getInfo`, `g_rpcstructurecopy_private.h:1704-...`) is re-selected
   for the **agreed** pair. The version table runs `v03_00 ... v2D_01`
   (`g_rpc_private.h:3596-4011`); a pair below the min or above `v2D_01` still maps to the nearest lower range
   because every range is `[start, 0xFFFFFFFF]`.

What the guest does if the handshake fails at init: `initRpcInfrastructure_VGPU` logs "SET_GUEST_SYSTEM_INFO :
failed" and, for `NV_ERR_NOT_SUPPORTED`, `nvErrorLog_va(VGPU_START_ERROR, "vGPU type is not supported")`
(`rpc.c:1711-1727`); `gpuPostConstruct` turns `NV_ERR_LIB_RM_VERSION_MISMATCH` (0x6A, "Library version doesn't
match driver version", `ogkm-595: kernel-open/common/inc/nvstatuscodes.h:135`) into `nvErrorLog_va(VGPU_START_ERROR,
"Guest driver is incompatible with host driver")` (`gpu.c:556-566`) and fails device construction. The
protocol's dedicated code for this is `NV_VGPU_MSG_RESULT_GUEST_HOST_DRIVER_MISMATCH =
NV_ERR_LIB_RM_VERSION_MISMATCH` (`ogkm-595: src/nvidia/inc/kernel/vgpu/rpc_headers.h:116`); nothing in ogkm
**uses** it (it is for the closed plugin) - *INFERRED*.

### 2.3 (a) vGPU-guest-only vs (b) also-in-GSP-client (MEASURED)

| Piece | vGPU guest | Ordinary GSP-client driver |
|---|---|---|
| `RmRpcSetGuestSystemInfo` body | runs (`rpc.c:1711`, `:1028/1045` on resume) | **runs** after GSP boot: `NV_RM_RPC_SET_GUEST_SYSTEM_INFO` then `GET_GSP_STATIC_INFO` (`kernel_gsp.c:5060-5075`) |
| Multi-GPU "skip" rule | exempted for offload guests | exempted for `IS_GSP_CLIENT` (`rpc.c:8859`) |
| rpc version table select | from the agreed pair | `initRpcObject` sets it to own `VGX_*` (`rpc_common.c:101-104`); fn-1 reply may change it |
| Control-buffer/doorbell/scratch ABI, event ring, shared memory | yes | no |
| `GET_STATIC_DATA` + per-version struct copy | yes | no (`GET_GSP_STATIC_INFO`, unversioned memcpy) |
| `.fwversion` ELF check | no (no ELF) | **yes** |
| `NV_ERR_LIB_RM_VERSION_MISMATCH` -> VGPU_START_ERROR | yes (`gpu.c:560`) | not reached |

**The one comparison of the driver's version against the firmware's in an ordinary GSP-client driver**
(MEASURED, `ogkm-595: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:5571-5624`): `_kgspFwContainerVerifyVersion`
reads the ELF section `.fwversion` (`GSP_VERSION_SECTION_NAME`, `g_kernel_gsp_nvoc.h:271`) of the firmware
image the OS handed over (`nvidia/<NV_VERSION_STRING>/gsp_*.bin`, `ogkm-595: kernel-open/nvidia/nv.c:27`) and
requires `fwversionSize == strlen(NV_VERSION_STRING)+1` **and** `memcmp == 0`. Mismatch -> logs "GSP firmware
image version mismatch: got version X, expected version Y" (or "unknown or malformed") and returns
`NV_ERR_INVALID_DATA` (`:5607-5622`). Called for the main image (`:5689`, from `kgspInitRm` `:4924`) and for the
log ELF (`:3657`, `:4939`). Failure aborts `kgspInitRm` -> `RmInitAdapter` fails. **There is no tolerance,
range or fallback.** Beyond the ELF string, the only other driver<->firmware interlocks in the open code
are the structs the CPU-RM writes for the (closed) firmware to validate - `GspFwWprMeta` with
`magic/revision` (`kernel_gsp_tu102.c:861-862`) and `GSP_SR_META` `magic/revision`
(`gsp_fw_sr_meta.h:29-30`) - and the fn-1 handshake above. What GSP firmware does on a mismatch of those is
in closed firmware (*Unknowns U1*).

Other (non-vGPU, non-GSP) version comparisons in ogkm, for completeness (MEASURED): userspace library vs
kernel module `NV_ESC_CHECK_VERSION_STR` - exact-match or "relaxed" (compare to first `.`), mismatch logs
"API mismatch" and returns `NV_ERR_GENERIC` (`ogkm-595: src/nvidia/arch/nvalloc/unix/src/osapi.c:1128-1228`);
Tegra DCE: logs a **warning only** "Possibly incompatible DCE RM version" and continues (`ogkm-595:
src/nvidia/src/kernel/gpu/dce_client/dce_client_rpc.c:703-720`) - the only place ogkm tolerates a
driver-string disagreement.

---

## 3. Q3 evidence — display in a vGPU guest (MEASURED)

* `PDB_PROP_KDISP_IS_MISSING = TRUE` for `RmVariantHal: VF`, `FALSE` for `PF_KERNEL_ONLY` (`ogkm-595:
  src/nvidia/generated/g_kern_disp_nvoc.c:278-286`). `kdispConstructEngine` begins with
  `NV_ASSERT(IS_VIRTUAL(pGpu) || IS_FW_CLIENT(pGpu) || RMCFG_MODULE_DISP)` then
  `if (IS_MISSING) return NV_ERR_NOT_SUPPORTED` (`ogkm-595: src/nvidia/src/kernel/gpu/disp/kern_disp.c:95-122`); the
  engine is dropped by the "engine removal = `ConstructEngine` returns `NV_ERR_NOT_SUPPORTED`" convention
  (`kern_gpu_tu102.c:103-107` comment). Result: `GPU_GET_KERNEL_DISPLAY(pGpu) == NULL`.
* `DispCommon`/display-API allocation returns `NV_ERR_NOT_SUPPORTED` when `pKernelDisplay == NULL` (`ogkm-595:
  src/nvidia/src/kernel/gpu/disp/disp_objs.c:96-99`). So **no NV0073, no heads, no SORs, no DFP** in a VF guest.
* **Substitute: `NVA083_GRID_DISPLAYLESS`**. It is allocatable only if there is **no** `KernelDisplay` (classes are
  mutually exclusive) **and** `osIsGridSupported()` (= `os_is_grid_supported()` [`GRID_BUILD`] ||
  `os_get_grid_csp_support()` || `hypervisorIsVgxHyper()`) (`ogkm-595: src/nvidia/src/kernel/gpu/gpu.c:1959-1982`,
  class pruned otherwise at `:2281-2284`; `ogkm-595: src/nvidia/arch/nvalloc/unix/src/os-hypervisor.c:918-921`).
  `RMCFG_CLASS_NVA083_GRID_DISPLAYLESS 1` is compiled in (`rmconfig.h:613`).
* Its data: number of heads, max heads, max resolution per head, max pixels, "display active" flags. In a VF guest
  the constructor does **not** fill defaults (`if (!IS_VIRTUAL) griddisplaylessSet...`,
  `ogkm-595: src/nvidia/src/kernel/griddisplayless/objgriddisplayless.c:111-122`); the plugin supplies them in
  `VGPU_DISPLAYLESS_INFO` inside `GET_STATIC_DATA` (`rpcstructurecopy.c:1302-1370`, versions `v24_08`,
  `v2D_00`). The ctrls `NVA083_CTRL_CMD_VIRTUAL_DISPLAY_{GET_NUM_HEADS,GET_MAX_RESOLUTION,GET_DEFAULT_EDID,
  IS_ACTIVE,IS_CONNECTED,GET_MAX_PIXELS}` (`ogkm-595: src/common/sdk/nvidia/inc/ctrl/ctrla083.h:80-219`) are answered
  **locally** (`griddisplaylessctrl.c`), with a built-in 256-byte EDID ("NVIDIA VGX", DVI-D or VGA variants,
  `objgriddisplayless.c:67-109`, selected by `griddisplaylessGetDefaultEDID_IMPL`).
* Console / VGA: a VF is never primary (`ogkm-595: src/nvidia/arch/nvalloc/unix/src/osinit.c:1108-1116`), skips the
  Unix console save/restore (`unix_console.c:90`, `:159`), and "restore low-res mode" becomes an RPC to the
  host (`NV_RM_RPC_SWITCH_TO_VGA`, `unix_console.c:237-243`).
* Other display-adjacent: NvFBC sessions branch on `IS_VIRTUAL` (`ogkm-595: src/nvidia/src/kernel/disp/nvfbc_session.c:84,
  179, 318`); `RM_USER_SHARED_DATA` is removed for virtual GPUs (`gpu.c:2287-2290`).
* Control routing: in a VF guest a `ROUTE_TO_PHYSICAL` control without `ROUTE_TO_VGPU_HOST` or
  `PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST` is rejected locally `NV_ERR_NOT_SUPPORTED` and logged
  "Unsupported ROUTE_TO_PHYSICAL control ... on vGPU guest" (`ogkm-595: src/nvidia/src/kernel/rmapi/control.c:774-785`).
  That is the general mechanism by which "which controls the guest forwards vs handles locally" is decided.

INFERRED: nvidia-modeset / nvidia-drm in a VF guest therefore see a GPU with no display class; I did not read
`src/nvidia-modeset` for how it reacts (*Unknowns U5*).

---

## 4. Q4 evidence — what kayfabe can learn or reuse

### 4.1 Pieces that describe NVIDIA's handling of a guest whose version differs from the host's

| # | Piece | Where | What it tells kayfabe |
|---|---|---|---|
| R1 | Fn-1 request/reply struct + its two result fields | `g_rpc-structures.h:36-47`; `g_rpc-message-header.h:41-52`; `rpc.c:8947-9006` | The host-side mismatch answer is `rpc_result = OK` **and** `rpc_result_private != OK`, with the host's own pair written into the body's `vgx*` fields. The guest then retries once (>= min) or gives up with a named log. A non-zero `rpc_result` is a hard stop, not a negotiation. |
| R2 | `RPC_VERSION_FROM_VGX_VERSION`, `VGX_*`, named releases | `vgpu_version.h:28-40` | Pair packing; 17.0 = `0x25`, 18.0 = `0x29.0B`, 19.0 = `0x2B.13`. |
| R3 | Min-supported internal pair `0x25.0x0E` | `g_rpc-structures.h:12822-12825` | The guest's floor. Everything below is "host too old". The guest does **not** carry a ceiling check. |
| R4 | Per-version function/struct tables | `g_rpc_private.h:3045-3427` (ranges), `:3596-4011` (versions), `rpcstructurecopy.c` (per-version `deserialize_*`) | Exactly which messages change layout at which pair. A host that wants to talk to older/newer guests needs these ranges, and ogkm contains the guest half of them for every pair `0x25.0E .. 0x2D.01`. |
| R5 | Internal -> external version map + min/max external | `vgpu_version.h:52-80, 110-117` | NVIDIA's own coarse compatibility bucketing (migration), and the registry override keys. |
| R6 | Result-code space | `rpc_headers.h:63-139` | `< 0xFF000000` = RM status, passes through; `0xFF00000x` = VMIOP plugin, mapped to `NV_ERR_GENERIC` by the guest; `0xFF10000x` = RPC-specific, `…09 API_CONTROL_NOT_SUPPORTED` is deliberately quiet; `0xFFFFFFFF` = pending. `GUEST_HOST_DRIVER_MISMATCH = 0x6A`. |
| R7 | Buffer ABI | `dev_vgpu.h:176-290`, `nv_sriov_defines.h:37-96`, `rpcga102.c:35-48` | A complete description of what a host plugin must implement to talk to an ogkm VF guest. It is a **different** protocol from GSP's libos message queue kayfabe emulates today. |
| R8 | Driver strings the guest announces | `rpc.c:8897-8943` | `NV_VERSION_STRING`, branch, title, CL number - everything NVIDIA's plugin gets to identify the guest. |

### 4.2 Does any of this run in a stock (non-vGPU) driver, so kayfabe could rely on the guest's own compatibility behaviour?

* **Yes - the shared code.** A stock GSP-client driver executes `RmRpcSetGuestSystemInfo` (R1, R2, R3, R4, R8) in
  full: the same function, the same retry branch, the same `rpcSetIpVersion` re-selection (`kernel_gsp.c:5060`;
  `rpc.c:8836-9017`). (MEASURED.)
* **But it protects a different set of messages.** In GSP-client mode the traffic is mostly
  `GSP_RM_CONTROL`/`GSP_RM_ALLOC` and `GET_GSP_STATIC_INFO`, whose bodies are compile-time structs (e.g.
  `GspStaticConfigInfo` memcpy, `rpc.c:9962-9994`), not the `rpc_*_vNN_MM` family that `rpcSetIpVersion` swaps.
  *INFERRED:* answering a stock GSP-client guest with a different pair would re-select the
  `rpc*_vNN_MM` functions the GSP-client **does** call, but it cannot make the driver use an older
  `GspStaticConfigInfo` or `GSP_MSG_QUEUE_ELEMENT` layout. In other words, **down-negotiation in fn 1 is not a
  skew-compat mechanism for the GSP-client protocol**; for that protocol NVIDIA's policy is "firmware ==
  driver" (section 2.3). The repo already measures the per-tag layout moves (`kf-gsp/src/element.rs:22`
  "break is (595.84, 610.43.02]").
* **vGPU-only parts are compiled in a stock build but refuse to bind.** Compiled: `rpc.c`, `rpcstructurecopy.c`,
  `objvgpu.c`, `vgpu_util.c`, `vgpuapi.c`, hypervisor files are all in the build list (`ogkm-595:
  src/nvidia/srcs.mk:892-913`); `RMCFG_MODULE_RPC 1`, `RMCFG_MODULE_RPCSTRUCTURECOPY 1`,
  `RMCFG_MODULE_KERNEL_VGPUMGR 1`, `RMCFG_MODULE_VGPUMGR 0` (`rmconfig.h:769-770, 808-809`). Runtime gates:
  * `nv_pci_probe`: for `pci_dev->is_virtfn` (a VF seen **by the host**) the stock build **ignores** the device
    (`return -1`, "Ignoring probe for VF"); with `NV_VGPU_KVM_BUILD` (`VGX_KVM_BUILD`) it returns 0 after an IOMMU
    check (`ogkm-595: kernel-open/nvidia/nv-pci.c:1790-1818`). This concerns the **host side**; a *guest* sees an
    ordinary PCI function (the hypervisor shows no SR-IOV capability), so `is_virtfn` is 0 in a guest.
  * Guest-side: `rm_is_supported_device` -> `rm_is_vgpu_supported_device` (`osapi.c:3733-3747`;
    `os-hypervisor.c:949-991`): *if* (HVM guest detected) *and* (`PMC_BOOT_1` is PV or VF): VF requires
    `os_is_grid_supported()` (compile-time `GRID_BUILD`, `os-interface.c:1441-1447`) **and**
    `gpumgrIsVgxRmFirmwareCapableChip` **and** the device's PCI `device_id` + `subsystem_id` in
    `sVgpuUsmTypes[]` (1124 entries, e.g. `{0x2BB5, ...}`, `{0x1EB8, 0x130C, "GRID T4-1Q"}`;
    `ogkm-595: src/common/shared/inc/g_vgpu_chip_flags.h:23-1146`); otherwise `NV_ERR_NOT_SUPPORTED` and the log
    "NVIDIA vGPU ... is not supported by open nvidia.ko". If CPUID shows no hypervisor the function returns
    `NV_OK` (VF treated as a normal device).
  * `GRID_BUILD`, `GRID_BUILD_CSP`, `VGX_BUILD`, `VGX_KVM_BUILD`, `VGX_DEVICE_VM_BUILD` are read **only** by
    `conftest.sh` (`kernel-open/conftest.sh:32-36, 677-731`) from the build environment; no Makefile in the tree
    sets them. MEASURED: a plain `make modules` of this tree leaves `NV_GRID_BUILD` undefined.
* **What identifies a device as a vGPU to the driver (MEASURED):** (1) `NV_PMC_BOOT_1[17:16]` (`_PV=1`, `_VF=2`)
  - the *only* test used to select the runtime variant; (2) the CPUID hypervisor signature (`bIsHVMGuest`) for the
  firmware-request and probe gates; (3) PCI `vendor 0x10de`, class `0x0300`/`0x0302` as for any NVIDIA GPU
  (`ogkm-595: kernel-open/nvidia/nv-pci-table.c:39-56`); (4) *only in GRID builds* the `(device_id, subsystem_id)` allowlist. There is no
  config-space signature byte and no special PCI ID range other than the allowlist. `NV_XVE_ID_DEVICE_CHIP_VGPU
  0xf00` is defined in `dev_vgpu.h:187-189` but referenced nowhere else in the tree.

### 4.3 What the source cannot tell (stated, not assumed)

* Whether NVIDIA's shipped Linux vGPU **guest** driver is built with `GRID_BUILD`, or is a different binary
  from the open `nvidia.ko` - packaging and licensing are not in the source. The open source only shows that
  *this tree*, built with `GRID_BUILD=1`, would accept a VF guest on the listed IDs.
* The host-side plugin (it is closed; only the `RESMAN_BUILD` half of the shared serialisers is here via
  `rm_plugin_shared_code.h`) - what it replies for out-of-window guests is **only** inferable from the guest code.
* Anything about the closed Windows driver (section 6).

---

## 5. Q5 evidence — what kayfabe presents today (REPO)

All paths are `kf:` at `origin/integration/windows-20261010`.

| Value | What is reported | Where it comes from |
|---|---|---|
| **Declared guest version** | `guest-driver=` QEMU property (`DEFINE_PROP_STRING("guest-driver", ...)`, `qemu/hw/misc/kf3/kf3.c:1906`, passed `:1606`); **unset -> the host's own version** | `crates/kf-qemu/src/device.rs:569-575` (`Some(v) => "guest-driver="`, `None => "defaulted to the host's"`); parsed and turned into a `DriverAbiTable` via `kf_abi::versions::table_for` (`:577-584`) |
| **Fn-1 reply (`SET_GUEST_SYSTEM_INFO`)** | `rpc_result = NV_OK`; body = `SET_GUEST_SYSTEM_INFO_SIZE` (6 x u32 + 3 x 256 B) of zeros except `vgxVersionMajorNum/MinorNum` = **the `guest-driver=` row's pair**; the guest's three strings and CL are **not** reflected | `crates/kf-rm/src/guestsysinfo.rs:177-205` (`GuestSystemInfoPolicy::respond`); `crates/kf-abi/src/guestsysinfo.rs:` `encode_set_guest_system_info_reply` |
| Pair value for `guest-driver=580.65.06` | `0x2B.0x13` (= vGPU 19.0 in ogkm's naming) - measured per tag, the run `580.65.06 ... 580.178.04` shares it | `crates/kf-abi/src/generated/matrix.rs:34507, 34530`; `crates/kf-abi/src/versions.rs:709-722` (`vgx` read from the matrix), test `:2419-2425` |
| Pair a 595.91.07 driver would speak | `0x2D.0x03` (run `595.84 ... 595.91.07`); **never sent**, because the host is not the guest | `matrix.rs:34509, 34532` |
| **Fn-64 reply** | bare `NV_OK` over a zero body | `kf-rm/src/guestsysinfo.rs:206-211` |
| **Cross-check** | guest's `guestDriverVersion` (fn 1) must equal the declared version (a Windows build "580.88" with the matching branch/CL is keyed as its Linux twin `580.65.06`), else **refuse**; for a *defaulted* version the chain **re-selects** to the guest's own tag at fn 1 if everything the guest consumed before fn 1 is identical, for a *declared* one it refuses | `kf-rm/src/guestsysinfo.rs:107-134`; `kf-rm/src/lib.rs:262-373` (`ReselectAtFn1`); `kf-abi/src/guestsysinfo.rs` `ReportedDriver::decode`, twin test `a_windows_twin_is_keyed_as_its_linux_tag_and_a_tag_as_itself` |
| **Refusal shape** | `Reply{ rpc_result: NV_ERR_NOT_SUPPORTED, body: [] }`; `Reply.rpc_result` is written to **both** `rpc_result` and `rpc_result_private` | `kf-rm/src/guestsysinfo.rs:160-165`; `kf-gsp/src/boot.rs:341-343, 2985`. The guest sees `SET_GUEST_SYSTEM_INFO failed: 0x56` (`kf-rm/src/guestsysinfo.rs:19-28`, the run `t127a`) |
| **Down-negotiation** | **Not built** (documented): "The down-negotiation branch is NOT built" | `kf-abi/src/guestsysinfo.rs` module docs, last section |
| **Firmware version** (`NV2080_CTRL_CMD_GSP_GET_FEATURES` `0x20803601`, `firmwareVersion[64]`) | the **guest's own `NV_VERSION_STRING`**, latched from its fn 1 and validated as printable ASCII <= 63 bytes; refused until fn 1 has been seen. Not the host's version, and not the table row's | `kf-rm/src/inittables.rs:992-1000, 252-274, 1552`; `kf-abi/src/gspfeatures.rs:40-125`. The comparison with ogkm's own `NV2080_CTRL_CMD_GSP_GET_FEATURES`: the control is a stub on every variant except the GSP-RM (physical) side - `kf-rm/inittables.rs:985-990` (`REPO`) |
| `GspStaticConfigInfo` (fn 65) | **has no version-string field at 595.84** (only `bVbiosValid`, `vbiosSubVendor/SubDevice`, `isGridBuild`, `gridBuildCsp` -> `ogkm-595: src/nvidia/inc/kernel/gpu/gsp/gsp_static_config.h:130-132, 206-208`). Wire shape per tag: `Pre610 / From610_43_02 / Unencoded` | `kf-abi/src/versions.rs:696-705` |
| **Host driver version** (595.91.07) | read once from the host with `NV_ESC_CHECK_VERSION_STR` (query form) on `/dev/nvidiactl`; selects host-side encoders (`rm.host_abi()`), e.g. NVOS46 `kindOverride` availability `>= 580.65.06`; **never** put into a guest-visible field | `kf-abi/src/host_driver.rs:51-66`; `kf-qemu/src/device.rs:1003-1013`; `crates/kayfabe-isolate-host/src/hostabi.rs:874-882` (accepted host set includes `595.91.07`) |
| Guest `firmware` ELF | not emulated: the guest loads its own `/lib/firmware/nvidia/<its version>/gsp_*.bin`, so `.fwversion == NV_VERSION_STRING` holds by construction in the guest; kayfabe never sees the ELF check | `ogkm-595: kernel-open/nvidia/nv.c:27`; `kernel_gsp.c:5571-5624` |

So the three version-like numbers in play are: **guest = whatever the guest driver is** (declared by
`guest-driver=` or defaulted to the host's, verified at fn 1); **vgx pair = the guest-driver row's**;
**host = 595.91.07, used only to pick host-side encodings**. They are deliberately decoupled ("the guest
axis and the host axis are disjoint", `kf-abi/src/host_driver.rs:1-20`).

---

## 6. LINUX guest with an ogkm-built driver vs WINDOWS guest with NVIDIA's closed driver

(c) from the owner's addition. **The open source says nothing about the closed Windows driver.** Everything
below is split accordingly.

### 6.1 Linux guest running an ogkm-built driver (MEASURED from source unless marked)

* As **GSP client** (the kayfabe design: a stock driver, no `GRID_BUILD`, PMC_BOOT_1 not VF): full
  `.fwversion` exact-match requirement on the firmware file the guest loads; fn 1 sent with the guest's
  `VGX_*` pair and strings; fn 64; `GET_GSP_STATIC_INFO`; no use of any vGPU buffer/doorbell. The guest's
  skew behaviour toward the "GSP" (kayfabe) is exactly sections 2.2-2.3: a handshake that can down-negotiate
  the vGPU function table, nothing that adapts `GspStaticConfigInfo` or the GSP message-queue element layout.
* As **vGPU guest** (a build with `GRID_BUILD=1` on an allowlisted ID, `PMC_BOOT_1=VF`, Ada/Hopper/Blackwell,
  hypervisor CPUID visible): sections 1.2-1.4 and 2.2. It copes with a host at any pair `>= 0x25.0x0E`
  by retrying at the host's pair and re-selecting its function tables, and with an older host by refusing to
  start. (INFERRED: it copes with a *newer* host only if the plugin keeps accepting the older guest pair.)
* It sends `SET_GUEST_SYSTEM_INFO` **first** (right after the control/response/message buffers are set up and
  the setup-doorbell exchange completes): `vgxVersionMajorNum=0x2D`, `vgxVersionMinorNum=0x03`,
  `guestDriverVersion="595.84"`, `guestVersion=NV_BUILD_BRANCH_VERSION`, `guestTitle=NV_DISPLAY_DRIVER_TITLE`,
  `guestClNum=NV_BUILD_CHANGELIST_NUM` (`rpc.c:8897-8943`, called at `rpc.c:1711`). **What it expects back:**
  `rpc_result == OK` (anything else is returned as the init error), `rpc_result_private == OK` for "agreed",
  and the agreed pair readable in the body (`vgxVersion*Num`), which it stores in a process-global and uses for
  `rpcSetIpVersion`; then it sends fn 64 (needs `rpc_result == OK`, reads nothing); then `GET_STATIC_DATA`
  decoded with the deserialiser the agreed pair selects. A pair mismatch with a prior GPU's stored pair
  yields `NV_ERR_INVALID_STATE` (`rpc.c:8987-8995`).

### 6.2 Windows guest with the closed driver

* **Source can say:** the vGPU control-buffer ABI reserves `NV_VGPU_GUEST_OS_TYPE_OS_WINDOWS10 = 3` and a
  `PAGE_SIZE` field (`dev_vgpu.h:45`), i.e. NVIDIA's plugin protocol anticipates Windows guests;
  `guest_os_type_os` is only ever `LINUX` or `INVALID` in the open code (`rpc.c:228-233`), consistent with
  Windows being a separate, closed implementation of the *same* protocol. (INFERRED; the source does not
  state it.)
* **Repo says (REPO, not re-run):** a Windows 580.88 guest booted against kayfabe sends
  `SET_GUEST_SYSTEM_INFO` as a GSP client with `vgx = 0x2B.0x13`, `guestDriverVersion = "580.88"`,
  `guestVersion = "r580_78-7"` and keeps working when answered as Linux `580.65.06` (same changelist
  36308443) - `kf-abi/src/guestsysinfo.rs` identity tests and `kf-abi/src/windows_twin.rs`; run `vwin`,
  kf3 `b98bdbec` per the test comment. That is a measurement about the **Windows driver as a GSP client
  of kayfabe**, not about Windows as a vGPU guest.
* **Not determinable from source:** whether the Windows driver implements the same `.fwversion` exact-match
  rule, the same down-negotiation, a different min-supported pair, or any tolerance of a mismatched
  firmware. Nothing in ogkm can answer this and nothing here assumes it. Any claim that "the Windows
  guest tolerates skew" would be an extrapolation from the Linux code and is labelled unsupported.

---

## 7. What this suggests for the design decision (INFERRED, for the owner)

1. **The Linux open driver gives kayfabe no skew-tolerance to lean on in GSP-client mode.** The only hard
   check is driver == firmware ELF, and the guest brings its own ELF, so it always passes. The real skew
   problem for kayfabe is *kayfabe's emulation of GSP* being tied to the layouts of one guest tag (this is
   what `DriverAbiTable`, `ReselectAtFn1` and "refuse by name" solve). That is consistent with how the repo
   already works; nothing in ogkm suggests a cheaper channel.
2. **Fn 1's retry branch is unused leverage, but narrow.** If kayfabe wanted a "too-old/too-new guest"
   answer the stock guest understands, it would be `rpc_result = OK`, `rpc_result_private =
   NV_ERR_LIB_RM_VERSION_MISMATCH (0x6A)`, host pair in the body. Today `Reply` cannot express it (it sets both
   result fields from one value, `kf-gsp/src/boot.rs:341-343, 2985`). Even so, as section 4.2 shows, a
   re-selected pair does not change the layouts that matter in GSP-client mode, so this would only give a
   **cleaner failure message** (`"Incompatible Guest/Host drivers ... Disabling vGPU"` / `Guest driver is
   incompatible with host driver` only on the VF path), not a working older-guest mode.
3. **If kayfabe ever wants a true vGPU-style device** (host plugin protocol, display-less), the source
   defines the whole guest-side ABI (R7) - but a stock build will not bind a VF without `GRID_BUILD` and an
   allowlisted ID, and only Ada/Hopper/Blackwell are plugin-offload-capable in 595.84. That is a separate
   product, not a variation of the current one.
4. **The VF design has no display** (section 3). kayfabe's display work (`display=on`, NV0073) has no vGPU
   analogue to borrow from; the closest is `NVA083_GRID_DISPLAYLESS`'s numbers-only model.

---

## 8. Unknowns

* **U1.** What GSP firmware does when the CPU-RM's `WPR_META`/`SR_META` revision or the fn-1 pair is not
  what it expects (closed). The repo measured the *driver-written* side only.
* **U2.** What the closed vGPU host plugin returns for a guest outside its window (newer than the host; older
  than its minimum) - only the guest half of the protocol is here. Does it set `rpc_result_private` to
  `GUEST_HOST_DRIVER_MISMATCH`, or fail with `rpc_result`? The guest handles either, differently
  (retry/abort vs hard stop).
* **U3.** Whether NVIDIA's shipped Linux guest driver is a `GRID_BUILD` of this source or a separate binary;
  licensing/packaging (not in source). Also whether it enforces the `sVgpuUsmTypes` allowlist the same way.
* **U4.** The complete BAR0 register footprint of a VF guest across engine init (I traced the RPC transport,
  `PMC_BOOT_*`, the scratch/doorbell pair and the interrupt tree, not every `GPU_REG_RD32` reachable in a
  VF). A trace-first comparison is the right method (`feedback_trace_first_vfio_reference`).
* **U5.** How `nvidia-modeset` / `nvidia-drm` behave in a VF guest with no `KernelDisplay`; not read.
* **U6.** The interaction between a hidden hypervisor (QEMU `kvm=off`/`hv_vendor_id` style) and the VF gates:
  `isVirtual` is set from `PMC_BOOT_1` alone (`gpu_mgr.c:1262`) but the firmware-skip and the vGPU-supported
  checks require `bIsHVMGuest` (`osapi.c:4995-5030`, `os-hypervisor.c:961-964`); the combination "VF register
  value + CPUID hidden" would, by reading, request firmware for a device whose HAL is VF. Not tested.
* **U7.** The version at which ogkm's `VGX_*` pair stopped being monotonic across tags: the repo's measured
  table is `0x23.05 (535) ... 0x2B.13 (580) -> 0x2C.07 (590.48.01) -> 0x2D.03 (595.84..595.91.07) -> 0x2E.0D
  (610) -> 0x2F.06 (615.71.09)` (`matrix.rs:34495-34536`); the minor is not monotonic (`0x13 -> 0x07`), so a
  pair is a *tag*, not an order. Not relevant to correctness, relevant to any "newer/older" comparison that
  uses only the minor.
* **U8.** The meaning of the external version `0x1C` in vGPU release naming (the source only labels
  17.0/18.0/19.0 internal pairs); the table is a mapping, not documentation.
