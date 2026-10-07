# nvkvm Isolate Architecture Plan

Goal: enable ChatGPT-scale model inference inside a KVM guest with full CUDA/UVM support.
The blocker is **VA consistency**: when NVIDIA's kernel driver processes an ioctl it checks that
pointer-valued arguments are mapped in `current->mm`. Today's model forwards ioctls from a QEMU
thread whose `mm` is QEMU's — not the guest process's — so any ioctl that contains a userptr
(UVM allocations, graph launches, channel submits) fails with EFAULT or NV_ERR_INVALID_ADDRESS.

## Architecture: nvidia isolates

One **isolate** process per guest userspace virtual address space (`struct mm_struct`).
An isolate is a minimal static process spawned by QEMU with no libc, raw syscalls, seccomp
allowlist, and a Unix socket back to QEMU. It mirrors the guest GVA layout:
- All GPU device fds opened by the guest are `dup`'d into the isolate via SCM_RIGHTS.
- Only mmaps **backed by actual guest physical pages** are replayed on the isolate at the
  exact same GVA (`MAP_FIXED`). A file-backed `mmap()` whose pages have not been faulted
  in yet has VMAs in the guest page table but no physical backing — no isolate mmap happens
  until the page is actually present in RAM.
- When a new physical page is allocated (page fault handler runs, file content loaded, or
  anonymous page zero-filled), the module intercepts and migrates it to a memfd so the
  isolate can map it by fd rather than by physical address.

QEMU drives every ioctl on behalf of the guest by sending an `IOCTL` command to the
correct isolate. The isolate executes the ioctl and returns the result; QEMU relays it
to the guest over virtio.

```
guest userspace
      │  syscall open/ioctl/mmap
      ▼
guest kernel (nvkvm-guest.ko)
      │  virtio TX request
      ▼
QEMU virtio-nvgpu backend
      │  unix socket + SCM_RIGHTS
      ▼
isolate process  (one per guest mm)
      │  real syscall  ioctl/mmap/munmap
      ▼
NVIDIA kernel driver + GPU hardware
```

## Handle system

Two opaque handle IDs span the entire protocol. QEMU is the single owner of all
underlying fds; handle IDs are what the guest and isolates see.

| Type | Underlying fd | Created by | Distributed by |
|------|--------------|------------|----------------|
| nvidia handle | `open("/dev/nvidia*")` in QEMU | `open_nvidia_handle` | SCM_RIGHTS to isolates |
| memory handle | `memfd_create()` in QEMU | `open_memory_handle` | SCM_RIGHTS to isolates |

Handle IDs are 32-bit integers drawn from a monotonic counter, unique across all sessions.
A handle may be held by zero or more isolates simultaneously. Reference counting:
- `copy_handle_to_isolate` → send fd via SCM_RIGHTS, increment refcount on isolate's side
- `close_handle_on_isolate` → send CLOSE_FD command to isolate, decrement refcount
- `close_handle` → allowed only when no isolate holds the handle; closes the underlying fd

## Double-mmap for GPU memory

For every GPU memory region (BAR-backed or UVM-managed):
1. QEMU calls `mmap(NULL, len, prot, MAP_SHARED, nvidia_fd, offset)` → gets QVA.
2. QEMU registers GPA→QVA with `KVM_SET_USER_MEMORY_REGION` so the guest can read/write
   GPU VRAM via the GPA window (zero-copy guest access).
3. QEMU sends `mmap_on_isolate(isolate_id, handle_id, gva, len, prot, MAP_SHARED|MAP_FIXED, offset)`
   so the isolate maps the same physical pages at the exact GVA. Now `current->mm` in the
   isolate has the GVA mapped, so NVIDIA ioctl pointer checks pass.

Same underlying physical pages — guaranteed by MAP_SHARED on the same fd+offset (confirmed
by the CUDA IPC mechanism and NVIDIA RM memory object idempotency).

## Physical page migration to memfd

NVIDIA's `get_user_pages()` and UVM's `looks_like_userptr` work on real physical pages
mapped in `current->mm`. For the isolate to serve these, every backed page must be
accessible via a fd the isolate holds, not via a KVM-managed guest-physical address.

**Rule: every continuous physical page range gets its own memfd** (independent permissions,
independent lifetime, independently `munmap`-able in each isolate).

**Flow when a guest page fault allocates a new physical frame:**
1. Guest kernel `do_page_fault()` (or `handle_mm_fault()`) runs normally.
2. Our `vm_ops->fault` or `mmu_notifier::change_pte` hook fires for any VMA owned by a
   process that has a registered isolate.
3. For the newly-backed GVA range: guest kernel allocates a memfd memory handle (via
   `open_memory_handle` virtio call). QEMU creates `memfd_create()`.
4. Guest kernel copies the physical page content into the memfd (`/proc/self/mem` or a
   kernel copy path).
5. QEMU sends `mmap_on_isolate(isolate_id, memfd_handle, gva, PAGE_SIZE, PROT_RW,
   MAP_SHARED|MAP_FIXED, 0)` to the isolate.
6. Isolate maps the memfd at the exact GVA. From this point, `get_user_pages()` in the
   isolate's mm returns pages from the memfd — the NVIDIA driver is satisfied.
7. For future writes by the guest: guest writes to the GPA (backed by QEMU's QVA mmap of
   the memfd) propagate through the memfd to the isolate's view automatically (MAP_SHARED).

**Lazy conversion:** processes already running when an isolate is first created have
existing backed pages. These are lazily converted to memfd on the first EFAULT from an
ioctl — not eagerly. The demand-fault mechanism handles this.

**New allocations after isolate registration:** `mmap(MAP_ANONYMOUS)` or `malloc()` that
triggers a page fault goes through the above flow. Any anonymous page whose backing GPA
is newly allocated is immediately funnelled into a memfd.

**VMA whitelist = ALL VMAs, not just backed ones.** The whitelist sent alongside an ioctl
for demand-fault checking includes all VMA entries (file-backed, anonymous, device-mapped),
regardless of whether they currently have physical backing. This is correct because:
- The NVIDIA driver's `looks_like_userptr` tests pointer values against the VMA list, not
  against the page table.
- A file-backed VMA that hasn't been paged in yet is still a valid userptr candidate —
  the driver will `get_user_pages()` and trigger a page fault if needed.

## Demand-fault for userptr allocations

UVM userptr allocations (cuMemHostAlloc → cuMemMap flow) register guest VA ranges as
GPU-accessible memory. The NVIDIA driver internally calls `get_user_pages()` on the
registered range. If the pages are not yet mapped in the isolate's mm (e.g. first access
after a `mmap(MAP_ANONYMOUS)` + `UVM_REGISTER_GPU_VASPACE`), the ioctl returns EFAULT.

Resolution:
1. Isolate returns EFAULT to QEMU.
2. QEMU notifies guest kernel module via virtio response with `NVKVM_STATUS_EFAULT` flag.
3. Guest kernel collects its VMA list (whitelist) for the current process.
4. Guest kernel sends `ioctl_on_isolate` again with VMA whitelist attached.
5. QEMU scans aux_buf for pointer-sized values in whitelist ranges.
6. For each such pointer `p`: guest kernel does CoW of the physical page at `p` into a
   newly allocated memory handle (memfd). QEMU sends `mmap_on_isolate` at GVA=p.
7. QEMU retries the ioctl on the isolate. Now `get_user_pages()` succeeds.

This is bounded: each re-fault maps at most one new page; the loop terminates when all
referenced pages are present or a hard error occurs.

## Fork handling

When guest userspace calls `fork(2)`, Linux calls `dup_mmap()` which in turn calls
`vm_ops->open()` on every VMA. The guest kernel module's `vm_ops->open` hook detects
the new `current->mm` pointer:
1. Sends `create_isolate` to QEMU → gets `new_isolate_id`.
2. Iterates over `MAP_SHARED` VMA regions in the new mm (GPU memory regions).
3. For each such region: sends `copy_handle_to_isolate(handle_id, new_isolate_id)` then
   `mmap_on_isolate(new_isolate_id, handle_id, gva, len, prot, MAP_SHARED|MAP_FIXED, offset)`.
4. Records `new_isolate_id` in the new process's session.

QEMU spawns all isolates directly (flat process tree), not from each other, so the isolate
process image stays small and never inherits unexpected state.

## Process exit / mm teardown

When a guest process exits, `mmu_notifier::invalidate_range_end` is called. The guest
kernel module registers an mmu_notifier per session:
1. On `::release`: send `kill_isolate(isolate_id)` to QEMU.
2. QEMU: for each handle on this isolate: send `CLOSE_FD` to isolate.
3. QEMU: send `EXIT` to isolate, wait for it to exit.
4. QEMU: for memory handles created for this session: release KVM memory slots and munmap QVAs.
5. QEMU: decrement handle refcounts; `close_handle` any with refcount 0.

## Security model

**Two trust boundaries:**
- Guest userspace → guest kernel module: module validates all sizes, no raw VA forwarding.
- Guest kernel module → QEMU: QEMU validates every field; unknown handle/isolate IDs cause
  QEMU to panic the VM immediately (no graceful recovery — a misbehaving guest kernel is
  considered a compromised or malicious guest).
- QEMU → isolate: QEMU does not trust the isolate. QEMU validates every response from the
  isolate. Isolate cannot call `KVM_SET_USER_MEMORY_REGION` (never given KVM fd).

**Seccomp on isolates:**
Allowlist: `read, write, recvmsg, sendmsg, ioctl, mmap, mprotect, munmap, poll, ppoll,
close, exit_group`. No `clone`. Applied after setup; fail gracefully if rootless kernel
policy blocks seccomp.

**VMA whitelist as DoS guard:**
Only pointer-sized values in aux_buf that fall within actual mapped VMAs from the guest's
whitelist are treated as demand-fault candidates. This prevents a malicious guest kernel
from triggering arbitrary memory reads in the isolate.

---

## Protocol (nvkvm_proto.h — new commands)

```
NVKVM_REQ_LIST_NVIDIA_DEVICES   → list of (dev_id, major, minor)
NVKVM_REQ_OPEN_NVIDIA_HANDLE    → open /dev/nvidia* in QEMU    → handle_id
NVKVM_REQ_OPEN_MEMORY_HANDLE    → memfd_create in QEMU          → handle_id
NVKVM_REQ_CLOSE_HANDLE          → close underlying fd            → status
NVKVM_REQ_CREATE_ISOLATE        → fork isolate process           → isolate_id
NVKVM_REQ_KILL_ISOLATE          → exit isolate process           → status
NVKVM_REQ_COPY_HANDLE_TO_ISOLATE→ SCM_RIGHTS send               → status
NVKVM_REQ_CLOSE_HANDLE_ON_ISOLATE→ CLOSE_FD to isolate          → status
NVKVM_REQ_IOCTL_ON_ISOLATE      → IOCTL command to isolate      → retval+status
NVKVM_REQ_MMAP_ON_ISOLATE       → MMAP command to isolate       → gpa
NVKVM_REQ_MUNMAP_ON_ISOLATE     → MUNMAP command to isolate     → status
NVKVM_REQ_POLL_ON_ISOLATE       → POLL registration             → status
NVKVM_REQ_UNPOLL_ON_ISOLATE     → POLL deregistration           → status
```

## Isolate stub protocol (unix socket)

QEMU and the isolate communicate over a `SOCK_SEQPACKET` socket pair.
All messages start with a 4-byte type field.

```
ISOLATE_CMD_RECEIVE_FD   → fd arrives via SCM_RIGHTS; isolate records it keyed by handle_id
ISOLATE_CMD_CLOSE_FD     → close fd for handle_id
ISOLATE_CMD_IOCTL        → fd + cmd + param_size + aux_size; data follows in-band
ISOLATE_CMD_MMAP         → fd + gva + len + prot + flags + offset
ISOLATE_CMD_MUNMAP       → gva + len
ISOLATE_CMD_POLL         → fd + events; isolate starts polling in background
ISOLATE_CMD_UNPOLL       → fd; stop polling
ISOLATE_CMD_EXIT         → clean shutdown
```

Isolate responses mirror commands with a `retval` and `errno` field.

---

## Implementation phases

### Phase 1 — new protocol (this PR)
- [ ] Rewrite `src/common/nvkvm_proto.h` with isolate/handle commands
- [ ] Add `nvkvm_isolate_proto.h` for stub-side command structs

### Phase 2 — QEMU isolate and handle managers
- [ ] `src/qemu/nvkvm_handle.c` — global handle table (nvidia + memory handles)
- [ ] `src/qemu/nvkvm_isolate.c` — spawn/kill/manage isolate processes + socket dispatch
- [ ] Update `src/qemu/virtio_nvgpu.h` with new session/handle/isolate structs

### Phase 3 — isolate stub binary
- [ ] `src/stub/nvkvm_stub.c` — self-relocating, no libc, raw syscalls, seccomp, SCM_RIGHTS, main loop
- [ ] `src/stub/Makefile` — static link, `-nostdlib`, `-static`, stripped

### Phase 4 — QEMU virtio handler updates
- [ ] Update `nvkvm_dispatch.c` to route `ioctl_on_isolate` / `mmap_on_isolate`
- [ ] Update `nvkvm_frontend.c` to send ioctls to isolate instead of direct host_ioctl
- [ ] Update `nvkvm_mmap_host.c` for double-mmap

### Phase 5 — guest kernel module
- [ ] Add mmu_notifier per session in `nvkvm_session.c`
  - `::release` → send kill_isolate to QEMU
  - `::change_pte` → detect newly-backed pages in UVM-registered ranges,
    trigger memfd migration
- [ ] Add vm_ops hooks in `nvkvm_mmap.c`
  - `::open` → detect fork (new mm), create_isolate + replay MAP_SHARED mmaps
  - `::close` → send munmap_on_isolate when last VMA reference drops
- [ ] Physical page migration path:
  - `scan_and_migrate_range(gva, len)`: walk PTEs, for each backed page:
    (a) send open_memory_handle to QEMU → get memfd handle
    (b) copy physical page content into memfd (/proc/self/mem or kernel copy)
    (c) send mmap_on_isolate(memfd_handle, gva, PAGE_SIZE, MAP_FIXED|MAP_SHARED)
  - Called from: UVM_REGISTER_GPU handler, demand-fault retry path
- [ ] Demand-fault VMA whitelist generation: iterate all VMAs (backed or not)
  and pack as nvkvm_vma_entry array in shm slot
- [ ] Update `nvkvm_virtio.c` for new request types
- [ ] Update `nvkvm_main.c` open/close/ioctl/mmap paths

### Phase 6 — tests
- [ ] Unit tests for handle table (alloc/free/refcount)
- [ ] Unit tests for isolate manager (spawn/kill/dispatch)
- [ ] Unit tests for new virtio protocol framing
- [ ] Integration test: open nvidiactl via isolate path (needs VM)

---

## Files created / modified

| File | Action |
|------|--------|
| `src/common/nvkvm_proto.h` | rewrite |
| `src/common/nvkvm_isolate_proto.h` | new |
| `src/qemu/nvkvm_handle.c` | new |
| `src/qemu/nvkvm_handle.h` | new |
| `src/qemu/nvkvm_isolate.c` | new |
| `src/qemu/nvkvm_isolate.h` | new |
| `src/qemu/virtio_nvgpu.h` | extend structs |
| `src/qemu/virtio_nvgpu.c` | new request handlers |
| `src/qemu/nvkvm_dispatch.c` | route to isolate |
| `src/qemu/nvkvm_frontend.c` | send via isolate |
| `src/qemu/nvkvm_mmap_host.c` | double-mmap |
| `src/stub/nvkvm_stub.c` | new (stub binary) |
| `src/stub/Makefile` | new |
| `src/guest/nvkvm_session.c` | mmu_notifier |
| `src/guest/nvkvm_mmap.c` | vm_ops hooks |
| `src/guest/nvkvm_virtio.c` | new request types |
| `src/guest/nvkvm_main.c` | open/close/ioctl/mmap paths |
| `tests/unit/test_handle.c` | new |
| `tests/unit/test_isolate.c` | new |
