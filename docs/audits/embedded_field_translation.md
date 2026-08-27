# Embedded-field translation audit (read-only correctness/security)

Scope: every ioctl / RM control cmd / RM_ALLOC class nvkvm **forwards to the host
driver**, enumerating the embedded fields that carry a guest **pointer (NvP64)**,
**fd**, **pid**, or **cross-context handle**, and whether nvkvm translates or
forwards each RAW. RAW-forwarded pointers/fds/pids are flagged as gaps: a raw
guest VA makes the host NVIDIA driver dereference an address in the *stub's*
address space (wrong/zero pages), and a raw guest pid/fd makes the host driver
key its tables on a value meaningless across the isolate boundary.

Method: field layouts verified against `src/abi/nvgpu.h` / `src/abi/uvm.h`
(driver 575.51.03) and gVisor nvproxy `pkg/sentry/devices/nvproxy/` (the
canonical "which fields carry ptr/fd/handle" checklist:
`ctrlIoctlHasInfoList`, `ctrlGetNvU32List`, `ctrlDevGetCaps`, `ctrlHasFrontendFD`,
`rmAllocChannel*`, `rmAllocEventOSEvent`, `rmVidHeapControl`, `rmIdleChannels`).

Translation sites in nvkvm:
- **G** = guest module `src/guest/nvkvm_main.c` (aux extraction + restore) and
  `src/guest/nvkvm_ioctl.c` (`nvkvm_sanitize_ioctl_params`, `guest_fd_to_handle_id`).
- **Q** = QEMU `src/qemu/nvkvm_isolate_handlers.c` (GET_PID_INFO pid→isolate map,
  DUP_OBJECT src-client gate, per-VM hClient allowlist, UVM fd schema).
- **S** = stub `src/stub/nvkvm_stub.c` (repoints inner ptrs into aux, handle_id→fd).

---

## 1. RM control commands (NV_ESC_RM_CONTROL, NR 0x2a)

The outer `nvos54.params` NvP64 is always zeroed by **G** and the inner params
struct travels in the aux slot; **S** repoints it. The question per cmd is
whether the *inner* params struct embeds a further ptr/fd/pid the host writes
through or keys on.

| cmd | number | embedded field(s) & type | translated? |
|-----|--------|--------------------------|-------------|
| FIFO_GET_CHANNELLIST | 0x0080170d | `PChannelHandleList`, `PChannelList` (2× NvP64, IN+OUT u32 arrays) | **yes** — G extends aux with both lists, zeroes ptrs, S repoints, G copies back |
| GR_GET_INFO (dev) | 0x00801104¹ | InfoList preamble: `info_list` NvP64 @8 (IN+OUT) | **yes** — G `has_info_list` switch |
| GR_GET_INFO (subdev) | 0x20801201 | `info_list` NvP64 @8 | **yes** — G `has_info_list` |
| FB_GET_INFO | 0x20801301¹ | `info_list` NvP64 @8 | **yes** — G `has_info_list` |
| BUS_GET_INFO | 0x20801802 | `info_list` NvP64 @8 | **yes** — G `has_info_list` |
| BIOS_GET_INFO | 0x20800802 | `info_list` NvP64 @8 | **yes** — G `has_info_list` |
| GET_SURFACE_INFO | 0x00410110 | `info_list` NvP64 @8 | **yes** — G `has_info_list` |
| SYSTEM_GET_BUILD_VERSION | 0x00000101 | `pDriverVersionBuffer`, `pVersionBuffer`, `pTitleBuffer` (3× NvP64 OUT strings) | **yes** — G extends aux, zeroes ptrs, S repoints, G copies strings back |
| GPU_GET_ID_INFO | 0x00000202 | `szName` NvP64 @16 (OUT string) | **yes** — G zeroes ptr @16 (driver null-checks; name unused) |
| GPU_GET_PIDS | 0x2080018d | `pid_tbl[]` of guest pids (synthesized) | **yes** — G `nvkvm_synth_get_pids` (not forwarded; ns-scoped) |
| GPU_GET_PID_INFO | 0x2080018e | `pidInfoList[].pid` (pid, 72B stride) | **yes** — G tags pid→`0x80000000\|isolate`, Q resolves to host pid, G restores |
| **GPU_GET_CLASSLIST** | **0x00800201** | **`List` NvP64 @8 (OUT u32 array, NvU32List)** | **RAW — GAP (see §A.1)** |
| GPU_GET_CLASSLIST_V2 | 0x00800292 | inline `classList[]` array, no ptr | n/a (inline — safe) |
| FB_GET_CAPS_V2 | 0x00801307 | inline `capsTbl[]` byte array | n/a (inline) |
| HOST_GET_CAPS_V2 | 0x00801402 | inline `capsTbl[]` | n/a (inline) |
| GR_GET_CAPS_V2 | 0x00801909² | inline `capsTbl[]` | n/a (inline) |
| CE_GET_CAPS_V2 | 0x20802a03 | inline `capsTbl[]` | n/a (inline) |
| all other allowlisted ctrl cmds | (see allowlist) | scalar/handle-only inner params, no ptr/fd/pid | n/a — RAW forward is correct |

¹ Old non-V2 caps cmds with **embedded `CapsTbl` NvP64** (`ctrlDevGetCaps`: GR_GET_CAPS
0x801102, FB_GET_CAPS 0x801301-caps, FIFO_GET_CAPS, MSENC_GET_CAPS) are **NOT in our
allowlist** — only their inline V2 forms are. So they are denied, not forwarded:
no gap. (0x00801104 is the GR_GET_INFO info-list cmd, handled; do not confuse with
0x801102 GR_GET_CAPS.)
² 0x00801909 is in our allowlist and is an inline-caps V2 form (CapVideo), safe.

**Handles inside ctrl params** (hClient @0, hObject @4) are intra-VM names; per the
access-model split (QEMU vets only cross-VM/host boundary) the host-facing hClient
@0 is additionally gated against the per-VM allowlist in **Q** (§ "Audit H-3"),
and DUP_OBJECT's `h_client_src` @12 is gated in **Q**. No raw-handle escape.

---

## 2. RM_ALLOC classes (NV_ESC_RM_ALLOC, NR 0x2b — NVOS21/NVOS64)

Outer `p_alloc_parms` / `p_rights_requested` NvP64 zeroed by **G**; class params
travel in aux, sized per-class. Per nvproxy, **only `NV01_EVENT_OS_EVENT` carries
an embedded fd**; channels/memory/gr/dma carry handles + GPU VAs only.

| class | id | embedded field(s) & type | translated? |
|-------|----|--------------------------|-------------|
| NV01_EVENT_OS_EVENT | 0x0079 | `data` = guest fd (NV0005 @16) | **yes** — G fget→handle_id; S handle_id→local fd; G restores. ALLOC/FREE_OS_EVENT fd handled in parallel (§3) |
| NV50_MEMORY_VIRTUAL / NV01_MEMORY_SYSTEM / NV01_MEMORY_LOCAL_USER | 0x50a0 / 0x0002 / 0x003e | `address` NvP64 in NV_MEMORY_ALLOCATION_PARAMS_V545 (IN hint for FIXED-VA / OUT) | **RAW** — nvproxy also forwards raw (`rmAllocSimple`); IN value is a guest VA *hint*, OUT is host VA. Correctness-only, no host deref. (see §A.4) |
| TURING/AMPERE/HOPPER_CHANNEL_GPFIFO_A | 0xc46f/0xc56f/0xc86f … | handles (hObjectError/Buffer/ContextShare/VASpace) + `nv_memory_desc_params` (GPU PAs) + `process_id`/`sub_process_id` (pid) | handles=intra-VM ok; mem-descs=GPU addrs ok. **`process_id`/`sub_process_id`: RAW pid — but driver-internal, see §A.3.** nvproxy forwards raw (`rmAllocChannel*`). |
| KEPLER_CHANNEL_GROUP_A | 0xa06c | handles only | RAW forward correct |
| FERMI_CONTEXT_SHARE_A | 0x9067 | handles only | RAW forward correct |
| *_DMA_COPY_* | 0xc5b5… | `engineType` (scalar) | RAW forward correct (sized so kernel sees engineType) |
| FERMI_VASPACE_A, *_COMPUTE_*, gr classes, NV01_DEVICE_0, NV20_SUBDEVICE_0, RM_USER_SHARED_DATA, GT200_DEBUGGER | … | handles + scalars, no ptr/fd/pid | RAW forward correct |
| classes in allowlist but NOT in G's size table (e.g. NV_SEMAPHORE_SURFACE 0x00fb, NV50_THIRD_PARTY_P2P 0x503c, semaphore/fabric classes) | — | unknown params | aux not sized → alloc params dropped if used. Correctness gap only if exercised (§A.5) |

Note: the host-facing **hClient (`h_root` @0) is gated** by **Q**'s per-VM allowlist
for RM_ALLOC; the client-creating root alloc is the only exemption.

---

## 3. Other frontend ioctls (FE NR allowlist: 27 29 2a 2b 34 35 41 4a 4e 4f 54 57 58 5e 70 + UVM/event/regfd NRs)

| ioctl | NR | embedded field(s) & type | translated? |
|-------|----|--------------------------|-------------|
| RM_ALLOC_MEMORY (nvos02+fd) | 0x27 | `fd` (NvS32); `p_memory` NvP64 (OUT, or IN for OS_DESCRIPTOR hClass 0x71) | **yes** — G translates `fd>0`→handle_id (S→local fd, G restores); `p_memory` zeroed normally, OR for hClass 0x71 left as guest VA after `nvkvm_cpu_pages_migrate_range` aliases the pages into the stub (correct) |
| RM_FREE (nvos00) | 0x29 | handles only | RAW forward correct |
| RM_CONTROL | 0x2a | see §1 | — |
| RM_ALLOC | 0x2b | see §2 | — |
| RM_DUP_OBJECT (nvos55) | 0x34 | `h_client_src` @12, `h_src_object` @16 (cross-context handles) | **yes (gate)** — Q denies foreign `h_client_src` not in per-VM allowlist |
| RM_SHARE (nvos57) | 0x35 | handles; hClient @0 | hClient @0 gated by Q allowlist; share_policy scalar — RAW ok |
| RM_IDLE_CHANNELS (nvos30) | 0x41 | `p_clients`, `p_devices`, `p_channels` (3× NvP64, IN u32 arrays) | **PARTIAL — GAP** — G zeroes all 3 ptrs but does NOT aux-copy the arrays; if `num_channels>0` host gets NULL lists. nvproxy copies all 3 in. No raw-VA deref (NULL), but data dropped (§A.2) |
| RM_VID_HEAP_CONTROL (nvos32) | 0x4a | `p_memory` NvP64 @32 (OUT for ALLOC_SIZE) | **PARTIAL** — G zeroes `p_memory`, no aux-copy/restore. nvproxy uses dedicated `rmVidHeapControl` (only allows ALLOC_SIZE). OUT addr dropped; mapping done via GPA window → correctness-only. Q also does NOT gate on `function` like nvproxy (minor, §A.6) |
| RM_MAP_MEMORY (nvos33+fd) | 0x4e | `p_linear_address` NvP64 (OUT); `fd` (NvS32) | **yes** — G zeroes `p_linear_address` (host fills); `fd≥0`→handle_id (S→fd, G restores) |
| RM_UNMAP_MEMORY (nvos34) | 0x4f | `p_linear_address` NvP64 (lookup key) | **yes** — G zeroes ptr, fakes status=NV_OK on response (VA-lookup unmatchable in our model; unmap via GPA window) |
| RM_ALLOC_CONTEXT_DMA2 | 0x54 | handles + va_space/va_base/limit (GPU addrs) | RAW forward correct — no guest userspace ptr/fd. nvproxy `rmAllocSimple`-style |
| RM_MAP_MEMORY_DMA (nvos46) | 0x57 | handles + offset/length/dma_offset (GPU DMA addrs, no host VA) | RAW forward correct — `nvos46_parameters` has **no NvP64** (dma_offset is a GPU address). Matches nvproxy `frontendIoctlSimple` |
| RM_UNMAP_MEMORY_DMA (nvos47) | 0x58 | handles + dma_offset (GPU addr) | RAW forward correct — no NvP64 |
| RM_UPDATE_DEVICE_MAPPING_INFO (nvos56) | 0x5e | `p_old_cpu_address`, `p_new_cpu_address` (2× NvP64 lookup keys) | **yes** — G zeroes both, fakes status=NV_OK on response, restores caller's values |
| EXPORT_TO_DMABUF_FD | 0x70 | `fd` (OUT dmabuf fd) | nvkvm-only sanitized extension; result fd is host-side (handled by stub fd path / out-of-scope of cross-tenant deref) |
| REGISTER_FD | NR_BASE+1 (0xc9) | `ctl_fd` (NvS32) | **yes** — G translates→handle_id; S→local nvidiactl fd; G restores |
| ALLOC_OS_EVENT / FREE_OS_EVENT | 0xce / 0xcf | `fd` (NvU32, indexes driver event_list) | **yes** — G fget→handle_id; S→local fd; G restores. Must match NV01_EVENT_OS_EVENT.data translation (it does) |
| NUMA_INFO | NR_BASE+15 | scalars only (nid, sizes) | RAW forward correct — no ptr/fd |
| CARD_INFO / CHECK_VERSION_STR / SYS_PARAMS / WAIT_OPEN_COMPLETE | … | scalars/inline arrays | RAW forward correct |

---

## 4. UVM ioctls (nvkvm_uvm_schema in QEMU, full cmd word)

UVM runs in the isolate; the only embedded fields that cross the boundary are the
**frontend fd** fields (RMCtrlFD / UvmFD), which the guest rewrites to a handle_id
and the stub resolves to its local fd.

| ioctl | cmd | embedded field(s) | translated? |
|-------|-----|-------------------|-------------|
| MM_INITIALIZE | 75 | `uvm_fd` @0 (fd) | **yes** — G `guest_fd_to_handle_id`; schema fd_off=0; S→local fd |
| REGISTER_GPU_VASPACE | 25 | `rm_ctrl_fd` @16 (fd) | **yes** — G translates; schema fd_off=16; S→local fd |
| REGISTER_CHANNEL | 27 | `rm_ctrl_fd` @16 (fd) | **PARTIAL** — G *sanitizer* translates it (ioctl.c case), but **schema fd_off=0xffff** so **S does not re-resolve**; the handle_id reaches the kernel as-is. Works only if handle_id≈fd by luck. (§A.7) |
| MAP_EXTERNAL_ALLOCATION | 33 | `rm_ctrl_fd` @16 (fd) | **PARTIAL** — same as REGISTER_CHANNEL: G translates, schema fd_off=0xffff, S does not resolve (§A.7) |
| INITIALIZE / DEINITIALIZE / REGISTER_GPU / FREE / MIGRATE / range-group / pref-loc / accessed-by / peer-access / sem-pool / … | various | handles, GPU VAs, uuids, scalars | RAW forward correct — no guest ptr/fd |
| TOOLS_READ/WRITE_PROCESS_MEMORY | 62/63 | (cross-proc peek/poke) | **denied** (default-deny by omission from schema) — correct |

All UVM cmds are **default-deny**: a cmd absent from `nvkvm_uvm_schema` is refused.

---

## A. TRUE GAPS (raw-forwarded ptr/fd/pid), prioritized

### A.1 — GPU_GET_CLASSLIST (0x00800201): raw OUT pointer `List` — **CORRECTNESS, high-confidence**
`RmapiParamNvU32List` = `{ NumElems u32 @0, pad, List NvP64 @8 }`. The cmd is in
`nvkvm_ctrl_allowlist[]` but **not** in the guest `has_info_list` switch and has no
NvU32List handler anywhere. The inner `List` ptr sits in the aux buffer and is
forwarded **verbatim** (the RM_CONTROL sanitizer only zeroes the *outer*
`nvos54.params`, never the inner `List`). The host driver writes the class array
through this guest VA → **writes into the stub's address space at a guest VA**
(garbage/zero page in the stub, or a wild write if that VA happens to be mapped in
the stub). Severity: **self-stub-harm / correctness** (the write lands in the
isolate's own mm, not another tenant's), and the returned class list is wrong.
Why not seen yet: libcuda uses the inline **GET_CLASSLIST_V2 (0x800292)**, so
0x800201 is currently unexercised — but it is *allowed*, so a guest can invoke it.
Fix: add 0x00800201 to the NvU32List/info-list extend+zero+copyback path (treat like
FIFO_GET_CHANNELLIST: NumElems@0, List@8), or drop it from the allowlist (V2 suffices).

### A.2 — RM_IDLE_CHANNELS (0x41): IN list pointers zeroed but arrays dropped — **CORRECTNESS**
G zeroes `p_clients`/`p_devices`/`p_channels` but never copies the
`num_channels`-entry arrays into aux. With `num_channels>0` the host driver sees
three NULL lists. No raw-VA deref (NULL, not a guest VA), so not a security
escape — purely a dropped-data correctness bug. nvproxy copies all three arrays in
(`rmIdleChannels`). Low priority (idle-channels with explicit lists is rare on the
CUDA path). Fix: aux-extend like FIFO_GET_CHANNELLIST (3 arrays) if ever exercised,
or special-case `num_channels==0`.

### A.3 — Channel GPFIFO alloc `process_id`/`sub_process_id`: raw pid — **CORRECTNESS, low**
`nv_channel_alloc_params_v570` carries `process_id`/`sub_process_id` (offsets 584/588).
These are guest pids forwarded raw to the host driver. nvproxy also forwards them
raw (`rmAllocChannel*`) — the driver uses them for its own bookkeeping/debug, not as
a host pid-table key, and the open driver tolerates the isolate's pid. Not a leak
(value stays within the isolate's RM client). Flag for the pid-translation
milestone: if any host-visible reporting keys on it, ns-translate like GET_PIDS.

### A.4 — Memory alloc `address` NvP64 (NV50_MEMORY_VIRTUAL etc.): raw — **CORRECTNESS, low / accepted**
`NV_MEMORY_ALLOCATION_PARAMS_V545.address` @96 is forwarded raw in aux. For the
common (non-FIXED) path it is OUT (driver writes the *host* VA, which the guest then
ignores in our GPA-window model). For `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE`
it would be an IN guest-VA hint the host can't honor. nvproxy forwards it raw too
(`rmAllocSimple`); accepted as correctness-only. No deref of the value as a pointer
by the driver in the observed flows.

### A.5 — Allowlisted alloc classes with no aux-size entry — **CORRECTNESS, conditional**
Classes present in `nvkvm_alloc_class_allowlist[]` but absent from G's per-class
`ap_size` switch (e.g. NV_SEMAPHORE_SURFACE 0x00fb, NV50_THIRD_PARTY_P2P 0x503c,
fabric/multicast classes) → `ap_size=0` → alloc params never copied to aux, so if
such an alloc is issued with non-NULL params the params are silently dropped. Not a
ptr/fd leak (the outer ptr is zeroed), but the alloc would misbehave. Add sizes if
these classes are ever used by the compute path.

### A.6 — RM_VID_HEAP_CONTROL: no `function` gate; `p_memory` OUT dropped — **CORRECTNESS, low**
G zeroes `p_memory` and never restores the driver-written address; mapping is done
via the GPA window so this is informational. Unlike nvproxy (`rmVidHeapControl`
allows only `NVOS32_FUNCTION_ALLOC_SIZE`), QEMU forwards any `function`. Per the
access-model split this is intra-VM and the host driver validates, but mirroring
nvproxy's ALLOC_SIZE-only gate would harden the surface. Low priority.

### A.7 — UVM REGISTER_CHANNEL (27) / MAP_EXTERNAL_ALLOCATION (33): fd translated by guest but stub does not re-resolve — **CORRECTNESS/robustness, medium**
The guest sanitizer rewrites `rm_ctrl_fd` @16 → handle_id for these two cmds
(`nvkvm_ioctl.c`), but their `nvkvm_uvm_schema` rows have `fd_off = {0xffff,0xffff}`,
so the **stub never maps handle_id → its local fd**. The kernel therefore receives a
handle_id where it expects an fd. The schema comment explicitly says fd-translation
was "limited to the two cmds the prior code translated (MM_INITIALIZE@0,
REGISTER_GPU_VASPACE@16)", so for 27/33 the guest-side translation and the
stub-side resolution are **out of sync**. It works today only when the cmd passes
`rm_ctrl_fd = -1` (the common sentinel, left untouched). Any caller that passes a
real ctrl fd to REGISTER_CHANNEL/MAP_EXTERNAL_ALLOCATION will hand the kernel a
bogus fd (EBADF / wrong object). Fix: set `fd_off[0]=16` for cmds 27 and 33 in
`nvkvm_uvm_schema` so the stub resolves the handle_id, OR stop translating them in
the guest. (The guest also captures/restores @16 via `have_uvm_rm_ctrl` for
REGISTER_CHANNEL/MAP_EXTERNAL_ALLOCATION, reinforcing that the translate path is
"on" guest-side but dead stub-side.)

---

## B. Cross-tenant escape assessment

No **cross-tenant** raw-handle/fd/pid escape was found among forwarded fields:
- Cross-context **handles**: DUP_OBJECT `h_client_src` is gated (Q), and every
  host-facing `hClient @0` is vetted against the per-VM client allowlist (Q, H-3).
- **fds**: every embedded fd (REGISTER_FD ctl_fd, OS_EVENT fd, NV0005 data,
  nvos02/nvos33 fd, UVM uvm_fd/rm_ctrl_fd@0/@16) is rewritten to a session-scoped
  handle_id that the stub resolves only within its own isolate handle table — a
  guest cannot name another isolate's fd.
- **pids**: GET_PIDS synthesized ns-scoped; GET_PID_INFO tagged→isolate-validated.
  Remaining raw pids (channel alloc `process_id`) stay inside the isolate's own RM
  client (A.3).

The flagged gaps are **self-stub-harm / correctness**, not cross-tenant. The one to
fix before the signal-delivery milestone (it touches an *allowed* cmd a guest can
invoke today): **A.1 GPU_GET_CLASSLIST raw OUT pointer**. A.7 (UVM fd desync) is the
next most likely to cause "weird" behavior once channel registration uses a real
ctrl fd. A.2/A.5/A.6 are conditional/dropped-data; A.3/A.4 are accepted parity with
nvproxy.
