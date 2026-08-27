# nvkvm security audit — 2026-05-31 (graphics / Vulkan surface)

Scope: source-level audit of the boundaries that changed for the #82/#84
graphics milestone (NVKMS forwarding, nvidia-drm render node + GEM proxy,
SEMSURF fences, EXPORT_OBJECT_TO_FD, NV0005/NV01_EVENT alloc-fd translation,
the per-class alloc-params size switches), plus the specific writeback
truncation already visible in dmesg
(`AUDIT param_size MISMATCH cmd=0xc0384641 nr=0x41 iocsz=56 our=40`).

Threat model: guest fully malicious (controls the guest kernel module and all
ioctl payloads). QEMU + the per-process stub are the cross-VM/host trust
boundary. Missing *intra-VM* access checks are NOT findings (the guest kernel
owns intra-VM rights). Host/cross-guest impact = critical; guest-self-crash =
low. Verified against the 2026-05-29 and 2026-05-30 audits + git log to avoid
re-reporting fixed issues (C1 TOCTOU, C3/C4 cross-session reach, M6 fd
inheritance, the #80 teardown reaper, the seccomp/TSYNC fixes, slot_blob
bounding, the GET_PID_INFO OOB, the non-'F' bypass — all confirmed still in
place).

## Executive summary

The single most serious finding is **G-1**: `/dev/nvidia-modeset` (NVKMS) is
forwarded to the host with **no subcommand allowlist** — a default-ALLOW on a
privileged, host-global display device that includes cross-client sharing/
permission primitives (GRANT/ACQUIRE/REVOKE_PERMISSIONS, GRANT_SURFACE). The
project's own design doc committed the same day
(`docs/design/virtual_modeset.md`, commit c8130eb) explicitly **rejects** NVKMS
forwarding as "the largest/gnarliest attack surface in the driver," yet the
forwarding code (commit a895f95) is still live and reachable whenever graphics
is enabled. This is the one finding with a plausible cross-VM/host impact.

The dmesg truncation (**G-2**) is a real wrong-size + wrong-layout bug in
`NV_ESC_RM_IDLE_CHANNELS` (nr 0x41): nvkvm's `nv_ioctl_idle_channels` is 40 bytes
with the fields in a different order than the driver's 56-byte `NVOS30_PARAMETERS`,
so the stub hands the host driver a 40-byte buffer it reads 56 bytes from (OOB
read of stub heap) and the guest's scalar fields land where the driver expects
list pointers (guest-controlled value dereferenced as a pointer by the host
driver, in the stub's address space).

The remaining graphics findings are default-ALLOW DRM ioctls with no guest-VA
marshalling (G-3), and embedded-fd translation asymmetries (G-4/G-5) that are
self-stub-harm / correctness rather than cross-tenant. No new cross-VM
handle/fd/pid escape was found beyond G-1; the per-VM hClient allowlist, the
DUP_OBJECT src gate, and the session-scoped handle table all still hold for the
graphics paths.

## Findings

| ID  | Sev      | Title | Boundary |
|-----|----------|-------|----------|
| G-1 | CRITICAL | NVKMS `/dev/nvidia-modeset` forwarded with no subcommand allowlist (default-ALLOW on privileged host device incl. GRANT/ACQUIRE_PERMISSIONS) | QEMU |
| G-2 | HIGH     | `NV_ESC_RM_IDLE_CHANNELS` (nr 0x41) struct is 40B/wrong-layout vs driver's 56B NVOS30 → stub-heap OOB read + guest-value-as-pointer deref in host driver | guest + stub |
| G-3 | MEDIUM   | DRM allowlist permits GEM_IMPORT_USERSPACE_MEMORY / MAP_OFFSET / EXPORT_DMABUF etc. with no guest-VA marshalling — raw guest VA forwarded to host renderD128 | QEMU + stub |
| G-4 | MEDIUM   | NVKMS REGISTER_SURFACE & EXPORT_OBJECT_TO_FD embedded fds: handle_id resolved to a stub fd with no handle-type check; fd/handle namespace confusion | guest + stub |
| G-5 | LOW      | NV01_EVENT_OS_EVENT via **nvos21** (32B) alloc does not translate the embedded `data` fd; stub treats the raw guest fd as a handle_id | guest + stub |
| G-6 | LOW      | nvidia-drm GEM proxy stores `nvkvm_fd_ctx*` without a refcount; GEM_CLOSE forward on free assumes ctx liveness (UAF if a proxy outlives its file via dma-buf) | guest |
| G-7 | LOW      | EXPORT_TO_DMABUF_FD (0x70) allowed but unhandled — created host dmabuf fd is leaked in the stub (no close); OUT fd has no passback | stub |
| G-8 | INFO     | Async IOCTL_ON_ISOLATE path forwards any guest param_size ≤ slot_size with no per-cmd expected-size check; struct-size correctness rests entirely on the guest's table (see G-2) | QEMU |

---

### G-1 — NVKMS forwarded with no subcommand allowlist (CRITICAL)

**Location:** `src/qemu/nvkvm_isolate_handlers.c:902-916` (the
`req->cmd == NVKVM_NVKMS_IOCTL_CMD` branch falls straight through to the generic
forward with no `cmdType` allowlist); device-open gate at
`nvkvm_req_open_nvidia_handle` `:171-178`; stub forward at
`src/stub/nvkvm_stub.c:749-799`. Enabled by commit a895f95 ("modeset: forward
/dev/nvidia-modeset (NVKMS) for graphics").

**Description:** NVKMS is a single wrapper ioctl
`NVKMS_IOCTL_CMD = _IOWR('m',0,{u32 cmdType; u32 size; u64 address;})`
(`0xC0106D00`); the real operation is selected by the inner `cmdType` and the
payload lives behind `address`. The DRM render-node path is gated by a
default-deny allowlist (`nvkvm_drm_nr_allowed`) and RM control/alloc/frontend
all have default-deny allowlists, but **NVKMS has none**: once `nv->graphics`
is set, the QEMU handler forwards *every* `cmdType` verbatim to the host
`/dev/nvidia-modeset`. The guest module only special-cases `cmdType ==
REGISTER_SURFACE` (plane-fd translation); all other subcommands pass through
untouched.

`/dev/nvidia-modeset` is a **single host-global, privileged display device**
(not per-guest like the RM client graph). Its command set (per
`docs/design/virtual_modeset.md` and `nvkms-api.h`) includes the cross-client
sharing/permission group: `GRANT_PERMISSIONS`, `ACQUIRE_PERMISSIONS`,
`REVOKE_PERMISSIONS`, `GRANT_SURFACE`/`ACQUIRE_SURFACE`/`RELEASE_SURFACE`,
swap-groups and framelock, plus `ALLOC_DEVICE`, `SET_MODE`, `FLIP`,
`REGISTER_DEFERRED_REQUEST_FIFO`, etc. The same-day design doc (commit c8130eb)
explicitly **rejects** forwarding NVKMS: *"NVKMS forwarding … Rejected — largest/
gnarliest attack surface in the driver."* The code contradicts the design.

**Exploit sketch:** A malicious guest on a graphics-enabled VM crafts an
`IOCTL_ON_ISOLATE(cmd=0xC0106D00)` with `cmdType = NVKMS_IOCTL_GRANT_PERMISSIONS`
(or ACQUIRE_PERMISSIONS) and a forged payload in the aux slot. QEMU performs no
`cmdType` check and the stub forwards it to the host NVKMS, which processes a
permission-grant / surface-acquire against the host's global display state —
reaching a surface or device another guest (or the host compositor) owns, or
driving SET_MODE/FLIP against the real display. Even absent a clean cross-guest
read primitive, this is an unbounded privileged host display surface exposed to
an untrusted guest, with permission-sharing verbs that are precisely the
cross-tenant primitive the threat model forbids.

**Suggested fix:** Do not forward NVKMS at all (adopt the design doc's
virtual-KMS + present-path plan). If forwarding must stay short-term, add a
default-deny `cmdType` allowlist in QEMU mirroring the DRM/ctrl allowlists,
permitting only the render-sync subset actually needed (and *never*
GRANT/ACQUIRE/REVOKE_PERMISSIONS, GRANT/ACQUIRE_SURFACE, SET_MODE, ALLOC_DEVICE
on a shared device). Bound `size`/`address` payload like the other aux paths
(it is, via `NVKVM_SHM_SLOT_DEFAULT_SIZE`, but the subcommand gate is the
missing control).

---

### G-2 — RM_IDLE_CHANNELS struct size/layout mismatch (HIGH)

**Location:** struct `src/abi/nvgpu.h:359-367` (`nv_ioctl_idle_channels`, 40B);
size table `src/guest/nvkvm_ioctl.c:184-185`; sanitizer
`src/guest/nvkvm_ioctl.c:434-441`. This is the exact cmd in the dmesg line
(`cmd=0xc0384641` → `_IOC_SIZE = 0x38 = 56`, `nr = 0x41`,
`our = 40`). 0x41 is in the frontend allowlist (`nvkvm_fe_alloc_allowlist.h:26`),
so it is guest-reachable.

**Description:** The driver's `NVOS30_PARAMETERS` (verified against gVisor
`pkg/abi/nvgpu/frontend.go`) is **56 bytes**:
`Client u32, Device u32, Channel u32, NumChannels u32, Clients P64, Devices P64,
Channels P64, Flags u32, Timeout u32, Status u32, Pad0[4]`. nvkvm's
`nv_ioctl_idle_channels` is **40 bytes** and a *different layout*: the three
P64 pointers come **first** (`p_clients@0, p_devices@8, p_channels@16`), then
four u32 scalars. So:

1. **Size truncation / stub-heap OOB read.** `nvkvm_ioctl_param_size` returns 40
   for nr 0x41, so the guest copies 40 bytes from a 56-byte user struct and
   forwards a 40-byte `param_buf`. The stub `malloc`s 40 bytes and calls
   `ioctl(fd, 0xc0384641, param_buf)`; the host nvidia kmd reads `_IOC_SIZE`=56
   bytes from the 40-byte buffer → **16-byte OOB read of stub heap** feeding the
   driver's idle-channels logic.
2. **Guest-value-as-pointer deref in the host driver.** The sanitizer zeroes the
   *nvkvm-layout* pointer slots (offsets 0/8/16). But in the *driver* layout,
   offset 16 is `Clients`, while `Devices`@24 and `Channels`@32 are NOT zeroed —
   in nvkvm's layout those bytes are the `notify_clients`/`timeout_us`/`status`
   scalars, fully guest-controlled. The driver, seeing `NumChannels` (its @12,
   which is nvkvm's `p_channels` low dword) non-zero, walks `Devices`/`Channels`
   as list pointers → **dereferences a guest-controlled 64-bit value as a pointer
   in the stub's address space.**

**Exploit sketch:** Guest issues IDLE_CHANNELS with the 56-byte buffer arranged
so that, after nvkvm's 40-byte copy + offset-0/8/16 zeroing, the bytes the driver
reads as `NumChannels` are non-zero and the bytes it reads as `Devices`/`Channels`
are an attacker value V. The host kmd dereferences V inside the stub → arbitrary
stub-memory read (data ends up in driver bookkeeping) or stub crash (SIGSEGV →
`stub_exit(139)`, i.e. a self-DoS of the guest's own isolate, contained but
guest-triggerable). The OOB heap read is unconditional.

**Suggested fix:** Replace `nv_ioctl_idle_channels` with the correct 56-byte
NVOS30 layout (handles first, then NumChannels, then the three P64s, then
Flags/Timeout/Status/Pad), return 56 from the size table, and aux-stage the three
`NumChannels`-entry u32 arrays (zero the real P64 slots at offsets 16/24/32 and
copy the arrays into aux like FIFO_GET_CHANNELLIST) — or special-case
`NumChannels==0` and reject `>0` until the arrays are marshalled. This closes
both the OOB read and the pointer-deref. (Generalises audit gap A.2 in
`embedded_field_translation.md`, which only flagged the dropped-data correctness
half.)

---

### G-3 — DRM allowlist permits unmarshalled guest-VA ioctls (MEDIUM)

**Location:** `src/qemu/nvkvm_drm_allowlist.h:31,37,38,39` (allows
GEM_IMPORT_USERSPACE_MEMORY 0x02, GEM_MAP_OFFSET 0x0a, GEM_EXPORT_DMABUF_MEMORY
0x0d, GEM_IDENTIFY_OBJECT 0x0e, plus FENCE_SUPPORTED 0x04, PRIME_FENCE_CONTEXT
0x05, GEM_PRIME_FENCE_ATTACH 0x06, GET_CLIENT_CAPABILITY 0x08, SEMSURF_FENCE_WAIT
0x16, SEMSURF_FENCE_ATTACH 0x17). Stub forward: `src/stub/nvkvm_stub.c:1106`
generic `stub_ioctl`. The guest module (`nvkvm_drm.c`) only registers handlers
for 0x03/0x0f/0x14/0x15, so a *benign* guest can't reach the others — but the
threat model gives the attacker the guest kernel module, which can emit any
`'d'`-type IOCTL_ON_ISOLATE directly over virtio, and QEMU is the real boundary.

**Description:** `DRM_NVIDIA_GEM_IMPORT_USERSPACE_MEMORY` (0x02) takes
`{ u64 address; u64 size; u32 generic_handle; }` where `address` is a **guest
userspace VA** the host pins (`pin_user_pages`). There is no aux-staging for any
DRM ioctl except `'d'` nr 0x54 (SEMSURF_FENCE_CTX_CREATE) in both guest and stub.
So the `address` is forwarded verbatim and the host driver pins pages at that VA
**in the stub's address space**. GEM_MAP_OFFSET / EXPORT_DMABUF similarly mint a
GEM object / dma-buf the guest can subsequently map through the GPA window.

**Exploit sketch:** Malicious guest sends `IOCTL_ON_ISOLATE(cmd = DRM 'd' nr
0x02)` with `address` = a stub heap VA (the stub's layout is fairly
deterministic — fixed memfd/mmap window). The host pins stub pages into a GEM
object; the guest then GEM_MAP_OFFSET + maps it via the GPA window and reads
**stub heap contents** (other in-flight ioctl params, handle table fds as
integers, etc.). Self-stub-harm/info-disclosure within the isolate — but it is
the isolate that holds the cross-VM trust (its fds reach the host driver), so
disclosing its memory weakens the boundary. Not a clean cross-*guest* read
(each guest has its own stub), hence MEDIUM not CRITICAL.

**Suggested fix:** Remove from the allowlist every DRM nr that carries a guest
VA / unimplemented marshalling until it has a proper aux path: at minimum drop
0x02 (IMPORT_USERSPACE_MEMORY), 0x0a (MAP_OFFSET), 0x0d (EXPORT_DMABUF), 0x0e
(IDENTIFY_OBJECT) — keep only the nrs with a real guest+stub handler (0x03,
0x0f, 0x14, 0x15) and 0x09 (GEM_CLOSE). Re-add each with guest-VA marshalling
when its render milestone lands. (The header comment claims "only the
compute/render-relevant render-node ioctls are permitted," but the set is wider
than what is actually marshalled.)

---

### G-4 — Embedded fd→stub-fd resolution has no handle-type check (MEDIUM)

**Location:** stub NVKMS plane fds `src/stub/nvkvm_stub.c:780-797`;
EXPORT_OBJECT_TO_FD `:856-875`; NV0005/EVENT data `:1059-1088`; UVM/FE fd blocks
`:964-1047`. Guest side: NVKMS plane `src/guest/nvkvm_main.c:1024-1041`,
EXPORT `:1076-1087`, EVENT data `:1471-1503`.

**Description:** Every embedded-fd path resolves a guest-supplied `handle_id`
through `handle_lookup(id)` (`nvkvm_stub.c:449`), which is a bare
`handle_fds[id]` array with **no type information** — the stub has no notion of
"this handle is an nvidiactl fd vs nvidia0 vs eventfd vs memory fd." The guest
chooses which `handle_id` goes into the NVKMS plane-fd slot, the
EXPORT_OBJECT_TO_FD fd, the NV0005 `data` fd, etc. So a guest can put *any*
handle_id it owns into *any* of these embedded-fd fields, and the stub hands the
corresponding fd to the host driver. The host driver does its own `f_op` check
(e.g. `osUserHandleToKernelPtr` verifies `nv_frontend_fops`), so a wrong fd type
generally fails — but this is the guest substituting one of its *own* handles for
another, i.e. intra-VM confusion; no cross-tenant reach (the handle table is
per-isolate). Flagged MEDIUM because it is a new, unaudited fd path on a
privileged device (NVKMS) and the type-blindness is a latent footgun (audit
L-4 in 2026-05-30 flagged the same dup-without-type-check pattern for UVM).

**Exploit sketch:** Intra-VM only: guest passes the handle_id of its
`/dev/nvidiactl` handle in the NVKMS REGISTER_SURFACE plane-fd slot; the stub
dups its real nvidiactl fd into the NVKMS surface registration. Host NVKMS
likely rejects, but any path where the host accepts a mismatched fd type is a
guest-self-corruption vector. No host/cross-guest impact.

**Suggested fix:** Carry a `type` byte per handle in the stub table (set at
OPEN_NVIDIA_HANDLE / OPEN_MEMORY_HANDLE time, mirroring QEMU's
`NVKVM_HANDLE_TYPE_*`) and assert the expected type in each embedded-fd
resolution (frontend fd → device handle; OS_EVENT data → eventfd). Cheap
defense-in-depth; aligns with N-1 (2026-05-30) which added the same TYPE_MEMORY
check on READ/WRITE_MEMORY_HANDLE.

---

### G-5 — NV01_EVENT_OS_EVENT via nvos21 alloc skips data-fd translation (LOW)

**Location:** guest nvos21 RM_ALLOC branch
`src/guest/nvkvm_main.c:1244-1345` sizes the aux for `NV01_EVENT_OS_EVENT` /
`NV01_EVENT` (`:1299-1302`) but contains **no `ep->data` fd translation**; only
the nvos64 branch translates it (`:1471-1503`). The stub's data-fd resolver
(`nvkvm_stub.c:1061-1088`) keys on `(cmd&0xff)==0x2b && param_size>=16` and reads
h_class at param+12, so it fires for **both** nvos21 and nvos64.

**Description:** If a guest issues an OS_EVENT alloc in the 32-byte nvos21 form,
the guest forwards the **raw guest fd** in `data` (untranslated). The stub then
treats that raw guest fd value as a `handle_id` and `handle_lookup`s it: either
EBADF (event alloc fails — correctness) or, if the integer collides with a live
handle_id, it resolves to an unrelated fd **within the same isolate** (intra-VM
fd/handle confusion). No cross-tenant reach. libcuda uses the nvos64 form so this
is currently unexercised, but it is guest-reachable.

**Suggested fix:** Mirror the nvos64 `ep->data` translation in the nvos21 branch
(fget → handle_id), or have the stub gate the EVENT data translation on the
nvos64 param_size so a raw nvos21 fd is never misread as a handle_id.

---

### G-6 — DRM GEM proxy holds ctx without a refcount (LOW)

**Location:** `src/guest/nvkvm_drm.c:93-111` (`nvkvm_gem_proxy_create` stores
`ng->ctx = ctx` with no get), `:70-86` (`nvkvm_gem_free` dereferences `ng->ctx`
to forward GEM_CLOSE).

**Description:** The proxy GEM caches the `nvkvm_fd_ctx *` of the DRM file and,
on free, forwards `DRM_IOCTL_GEM_CLOSE` to the isolate via `ng->ctx`. The comment
asserts "guest GEM handles are released before the driver's postclose runs, so
ctx is still live." That holds for the normal `drm_release` ordering, but a GEM
object whose refcount is elevated by a dma-buf export (or any future
cross-file/FLINK sharing) can outlive the `drm_file`; `nvkvm_drm_postclose` then
`nvkvm_fd_ctx_close(ctx)` + frees it, and a later `nvkvm_gem_free` dereferences a
freed `ctx` → UAF in the guest kernel. Today no export/FLINK path is wired, so
it is latent (LOW), but the GEM proxy was added for exactly the
dma-buf/PRIME milestone that will introduce that lifetime.

**Suggested fix:** Take a reference that keeps the forwarding context alive for
the proxy's lifetime (refcount the `nvkvm_fd_ctx`, or the session/isolate it
forwards through), released in `nvkvm_gem_free`; or null `ng->ctx` in
`postclose` and skip the GEM_CLOSE forward when it is gone (the isolate teardown
reaper already releases the underlying objects).

---

### G-7 — EXPORT_TO_DMABUF_FD allowed but unhandled → stub fd leak (LOW)

**Location:** allowlisted `src/qemu/nvkvm_fe_alloc_allowlist.h:34` (0x70); no
special handling in QEMU or stub (`struct nv_ioctl_export_to_dmabuf_fd`,
`src/abi/nvgpu.h:458`, has an OUT `fd`).

**Description:** The export creates a real dma-buf **fd in the stub process**;
nothing closes it and there is no passback channel, so each call leaks one stub
fd (fd-exhaustion DoS of the guest's own isolate over many calls) and returns a
meaningless stub fd integer to the guest. `h_client`/`h_memory` are gated by the
per-VM client allowlist, so no cross-tenant reach; severity LOW (self-isolate
resource leak / non-functional). 

**Suggested fix:** Either drop 0x70 from the allowlist until the dma-buf
passback milestone, or have the stub track and close the exported fd on isolate
teardown (extend the #80 reaper) and zero the OUT fd before responding.

---

### G-8 — Async IOCTL path has no per-cmd expected-size check (INFORMATIONAL)

**Location:** `src/qemu/virtio_nvgpu.c:729-766` (the IOCTL_ON_ISOLATE thread-pool
path uses `slot_blob` for bounding only — it does **not** call
`nvkvm_ioctl_expected_param_size`, which only guards the dead synchronous path at
`:400-406`).

**Description:** `slot_blob` correctly bounds `param_size`/`aux_size` to
`slot_size` (audit C-1, still in place — no OOB), so this is memory-safe. But it
means the **only** thing enforcing that `param_size` matches the real driver
struct size is the guest's own `nvkvm_ioctl_param_size` table — a malicious guest
can send any size ≤ slot_size and the stub forwards it. The host kmd then reads
`_IOC_SIZE(cmd)` bytes regardless. This is the structural reason G-2 is
exploitable (and why any future wrong-size table entry is a latent stub-OOB-read).
Memory-safe at the VMM boundary; the residual risk is entirely "host driver reads
`_IOC_SIZE` from a possibly-shorter stub buffer."

**Suggested fix:** In the stub (which knows the cmd), allocate `param_buf` to
`max(param_size, _IOC_SIZE(cmd))` for `'F'`/`'d'`/`'m'` ioctls so the host driver
never reads past the allocation even when the guest under-sizes — a cheap
belt-and-suspenders that neutralises this whole bug class (including G-2's OOB
read) independent of fixing each table entry.

---

## Cross-tenant assessment (graphics paths)

- **NVKMS (G-1)** is the one path with plausible cross-VM/host impact, via the
  permission/surface-sharing subcommands on a host-global device — fix before any
  multi-tenant graphics exposure.
- **DRM render node**: per-isolate; GEM proxy + handle translation are intra-VM.
  The allowlist over-permits guest-VA ioctls (G-3) = self-stub info-disclosure,
  not cross-guest.
- **Embedded fds** (NVKMS plane / EXPORT / EVENT / UVM / FE): all resolve only
  within the calling isolate's own handle table; no guest can name another
  isolate's fd. Type-blindness (G-4) and the nvos21 gap (G-5) are intra-VM
  confusion/correctness.
- **hClient / DUP_OBJECT gates** (audit H-3 / Phase-4): confirmed still applied
  to the 'F' graphics allocs (`nvkvm_isolate_handlers.c:1053-1089`,
  `:1024-1039`). The alloc-class allowlist now includes the AMPERE_B graphics +
  NV01_EVENT classes (commit d289ade) — still default-deny.

## Priorities

1. **G-1** — add an NVKMS subcommand default-deny allowlist (or drop NVKMS
   forwarding per the design doc). Cross-VM/host.
2. **G-2** — fix the NVOS30 struct (size + layout) and marshal/clamp the lists;
   or apply G-8's stub-side `max(param_size,_IOC_SIZE)` allocation to kill the
   OOB-read class.
3. **G-3** — trim the DRM allowlist to marshalled nrs only.
4. **G-4/G-5/G-6/G-7** — defense-in-depth / correctness, schedule with the
   render + present-path milestone.
