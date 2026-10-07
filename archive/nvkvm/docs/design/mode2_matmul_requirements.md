# Mode-2 matmul implementation requirements (cuInit → memcpy → kernel launch, host parity)

Status: research/spec, 2026-06-03. No code changed to produce this.
Target: stock NVIDIA open kernel driver **580.159.04** in a KVM guest against the
emulated GA106 (`src/qemu/nvkvm_gpu_emul.c`), forwarding **real compute** to the
host's real GA106 (RTX 3060, driver 580.159.04) via the unprivileged Mode-1 stub.

## How this doc was produced (ground truth, not guesswork)
- Built + ran a real cuBLAS SGEMM (1024×1024 fp32) on the host (`ssh vh`):
  `cuInit → cuDeviceGet → cuCtxCreate → cuMemAlloc → cuMemcpyHtoD → cublasSgemm →
  cudaDeviceSynchronize → cuMemcpyDtoH`. Result byte-correct (C[0]=2048).
  Sources: `/tmp/mm.cu` (+`/tmp/mm2.cu` phase-instrumented) on `vh`.
- `strace -f -tt -T -e trace=openat,ioctl,mmap,munmap,poll,ppoll,read,write` →
  `/tmp/mm.strace` on `vh` (923 ioctls).
- An `LD_PRELOAD` ioctl decoder (`/tmp/shim*.so` on `vh`) that decodes NVOS54
  (`hClient/hObject/cmd/flags/params/paramsSize/status`), tags each call with a
  CUDA-phase marker, and **dumps the real GA106 response bytes** for the
  compute-capability controls. Logs: `/tmp/shim.log`, `/tmp/dump.log`, `/tmp/dump3.log`.
- Cross-referenced against `research_clones/ogkm/` (open-gpu-kernel-modules, driver
  matched), citing `file:line`.

`NV_ESC` numbers: `kernel-open/common/inc/nv-ioctl-numbers.h` (frontend `NV_IOCTL_BASE=200`)
and `src/nvidia/arch/nvalloc/unix/include/nv_escape.h` (RM escapes). `NV_IOCTL_MAGIC='F'=0x46`.
`NVOS54_PARAMETERS` (the RM_CONTROL arg) is at `src/common/sdk/nvidia/inc/nvos.h:2230-2239`:
`hClient(0) hObject(4) cmd(8) flags(12) params@8(16) paramsSize(24) status(28)` = 32 bytes.

---

## (A) Ordered open / ioctl / mmap / poll trace summary

### A.1 Device opens (in order; fds reused after close)
A real run opens **`/dev/nvidiactl`** many times (per-RM-client + version/probe),
**`/dev/nvidia0`** several times (per-subdevice/per-channel attach), and
**`/dev/nvidia-uvm`** twice (UVM global + per-process). Representative order
(from `/tmp/mm.strace`):

| order | device | purpose |
|------|--------|---------|
| 1 | `/dev/nvidiactl` | RM root client (NV01_ROOT), version check |
| 2 | `/dev/nvidiactl` | second root client (libcuda dedups per-session) |
| 3 | `/dev/nvidia-uvm` | UVM global fd (UVM_INITIALIZE) |
| 4 | `/dev/nvidia-uvm` | UVM per-address-space fd |
| 5 | `/dev/nvidia0` | per-GPU device attach (NV20_SUBDEVICE_0, channels) |
| 6.. | `/dev/nvidia0` ×N, `/dev/nvidiactl` ×N | per-object client/device/subdevice/channel fds |

Frontend escapes seen on these fds (decoded by `_IOC` nr, type `0x46`):
`CARD_INFO (0xc8=200+0)`, `WAIT_OPEN_COMPLETE (0xd6=200+18)`, plus the RM escapes
`RM_ALLOC (0x2b)`, `RM_CONTROL (0x2a)`, `RM_FREE (0x29)`, `RM_MAP_MEMORY (0x4e)`,
`RM_ALLOC_OS_EVENT (0xce=200+6)`, `CHECK_VERSION_STR (0xd2=200+10)`.

### A.2 RM_ALLOC object classes used (first-seen order)
From the decoded `cls` field of `NVOS21/NVOS64` (`hRoot,hParent,hObject,hClass`):

| class | name (ogkm) | role |
|-------|-------------|------|
| `0x0000` (root) | NV01_ROOT/NV01_ROOT_CLIENT | RM client |
| `0x0080` | NV01_DEVICE_0 | device |
| `0x2080` | NV20_SUBDEVICE_0 | subdevice (most NV2080 controls target this) |
| `0x2081` | NV2081_BINAPI | binary-API |
| `0x0041` | NV01_ROOT_NON_PRIV / device-tied | (alloc-class 0x41) |
| `0xcb33` | NV_CONFIDENTIAL_COMPUTE | CC query object |
| `0xc461` | TURING/AMPERE channel-group/USERMODE (VOLTA_USERMODE_A family) | usermode/doorbell object |
| `0x00de` | NV01_MEMORY_SYSTEM_OS_DESCRIPTOR / S, mapping memory | sysmem desc |
| `0x90f1` | FERMI_VASPACE_A | GPU virtual address space |
| `0x50a0` | NV50_MEMORY_VIRTUAL | virtual memory |
| `0x0040` | NV01_MEMORY_LOCAL_USER (FB) | device memory |
| `0xa06c` | KEPLER_CHANNEL_GROUP_A (TSG) | channel group / runlist |
| `0x9067` | (FERMI / compute-context family) | per-channel ctx |
| `0x003e` | NV01_MEMORY_SYSTEM | sysmem |
| `0xc56f` | AMPERE_CHANNEL_GPFIFO_A (0xC56F) | **GPFIFO compute/CE channel** |
| `0xc7c0` | (HOPPER/AMPERE) | context object |
| `0xc7b5` | AMPERE_DMA_COPY_B (0xC7B5) | **CE (copy-engine) object** |
| `0x0079` | NV04_DISPLAY_COMMON / SW class | SW object |
| `0x83de` | GT200_DEBUGGER / compute debug | debug object |

The compute class used for the SGEMM launch is the AMPERE compute class
(`AMPERE_COMPUTE_B = 0xC7C0` family) bound on the `AMPERE_CHANNEL_GPFIFO_A
(0xC56F)` channel; CE is `AMPERE_DMA_COPY_B (0xC7B5)`. These match the host GA106
because the guest is advertised as GA106 (same class IDs).

### A.3 mmap regions against nvidia fds (the BAR/USERD/doorbell/semaphore set)
From `/tmp/mm.strace`, the `MAP_SHARED` mmaps on `/dev/nvidia0` (fds 15/17/19/21/23/24)
and `/dev/nvidia-uvm` (fd 9). The fixed GPU-VA targets (`0x2xxxxxxxx`) are the
driver's own choice of GPU virtual addresses (Mode-2's RM picks these — see §C):

| guest addr | length | fd | classification (cross-ref §C) |
|-----------|--------|----|-------------------------------|
| `0x...e51000` | 64 KiB, `PROT_WRITE`, off 0 | 15 (nvidia0) | **USERD / work-submit** page (write-only mailbox) |
| `0x...ee6000` | 4 KiB, `PROT_READ`, off 0 | 15 | **semaphore / notifier** read page |
| `0x200200000` | 2 MiB | 17 | GPU-VA window (channel instance / ctx buffer) |
| `0x200400000` | 48 MiB | 17 | GPU-VA window (large ctx / pushbuffer / pool) |
| `0x204400000` | 2 MiB | 17 | GPU-VA window |
| `0x...ddf000`..`de6000` | 4 KiB ×N | 17 | **doorbell / USERD** 4 KiB pages (per channel) |
| `0x...de7000`..`dea000` | 4 KiB ×N | 19 | doorbell/USERD pages (channel 2) |
| `0x...deb000`..`dee000` | 4 KiB ×N | 21 | doorbell/USERD pages (channel 3) |
| `0x204600000` | 2 MiB | 23 | GPU-VA window |
| `0x204800000` | 2 MiB, off=addr | 9 (uvm) | **UVM-managed VA range** (offset == VA) |
| `0x...8400000`,`9000000` | 2 MiB | 24 | GPU-VA windows (FB-backed) |
| `0x...9600000` | ~5.2 MiB | 24 | GPU-VA window |

Interpretation (cross-ref open driver, see §C):
- The **4 KiB `PROT_READ|PROT_WRITE MAP_SHARED` pages** are USERD + work-submit
  doorbell pages → for parity these must be backed by the **real host channel's
  USERD/doorbell MMIO** (forwarded mmap), so a guest userspace doorbell write hits
  real HW.
- The **64 KiB write page (fd 15)** is the usermode/work-submit ("VOLTA_USERMODE_A"
  BAR-window) region; the **4 KiB read page** is the matching semaphore/notifier.
- The **2 MiB / 48 MiB `0x2xxxxxxxx` windows** are GPU virtual-address mappings of
  channel/context/pushbuffer/instance buffers (RM_MAP_MEMORY + GPU VAS). These do
  **not** need real host MMIO per se; they need to resolve, via §D's address
  bridge, to the host context's real backing.
- The **UVM 2 MiB range (fd 9, offset==VA)** is UVM managed memory; CUDA device
  allocations land here.

### A.4 poll / ppoll
The trace shows **no `poll`/`ppoll`/`epoll` on nvidia fds** for this workload.
Completion is by **busy-poll on the mapped semaphore/notifier read page** (the
4 KiB `PROT_READ` page) and `cudaDeviceSynchronize` spins on the report-semaphore
value the GPU writes there. (Event-fd / `NV_ESC_RM_GET_EVENT_DATA` paths exist but
were not exercised by this synchronous cuBLAS run.) **Implication: completion is
detected by memory, not by a syscall — the semaphore page must be real host memory
the host GPU writes (see §C/§D).**

---

## (B) RM control command catalog (cmd IDs → struct → phase → compute-cap?)

Phase tagging is from the instrumented run (`/tmp/shim.log` markers). "CC?" = does
the response carry compute-capability / topology data that libcuda reads back (a
`cuInit=100` suspect)? GA106 values are the **real captured bytes** from `vh`
(`/tmp/dump.log`, `/tmp/dump3.log`); "needs host capture" = value depends on
per-channel/per-context runtime state and must be forwarded, not hard-coded.

### B.1 cuInit / enumeration phase (the cuInit=100 fix lives here)
Issued on `/dev/nvidiactl` root client (NV0000) and the subdevice (NV2080):

| cmd | name (ogkm cite) | struct | CC? | GA106 value / note |
|-----|------------------|--------|-----|--------------------|
| `0x101` | NV0000_CTRL_CMD_SYSTEM_GET_BUILD_VERSION | ctrl0000system.h | no | version string (must match guest driver 580.159.04) |
| `0x1f0` | NV0000_CTRL_CMD_SYSTEM_GET_FEATURES | ctrl0000system.h | no | feature bitmask |
| `0x201` | NV0000_CTRL_CMD_GPU_GET_ATTACHED_IDS | ctrl0000gpu.h | no | gpuId list (1 GPU) |
| `0x214` | NV0000_CTRL_CMD_GPU_GET_PROBED_IDS | ctrl0000gpu.h | no | probed gpuId |
| `0x215` | NV0000_CTRL_CMD_GPU_ATTACH_IDS | ctrl0000gpu.h | no | attach |
| `0x205` | NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2 | ctrl0000gpu.h | no | deviceInstance/subInst/gpuId |
| `0x288` | NV0000_CTRL_CMD_GPU_GET_ACTIVE_DEVICE_IDS | ctrl0000gpu.h | no | active ids |
| `0xcb330101` | NV_CONF_COMPUTE_CTRL_CMD_SYSTEM_GET_CAPABILITIES | ctrlcb33.h | no | CC off |
| `0x800280` | NV0080_CTRL_CMD_GPU_GET_NUM_SUBDEVICES | ctrl0080gpu.h | no | 1 |
| `0x800292` | NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 | ctrl0080gpu.h | **yes** | **supported class list** (804 B). Captured GA106 list includes 0xC56F, 0xC7B5, 0xC7C0, 0x90F1, 0xC461 etc. libcuda checks the compute/CE classes are present. |
| `0x801402` | NV0080_CTRL_CMD_HOST_GET_CAPS_V2 | ctrl0080host.h | yes | host caps bits |
| `0x801307` | NV0080_CTRL_CMD_FB_GET_CAPS_V2 | ctrl0080fb.h | yes | FB caps bits |
| `0x20801701` | NV2080_CTRL_CMD_MC_GET_ARCH_INFO | ctrl2080mc.h:65 | **yes** | **arch=0x170, impl=0x06, revision=0xa1** (GA106; `MC_ARCH_INFO_IMPLEMENTATION_GA106=6` ctrl2080mc.h:115). libcuda derives the chip from this. |
| `0x20800102` | NV2080_CTRL_CMD_GPU_GET_INFO_V2 | ctrl2080gpu.h:298 | **yes** | 11 entries (idx,val pairs). Indices queried: 0x11 GEMINI_BOARD=0, 0x22=0, 0x27 GLOBAL_POISON_FUSE=0, 0x2a GPU_SMC_MODE=0, 0x37 GPU_DEBUGGING_CAPABILITY=1, 0x3b SELF_HOSTED=0, 0x3c CMP_SKU=0, 0x3d DMABUF_CAPABILITY=1, 0x2d FLA=0, 0x3a LOCAL_EGM=0, 0x44 COHERENT_GPU_MEM=0. |
| `0x2080014a` | NV2080_CTRL_CMD_GPU_GET_GID_INFO | ctrl2080gpu.h:1749 | no | GPU UUID |
| `0x20800110` | NV2080_CTRL_CMD_GPU_GET_NAME_STRING | ctrl2080gpu.h:325 | no | "NVIDIA GeForce RTX 3060" |
| `0x20800111` | NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING | ctrl2080gpu.h:361 | no | short name |
| `0x20800119` | NV2080_CTRL_CMD_GPU_GET_SIMULATION_INFO | ctrl2080gpu.h:471 | yes | **must report NOT-simulation** (real silicon = 0). A wrong value here can make libcuda treat the device as non-CUDA. |
| `0x2080012f` | NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS | ctrl2080gpu.h:1119 | no | ECC off |
| `0x20800131` | NV2080_CTRL_CMD_GPU_QUERY_COMPUTE_MODE_RULES | ctrl2080gpu.h:1306 | yes | compute-mode rules (DEFAULT) |
| `0x20800145` | NV2080_CTRL_CMD_GPU_GET_ENGINE_PARTNERLIST (0x147 area) / GET_INFOROM | ctrl2080gpu.h | no | engine partners |
| `0x20800170` | NV2080_CTRL_CMD_GPU_GET_ENGINES_V2 | ctrl2080gpu.h:778 | **yes** | **engine list (340 B)**. Captured: 9 engines = {GR(0x1), COPY0..n, NVDEC, NVENC, SEC2, ...} (ids 0x9,0xa,0xb,0xc,0x13,0x1b,0x22,0x33). libcuda requires the GR + CE engines present. |
| `0x20801303` | NV2080_CTRL_CMD_FB_GET_INFO_V2 | ctrl2080fb.h | **yes** | **FB config (1028 B)**. Captured first entry: index 1 → 0x00c00000 (FB size / heap params). Drives total device memory. |
| `0x20801201` | NV2080_CTRL_CMD_GR_GET_INFO | ctrl2080gr.h:412 | **YES (primary)** | **58-entry GR info list** (see B.3 table — SM version, GPC/TPC/SM counts, warps). This is the table whose zeroing causes `cuInit=100`. |
| `0x2080122a` | NV2080_CTRL_CMD_GR_GET_GPC_MASK | ctrl2080gr.h:1452 | **YES** | gpcMask=**0x07** (3 enabled GPCs) at param off 16. |
| `0x2080122b` | NV2080_CTRL_CMD_GR_GET_TPC_MASK | ctrl2080gr.h:1480 | **YES** | per-gpcId tpcMask at off 20 (gpcId at off 16). Captured: **GPC0=0x1e (4 TPC), GPC1=0x1f (5 TPC), GPC2=0x1f (5 TPC)** → 14 TPC total. |
| `0x20801227` | NV2080_CTRL_CMD_GR_GET_CAPS_V2 | ctrl2080gr.h:1421→ctrl0080gr.h:284 | **YES** | 23-byte `capsTbl` (NV0080_CTRL_GR_CAPS_TBL_SIZE=23, ctrl0080gr.h:78). Captured GA106: `b0 62 00 00 00 00 00 01 00 00 00 10 01 08 00 10 10 00 00 00 04 c0 05`. |
| `0x2080121b` | NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER | ctrl2080gr.h:1213 | **YES** | per-SM topology array (9240 B): `globalSmId[512]` of {gpcId,localTpcId,localSmId,globalTpcId,virtualGpcId,...} (18 B each) + **numSm@off9216=28, numTpc@off9218=14** (captured). libcuda uses this for SM scheduling. |
| `0x20803801` | NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO | ctrl2080grmgr.h:57 | **YES** | floorsweeping info (1928 B): GPC/TPC/PES enable masks per query. Captured. |
| `0x20803601` | NV2080_CTRL_CMD_GSP_GET_FEATURES | ctrl2080gsp.h | no | GSP feature flags |
| `0x20801823` | NV2080_CTRL_CMD_BUS_GET_INFO_V2 | ctrl2080bus.h | no | bus/PCIe info |
| `0x20801801` | NV2080_CTRL_CMD_BUS_GET_PCI_INFO | ctrl2080bus.h | no | PCI ids |
| `0x20801803` | NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO | ctrl2080bus.h | yes | **BAR sizes/addrs** — must match emulated BARs |
| `0x2080182a` | NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS | ctrl2080bus.h | no | atomics caps |
| `0x2080182b` | NV2080_CTRL_CMD_BUS_GET_C2C_INFO | ctrl2080bus.h | no | C2C (none) |
| `0x20803002` | NV2080_CTRL_CMD_NVLINK_GET_NVLINK_STATUS | ctrl2080nvlink.h | no | no NVLink |
| `0x2080220c` | NV2080_CTRL_CMD_RC_RELEASE_WATCHDOG_REQUESTS | ctrl2080rc.h | no | RC watchdog |
| `0x20802210` | NV2080_CTRL_CMD_RC_SOFT_DISABLE_WATCHDOG | ctrl2080rc.h | no | RC watchdog |
| `0x27b` | NV0000_CTRL_CMD_GPU_GET_MEMOP_ENABLE | ctrl0000gpu.h | yes | memop enable (CUDA fast-path) |
| `0x136` | NV0000_CTRL_CMD_SYSTEM_GET_FABRIC_STATUS | ctrl0000system.h | no | no fabric |
| `0x13a` | NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS_MATRIX | ctrl0000p2p.h | no | P2P (single GPU) |
| `0xd01` | NV0000_CTRL_CMD_CLIENT_GET_ADDR_SPACE_TYPE | ctrl0000client.h | yes | addr-space type for a handle (sysmem/vidmem) |
| `0xd04` | NV0000_CTRL_CMD_CLIENT_SET_INHERITED_SHARE_POLICY | ctrl0000client.h | no | share policy |

### B.2 cuCtxCreate phase
| cmd | name | CC? | note |
|-----|------|-----|------|
| `0x20800145` | GPU_GET_ENGINE info | no | engine query |
| `0x2080200a` | NV2080_CTRL_CMD_PERF_BOOST | no | clock boost (BOOST_TO_MAX then clear) |
| `0x80170d` | NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST | no | channel handle list |
| `0x906f0101` | NV906F (GPFIFO) ctrl GET_CLASS_ENGINEID / channel ctrl | yes | channel engine binding |
| `0xc36f0108` | NVC36F (AMPERE GPFIFO) GPFIFO_GET_WORK_SUBMIT_TOKEN | **yes** | **work-submit token** for the doorbell — needs the **real host channel token** (forward) |
| `0xa06c0101`,`0xa06c0103` | KEPLER_CHANNEL_GROUP_A (TSG) ctrls (SET_TIMESLICE / GET_INFO) | no | TSG setup |
| `0x83de0309` | GT200_DEBUGGER ctrl | no | debug attach |
| `0x20801218` | NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_SIZE | ctrl2080gr.h:1050 | **yes** | GR context-buffer sizes — must match host golden-ctx layout |
| `0x20801210` | NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE | ctrl2080gr.h:831 | no | preemption mode |
| `0x801909` | NV0080_CTRL_CMD_*_PERF / CUDA_LIMIT_SET_CONTROL | yes | CUDA limit |
| `0xd01` | CLIENT_GET_ADDR_SPACE_TYPE | yes | addr-space per handle |

`0xc36f0108` (GET_WORK_SUBMIT_TOKEN) is the **link between RM and the doorbell**:
libcuda gets a token here and writes it to the work-submit page. For parity the
token must be the **real host channel's** token (see §D).

### B.3 GR_GET_INFO (0x20801201) — the cuInit=100 payload (captured GA106, 58 entries)
The values live in the user buffer pointed to by `grInfoList` (off 8 of the
params); each entry is `{NvU32 index, NvU32 value}`. Names from
`ctrl0080gr.h:102-175`. **These are the bytes that must be filled for `cuInit` to
pass** (the emulator currently leaves them zero):

| idx | name | GA106 value |
|-----|------|-------------|
| 0x07 | SHADER_PIPE_COUNT | 3 |
| 0x08 | THREAD_STACK_SCALING_FACTOR | 28 |
| 0x09 | SHADER_PIPE_SUB_COUNT | 14 |
| 0x0C | **SM_VERSION** | **0x806** (=SM 8.6; `NV2080_CTRL_GR_INFO_SM_VERSION_8_06` ctrl2080gr.h:318) |
| 0x0D | MAX_WARPS_PER_SM | 48 |
| 0x0E | MAX_THREADS_PER_WARP | 32 |
| 0x13 | MAX_SP_PER_SM | 0 |
| 0x14 | **LITTER_NUM_GPCS** | **7** (max GPCs in family; enabled mask=0x07 via 0x122a) |
| 0x15 | LITTER_NUM_FBPS | 6 |
| 0x16 | LITTER_NUM_ZCULL_BANKS | 4 |
| 0x17 | **LITTER_NUM_TPC_PER_GPC** | **6** |
| 0x19 | LITTER_NUM_MXBAR_FBP_PORTS | 8 |
| 0x1B | LITTER_NUM_FBPAS | 6 |
| 0x1C | LITTER_NUM_PES_PER_GPC | 3 |
| 0x1D | GPU_CORE_COUNT | 0xe00 (3584) |
| 0x1E | LITTER_NUM_TPCS_PER_PES | 2 |
| 0x1F | LITTER_NUM_MXBAR_HUB_PORTS | 8 |
| 0x20 | **LITTER_NUM_SM_PER_TPC** | **2** |
| 0x21 | LITTER_NUM_HSHUB_FBP_PORTS | 2 |
| 0x22 | RT_CORE_COUNT | 28 |
| 0x23 | TENSOR_CORE_COUNT | 112 |
| 0x24 | LITTER_NUM_GRS | 1 |
| 0x25 | LITTER_NUM_LTCS | 12 |
| 0x26 | LITTER_NUM_LTC_SLICES | 8 |
| 0x28 | LITTER_NUM_LTC_PER_FBP | 2 |
| 0x29 | LITTER_NUM_ROP_PER_GPC | 2 |
| 0x2A | FAMILY_MAX_TPC_PER_GPC | 8 |
| 0x2B | LITTER_NUM_FBPA_PER_FBP | 1 |
| 0x2C | MAX_SUBCONTEXT_COUNT | 64 |
| 0x2D | MAX_LEGACY_SUBCONTEXT_COUNT | 2 |
| 0x2E | MAX_PER_ENGINE_SUBCONTEXT_COUNT | 64 |
| 0x32 | LITTER_NUM_SLICES_PER_LTC | 4 |
| 0x34 | GFX_CAPABILITIES | 0xf |
| 0x35 | MAX_MIG_ENGINES | 1 |
| 0x36 | MAX_PARTITIONABLE_GPCS | 7 |
| 0x37 | LITTER_MIN_SUBCTX_PER_SMC_ENG | 9 |

(Other indices 0x00-0x06,0x0A,0x0B,0x10-0x12,0x18,0x1A,0x33,0x38,0x39 = small
constants/0 as captured; full 58-row dump in `/tmp/dump3.log`.)

Derived (RTX 3060 floorswept GA106): **3 enabled GPCs (gpcMask=0x07) × {4,5,5}
TPC = 14 TPC total × 2 SM/TPC = 28 SM**, SM version 8.6, CUDA compute capability
8.6, 3584 CUDA cores (idx 0x1D). Confirmed by GR_GET_GLOBAL_SM_ORDER (0x121b):
**numTpc=14, numSm=28**, and the per-GPC TPC masks (0x122b).

### B.4 memalloc / HtoD / launch / DtoH / cublasCreate
| phase | controls (already seen in B.1/B.2 set, re-issued) | new control |
|-------|-----|-----|
| MEMALLOC | RM_ALLOC NV01_MEMORY_LOCAL_USER / sysmem desc; RM_MAP | — |
| HtoD | (no new RM_CONTROL; CE channel doorbell, §D) | — |
| cublasCreate / LAUNCH | `0x20802209` RC_GET_WATCHDOG_INFO; `0x20800110` name; perf/clk | `0x2080a084`,`0x2080a026`,`0x2080900x` (CE/perf subdevice ctrls — non-CC) |
| DtoH | (no new RM_CONTROL; CE doorbell) | — |

The compute-cap controls are **entirely in cuInit/enumeration (B.1+B.3)**. Memcpy
and launch issue almost no RM_CONTROL — they drive the **mapped doorbell/USERD**
(§D). So fixing `cuInit=100` is purely a B.1/B.3 data problem.

---

## (C) mmap regions → required host backing for parity

Reference behavior of the open driver: `kernel-open/nvidia/nv-mmap.c` and
`src/nvidia/src/kernel/gpu/mem_mgr/` map either device-memory apertures or
USERD/usermode regions into userspace. The four region kinds and their parity
backing:

1. **USERD + work-submit doorbell (4 KiB / 64 KiB `MAP_SHARED` pages, fds 15/17/19/21).**
   Parity requirement: back these with the **real host channel's USERD + doorbell
   MMIO** so a guest userspace write to the doorbell reaches real HW. This is the
   Mode-1 "forwarded mmap" pattern: the stub allocates the real host channel, the
   QEMU device installs the host page(s) into the guest's mmap target via a
   KVM memslot / `MAP_FIXED` (the GPA-window mechanism in
   `docs/design/gpa_window_pci_bar.md`). **Bring-up shortcut**: trap-to-forward the
   doorbell write (cheap, §D) before doing true direct-map.

2. **Semaphore / notifier read page (4 KiB `PROT_READ`, fd 15).**
   The host GPU writes the report-semaphore here; the guest busy-polls it
   (`cudaDeviceSynchronize`). Parity requirement: this page must be **real host
   memory the host GPU writes** — i.e. the host channel's semaphore surface mapped
   read-only into the guest. (If emulated/zero, sync hangs or returns stale → no
   completion.) This is why there is no `poll` syscall in the trace (§A.4).

3. **GPU-VA windows (`0x2xxxxxxxx`, 2/48 MiB, fds 17/23/24).**
   These are RM_MAP_MEMORY mappings of channel/instance/pushbuffer/ctx buffers at
   the GPU virtual addresses Mode-2's *guest* RM chose. Parity requirement: they
   must resolve through the address bridge (§D) to the host context's real
   backing. The guest VA value itself is local; the *contents* must be the host
   GPU's. The existing `nvkvm_walk_pdb` (guest GR-VA → GPA, aperture-aware) plus
   the OS-descriptor of guest RAM into the host VAS is the mechanism.

4. **UVM managed range (fd 9, offset==VA).**
   CUDA device allocations (`cuMemAlloc`) for this cuBLAS path go through UVM.
   Mode-1 already forwards UVM ioctls + mmap from QEMU's process; Mode-2 reuses
   that. Parity requirement: the UVM VA range is backed by host FB/sysmem via the
   stub's UVM fd (same as Mode-1).

Cross-cutting: QEMU stays **unprivileged**; it never touches `/dev/nvidia*`. All
real host channel/USERD/doorbell/semaphore allocation happens in the **stub**
(`src/stub/`), and QEMU installs the resulting host pages into guest GPAs via the
GPA window + KVM memslots (existing Mode-1 infra, `src/qemu/nvkvm_dispatch.c`).

---

## (D) DMA + kernel-launch submission requirements

### D.1 cuMemcpyHtoD / DtoH (CE copy via channel doorbell)
- libcuda builds a CE pushbuffer (AMPERE_DMA_COPY_B `0xC7B5` methods) in the
  channel's GPFIFO, advances the GPFIFO PUT, then **rings the work-submit
  doorbell** (the mapped page, §C.1) with the channel's work-submit token
  (from `0xc36f0108` GET_WORK_SUBMIT_TOKEN, §B.2). Completion = report semaphore
  on the read page (§C.2).
- **Parity bridge**: the CE pushbuffer references GPU VAs for src/dst. Those VAs
  must map (via the host context VAS) to (a) the host-side staging of guest RAM
  and (b) host FB. Required: **OS-descriptor the guest-RAM HVA into the host GR/CE
  context VAS** (RM `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR 0x00DE`, Mode-1 unprivileged)
  so the host CE engine reads/writes the guest's actual buffers. Then guest
  doorbell → real host doorbell → real CE copy. (Mode-1 already proves HtoD/DtoH
  byte-exact through this mechanism.)

### D.2 Kernel launch (GR / compute channel)
The SGEMM launch path:
1. **SET_OBJECT(AMPERE_COMPUTE_B 0xC7C0)** subchannel bind in the GPFIFO.
2. Compute methods: set grid/block dims, shared-mem, the kernel's GPU-VA entry
   point, constant-bank (kernel params), then **kernel-launch method** (QMD /
   SET_INLINE / LAUNCH_DMA depending on path).
3. A **report semaphore** release method (writes the completion value to the
   semaphore surface, §C.2).
4. Advance GPFIFO PUT, ring the **doorbell** with the channel token.
5. `cudaDeviceSynchronize` busy-polls the semaphore page.

**Parity requirement (the core of Phase B):** a **real host GR/compute channel +
golden context** whose GPFIFO/pushbuffer/USERD/doorbell/semaphore == the guest's,
with the kernel code + params + I/O buffers reachable in the host context VAS:
- guest GR-VA → GPA (`nvkvm_walk_pdb`) → host-GPU-VA (OS-descriptor of guest RAM,
  §D.1) → host GPU executes the **guest's actual** pushbuffer/kernel.
- This is why the GR golden-context can't be emulated (FECS/GPCCS microcode) and
  must be a real host context (per `docs/design/mode2_gr_forwarding.md`).

The guest-RAM↔GPU-VA↔host-GPU bridge is: **guest writes pushbuffer/kernel into
guest RAM (GPA); OS-descriptor maps that HVA into the host context VAS at the
guest's chosen GR-VA; the host GPU's MMU resolves the guest VAs to the same bytes.**
Bring-up can trap-and-forward the doorbell; parity direct-maps the host
doorbell/USERD so there is no per-submit trap (matches the measured zero-overhead
compute path in Mode-1).

---

## (E) Ordered implementation checklist for Mode-2 (prioritized)

Priority order = cuInit succeeds → memcpy works → kernel launch works
(correctness before perf). "EMUL" = add to `src/qemu/nvkvm_gpu_emul.c`
RM-control response filling; "FWD" = forward to host GPU via the stub;
"MMAP/DMA" = real-mmap / address bridge.

### Phase 1 — make `cuInit` return 0 (fill compute-cap data; pure EMUL, no host GPU)
All data below is captured GA106 ground truth (§B); fill the response buffers
instead of echoing NV_OK with zeros. Do these first, in this order:

1. **EMUL `GR_GET_INFO` (0x20801201)** — fill the 58-entry `{index,value}` list in
   the user buffer at `grInfoList` (params off 8) with the B.3 table. **Highest
   priority — this is the dominant `cuInit=100` cause** (SM_VERSION=0x806, GPC=7,
   TPC_PER_GPC=6, SM_PER_TPC=2, core count 3584, etc.).
2. **EMUL `GR_GET_GPC_MASK` (0x122a)** → gpcMask=0x07 at param off 16.
3. **EMUL `GR_GET_TPC_MASK` (0x122b)** → per-gpcId tpcMask at off 20: GPC0=0x1e,
   GPC1=0x1f, GPC2=0x1f (captured).
4. **EMUL `GR_GET_CAPS_V2` (0x1227)** → 23-byte capsTbl
   `b0 62 00 00 00 00 00 01 00 00 00 10 01 08 00 10 10 00 00 00 04 c0 05`.
5. **EMUL `GR_GET_GLOBAL_SM_ORDER` (0x121b)** → numSm=28 (off 9216), numTpc=14
   (off 9218), and the `globalSmId[512]` array (capture full bytes from
   `/tmp/dump.log`; 9240 B total).
6. **EMUL `GRMGR_GET_GR_FS_INFO` (0x3801)** → floorsweep masks (1928 B, captured).
7. **EMUL `MC_GET_ARCH_INFO` (0x1701)** → arch=0x170, impl=0x06, revision=0xa1.
8. **EMUL `GPU_GET_INFO_V2` (0x0102)** → the 11 idx/val pairs in B.1
   (esp. DMABUF_CAP=1, DEBUGGING_CAP=1).
9. **EMUL `GPU_GET_ENGINES_V2` (0x0170)** → the 9-engine list (GR + CEs + NVDEC/ENC/SEC2).
10. **EMUL `GPU_GET_SIMULATION_INFO` (0x0119)** → report **real silicon** (0), not sim.
11. **EMUL `FB_GET_INFO_V2` (0x1303)** → FB size/heap (captured first entry 0x00c00000).
12. **EMUL `GPU_GET_CLASSLIST_V2` (0x800292)** → the captured class list (must
    contain 0xC56F/0xC7B5/0xC7C0/0x90F1).
13. **EMUL `GPU_GET_NAME_STRING`/`SHORT_NAME` (0x0110/0x0111)** → "NVIDIA GeForce RTX 3060".
14. **EMUL caps**: `HOST_GET_CAPS_V2` (0x801402), `FB_GET_CAPS_V2` (0x801307),
    `BUS_GET_PCI_BAR_INFO` (0x1803, match emulated BARs), `GPU_GET_MEMOP_ENABLE`
    (0x27b), `QUERY_COMPUTE_MODE_RULES` (0x0131).
After this set, `cuInit`/`cuDeviceGet`/`cuDeviceGetAttribute` have non-zero
compute-cap data and should stop returning 100. **Validate**: re-run the shim in
the guest and diff each captured response against `vh` byte-for-byte.

### Phase 2 — `cuCtxCreate` + real host channel/context (FWD, reuses Mode-1 stub)
15. **FWD** at the guest's channel/TSG/context ALLOC (RM_ALLOC of `0xa06c`,
    `0xc56f`, `0xc7c0`, FERMI_VASPACE_A `0x90f1`): allocate the **real host**
    TSG + GPFIFO channel + GR/compute context via the stub (the stub holds the
    real RM client/device/subdevice fds).
16. **EMUL/FWD `GR_GET_CTX_BUFFER_SIZE` (0x1218)** — return the host golden-ctx
    buffer sizes so the guest allocates a matching ctx buffer.
17. **FWD `GET_WORK_SUBMIT_TOKEN` (0xc36f0108)** — return the **real host channel's**
    work-submit token (guest will write it to the doorbell page).

### Phase 3 — `cuMemAlloc` / memcpy (MMAP/DMA bridge; reuses Mode-1 UVM + GPA window)
18. **MMAP** back the UVM managed range (fd 9) and FB allocations with real host
    memory via the stub's UVM fd (existing Mode-1 forwarding).
19. **DMA** OS-descriptor the guest-RAM HVA into the host CE/GR context VAS
    (`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR 0x00DE`), so host CE reads/writes guest
    buffers at the guest's GPU VAs (via `nvkvm_walk_pdb` GPA resolution).
20. **MMAP** the CE channel's USERD/doorbell (4 KiB pages) + semaphore read page to
    the real host channel's MMIO/memory (forwarded mmap / GPA-window memslot).
21. **Bring-up shortcut**: trap-and-forward the doorbell write first (correctness),
    then direct-map for parity. **Validate**: `cuMemcpyHtoD`+`DtoH` byte-exact.

### Phase 4 — kernel launch (GR compute; FWD + same bridge)
22. Ensure the compute channel (0xc56f) + AMPERE_COMPUTE_B (0xc7c0) object are a
    **real host channel/object** (Phase 2). The guest's pushbuffer with SET_OBJECT
    + launch methods + report-semaphore runs unchanged on the host GPU because the
    GPU VAs resolve via the §D bridge.
23. Doorbell ring (trap-forward → direct-map) submits to the real host channel;
    report semaphore on the real semaphore page satisfies `cudaDeviceSynchronize`.
24. **Validate**: full SGEMM, C[0]=2048 byte-exact vs host.

### Verification harness
Re-use the host shim (`/tmp/shim*.c` on `vh`) inside the guest to capture the same
phase-tagged RM_CONTROL byte stream and diff against the host capture. Any control
whose response bytes differ from `vh` is a remaining Mode-2 gap. The compute-cap
controls in §B.1/§B.3 must match byte-for-byte for `cuInit` to pass.

## Key file references
- Emulator (add EMUL responses here): `src/qemu/nvkvm_gpu_emul.c`
- Forwarding dispatch / stub bridge (FWD): `src/qemu/nvkvm_dispatch.c`, `src/stub/`
- GA106 device-info verbatim replay: `src/qemu/mode2_devinfo_ga106.h`
- GPA window / memslot install (MMAP): `docs/design/gpa_window_pci_bar.md`
- GR forwarding design (Phase B): `docs/design/mode2_gr_forwarding.md`
- SDK: `research_clones/ogkm/src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gr.h`,
  `ctrl2080gpu.h`, `ctrl2080mc.h`, `ctrl0080/ctrl0080gr.h`; escapes:
  `kernel-open/common/inc/nv-ioctl-numbers.h`, `nv_escape.h`; `nvos.h:2230`.
- Live captures on `vh`: `/tmp/mm.strace`, `/tmp/shim.log`, `/tmp/dump.log`, `/tmp/dump3.log`.
