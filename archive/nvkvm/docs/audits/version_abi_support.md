# nvkvm NVIDIA driver ABI version-compatibility audit

Date: 2026-05-30
Scope: source-code-only. Target driver: NVIDIA 575.51.03 (open kernel modules) /
libcuda 575.51.03. Goal: understand what it takes for nvkvm to support a *range*
of officially-supported NVIDIA driver versions, using gVisor's nvproxy as the
reference design.

TL;DR — nvkvm is single-version by construction. It reads the host driver
version string (GET_BUILD_VERSION / CHECK_VERSION_STR) but **never uses it to
select an ABI layout**. Every struct size, every embedded-pointer offset, every
alloc-class size, and every allowlist entry is hardcoded to the 575.51.03 open
driver. nvproxy, by contrast, keys a per-version ABI table off an *exact*
`major.minor.patch` match and expresses each version as a delta on its parent.
Supporting a range in nvkvm means importing nvproxy's version-keyed-table idea
into three places: the QEMU isolate handlers, the guest sanitizers, and the ABI
headers.

---

## 1. Where nvkvm pins / assumes 575.51.03

### 1a. The version string is read but never drives layout selection

* `src/qemu/virtio_nvgpu.c:907-926` — on device realize, QEMU opens
  `/dev/nvidiactl` and issues `NV_ESC_CHECK_VERSION_STR` with `cmd='2'` (query,
  non-strict), copying `ver.version_string` into `nv->driver_version[64]`.
* `src/qemu/virtio_nvgpu.c:945-946` — copies that string into the shm control
  block (`ctrl->driver_version`).
* `src/guest/nvkvm_virtio.c:557-562` — guest reads it back into
  `state->driver_version`.
* `src/common/nvkvm_proto.h:78` and `src/guest/nvkvm.h:251` and
  `src/qemu/virtio_nvgpu.h:250` — the 64-byte field definitions.

Grepping for any *branch* on this value (`grep driver_version`) turns up only
copies and the GET_BUILD_VERSION embedded-pointer plumbing
(`src/guest/nvkvm_main.c:1018,1531-1552`, `src/qemu/nvkvm_frontend.c:265,296`).
**There is no `if (version >= ...)` anywhere.** The string exists purely so
libcuda's own version check passes; it is dead weight for ABI selection today.

### 1b. Hardcoded struct layouts (ABI headers)

`src/abi/nvgpu.h` and `src/abi/uvm.h` are flat C structs with no versioning.
Each carries a "verified on the 575 SDK / by sizeof" comment, which is exactly
where a version skew silently corrupts:

| Struct | File:line | Version pinned-to | Silent-corruption risk |
|---|---|---|---|
| `nv_channel_alloc_params_v570` (TURING/AMPERE/HOPPER GPFIFO) | `src/abi/nvgpu.h:555-592` | "driver >= 570" V570 layout = base + `TPCConfigID` + pad | **HIGH**. nvproxy switches this struct at 570.86.15 (base `NV_CHANNEL_ALLOC_PARAMS` → `_V570`). On a <570 driver we'd send 8 extra bytes; on a layout change we'd misplace `engineType` (the field that already caused the runlist bug, see `gpfifo_schedule_runlist_bug`). |
| `nv_memory_allocation_params_v545` (NV50_MEMORY_VIRTUAL / LOCAL_USER / SYSTEM) | `src/abi/nvgpu.h:596-623` | "V545 layout, adds `numa_node`+pad" | **HIGH**. nvproxy keys `NV_MEMORY_ALLOCATION_PARAMS` vs `_V545` at 545.23.06. Wrong size → kernel reads/writes past the param buffer. |
| `nv00de_alloc_parameters_v545` (RM_USER_SHARED_DATA) | `src/abi/nvgpu.h:497-500` | "V545, single u64" | MED. nvproxy switches at 545.23.06. |
| `nv_vaspace_allocation_parameters` (FERMI_VASPACE_A) | `src/abi/nvgpu.h:504-514` | "pre-580 layout" | **HIGH**. nvproxy switches to `NV_VASPACE_ALLOCATION_PARAMETERS_V580` at 580.65.06 (`version.go:1060`). This is the struct whose write-back already broke cuCtxCreate once (`cuctxcreate_401_diagnosis`). |
| `nvos46_parameters` (RM_MAP_MEMORY_DMA) | `src/abi/nvgpu.h:313-325` | 56B pre-580 | MED. nvproxy switches to `NVOS46_PARAMETERS_V580` at 580.65.06 (`version.go:1059`); 550.40.07 already switched the *unmap* path to `NVOS47_PARAMETERS_V550` (`version.go:847`). |
| `nvos21_parameters` (32B) / `nvos64_parameters` (48B) / `nvos55_parameters` (28B) / `nvos02_with_fd` (56B) / `nvos33_with_fd` (56B) | `src/abi/nvgpu.h:148-156,167-178,211-219,232-245,249-261` | "575 SDK", sizes asserted in `tests/abi_parity` | MED. These core escapes have been ABI-stable across 535→580, but their sizes are baked into the stub status-offset switch (see 1d). |
| `uvm_map_external_allocation_params` (V550, offset-9248 family) | `src/abi/uvm.h:157-174` | "550.54.14+ V550 layout; `UVM_MAX_GPUS_V2 = 256`, count widened to u64" | **HIGHEST**. nvproxy switches `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` → `_V550` at 550.54.14 (`version.go:899`). Pre-550 this array is 1 entry, not 256; getting it wrong is a 9KB buffer-size mismatch. |
| `uvm_alloc_semaphore_pool_params` (9248B) | `src/abi/uvm.h:268-282` | V550, `per_gpu_attributes[256]` | **HIGHEST**. nvproxy switches at 550.54.14 (`version.go:898`). The `9248` magic appears literally in code (see 1c/1d). |
| UVM `SET_PREFERRED_LOCATION` / `MIGRATE` | `src/abi/uvm.h:183-199` (no V550 suffix) | — | MED. nvproxy switched both to `_V550` at 550.40.07 (`version.go:860-861`); nvkvm uses one layout. |

The compute / graphics / DMA-copy hClass IDs (`src/abi/nvgpu.h:60-84`) and
`nv_gr_allocation_parameters` (16B, line 99-110) / `nvb0b5_allocation_parameters`
(8B, line 86-98) are arch-keyed not version-keyed, so they are relatively
version-stable — but new arches (BLACKWELL_COMPUTE_B, BLACKWELL_DMA_COPY_B,
BLACKWELL GPFIFO) are added by nvproxy *at* 570.86.15 (`version.go:999-1004`),
i.e. supporting newer GPUs is itself a version-gated change.

### 1c. The "offset 9248" constant (UVM V550 embedded fd)

* `src/stub/nvkvm_stub.c:836-844` — hardcodes `uvm_embedded_fd_off = 9248` for
  the V550 `UVM_MAP_EXTERNAL_ALLOCATION` layout ("driver >= 550.54.14, our
  575.51.03 included"). The comment derives 9248 from
  `base(8)+length(8)+offset(8)+per_gpu[256]*36(9216)+count(8)`.
* `src/qemu/nvkvm_isolate_handlers.c:450,465` — UVM schema `min_size` table:
  `UVM_MAP_EXTERNAL_ALLOCATION` = 9264, `UVM_ALLOC_SEMAPHORE_POOL` = 9248.
* `src/qemu/nvkvm_isolate_handlers.c:1815` and `src/stub/nvkvm_stub.c:1419` —
  "SEM_POOL is 9248 bytes" caps on the intent blob.
* `src/common/nvkvm_proto.h:513` — "9248 bytes; QEMU validates size exactly".

On a *pre-550* driver this offset is wrong (array is 1 entry), and on a future
layout change it would move. This is the single most version-fragile constant in
the tree because it is duplicated across stub + QEMU + proto.

### 1d. Stub status/fd-offset switch — sizes baked per escape

`src/stub/nvkvm_stub.c:1056-1085` switches the nvstatus read-back offset purely
on `_IOC_NR` and `param_size`, with "575 SDK" comments:

```
0x34: off = 24  /* NVOS55 28B status@24 (575 SDK) */
0x4e: off = 40  /* nvos33_with_fd 56B status@40, fd@48 */
0x57: off = 48  /* nvos46 56B status@48 */
...
```

If any of these structs changes size across versions (e.g. NVOS46 at 580), the
stub reads `status`/`fd` from the wrong offset → silent garbage status, possibly
a wrong fd dup. `src/stub/nvkvm_stub.c:887-905` similarly hardcodes the frontend
embedded-fd offsets (RM_MAP_MEMORY fd@48, MM_INITIALIZE@0, REGISTER_GPU_VASPACE@16).

### 1e. NV2080_CTRL_GPU_PID_INFO stride = 72

* `src/qemu/nvkvm_isolate_handlers.c:557-561` — `NVKVM_PIDINFO_STRIDE 72u`,
  "Verified via sizeof on the 575 open-driver SDK headers." A control-param
  struct size baked into a loop stride.

### 1f. Allowlists derived from the nvproxy 575 ABI snapshot

* `src/qemu/nvkvm_fe_alloc_allowlist.h:5-13` — frontend NR set + RM_ALLOC class
  set explicitly described as "gVisor nvproxy 575-ABI". These are point-in-time
  snapshots: nvproxy's class set grows at 570 (Blackwell), 550 (IMEX/fabric),
  545 (memory-export), 580 (more video encoders).
* `src/qemu/nvkvm_ctrl_allowlist.h:5` — "nvproxy 575-ABI compUtil-tagged control
  cmds". nvproxy adds/removes control cmds at nearly every version bump
  (e.g. 575.51.02 *deletes* two FB_QUERY_DRAM_ENCRYPTION cmds and adds the
  `_V575` variants, `version.go:1039-1042`). A frozen allowlist would deny
  legitimate cmds on other versions and (less likely) miss new sanitization
  needs.
* `src/qemu/nvkvm_isolate_handlers.c:418-426` — the UVM `min_size` table is
  *deliberately* the 575 sizes ("REGISTER_GPU is 32B here, not gVisor's
  40B-with-NUMA; REGISTER_CHANNEL 48 not 56; MIGRATE 48 not 56"). This is an
  explicit, documented divergence from gVisor's newer layouts — i.e. nvkvm has
  already chosen 575 sizes over gVisor's, which would mis-reject other versions.

---

## 2. How gVisor nvproxy handles multiple versions

### 2a. Version type and parsing — `nvconf/version.go`

`nvconf.DriverVersion{major,minor,patch int}` (`nvconf/version.go:26-30`) with
`DriverVersionFrom(string)` (`:38`) parsing `"575.51.03"`. Comparison helpers
`Equals` (`:67`) and `IsGreaterThan` (`:74`). Note: comparison assumes both
versions are on the *same dev branch*; cross-branch ordering is not meaningful
(NVIDIA forks branches off "main" at a point, see the branch comments below).

### 2b. The ABI table — `nvproxy/version.go`

The core type (`version.go:100-107`):

```go
type driverABI struct {
    frontendIoctl   map[uint32]frontendIoctlHandler
    uvmIoctl        map[uint32]uvmIoctlHandler
    controlCmd      map[uint32]controlCmdHandler
    allocationClass map[nvgpu.ClassID]allocationClassHandler
    getInfo         driverABIInfoFunc
}
```

The four maps are exactly the four documented branch points (`version.go:90-99`):
1. frontend ioctls keyed by `IOC_NR(cmd)`
2. uvm ioctls keyed by `cmd`
3. RM_CONTROL inner commands keyed by `NVOS54_PARAMETERS.Cmd`
   (cmds with `RM_GSS_LEGACY_MASK` are *not* versioned)
4. RM_ALLOC classes keyed by `NVOS64_PARAMETERS.HClass`

Global registry (`version.go:142`): `var abis map[nvconf.DriverVersion]abiConAndChecksum`.
`addDriverABI(major,minor,patch, checksumX86, checksumARM, cons)` (`version.go:149`)
registers a constructor + the SHA256 of the NVIDIA `.run` installer for that exact
version.

### 2c. Per-version deltas expressed as inheritance

Each version constructor *calls its parent constructor and mutates the maps*.
This is the key mechanism — a version is a **diff on its predecessor**:

* 545.23.06 (`version.go:804+`): adds `NV00DE_ALLOC_PARAMETERS_V545`,
  `NV_MEMORY_ALLOCATION_PARAMS_V545`, etc.
* 550.40.07 (`version.go:844-894`): swaps `NV_ESC_RM_UNMAP_MEMORY_DMA` →
  `NVOS47_PARAMETERS_V550`; `UVM_SET_PREFERRED_LOCATION`/`UVM_MIGRATE` →
  `_V550`; adds IMEX/fabric control cmds + alloc classes.
* 550.54.14 (`version.go:896-910`): swaps `UVM_ALLOC_SEMAPHORE_POOL` and
  `UVM_MAP_EXTERNAL_ALLOCATION` → `_V550` (**the offset-9248 layout**).
* 570.86.15 (`version.go:990-1026`): swaps TURING/AMPERE/HOPPER/BLACKWELL
  `*_CHANNEL_GPFIFO_A` → `rmAllocChannelV570` (`NV_CHANNEL_ALLOC_PARAMS_V570`,
  adds `TPCConfigID`); adds Blackwell compute/copy/usermode classes.
* 575.51.02 (`version.go:1037-1055`): builds on 570.133.20; *deletes* two
  control cmds and adds their `_V575` variants + `THERMAL_SYSTEM_EXECUTE_V2`.
* 580.65.06 (`version.go:1057-1078`): swaps `NV_ESC_RM_MAP_MEMORY_DMA` →
  `NVOS46_PARAMETERS_V580` and `FERMI_VASPACE_A` →
  `NV_VASPACE_ALLOCATION_PARAMETERS_V580`.
* 590.44.01 (`version.go:1087-1104`): `UVM_UNREGISTER_CHANNEL`/`UVM_FREE` →
  `_V590`; `NV50_P2P`/`NV_MEMORY_MULTICAST_FABRIC` → `_V590`.

The struct deltas live in `pkg/abi/nvgpu/`: e.g.
`NV_CHANNEL_ALLOC_PARAMS_V570` embeds the base struct + `TPCConfigID`
(`classes.go:477-485`); `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS_V550` uses
`[UVM_MAX_GPUS_V2]UvmGpuMappingAttributes` (`uvm.go:332-337`) where the base uses
`[UVM_MAX_GPUS]` (`uvm.go:303`). nvproxy keeps *both* struct versions compiled in
and the table picks which one a given driver version uses.

### 2d. Runtime selection

`nvproxy.Register` (`nvproxy.go:62-69`):

```go
abiCons, ok := abis[opts.DriverVersion]
if !ok { return nil, fmt.Errorf("unsupported Nvidia driver version: %s", ...) }
nvp := &nvproxy{ abi: abiCons.cons(), version: opts.DriverVersion, ... }
```

It is an **exact map lookup** — no range matching, no "nearest lower". The
`opts.DriverVersion` is supplied by the caller (runsc reads it from the host /
`/proc/driver/nvidia/version`). If the exact version isn't in `abis`, nvproxy
refuses to start. Helper APIs: `SupportedDrivers()` (`version.go:1182`),
`LatestDriver()` (`:1170`), `ExpectedDriverChecksum()` (`:1195`),
`SupportedIoctlsNumbers()`/`SupportedIoctls()` (`:1208,:1235`). The `.run`
checksum lets runsc *verify* the host driver is bit-identical to what the ABI
table was authored against — this is how nvproxy stays safe despite hardcoding
layouts: it never guesses, it only runs against drivers it has been pinned to.

---

## 3. Which NVIDIA versions matter

NVIDIA forks a "branch" off the "main" dev branch at a point and then ships
patch releases on that branch (the `version.go` comments call the main-branch
builds "intermediate unqualified versions" and the forked ones the real branch).
The branches that have shipped as production / LTS data-center drivers and
whether nvproxy already carries a registered (`addDriverABI`) table:

| Family | NVIDIA role | nvproxy `addDriverABI` entries (reference deltas available) |
|---|---|---|
| **535.xx** | Data-center **LTSB** (long-term support) | YES — 535.129.03, .183.06, .247.01, .261.03, .274.02, .288.01 (`version.go:797-802`) |
| 545.xx | New-feature (superseded) | constructor only (intermediate), no registered patch in this tree |
| **550.xx** | Production / "New Feature" prior gen | partial — only 550.90.12 registered (`version.go:927`); 550.40.07 / .54.14 / .90.07 exist as constructors with the V550 deltas |
| 555 / 560 / 565 | New-feature (superseded) | constructors only (intermediate, unqualified) |
| **570.xx** | Production branch | YES — 570.124.06, .133.20, .172.08, .195.03 (`version.go:1028-1034`) |
| **575.xx** | New-feature / current target | **NO registered patch** — only 575.51.02 exists as an *intermediate constructor* (`version.go:1037`). **575.51.03 (nvkvm's exact target) is NOT in nvproxy at all.** The closest reference is 575.51.02 (deltas vs 570.133.20) and the 580 deltas immediately above it. |
| **580.xx** | Latest production / **next LTSB candidate** | YES — 580.65.06, .105.08, .126.09, .126.20 (`version.go:1057-1085`) |
| 590.xx | Bleeding edge | YES — 590.48.01 (`version.go:1105`) |

Key takeaways for nvkvm:
* nvkvm's own target, **575.51.03, has no nvproxy ABI table** — the nearest
  reference is 575.51.02 (same main-branch point) which differs from 570.133.20
  only by 3 control cmds. So for the *frontend / uvm / alloc* struct layouts,
  575.51.x == the 570.x ABI (V550 UVM + V570 channel + pre-580 VASPACE/NVOS46).
  This is consistent with nvkvm's headers (`v570` channel, `v545` memory, V550
  UVM, pre-580 VASPACE).
* The two highest-value *additional* targets with full nvproxy reference tables
  are **535.xx (LTSB)** and **580.xx (latest production / LTSB candidate)**.
  570.xx is also fully covered and is the closest sibling to the current 575
  target.

---

## 4. A concrete plan for nvkvm to support a version range

### 4.0 Principle (borrowed from nvproxy)

Do **not** attempt fuzzy/range matching of layouts. Follow nvproxy: maintain a
small set of *exact* supported versions, each with a vetted layout set, and
refuse anything else. Optionally allow a "same-branch nearest-lower" policy only
after the branch's layout has been proven stable. The host version is already
available (`nv->driver_version` from GET_BUILD_VERSION,
`virtio_nvgpu.c:907-926`) — parse it into `{major,minor,patch}` at realize time.

### 4.1 Add a version table (mirror `driverABI`)

Introduce a single source of truth, e.g. `src/abi/abi_profile.h` +
`src/abi/abi_profile.c`, holding a struct like:

```c
struct nvkvm_abi_profile {
    /* alloc-param sizes by hClass (replaces the two switch tables in
       nvkvm_main.c:1044-1104 and 1136-1196) */
    /* embedded-fd / status offsets by escape NR (replaces stub switch
       nvkvm_stub.c:1056-1085 and 887-905) */
    uint32_t uvm_map_ext_fd_off;     /* 9248 for V550; differs pre-550 */
    uint16_t uvm_min_size[NR_UVM];   /* replaces nvkvm_isolate_handlers.c:439-469 */
    uint32_t pidinfo_stride;         /* 72 today */
    const uint8_t *fe_nr_allow; size_t fe_nr_allow_n;
    const uint32_t *ctrl_allow; size_t ctrl_allow_n;
    const uint32_t *alloc_class_allow; size_t alloc_class_allow_n;
    /* channel-alloc layout selector: base vs V570 vs ... */
    enum { CHAN_V570, CHAN_BASE } chan_layout;
    enum { VASPACE_PRE580, VASPACE_V580 } vaspace_layout;
    enum { NVOS46_PRE580, NVOS46_V580 } nvos46_layout;
    /* ... */
};
```

Select it from the parsed host version (like `abis[version]` in
`nvproxy.go:66`):

```c
const struct nvkvm_abi_profile *p = nvkvm_abi_select(major, minor, patch);
if (!p) error("unsupported NVIDIA driver version %s", nv->driver_version);
```

Pass the selected profile *id* (not the string) through the shm control block so
the guest module uses the same profile.

### 4.2 Where each switch lives

* **QEMU `nvkvm_isolate_handlers.c`** — the UVM `min_size`/fd-off table
  (`:439-469`), the three allowlists (`:513-541`), the pidinfo stride (`:561`),
  the SEM_POOL cap (`:1815`). These are the host/cross-VM trust boundary and
  must stay in QEMU. Drive all of them from `nvkvm_abi_profile`.
* **Stub `nvkvm_stub.c`** — the status/fd offset switch (`:1056-1085`), the
  frontend embedded-fd offsets (`:887-905`), the UVM embedded-fd offset
  (`:836-844` = 9248), the SEM_POOL intent cap (`:1419`). Drive from the profile
  id passed down.
* **Guest sanitizers `nvkvm_main.c`** — the two alloc-param size tables
  (`:1044-1104`, `:1136-1196`), which select `nv_channel_alloc_params_v570` vs a
  hypothetical base, `nv_vaspace_*` vs V580, `nv_memory_allocation_params_v545`,
  etc. Drive the per-hClass size from the profile.
* **ABI headers `src/abi/`** — keep *all* struct variants compiled in (exactly
  as nvproxy keeps both `NV_CHANNEL_ALLOC_PARAMS` and `_V570`). Add the missing
  variants needed for other targets: `nv_channel_alloc_params` (base, pre-570),
  `nv_vaspace_allocation_parameters_v580`, `nvos46_parameters_v580`,
  `uvm_map_external_allocation_params` pre-V550 (1-entry array),
  `uvm_*_params_v590` if 590 is ever targeted. Source the deltas directly from
  `gvisor/pkg/abi/nvgpu/{classes,frontend,uvm}.go`.

### 4.3 Highest-risk version-variant fields (silent-corruption order)

1. **`uvm_map_external_allocation_params` / `uvm_alloc_semaphore_pool_params`
   array size + the 9248 offset** — duplicated across stub + QEMU + proto; a
   pre-550 driver flips the array from 256→1 entries. (`uvm.h:157-174,268-282`,
   `nvkvm_stub.c:844`, `isolate_handlers.c:450,465`.)
2. **`nv_channel_alloc_params_v570` `engineType`/`TPCConfigID`** — wrong layout
   re-triggers the runlist-binding class of bug. (`nvgpu.h:555-592`.)
3. **`nv_vaspace_allocation_parameters` (pre-580 vs V580)** — write-back struct;
   wrong layout corrupts `vaSize` and breaks cuCtxCreate. (`nvgpu.h:504-514`.)
4. **`nv_memory_allocation_params_v545` `numa_node`** — size mismatch on memory
   alloc. (`nvgpu.h:596-623`.)
5. **Stub status/fd offsets** — if NVOS46/NVOS47 sizes change at 580, the stub
   reads status/fd from the wrong byte. (`nvkvm_stub.c:1056-1085`.)

These five must be profile-driven *before* enabling any non-575 version;
everything else (control-cmd allowlist deltas, new alloc classes) merely affects
which features work, not memory safety.

### 4.4 Phased rollout

* **Phase 0 (now):** parse `nv->driver_version` into `{maj,min,patch}` at realize
  and *assert* it is in the supported set; refuse otherwise (today nvkvm silently
  runs whatever the host has). This alone removes the silent-corruption-on-skew
  footgun. Cheap, no struct work.
* **Phase 1 — pin 575.51.x + the 570 production branch.** These share the same
  struct layouts (V550 UVM, V570 channel, pre-580 VASPACE/NVOS46), so the only
  per-version delta is the control-cmd allowlist (3 cmds, `version.go:1039-1051`).
  Lowest-effort first range; proves the table machinery end-to-end.
* **Phase 2 — add 535 LTSB.** Highest product value (it is the long-term-support
  data-center driver). This is the largest layout delta (pre-V550 UVM 1-entry
  arrays, base channel params, pre-V545 memory params), so it exercises all five
  high-risk switches. Full nvproxy reference exists (`version.go:797-802`).
* **Phase 3 — add 580 production / LTSB-candidate.** Adds `NVOS46_PARAMETERS_V580`
  and `NV_VASPACE_ALLOCATION_PARAMETERS_V580` (`version.go:1059-1060`); reference
  exists. Position for the next LTS.

Defer 545/555/560/565 (superseded new-feature) and 590 (bleeding edge) unless a
customer needs them.

### 4.5 How to test per version (source-only here; HW later)

* **Static parity:** extend `tests/abi_parity` so each supported profile asserts
  `sizeof(struct)` for every variant against the value nvproxy encodes in
  `pkg/abi/nvgpu/*.go` for that version (cross-check via
  `nvproxy.SupportedIoctls(version)` `DriverABIInfo`). This catches layout drift
  at build time without hardware.
* **Per-version runtime matrix (HW):** for each target version install the
  matching open kernel module **and a version-matched libcuda +
  libnvidia-ptxjitcompiler** (the `ptxjit_version_match` memory shows mismatched
  ptxjit silently fails). Run the existing smoke ladder per version: cuInit →
  cuMemAlloc → vector_add → 1024² fp32 matmul → nvidia-smi → 7B LLM. Serialize
  GPU runs (per `remote_test_serialization`).
* **Checksum gate (optional, nvproxy-style):** record the `.run` installer SHA256
  per supported version so the host driver can be verified bit-identical to the
  table it was authored against (`ExpectedDriverChecksum`,
  `version.go:1195`).

---

## Appendix — file:line index

* Version read, never branched: `src/qemu/virtio_nvgpu.c:907-926,945-946`;
  `src/guest/nvkvm_virtio.c:557-562`.
* ABI structs (575-pinned): `src/abi/nvgpu.h:60-110,148-261,497-623`;
  `src/abi/uvm.h:148-282`.
* 9248 constant: `src/stub/nvkvm_stub.c:836-844,1419`;
  `src/qemu/nvkvm_isolate_handlers.c:450,465,1815`;
  `src/common/nvkvm_proto.h:513`.
* Stub offset switches: `src/stub/nvkvm_stub.c:887-905,1056-1085`.
* Guest alloc-size tables: `src/guest/nvkvm_main.c:1044-1104,1136-1196`.
* Allowlists: `src/qemu/nvkvm_fe_alloc_allowlist.h`,
  `src/qemu/nvkvm_ctrl_allowlist.h`,
  `src/qemu/nvkvm_isolate_handlers.c:439-541,557-561`.
* nvproxy mechanism: `gvisor/pkg/sentry/devices/nvproxy/version.go:100-159`
  (`driverABI`, `addDriverABI`), `:797-1105` (per-version deltas),
  `:1162-1242` (lookup APIs); `gvisor/pkg/sentry/devices/nvproxy/nvproxy.go:62-69`
  (exact-match selection); `gvisor/pkg/sentry/devices/nvproxy/nvconf/version.go`
  (DriverVersion type).
* nvproxy struct variants: `gvisor/pkg/abi/nvgpu/classes.go:447-485`
  (channel base/_V570), `:367-371` (VASPACE);
  `gvisor/pkg/abi/nvgpu/uvm.go:303,332-337,790-794` (UVM base/_V550);
  `gvisor/pkg/abi/nvgpu/frontend.go:625,654` (NVOS46 / _V580).

---

## Live multi-driver validation (2026-05-30)

The version-keyed `abi_profile` system (commits cbcdd65, 4ba139b) was validated
on **real hardware** (vast.ai RTX 3060 / GA106) across two distinct open driver
**major versions**, by physically swapping the host kernel module + GSP firmware
and matching the guest libcuda:

| Host driver (open) | Detected profile | matmul | test_ioctl_fwd | Qwen2.5-7B |
|---|---|---|---|---|
| 575.51.03 | 570 | PASS | 48/48 | PASS (prior) |
| **580.159.04** | **580** | **PASS** | **48/48** | **PASS** |

QEMU auto-detected each version from `CHECK_VERSION_STR` and logged
`nvkvm: host driver <v> → ABI profile <id>` with **zero code changes** between
runs — only the host driver and guest libcuda were swapped. The 580 run
exercises the hardest profile deltas vs 575: `FERMI_VASPACE_A` params 48→56B
(+`Pasid`) and `NVOS46` 56→64B with `status` moving 48→56. Both flowed correctly
through guest→QEMU→stub via the `abi_profile` field in `ISOLATE_CMD_IOCTL`.

**Conclusion:** the profile-570 (V550-era: 535-derived-but-V550) ↔ profile-580
range is hardware-proven end-to-end including a 7B LLM. The profile-535
(pre-V550) branch is wired in the table but **not yet hardware-validated** — it
needs the guest to additionally emit the 1-entry (vs 256-entry) UVM array layout
for `UVM_MAP_EXTERNAL_ALLOCATION` / `ALLOC_SEMAPHORE_POOL`, which the current
guest builds V550-only. That is the next increment for full 3-version coverage.

### 3rd driver attempt: 535.309.01 — blocked by the host driver, not nvkvm

Attempted the distinct profile-535 (pre-V550) branch with the 535.309.01 open
driver. nvkvm behaved correctly: QEMU auto-detected `535.309.01 → ABI profile
535` and forwarded ioctls. But the run failed early (`open /dev/nvidia0` → EIO,
`RM_ALLOC failed`, matmul `cuInit 999`).

Root cause is **the host driver, not the forwarder**: the 535.309.01 *open*
kernel module fails to initialize this RTX 3060 (GA106) on bare metal —

```
NVRM: GPU 0000:00:07.0: RmInitAdapter failed! (0x62:0x0:1929)
NVRM: GPU 0000:00:07.0: rm_init_adapter failed, device minor number 0
Video BIOS: ??.??.??.??.??     # GSP couldn't even read the VBIOS
```

A bare-metal CUDA program (no VM, host libcuda 535.309.01) also returns
`cuInit 999`. The 535 open driver's GSP-RM init (status 0x62) is incompatible
with this GPU/VBIOS on the 6.8 kernel; the open modules were still maturing for
Ampere consumer parts in the 535 branch. nvkvm cannot be validated on a driver
that cannot drive the GPU at all.

**Net result of the multi-driver effort:** 2 of 3 open driver *major* versions
hardware-validated end-to-end (575 → profile 570, 580 → profile 580, the latter
through a 7B LLM); the abi_profile auto-detect proven on all three majors. The
profile-535 *code path* remains exercised-by-detection but not GPU-validated,
blocked by an external 535-open/GA106 init bug. A datacenter GPU (Turing/Ampere
server class, where 535-open is mature) would be the right target to close it.
