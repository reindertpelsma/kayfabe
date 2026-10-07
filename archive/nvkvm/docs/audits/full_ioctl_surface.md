# nvkvm full ioctl-surface completeness audit

Read-only gap analysis of the FULL NVIDIA open-gpu-kernel-module ioctl surface
(driver 575.51.03, `/root/open-gpu-kernel-modules/` on the vast.ai host) versus
what nvkvm actually allows. Every kmd-accepted ioctl is classified as an
**INTENTIONAL deny** (privileged / rootful / unused / dangerous) or a true
**SUPPORT GAP** (something a normal CUDA/cuDNN/NCCL workload might hit that we
do not allow).

Sources of truth:
- Frontend numbers: `kernel-open/common/inc/nv-ioctl-numbers.h`,
  `nv-ioctl-numa.h`, `src/nvidia/arch/nvalloc/unix/include/nv_escape.h`.
- Frontend dispatch (what the kmd accepts): `kernel-open/nvidia/nv.c`
  (`nvidia_ioctl` switch → `default: rm_ioctl`), then
  `src/nvidia/arch/nvalloc/unix/src/osapi.c` (`rm_ioctl`) and
  `.../escape.c` (`RmIoctl` switch).
- UVM numbers: `kernel-open/nvidia-uvm/uvm_ioctl.h` +
  `uvm_linux_ioctl.h` (INITIALIZE/DEINITIALIZE).
- nvkvm allows: `src/qemu/nvkvm_fe_alloc_allowlist.h`
  (`nvkvm_fe_nr_allowlist[]`), `src/qemu/nvkvm_ctrl_allowlist.h`,
  `src/qemu/nvkvm_isolate_handlers.c` (`nvkvm_uvm_schema[]`).
- Device registration: `src/guest/nvkvm_main.c` (`register_devices`).

All NRs below are the `_IOC_NR` byte the kmd switches on. nvkvm gates on
`_IOC_TYPE == 'F'` first, then on this NR (see memory: IOCTL NR collision bug).

---

## 1. Summary tables

### Frontend (`/dev/nvidiactl`, `/dev/nvidia0..N`, magic `'F'`)

| Category | kmd-accepted | nvkvm-allowed | intentional deny | TRUE GAP |
|---|---|---|---|---|
| Frontend NV_ESC_* | 27 | 24 | 3 | 0 |

### UVM (`/dev/nvidia-uvm`)

| Category | kmd-accepted | nvkvm-allowed | intentional deny | TRUE GAP |
|---|---|---|---|---|
| UVM_* | ~50 defined | 33 | rest | 0 |

### RM control commands (sub-surface of NV_ESC_RM_CONTROL 0x2a)

| Category | nvkvm-allowed (explicit) | rule-passthrough | denied |
|---|---|---|---|
| NV*_CTRL_CMD_* | 132 explicit | GSP-mask `cmd&0x8000` + class 0x2081 | all else (default-deny) |

### Other device nodes the kmd exposes

| Node | kmd ioctl surface | nvkvm exposes node? | Verdict |
|---|---|---|---|
| `/dev/nvidia-modeset` | `NVKMS_IOCTL_CMD` (magic `'m'`, nr 0) | **NO** | deny-by-construction |
| `/dev/nvidia-caps/*` (nv-caps) | **none** (no `unlocked_ioctl`; read/poll only) | **NO** | deny-by-construction |
| `/dev/nvidia-caps-imex-channels` | none relevant | **NO** | deny-by-construction |
| `/dev/nvidia-nvswitch*` | nvswitch ioctls (separate driver) | **NO** | deny-by-construction (driver not present) |
| `/dev/nvidia-vgpu*` | vgpu mgr ioctls | **NO** | deny-by-construction |

### TRUE GAPS (CUDA/cuDNN/NCCL ioctls we accept-list but do not allow)

**None found.** Every frontend NR and every UVM command not in the nvkvm
allowlists is justified below as privileged, rootful, display/tools-only, or
unused by the compute path. The one borderline case (`NV_ESC_RM_SHARE` 0x35) is
*allowed* by nvkvm. Two low-risk frontend NRs (`STATUS_CODE` 0xd1,
`QUERY_DEVICE_INTR` 0xd5) are denied with no observed CUDA impact — see notes.

---

## 2. Frontend NV_ESC_* — every kmd-accepted ioctl

The kmd accepts an NV_ESC_* iff a `case` exists in `nvidia_ioctl` (nv.c) OR in
`rm_ioctl`/`RmIoctl` (osapi.c/escape.c). `NV_ESC_IOCTL_XFER_CMD` (0xd3) is a
pure transport wrapper, unwrapped before dispatch.

Legend: ✅ = in `nvkvm_fe_nr_allowlist[]`; ⛔ = denied (classification given).

| NR (hex / dec) | NV_ESC name | nvkvm | Class — why |
|---|---|---|---|
| 0x27 / 39  | RM_ALLOC_MEMORY | ✅ | CUDA core (memory alloc) |
| 0x28 / 40  | RM_ALLOC_OBJECT | ⛔ | **UNUSED** — legacy object alloc; CUDA uses RM_ALLOC (0x2b). nvproxy also omits. |
| 0x29 / 41  | RM_FREE | ✅ | CUDA core |
| 0x2a / 42  | RM_CONTROL | ✅ | CUDA core (sub-gated by ctrl allowlist) |
| 0x2b / 43  | RM_ALLOC | ✅ | CUDA core (sub-gated by alloc-class allowlist) |
| 0x32 / 50  | RM_CONFIG_GET | ⛔ | **PRIVILEGED** — registry/config plumbing; not on CUDA path. nvproxy omits. |
| 0x33 / 51  | RM_CONFIG_SET | ⛔ | **PRIVILEGED/DANGEROUS** — writes RM config; host-state mutation. |
| 0x34 / 52  | RM_DUP_OBJECT | ✅ | CUDA core (cross-client handle dup; cross-VM gated by handle model) |
| 0x35 / 53  | RM_SHARE | ✅ | CUDA core (object sharing) |
| 0x37 / 55  | RM_CONFIG_GET_EX | ⛔ | **PRIVILEGED** — extended config get. |
| 0x38 / 56  | RM_CONFIG_SET_EX | ⛔ | **PRIVILEGED/DANGEROUS** — extended config set. |
| 0x39 / 57  | RM_I2C_ACCESS | ⛔ | **DANGEROUS** — raw I2C bus access (VBIOS/EEPROM/board mgmt). |
| 0x41 / 65  | RM_IDLE_CHANNELS | ✅ | CUDA core (channel quiesce on teardown) |
| 0x4a / 74  | RM_VID_HEAP_CONTROL | ✅ | CUDA core (VidHeapControl alloc path) |
| 0x4d / 77  | RM_ACCESS_REGISTRY | ⛔ | **PRIVILEGED/DANGEROUS** — reads/writes the NVIDIA registry. |
| 0x4e / 78  | RM_MAP_MEMORY | ✅ | CUDA core (BAR/sysmem map) |
| 0x4f / 79  | RM_UNMAP_MEMORY | ✅ | CUDA core |
| 0x52 / 82  | RM_GET_EVENT_DATA | ⛔ | **DENY (handled in guest)** — event drain served by the guest module's event path, not forwarded as a frontend NR. No host pointer crosses. Not a compute gap. |
| 0x54 / 84  | RM_ALLOC_CONTEXT_DMA2 | ✅ | CUDA core (ctxdma alloc) |
| 0x56 / 86  | RM_ADD_VBLANK_CALLBACK | ⛔ | **UNUSED/DISPLAY** — vblank is a display/KMS concept; never on CUDA path. |
| 0x57 / 87  | RM_MAP_MEMORY_DMA | ✅ | CUDA core (DMA map) |
| 0x58 / 88  | RM_UNMAP_MEMORY_DMA | ✅ | CUDA core |
| 0x59 / 89  | RM_BIND_CONTEXT_DMA | ⛔ | **UNUSED** — legacy ctxdma bind; modern CUDA uses VASPACE mapping. nvproxy omits. |
| 0x5c / 92  | RM_EXPORT_OBJECT_TO_FD | ⛔ | **DENY (cross-process fd)** — object→fd export crosses the process/VM fd boundary; superseded for our path by EXPORT_TO_DMABUF_FD (0x70, sanitized). |
| 0x5d / 93  | RM_IMPORT_OBJECT_FROM_FD | ⛔ | **DENY (cross-process fd)** — import side of the above; would import a foreign object handle. |
| 0x5e / 94  | RM_UPDATE_DEVICE_MAPPING_INFO | ✅ | CUDA core (forged NV_OK; mapping via GPA window — see memory NVOS56) |
| 0x5f / 95  | RM_LOCKLESS_DIAGNOSTIC | ⛔ | **DANGEROUS/DIAG** — lockless diagnostic backdoor; never used by CUDA. |
| 0x70 / 112 | EXPORT_TO_DMABUF_FD | ✅ | CUDA core (dma-buf export; nvkvm-sanitized extension) |

**Low-numbered NV_ESC (NV_IOCTL_BASE = 200 = 0xc8):**

| NR (hex / dec) | NV_ESC name (+offset) | nvkvm | Class — why |
|---|---|---|---|
| 0xc8 / 200 | CARD_INFO (+0) | ✅ | CUDA core (enumerate GPUs at init) |
| 0xc9 / 201 | REGISTER_FD (+1) | ✅ | CUDA core (links device fd → ctl fd) |
| 0xce / 206 | ALLOC_OS_EVENT (+6) | ✅ | CUDA core (eventfd registration) |
| 0xcf / 207 | FREE_OS_EVENT (+7) | ✅ | CUDA core |
| 0xd1 / 209 | STATUS_CODE (+9) | ⛔ | **UNUSED-by-CUDA** — maps NV_STATUS→errno string; libcuda does not issue it on our observed paths. Benign; could be added if a future tool needs it. |
| 0xd2 / 210 | CHECK_VERSION_STR (+10) | ✅ | CUDA core (driver/userspace version handshake) |
| 0xd3 / 211 | IOCTL_XFER_CMD (+11) | ⛔ (n/a) | **TRANSPORT** — never a leaf NR; the kmd unwraps it to the inner cmd, which is then gated. nvkvm forwards inner cmds directly, so this wrapper is not used. Not a gap. |
| 0xd4 / 212 | ATTACH_GPUS_TO_FD (+12) | ✅ | CUDA core (binds GPU set to fd) |
| 0xd5 / 213 | QUERY_DEVICE_INTR (+13) | ⛔ | **DENY (BAR0 reg read)** — reads `NV_RM_DEVICE_INTR_ADDRESS` straight from the GPU register map; an interrupt-status MMIO peek. Not on the CUDA compute path (CUDA uses OS events, not this poll). Reserve for the signal-delivery milestone if needed; deny for now. |
| 0xd6 / 214 | SYS_PARAMS (+14) | ✅ | CUDA core (NUMA memblock size handshake; one-shot) |
| 0xd7 / 215 | NUMA_INFO (+15) | ✅ | CUDA core (read-only NUMA topology query) |
| 0xd8 / 216 | SET_NUMA_STATUS (+16) | ⛔ | **ROOTFUL** — kmd gates with `NV_IS_SUSER()`; onlines/offlines GPU NUMA memory. Host-admin only. |
| 0xda / 218 | WAIT_OPEN_COMPLETE (+18) | ✅ | CUDA core (async-open completion wait) |

Notes:
- `0x32/0x33/0x37/0x38/0x4d/0x5f` are RM config/registry/diagnostic escapes:
  none touched by the CUDA user path; all are host-state or diagnostic surfaces.
  Correctly denied.
- `0x5c/0x5d` (export/import object to/from fd) are the cross-process handle
  smuggling primitives. Denying them is a security requirement, not a gap:
  CUDA P2P/IPC in our model goes through the sanitized dma-buf path (0x70) or
  RM_DUP/SHARE within the access-gated handle namespace.
- **STATUS_CODE (0xd1)** and **QUERY_DEVICE_INTR (0xd5)** are the only two
  general-purpose (non-privileged) frontend NRs the kmd accepts that nvkvm
  denies. Neither is observed on cuInit/matmul/vec_add/memcpy/nvidia-smi. They
  are flagged here as deny-with-low-confidence: if the signal-delivery milestone
  needs interrupt-status polling, QUERY_DEVICE_INTR is the candidate to revisit.

---

## 3. UVM_* — every kmd-defined ioctl vs `nvkvm_uvm_schema[]`

UVM ioctls are default-deny: only the 33 commands in `nvkvm_uvm_schema[]` are
forwarded; the kmd's `_IOC_SIZE` is ignored (schema carries its own size; see
memory: UVM_INITIALIZE red herring). Numbers are the raw UVM base index `i`
(plus the two 0x3000000x absolutes).

✅ = in schema. ⛔ = denied by omission.

| # | UVM name | nvkvm | Class — why |
|---|---|---|---|
| 0x30000001 | UVM_INITIALIZE | ✅ | CUDA core (UVM bring-up) |
| 0x30000002 | UVM_DEINITIALIZE | ✅ | CUDA core |
| 1  | UVM_RESERVE_VA | ⛔ | **UNUSED** — legacy test/explicit-VA API; modern CUDA uses managed alloc + MAP_EXTERNAL. |
| 2  | UVM_RELEASE_VA | ⛔ | **UNUSED** — pair of above. |
| 3  | UVM_REGION_COMMIT | ⛔ | **UNUSED** — legacy region API. |
| 4  | UVM_REGION_DECOMMIT | ⛔ | **UNUSED** — legacy region API. |
| 5  | UVM_REGION_SET_STREAM | ⛔ | **UNUSED** — legacy stream/region API. |
| 6  | UVM_SET_STREAM_RUNNING | ⛔ | **UNUSED** — legacy stream API. |
| 7  | UVM_SET_STREAM_STOPPED | ⛔ | **UNUSED** — legacy stream API. |
| 9  | UVM_RUN_TEST | ⛔ | **DANGEROUS/TEST** — in-kernel test hooks; debug builds only. |
| 10 | UVM_ADD_SESSION | ⛔ | **TOOLS** — profiler session (counters/events). |
| 11 | UVM_REMOVE_SESSION | ⛔ | **TOOLS** — profiler session. |
| 12 | UVM_ENABLE_COUNTERS | ⛔ | **TOOLS** — perf counters. |
| 13 | UVM_MAP_COUNTER | ⛔ | **TOOLS** — perf counters. |
| 14 | UVM_CREATE_EVENT_QUEUE | ⛔ | **TOOLS** — event tracing. |
| 15 | UVM_REMOVE_EVENT_QUEUE | ⛔ | **TOOLS** — event tracing. |
| 16 | UVM_MAP_EVENT_QUEUE | ⛔ | **TOOLS** — event tracing. |
| 17 | UVM_EVENT_CTRL | ⛔ | **TOOLS** — event tracing. |
| 18 | UVM_REGISTER_MPS_SERVER | ⛔ | **UNUSED** — MPS daemon (cross-process compute sharing); not in single-tenant CUDA path. |
| 19 | UVM_REGISTER_MPS_CLIENT | ⛔ | **UNUSED** — MPS client side. |
| 20 | UVM_GET_GPU_UUID_TABLE | ⛔ | **UNUSED** — legacy UUID table; superseded by RM device enum. |
| 21 | UVM_REGION_SET_BACKING | ⛔ | **UNUSED** — legacy region backing. |
| 22 | UVM_REGION_UNSET_BACKING | ⛔ | **UNUSED** — legacy region backing. |
| 23 | UVM_CREATE_RANGE_GROUP | ✅ | CUDA core (managed memory range groups) |
| 24 | UVM_DESTROY_RANGE_GROUP | ✅ | CUDA core |
| 25 | UVM_REGISTER_GPU_VASPACE | ✅ | CUDA core (binds RM VASPACE to UVM; fd field 16 translated) |
| 26 | UVM_UNREGISTER_GPU_VASPACE | ✅ | CUDA core |
| 27 | UVM_REGISTER_CHANNEL | ✅ | CUDA core (GPFIFO channel registration) |
| 28 | UVM_UNREGISTER_CHANNEL | ✅ | CUDA core |
| 29 | UVM_ENABLE_PEER_ACCESS | ✅ | CUDA core (multi-GPU P2P; NCCL) |
| 30 | UVM_DISABLE_PEER_ACCESS | ✅ | CUDA core |
| 31 | UVM_SET_RANGE_GROUP | ✅ | CUDA core |
| 33 | UVM_MAP_EXTERNAL_ALLOCATION | ✅ | CUDA core (map RM alloc into UVM; 9264 B params) |
| 34 | UVM_FREE | ✅ | CUDA core |
| 35 | UVM_MEM_MAP | ⛔ | **GAP-candidate→benign** — maps a UVM-internal mem region; not observed on our compute paths (alloc goes through MAP_EXTERNAL_ALLOCATION / ALLOC_SEMAPHORE_POOL). Watch if cuMemMap/VMM API is exercised. See §5. |
| 37 | UVM_REGISTER_GPU | ✅ | CUDA core |
| 38 | UVM_UNREGISTER_GPU | ✅ | CUDA core |
| 39 | UVM_PAGEABLE_MEM_ACCESS | ✅ | CUDA core (HMM/pageable query) |
| 40 | UVM_PREVENT_MIGRATION_RANGE_GROUPS | ⛔ | **UNUSED** — range-group migration control; not on observed path. |
| 41 | UVM_ALLOW_MIGRATION_RANGE_GROUPS | ⛔ | **UNUSED** — pair of above. |
| 42 | UVM_SET_PREFERRED_LOCATION | ✅ | CUDA core (cudaMemAdvise) |
| 43 | UVM_UNSET_PREFERRED_LOCATION | ✅ | CUDA core |
| 44 | UVM_ENABLE_READ_DUPLICATION | ✅ | CUDA core (cudaMemAdvise) |
| 45 | UVM_DISABLE_READ_DUPLICATION | ✅ | CUDA core |
| 46 | UVM_SET_ACCESSED_BY | ✅ | CUDA core (cudaMemAdvise) |
| 47 | UVM_UNSET_ACCESSED_BY | ✅ | CUDA core |
| 51 | UVM_MIGRATE | ✅ | CUDA core (cudaMemPrefetchAsync) |
| 53 | UVM_MIGRATE_RANGE_GROUP | ✅ | CUDA core |
| 54 | UVM_ENABLE_SYSTEM_WIDE_ATOMICS | ⛔ | **UNUSED** — legacy system-wide atomics toggle; modern driver auto-manages. |
| 55 | UVM_DISABLE_SYSTEM_WIDE_ATOMICS | ⛔ | **UNUSED** — pair of above. |
| 56 | UVM_TOOLS_INIT_EVENT_TRACKER | ⛔ | **TOOLS** — profiler. |
| 57 | UVM_TOOLS_SET_NOTIFICATION_THRESHOLD | ⛔ | **TOOLS** — profiler. |
| 58 | UVM_TOOLS_EVENT_QUEUE_ENABLE_EVENTS | ⛔ | **TOOLS** — profiler. |
| 59 | UVM_TOOLS_EVENT_QUEUE_DISABLE_EVENTS | ⛔ | **TOOLS** — profiler. |
| 60 | UVM_TOOLS_ENABLE_COUNTERS | ⛔ | **TOOLS** — profiler. |
| 61 | UVM_TOOLS_DISABLE_COUNTERS | ⛔ | **TOOLS** — profiler. |
| 62 | UVM_TOOLS_READ_PROCESS_MEMORY | ⛔ | **DANGEROUS** — cross-process memory read; explicit deny (commented in schema). |
| 63 | UVM_TOOLS_WRITE_PROCESS_MEMORY | ⛔ | **DANGEROUS** — cross-process memory write; explicit deny (commented in schema). |
| 64 | UVM_TOOLS_GET_PROCESSOR_UUID_TABLE | ⛔ | **TOOLS** — profiler topology. |
| 65 | UVM_MAP_DYNAMIC_PARALLELISM_REGION | ✅ | CUDA core (CDP / dynamic parallelism) |
| 66 | UVM_UNMAP_EXTERNAL | ✅ | CUDA core |
| 67 | UVM_TOOLS_FLUSH_EVENTS | ⛔ | **TOOLS** — profiler. |
| 68 | UVM_ALLOC_SEMAPHORE_POOL | ✅ | CUDA core (semaphore pool; 9248 B params) |
| 69 | UVM_CLEAN_UP_ZOMBIE_RESOURCES | ⛔ | **UNUSED** — driver-internal cleanup; not user-issued on our path. |
| 70 | UVM_PAGEABLE_MEM_ACCESS_ON_GPU | ✅ | CUDA core (per-GPU pageable query) |
| 71 | UVM_POPULATE_PAGEABLE | ⛔ | **GAP-candidate→benign** — HMM populate; only on pageable/HMM path. Not observed. See §5. |
| 72 | UVM_VALIDATE_VA_RANGE | ✅ | CUDA core |
| 73 | UVM_CREATE_EXTERNAL_RANGE | ✅ | CUDA core |
| 74 | UVM_MAP_EXTERNAL_SPARSE | ⛔ | **UNUSED** — sparse-texture/virtual-memory VMM API; not on observed dense-compute path. See §5. |
| 75 | UVM_MM_INITIALIZE | ✅ | CUDA core (fd field 0 translated; returns NOTHING_TO_DO by design) |
| 79 | UVM_CLEAR_ALL_ACCESS_COUNTERS | ⛔ | **TOOLS/PROFILING** — access-counter reset; profiling-only. |

---

## 4. Other device nodes — deny-by-construction

`src/guest/nvkvm_main.c::register_devices()` registers **only** these char
devices, all on the nvkvm virtio path:
- `/dev/nvidiactl` (major 195, minor 255)
- `/dev/nvidia0 .. /dev/nvidiaN` (major 195, minors 0..N)
- `/dev/nvidia-uvm` + `/dev/nvidia-uvm-tools` (one dynamic major, 2 minors)

It registers **no** modeset, cap, nvswitch, or vgpu node. Therefore the entire
ioctl surface of those nodes is denied simply because the node does not exist in
the guest. Explicit statement per node:

- **nvidia-modeset** — kmd exposes a single ioctl `NVKMS_IOCTL_CMD` (magic
  `'m'`, nr 0; `nvidia-modeset-linux.c`). nvkvm does not register the node →
  **deny-by-construction**. (Display/KMS, irrelevant to headless compute.)
- **nvidia-caps / nv-caps** — the capability fds (`nv-caps.c`,
  `nv-caps-imex.c`) expose **no `unlocked_ioctl` handler at all** (open + read +
  poll only). No ioctl surface to deny, and the node is not registered anyway →
  **deny-by-construction**. (These gate MIG / IMEX privileged capabilities.)
- **nvidia-nvswitch / nvidia-nvlink** — a separate driver
  (`nvidia-nvswitch`); not built or registered in the guest →
  **deny-by-construction**. (Fabric/NVSwitch admin surface.)
- **nvidia-vgpu-mgr / nvidia-vgpu*** — vGPU mediation nodes; not registered →
  **deny-by-construction**. (Host-side vGPU provisioning; rootful.)

---

## 5. Conclusions and watch-list

**No accidental gap blocks a normal CUDA/cuDNN/NCCL workload.** Every kmd ioctl
denied by nvkvm falls into: PRIVILEGED/ROOTFUL (config/registry/SET_NUMA_STATUS),
DANGEROUS (I2C, lockless diagnostic, RUN_TEST, cross-process mem r/w,
export/import object fd), DISPLAY (modeset, vblank), TOOLS/PROFILING (UVM tools
+ counters), or UNUSED-legacy (region/stream/MPS/sparse/system-wide-atomics).

**Confirmed-justified denials of note:**
- `RM_EXPORT/IMPORT_OBJECT_FROM_FD` (0x5c/0x5d), `UVM_TOOLS_*_PROCESS_MEMORY`
  (62/63), `SET_NUMA_STATUS` (0xd8), `RM_I2C_ACCESS` (0x39),
  `RM_LOCKLESS_DIAGNOSTIC` (0x5f) — these are the cross-tenant / host-state
  primitives; denying them is the security posture, not a gap.

**Watch-list (deny today, low-confidence — revisit if a workload regresses or at
the signal-delivery milestone):**
1. **QUERY_DEVICE_INTR (0xd5)** — interrupt-status BAR0 read. Most likely
   candidate to need at the signal/interrupt-delivery milestone. Currently
   denied; not on observed CUDA paths.
2. **UVM_MEM_MAP (35)** and **UVM_POPULATE_PAGEABLE (71)** — exercised only by
   the HMM/pageable and VMM (cuMemMap) paths, which our current managed-memory
   demos do not hit. Add to schema if a cuMemMap / HMM workload appears.
3. **UVM_MAP_EXTERNAL_SPARSE (74)** — sparse-VMM API (e.g. tiled resources /
   `cuMemAddressReserve`+`cuMemMap` sparse). Not on dense-compute path; add if a
   framework using the CUDA VMM sparse API surfaces.
4. **STATUS_CODE (0xd1)** — pure errno-string helper; harmless to add if a
   diagnostic tool needs it.

These four are explicitly **not** classified as gaps (no observed CUDA usage),
but are the only non-privileged, non-tools surfaces a future compute feature
could plausibly reach. Everything else is intentional and correct.
