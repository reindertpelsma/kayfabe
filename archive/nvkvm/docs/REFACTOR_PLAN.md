# nvkvm-tables-refactor — plan + decision record

Branch: `nvkvm-tables-refactor` (cut from `master` after commit `8ff18f4`).
Goal: replace the two parallel ID spaces (fd_token + handle_id) with a single
authoritative four-table model in QEMU, and remove the `dup()` workaround
that's currently masking a referencing bug.

Priority: getting a ChatGPT-scale LLM running inside the guest. Open driver
only (closed driver is being deprecated). Everything below serves that goal.

---

## 1. Architecture summary

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Guest userspace (libcuda)                                               │
└────────────────┼────────────────────────────────────────────────────────┘
                 ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Guest kernel (nvkvm-guest.ko) — speaks handle_id only, owns txn_id      │
│   maintains its own (multi-isolate, multi-handle) map for fork-aware    │
│   routing                                                               │
└────────────────┼────────────────────────────────────────────────────────┘
                 ▼ virtio
┌─────────────────────────────────────────────────────────────────────────┐
│ QEMU — four tables, single source of truth                              │
│   1. Handle table     handle_id → {qemu_fd, type, ready, refcount, poll}│
│   2. Isolate table    isolate_id → {pid, comm_fd, alive}                │
│   3. Iso↔Hnd map      M:N  (isolate_id, handle_id) → fd_on_isolate     │
│   4. MMAP table       mmap_id → handle_id, offset, vmm_va, gpa, size... │
│                                                                         │
│   GPA windows (one big sparse memfd per window, sub-allocate)           │
│                                                                         │
│   Allocates handle_id BEFORE telling isolate to open anything           │
│   Wire to isolate carries stub-local fd numbers (not handle_ids)        │
│   Strict cleanup: deps before owner                                     │
└────────────────┼────────────────────────────────────────────────────────┘
                 ▼ unix socket + SCM_RIGHTS (both directions)
┌─────────────────────────────────────────────────────────────────────────┐
│ Stub — table-free                                                       │
│   Obeys "ioctl(fd=N, …)" / "mmap(fd=N, …)" / "open(path)"               │
│   open() reply atomically carries (stub_fd_number + SCM_RIGHTS fd)      │
│   One I/O thread (strict FIFO). Worker pool, clone3, auto-scale, ≥1 idle│
│   Tiny per-process bookkeeping only for WRITE_MEMORY_HANDLE cleanup     │
└─────────────────────────────────────────────────────────────────────────┘
```

### Open ownership

| Device                          | Opener | Why                                       |
|---------------------------------|--------|-------------------------------------------|
| `/dev/nvidiactl`                | stub   | RM control; nvfp identity = stub's process |
| `/dev/nvidia0..N`               | stub   | events want distinct nvfp/fd; multi-client OK on driver |
| `/dev/nvidia-uvm`               | QEMU   | driver enforces "opener does mmap"        |
| memfd_create (GPA backing)      | QEMU   | KVM region installation lives here        |
| eventfd                         | either | no opener-binding; stub for consistency   |

**QEMU always holds an SCM_RIGHTS copy of every stub-opened fd**. `qemu_fd == -1`
window is bounded to the open syscall itself. Single sendmsg from stub
carries (reply, SCM_RIGHTS fd). If isolate dies, kernel struct file stays
alive via qemu_fd; ioctls return EBADF cleanly; close only happens when
guest releases its guest fd.

### ID semantics

- `handle_id`, `isolate_id`, `mmap_id`: all u32, low 12 bits = slot index,
  high 20 bits = generation. Stale ids fail lookup → no fd-reuse confusion.
- Allocated by QEMU only. Stub never invents IDs.
- Wire format QEMU↔stub uses stub-local fd numbers, not handle_ids.
- `txn_id` (u32) chosen by guest kernel, mirrored on every reply. Enables
  out-of-order completion. Track in-flight set as bitmap; never reuse a
  currently-in-flight id.

### Refcount + ready

- Every operation `acquire`s the handle: lookup under lock, check `ready`,
  `refcount++`, release lock.
- Operation runs, then `release`: lock, `refcount--`, signal cv if zero.
- Close: lock, set `ready=false`, wait on cv for `refcount==0`, drop deps,
  free slot.
- **Refcount bracket extends from request reception through reply send**,
  not just dispatch.

### CPU memory paths (in fast-path order)

**0. memfd-passthrough (the dominant case — must be the primary fast path).**
   When guest userspace passes a CPU pointer into an ioctl, and that
   pointer falls within a region already backed by one of our memfd-backed
   GPA windows, *nothing has to move*. We already have a handle for the
   memfd, the stub already has it mapped at a known VA, the guest writes
   already land in the same physical pages the stub sees. The guest module:
     - walks `find_vma(P)` → identifies the GPA window
     - derives (memfd_handle_id, offset_within_handle, size)
     - sends the ioctl request to QEMU carrying the triple; QEMU forwards
       to the stub along with the precomputed stub-side VA
     - stub rewrites the ioctl arg buffer to point at its own VA and calls
   Zero copy, zero allocation, zero coordination. This is what every
   CUDA-allocated buffer hits.

1. **userfaultfd promote (the medium-rare case).**
   Guest userspace allocated a buffer in plain anonymous memory (e.g. via
   `cudaHostRegister` on existing-malloc'd memory). We migrate that GPA
   region into a memfd window once, then the page falls into path (0)
   forever after. Step 5 PoC. If KVM-UFFD doesn't cooperate on 6.8 kernel,
   fallback to brief vCPU pause.

2. **WRITE_MEMORY_HANDLE (the residual edge case).**
   Buffers whose backing memory genuinely can't be swapped — vfio MMIO,
   PCI BAR pass-throughs from other devices, etc. Pointer rewrite in the
   ioctl, buffer bytes copied inline via virtio. Invariant: per-page
   mutex; never two concurrent writes to overlapping range. Stub has a
   tiny post-ioctl cleanup list for these (the only table on stub side).
   Slow but correct. Only path that doesn't hit the kernel struct file
   that's already shared.

### GPA windows

- 128 GiB upfront, one KVM memslot, one memfd backing.
- Sparse sub-allocation: free list of regions sorted by offset, coalesce
  on insert.
- `fallocate(FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE)` on free, with
  ±1 adjacency check (look at predecessor and successor free regions,
  punch the entire contiguous gap). Double-punch is a kernel no-op.
- Additional 128 GiB windows lazily on first ENOSPC.
- Max 8 windows = 1 TiB total addressable.

### Cleanup ordering

```
A handle entry may be freed only after:
  - all entries in MMAP table referencing it are freed,
  - all entries in Iso↔Hnd map referencing it are unlinked,
  - poll registrations are deregistered,
  - in-flight refcount has drained to 0.

A window may be destroyed only after:
  - no MMAP entry references it.

An isolate may be closed only after:
  - all its Iso↔Hnd links are unlinked.
```

---

## 2. Race/blocker decision record

### Resolved with action

| Ref | Issue                                          | Decision                                     |
|-----|------------------------------------------------|----------------------------------------------|
| R1  | UFFD-on-KVM-memory                             | Sub-agent PoC. Fall back to vCPU pause if KVM-UFFD has gaps on 6.8. |
| R2  | KVM memslot exhaustion (512 cap)               | One GPA window = one slot. Sub-allocate via memfd offsets. Lazy 2nd window. |
| R3  | Refcount lifecycle bracket                     | `refcount++` at request reception, before queue; `refcount--` after reply leaves QEMU. Unit test for concurrent close + ioctl. |
| R4  | fork() in guest libcuda                        | Lazy clone of handle to child isolate on first use. M:N map supports this. Same behavior as bare metal (works for memfds, undefined for nvidia per upstream). |
| R5  | Memfd ↔ isolate binding                        | MMAP entries link to specific isolate_id; never re-linked. |
| R6  | Isolate kill mid-op                            | Socket EOF → mark all this isolate's handle entries `ready=false`, drain, error in-flight txns as EIO. Handles owned only by dead isolate stay valid via qemu_fd until guest close. |
| R7  | KVM_SET_USER_MEMORY_REGION during live DMA     | RM_UNMAP_MEMORY must precede KVM mutation. Natural ordering; enforce via assertion. |
| R8  | Stub message ordering                          | One I/O thread, strict FIFO. Workers parallel internally. Close-then-open-fd-reuse race impossible. |
| R9  | txn_id exhaustion                              | u32 + bitmap of in-flight. Hours of headroom at realistic rates. |
| R10 | Sub-allocator fragmentation                    | Sorted free list, coalesce on insert. Unit test churn pattern. Free-list-per-size-class is post-MVP. |
| R11 | PUNCH_HOLE vs concurrent write                 | Punch only after guest munmap completes for that GPA range. Hard ordering invariant. |
| R12 | Cross-isolate handle clone race                | Double-checked locking on the M:N link insert. Test for parallel first-use from two child threads. |
| R13 | Refcount bracket boundary                      | Per R3. Documented in code + tested. |
| R14 | Punch-hole 4 KiB granularity                   | Track only mmap allocations in MMAP table. On free, recompute the contiguous free region around the freed range (predecessor + this + successor if in free list) and punch the whole thing. Already-punched ranges are no-ops. Unit test verifies multiple out-of-order frees consolidate correctly. |
| R15 | Stub thread pool size cap                      | Bound to 256 worker threads. Real workloads stay well below. |
| R16 | Memfd hole reuse                               | Accept "max touched simultaneously" as physical RAM usage. Out of scope for now; revisit if reporting matters. |
| R17 | GPA window placement vs guest e820             | Place windows above guest RAM extent. QEMU CLI / config dictates the boundary. Out of scope for now (unbounded). |
| R18 | Multi-window growth notification               | virtio config-change notification → guest module appends to its window list. Standard pattern. |
| R19 | Stub crash mid-WRITE_MEMORY_HANDLE             | Socket EOF, drain, error txn. Target ioctl never runs. Safe. |
| R20 | KVM_RUN exit reason ordering on vCPU-pause     | Signal-based wakeup is idempotent. Standard pattern. |

### Carried forward as task #34 (Step 5)

- UFFD-on-KVM-memory composition needs a standalone PoC. Until then, the
  mmap-from-day-one path is the only supported zero-copy path.

### Gaps discovered during the cuCtxCreate-401 audit (2026-05-27)

These are NOT in the original R1-R20 list and are NOT covered by the four
tables alone — they are forwarding/ABI hazards that surface when libcuda
exercises the alloc-class path.

- **G1 — Per-alloc-class struct size table is duplicated and drift-prone.**
  The guest `nvkvm_main.c` has two near-identical switch tables (nvos21 and
  nvos64) that map hClass → sizeof(alloc_params). If libcuda is built
  against a driver version whose struct grew (e.g. V580 added PASID to
  NV_VASPACE_ALLOCATION_PARAMETERS), our 48-byte copy_from_user truncates
  the tail and we send a short aux_buf. The driver reads past the end into
  zeros — flags clear → VASize ends up 0. Fix: single shared table, keyed
  by (hClass, driver_version), and refuse to forward if
  `caller_supplied_size > our_known_size` instead of silently truncating.

- **G2 — RM_ALLOC status field (nvos64 offset 40) is read by stub but
  silently ignored on the cuCtxCreate path.** Stub extracts nvstatus and
  forwards it; QEMU forwards to guest. But the QEMU diagnostic only prints
  it when nvstatus != 0. If the driver succeeds (status=0) but vaSize=0,
  there is no log line indicating "driver wrote zero." For
  FERMI_VASPACE_A specifically, add a post-ioctl sanity check in
  `nvkvm_req_ioctl_on_isolate`: if hClass==0x90F1 and status==0 and
  va_size==0, log a loud warning with the full alloc params buffer.

- **G3 — Two parallel handle tables coexist.** `src/qemu/nvkvm_tables.c`
  (four-table model) is built and unit-tested but NOT yet wired into the
  ioctl/open/mmap dispatch paths — those still use `nvkvm_handle.c` (old
  table). The refactor sits mid-Step 3; until Step 3d lands, the old
  table's `isolate_refcount` (incremented in `nvkvm_req_open_nvidia_handle`)
  is the only ref-tracking. This works but means R3/R13's "refcount at
  request reception" invariant is NOT yet enforced for in-flight ioctls
  in the old code. Step 3 must complete and ALL dispatch sites migrate.

- **G4 — Sync command serialization is per-isolate, not per-class.**
  `iso->sync_lock` serializes OPEN_DEVICE, MMAP, MUNMAP, POLL, etc. against
  each other. An OPEN_DEVICE blocks every other sync op for that isolate
  while waiting for the stub round-trip. Not a correctness bug; mention so
  we don't forget it later when scaling to many concurrent threads in the
  guest.

- **G5 — `nvkvm_req_open_nvidia_handle` bumps `isolate_refcount` but does
  NOT insert into the iso↔hnd map of the new tables.** Both data structures
  describe the same physical fact (this isolate holds an fd for this
  handle). Step 3 migration must pick one and delete the other; do not let
  the duplicate persist or close-handle paths will diverge.

- **G6 — The aux_buf round-trip is correct for fixed-size structs but
  ambiguous for `alloc_parms_size=0` callers.** Guest fills
  `alloc->alloc_parms_size = ap_size` before forwarding (line ~690) and
  restores caller's original value (0) after (line ~881). The driver,
  however, may have populated a struct whose size differs from what CUDA
  expected. CUDA verifies `paramsSize` round-trips (we restore) but does
  NOT verify struct content beyond status/vaSize. Risk: if `ap_size` was
  wrong (per G1), CUDA never finds out. Tied to G1.

---

## 3. Step-by-step plan

Each step lands as one or a few commits on `nvkvm-tables-refactor`. Don't
move on until the prior step's tests are green.

### Step 1 — Four-table skeleton + unit tests (task #30, IN PROGRESS)

- `src/qemu/nvkvm_tables.h` — typedefs and API.  ✅ done
- `src/qemu/nvkvm_tables.c` — implementation.    ✅ done (review needed)
- `tests/unit/test_tables.c` — pipe()/memfd_create only, no /dev/nvidia*.
- `tests/unit/Makefile` — add target.
- Tests must cover at minimum:
  - alloc/lookup/close, gen-id stale rejection
  - concurrent close vs in-flight ioctl (R3, R13)
  - GPA allocator fragmentation + coalesce (R10)
  - punch-hole adjacency consolidation, double-punch no-op (R14)
  - isolate-kill mid-op (R6)
  - dependency-ordered cleanup refusal (close-handle-while-mmap-exists)
  - cross-isolate handle clone race on first use (R12)
  - generation rollover (smoke test that gen bits work)
- Existing `test_handle.c`, `test_dispatch.c`, `test_frontend.c`,
  `test_isolate.c` must stay passing.
- Integration: `test_ioctl_fwd` must still pass (no behavior change yet).

### Step 2 — txn IDs end-to-end (task #31)

- Guest module: allocate u32 `txn_id` per outbound request, mirror on reply
  match.
- Virtio header carries `txn_id`; today's `req_id` becomes `txn_id` where
  inconsistent.
- QEMU reader thread: enqueue (txn_id, request) to dispatch pool, reply
  copies `txn_id` back.
- Stub: pass `txn_id` through transparently (round-tripped to QEMU; QEMU
  forwards to guest).
- In-flight tracking: u32 bitmap in guest module (and one in QEMU too if
  we ever need to reject duplicate inbound). Reject reuse of a still-
  outstanding id.

### Step 3 — Reverse open ownership + open-with-SCM_RIGHTS atomic reply (task #32)

- New protocol message `NVKVM_REQ_OPEN_ON_ISOLATE` (guest → QEMU →
  isolate):
  1. QEMU `handle_alloc(type)` → preliminary `handle_id`, qemu_fd=-1, !ready.
  2. QEMU sends "open path X for handle_id N" to isolate.
  3. Stub `open(path)` → local fd, `sendmsg(reply_with_local_fd_number +
     SCM_RIGHTS(fd))` atomically.
  4. QEMU `recvmsg` gets both. `handle_attach_qemu_fd(N, fd_from_scm)` →
     ready=true. `iso_hnd_link(isolate_id, N, stub_local_fd)`.
  5. QEMU returns `handle_id` to guest.
- On any failure: `handle_abort_open(N)` rolls back. Guest sees error.
- Delete legacy NVKVM_REQ_OPEN/CLOSE/IOCTL/MMAP/MUNMAP. Delete
  `nvkvm_fd_lookup` + `session->fds`. Delete the dup() workaround in
  `nvkvm_handle.c`. Migrate every dispatch site to `handle_acquire`.
- `nvidia-uvm` and memfd stay opened in QEMU (separate request types).

### Step 4 — Stub becomes table-free + auto-scaling thread pool (task #33)

- Wire format isolate↔QEMU references stub-local fd numbers, not handle_ids.
- Stub has no handle table. Optional small list: "ranges populated by
  WRITE_MEMORY_HANDLE, cleanup after this ioctl".
- One I/O thread reads commands FIFO; worker pool spawned via clone3, no
  TLS, small stack, auto-scales to ≥1 idle, capped at 256, idle exit
  after N seconds.
- Strict FIFO ack of close before any open whose fd number could collide.

### Step 5 — UFFD-on-KVM-memory PoC (task #34, parallel via sub-agent)

- Standalone test: `tests/poc/uffd_kvm.c`. Registers UFFD-WP on a chunk of
  KVM-backed guest RAM, performs heap→memfd promote sequence under guest
  write pressure. Verifies all guest writes during promote are deferred
  and replayed against the new mapping with no data loss.
- If KVM-UFFD on 6.8 has gaps: switch design to vCPU-pause path. Document
  what we hit.
- Independent of Steps 1–4; can develop in parallel.

### Step 6 — Cross-isolate handle clone (task #35)

- Guest module: per-process map of (handle_id → set of isolate_ids it
  knows about).
- On ioctl issued from a process whose isolate doesn't already have the
  handle: send `NVKVM_REQ_CLONE_HANDLE_TO_ISOLATE` to QEMU. QEMU
  SCM_RIGHTS the qemu_fd to the target isolate, target opens new local
  fd, registers in iso_hnd map. Returns the new local fd to QEMU.
- Double-checked locking around the first-use race.
- This is what makes fork() not break for memfd-backed CPU memory.

### Step 7 — Full vast.ai rebuild + cumemalloc on open driver without dup() (task #36)

- Build everything on the vast.ai node.
- Run `cumemalloc_test` inside the guest under open nvidia.ko.
- Keep NVKVM_* printk instrumentation in the open driver source for this
  run (it's our diagnostic surface).
- PASS = the referencing bug is fixed at the root.

### Step 8 — Cleanup (task #37)

- Move the NVKVM_* open-driver patches to a `debug-instrumentation`
  branch. Revert the open-driver source tree on the vast.ai node.
- Write `docs/ARCHITECTURE.md` documenting tables, invariants, txn
  protocol, fork semantics, cleanup ordering.
- Merge `nvkvm-tables-refactor` → `master`.

---

## 4. Test contract (must hold at every step)

- All four tables: `tests/unit/test_tables.c`. No /dev/nvidia* dependency.
- Existing unit tests pass.
- `test_ioctl_fwd` integration test passes after Step 3 lands (it might
  fail mid-Step 3 while migration is in progress; that's allowed but
  must be green by end of Step 3).
- `cumemalloc_test` inside the guest passes by end of Step 7.

---

## 5. Open commitments / "if you forget anything, remember this"

1. **dup() removed**. There is no dedupe of /dev/nvidiactl opens. If
   `pOSInfo != clientOSInfo` re-appears, it's a real referencing bug —
   don't paper over it.
2. **No fd_token**. Single ID space: `handle_id` only. If you see
   `nvkvm_fd_lookup` or `session->fds`, that's dead code from before the
   refactor.
3. **Stub has no handle table**. It receives `(stub_local_fd, op, args)`
   from QEMU and just runs syscalls. The only state stub keeps is
   threads + the optional WRITE_MEMORY_HANDLE cleanup list.
4. **handle_id is allocated in QEMU before stub knows about it**. Stub
   never invents IDs.
5. **QEMU always holds a qemu_fd via SCM_RIGHTS** for every stub-opened
   fd, EXCEPT during the open transaction window (bounded to one
   sendmsg).
6. **GPA placement is QEMU's choice**, dictated to the guest in the
   mmap response. Guest doesn't pick GPAs.
7. **The data path for CPU memory is mmap, not virtio R/W**.
   WRITE/READ_MEMORY_HANDLE exists only for buffers whose backing can't
   be swapped (vfio MMIO etc).

7b. **memfd-passthrough is the dominant fast path**. If a pointer the
    guest passes into an ioctl falls within an already-memfd-backed GPA
    window, *do not copy anything*. Resolve `vma → (handle_id, offset,
    size)` in the guest module, forward to QEMU, QEMU instructs the
    stub to rewrite the ioctl arg to its own VA for that same memfd.
    This is what every CUDA-allocated buffer hits. The userfaultfd
    path and WRITE_MEMORY_HANDLE are fallbacks for pointers that
    *aren't* memfd-backed yet (or can't be).
8. **Closed driver is not a target**. Open driver only. Validate against
   open driver in Step 7.

---

## 6. ChatGPT-scale goal (the actual point)

Everything in this plan exists to get a 7B+ LLM running in the guest.
Order of operations once the refactor is green:

1. cumemalloc_test passes inside guest, open driver, without dup() — gate.
2. cuLaunchKernel: vec_add test inside guest — gate.
3. Small model (llama.cpp Q4 ≤1 GB, or GPT-2 124M) — gate.
4. 7B LLM (Llama-3.1-7B Q4, Mistral-7B-Q4, similar). RTX 3060 12 GB so
   quantized.

If any of (2)/(3)/(4) reveals more forwarding gaps (channel alloc, GR
engine setup, push buffer mapping), file them as their own tasks and
keep the refactor branch clean.

---

## 6b. Architectural seam, corrected understanding (May 2026)

Step 3 (stub-owned opens) is fully shipped and validated. cuInit works on
the open driver, test_ioctl_fwd 48/48. The next live blocker is
**cuCtxCreate returning CUDA_ERROR_ILLEGAL_STATE (401)**, manifesting as
`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` returning `NV_ERR_NOT_READY (0x40)`.

### Architectural one-liner (canonical)

> We do everything nvproxy does (transparent RM/UVM ioctl passthrough,
> handle tracking, byte-faithful struct marshalling) — PLUS a real
> memory and fd boundary that we mmap across between stub, QEMU and
> the guest.

That's the entire scope. Anything more ambitious than that (parallel
pClient trees, RM resource-graph mirroring) is out of scope and was
wrongly proposed earlier; see "Architectural lesson learned" below.

### The identity translation tables (what each layer holds)

The seam between guest libcuda and the host nvidia kernel is fd /
handle translations and mmap regions. Each layer holds specific things:

**File-descriptor identities:**

| Identity      | Guest libcuda | Guest kmod | QEMU     | Stub | Host kmod |
|---------------|---------------|------------|----------|------|-----------|
| Guest fd      | yes           | yes        | no       | no   | no        |
| handle_id     | no            | yes        | yes      | no   | no        |
| qemu_fd       | no            | no         | yes      | no   | yes       |
| stub_fd       | no            | no         | yes      | yes  | yes       |

**Memory regions:**

| Region   | Guest libcuda | Guest kmod | QEMU       | Stub       | Host kmod |
|----------|---------------|------------|------------|------------|-----------|
| GVA      | yes           | yes        | partially* | partially* | no        |
| GPA      | no            | yes        | yes        | no         | no        |
| QEMU VA  | no            | no         | yes        | no         | yes       |
| Stub VA  | partially*    | partially* | partially* | yes        | yes       |
| HPA      | no            | no         | no         | no         | yes       |

`*` = shared via memfd or via the WRITE/READ_MEMORY_HANDLE path; the
stub VA equals the GVA at a per-mmap level by construction (MAP_FIXED).

**Nvidia RM identities (pClient handles, e.g., `0xc1d025ae`):**

| Layer            | Holds table? |
|------------------|--------------|
| Guest libcuda    | yes (the authoritative view) |
| Guest kmod       | no — pure passthrough |
| QEMU             | no — pure passthrough |
| Stub             | no — pure passthrough |
| Host nvidia kmod | yes (the real driver state) |

**Critical implication**: pClients are NOT something we translate or
own at any of our boundaries. nvidia handles flow byte-for-byte from
libcuda to the host kernel and back. Our forwarding only needs to
get the fd-translation and mmap-region tables right.

### What `clientOSInfo` actually is (the resolved misunderstanding)

`rmclientValidate_IMPL` at `client.c:736` of the open driver is a
**pointer-equality check** between two `nv_file_private_t *` (nvfp)
pointers:

  - `pClient->pOSInfo` — the nvfp at NV01_ROOT_CLIENT alloc time.
  - `pSecInfo->clientOSInfo` — current ioctl's `nvfp->ctl_nvfp`
    (or `nvfp` if no ctl), set by escape.c.

The nvfp is per-`struct file`, not per-task and not per-mm. So:

  - Two threads / fork() / SCM_RIGHTS sharing a struct file → same
    nvfp → strict-validate PASSES across processes / mms.
  - Two independent `open("/dev/nvidiactl")` calls → two struct files
    → two nvfps → strict-validate FAILS only if libcuda then mixes
    them in ops on the same pClient.

**Consequence**: We can run memory-touching RM ops from QEMU on its
SCM_RIGHTS-shared copy of a stub-opened nvidiactl/nvidia0 fd, and
strict-validate will pass. Isolation is preserved via separate mm,
fd table, seccomp, pid namespaces — none of which the driver checks.

The earlier "Path A — parallel client graphs" proposal was over-
engineered. The correct architecture is the simpler **Path B'**:

> Stub opens nvidiactl + nvidia0 (security: minimal attack surface
> for RM control). QEMU receives those fds via SCM_RIGHTS. RM control
> ops run from the stub. Memory-touching RM ops (channel alloc,
> RM_MAP_MEMORY_DMA, etc.) run from QEMU on its SCM_RIGHTS copy.
> Strict validate passes either way — same struct file, same nvfp.

This is a small, per-ioctl routing decision, not a parallel-graph
refactor.

### Where the stack already does the right thing

- `nvkvm_req_mmap_on_isolate` (QEMU) mmaps on its qemu_fd → installs
  a KVM memory region (GPA → QEMU_VA) → tells the stub to MAP_FIXED
  at the same GVA so kernel-side resolves work when libcuda passes
  the VA in a subsequent ioctl. **Live-sync over BAR1 is real**:
  writes from any of guest GVA / guest GPA / QEMU_VA / stub_VA all
  land on the same physical BAR0/BAR1 page. Verified May 2026 via
  /proc/self/maps inside the guest — 2MB USERD/pushbuffer mapping
  lands at `0x200200000` (in the GPA window), doorbell 64KB write-
  only mapping at a high VA, both backed by `/dev/nvidia0`.
- `RM_ALLOC` (NVOS21/NVOS64) propagates the kernel's writes to
  `pAllocParms` back to libcuda (commit 97caf2f closed the gap).
- `UVM_REGISTER_CHANNEL` runs on the stub's pre-opened UVM fd
  (correct mm lineage) and returns nvstatus=0. (Whether the side
  effect — `bIsContextBound = TRUE` — actually took hold needs
  driver-side printk to confirm; the ioctl returning NV_OK is
  necessary but possibly not sufficient.)
- libcuda treats `nv_memory_desc_params.phys_addr` etc. as opaque
  kernel-owned values. It does NOT dereference them; it passes them
  back into subsequent ioctls. The kernel verifies / re-resolves
  internally. So physical-address propagation across our boundary
  is not a layer we need to track. **Scope simplification.**

### Where it still breaks

`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` → `NV_ERR_NOT_READY (0x40)`. The
`kchannelIsSchedulable_HAL` gate passes (UVM_REGISTER_CHANNEL is OK).
The error comes from the *internal* RM→GSP call that programs the
hardware runlist. Several missing pieces of channel state are most
plausible upstream causes, all of which are RM_CONTROL types the
**host issues but the guest does not** (libcuda forks its bring-up
path earlier based on something it read):

  - `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` (`0x20801210`) —
    sets WFI/GFXP/CILP preemption granularity per channel. Without
    this, GPU scheduler may not consider the channel runlist-ready.
  - `NV2080_CTRL_CMD_*` family `0x801909, 0x83de0309` — also missing.
  - `NVA06C_CTRL_CMD_*` family `0xa06c0103, 0xa06c0105` — also missing.
  - `RM_ALLOC` of class `0x83de` (`GT200_DEBUGGER`) — also missing.

These divergences are *downstream* of the upstream divergence. Find
the upstream RM_CONTROL whose response libcuda inspected to decide
to skip the per-channel preemption-mode setup → fix the propagation
of that response value.

### Pre-existing latent issue: multiple nvidiactl opens — experiment results

libcuda opens `/dev/nvidiactl` multiple times within a single process.
Each guest open in the current architecture becomes a fresh
`open("/dev/nvidiactl")` in the stub — distinct struct files,
distinct nvfps.

**Experiment (commit c5cb2bc, on branch `nvkvm-tables-refactor`)**: Added
`nvkvm_handle_dedupe_ctl` so a second nvidiactl open in the same session
returns the same `handle_id` with `guest_refcount++`. Multiple guest
fds → one underlying stub struct file → one nvfp → strict-validate
trivially matches across libcuda's repeated opens.

**Empirical result**: cuCtxCreate failure changed from CUDA_ERROR_
ILLEGAL_STATE (401) to CUDA_ERROR_ALREADY_ACQUIRED (210). RM_MAP_MEMORY
at block 117 of the trace returns NV_ERR_STATE_IN_USE (0x63).

**Root cause of the new failure**: `alloc_free.c:825` returns
NV_ERR_STATE_IN_USE when a "single-instance" resource class (one
allowed under a given parent) is already present. With dedupe,
libcuda's two logical "client trees" collapse into one — when libcuda
tries to add a second instance via what it thinks is a fresh fd, the
kernel says "already there". So **dedupe is over-merging at the kernel
state level**.

**Trade-off** (both currently broken):
- WITHOUT dedupe: 401 ILLEGAL_STATE, 2253 ioctls in trace, libcuda
  retries many times then gives up.
- WITH dedupe: 210 ALREADY_ACQUIRED, 871 ioctls in trace, libcuda
  fails immediately on a specific kernel error.

**Hypothesis for the correct fix**: NEITHER blanket dedupe NOR no
dedupe is right. libcuda's bare-metal pattern works with multiple opens
because each open establishes a fully independent client tree. We need
to look at what specifically goes wrong WITHOUT dedupe — what op
crosses fd boundaries — and either:
  (a) fix that single op (likely an embedded-fd translation bug
      somewhere in our pipeline that we missed); or
  (b) make the dedupe per-class-aware — share the nvfp for "simple
      identity" ops, fresh nvfp when libcuda's pattern requires
      independent state.

NOT_deduped: `/dev/nvidia0..N` (each ALLOC_OS_EVENT needs its own
nvfp), `/dev/nvidia-uvm` (per-mm rule), eventfd (per-event).

### Paranoid-debug root-cause chain (May 2026, end of session)

**Timing falsified** via `tools/diag/ioctl_jitter.so` LD_PRELOAD —
host bare-metal cumemalloc PASSES with 50ms random jitter on every
nvidia ioctl. Our multi-layer roundtrip's latency is irrelevant.

**The actual smoking gun** found via cross-run host stability mask
(3 host runs, mask the per-run-noise bytes via
`tools/diag/stable_host_diff.py`, then compare guest against the
mask):

1. Guest's `/proc/self/maps` has FEWER mapping entries for `libc.so`
   than host. Specifically: host shows a `---p` (PROT_NONE) guard
   page between libc.so's `r-xp` and `r--p` sections (5 mapping
   entries total); guest's libc has no guard (4 entries). Different
   glibc / dynamic-loader version.

2. libcuda PARSES `/proc/self/maps` and SERIALIZES the result into
   the `UVM_INITIALIZE` ioctl's input buffer. Strings visible in
   the strace dump confirm: `/proc/self/maps`, `%zx-` (a printf
   format for parsing the mapping lines), `cuda00001800007`
   (libcuda's section label it's looking for).

3. **At byte offset 136-143 of that buffer**, host writes NULL
   (stable across 3 host runs); guest writes a userspace VA
   (`10 cd b7 c8 ff 7f 00 00` = 0x00007fffc8b7cd10 = a guest VA).
   This is libcuda's decision: a pointer field is set to NULL on
   host but to a specific VA in guest because of the mapping
   layout difference.

4. The kernel's UVM driver processes UVM_INITIALIZE with that
   pointer set vs NULL → downstream state diverges → eventually
   surfaces as `NV_ERR_NOT_READY` at GPFIFO_SCHEDULE →
   `CUDA_ERROR_ILLEGAL_STATE (401)` from cuCtxCreate.

**Fix directions (next session's decision)**:
- **(a) `/proc/self/maps` rewriter LD_PRELOAD**: intercept libcuda's
  open/read of `/proc/self/maps` and synthesize a layout that
  matches the bare-metal host's expectation. Contained, no kernel
  changes. Most likely the right shape of fix.
- **(b) libcuda disassembly**: find which condition in libcuda's
  UVM_INITIALIZE serializer fills the pointer at offset 136-143
  and what it points to. Fix at the source of libcuda's decision.
- **(c) Match guest glibc to host's**: brittle, doesn't generalize.
- **(d) Patch guest libc post-load to insert the PROT_NONE guard**:
  hacky but precise.

(a) is the right starting point. Build a small library that
shims `open()` for `/proc/self/maps`, returns a synthesized file
whose mapping layout matches bare-metal's. Run guest cumemalloc
under it. If cuCtxCreate succeeds, we've confirmed the chain
completely and can productionize the fix.

If (a) doesn't fix it, the root cause is elsewhere — but the
diagnostic methodology (`stable_host_diff.py` against N host
runs) found a real semantic divergence we hadn't seen with any
earlier analysis. The tool is reusable for future bugs.

### §6c Update (2026-05-27): /proc/self/maps hypothesis falsified

Attempt (a) above ran into a complication: **libcuda bypasses libc
for `openat`** (uses raw syscall, so `LD_PRELOAD`-based `open()`
hooks don't intercept it). Built `tools/diag/maps_shim.c` anyway —
works for test programs but doesn't catch libcuda's openat.

In parallel, re-checked the actual nvidia-related entries in
`/proc/self/maps` on host vs guest at each cuInit/cuDeviceGet/
cuCtxCreate stage. Initial run from a self-dump inside the
process suggested guest had an *extra* 4KB `r--s` mapping on
`/dev/nvidiactl` that the host didn't. **This turned out to be
a measurement artifact**: reading /proc/self/maps from a child
process via `/proc/<parent_pid>/maps` reveals the host DOES have
the same 4KB `r--s` nvidiactl mapping. The self-read in dump_nv()
was just stopping short for some kernel-side reason.

**Verified**: at AFTER_CUINIT, both host and guest have:
- `-w-s 64KB /dev/nvidia0` (doorbell)
- `r--s  4KB /dev/nvidiactl` (info page)
Inodes and VAs differ (expected). PROT/FLAGS/OFFSET/SIZE match.

**Conclusion**: `/proc/self/maps` for nvidia mappings is NOT the
divergence source. The byte-136 field in UVM_INITIALIZE's input
buffer must come from something else libcuda touches — possibly
an earlier ioctl response, a sysfs read, or an env var. The
maps-as-cause theory in §6b should be considered dead until
re-validated; the strings `/proc/self/maps` / `%zx-` /
`cuda00001800007` in the buffer might just be format-string
constants libcuda has loaded statically, not evidence that the
file is being parsed in this specific code path.

Direction for next attempt: focus on what other early input
diverges between host and guest. Candidates: sysfs reads,
`/proc/driver/nvidia/params`, env vars (CUDA_*), output of
early CTL ioctls (NV_ESC_REGISTER_FD, NV_ESC_NUMA_INFO).

**Recommendation for next session**: revert dedupe and trace
WITHOUT it with smarter normalization (mask all handles + addresses).
The first ioctl whose semantic content differs between host and
guest in the NO-dedupe run is the smoking gun. The 401 in the
NO-dedupe path goes through ~ 2000 retries; somewhere early one
ioctl is getting wrong data that libcuda then thrashes against.

Use `tools/diag/diff_traces_semantic.py` (added this session). It
masks nvfp/VA noise and surfaces the actual semantic diffs.

Already-surfaced finding to track down next session:
**block 26 RM_ALLOC NV01_DEVICE_0**: paramsSize at offset 32 is 0
in host POST, 0x38=56 in guest POST — the restore at
nvkvm_main.c:881 (`a->alloc_parms_size = orig_nvos64_size`) is
supposed to set this back to libcuda's original 0, but isn't
taking effect. Either `have_nvos64_orig` isn't being set
(check the save block at lines 770-776), or the restore branch
is being skipped due to `ret` not matching, or copy_to_user is
not propagating. A printk in the restore branch is the next move.

If the restore IS taking effect on the guest module side, then
the byte at offset 32 must be getting written by something
downstream (QEMU or stub). But that's unlikely — the response
flow doesn't touch this field after the kernel ioctl returns.

Suspect this is the same root cause as the original 401 — a
field libcuda passed as 0, we overrode for the kernel, but
failed to restore on the response → libcuda reads its own field
back as our value, decides something is wrong, takes a wrong
code path.

### Architectural lesson learned

When the symptom is "kernel returns NV_OK but the side-effect doesn't
land" — first reflex MUST be to instrument the driver (printk the
internal path), NOT to design a refactor. Twice this session we
talked about Path A / parallel client graphs as if it was the
inevitable answer; both times the actual evidence (clientOSInfo is
just nvfp; mmaps DO live-sync; libcuda doesn't deal in phys_addrs)
pointed at a much smaller change. **Trust the trace, instrument the
driver, then design.**

### What's next (concrete sequence)

1. **Instrument the GSP-RPC return path** — not the kernel-side
   internal control. On GSP-firmware GPUs (RTX 3060 with open driver
   is *always* GSP-mode), `kchangrpapiSchedule_IMPL` takes the
   `if (IS_GSP_CLIENT(pGpu))` branch and calls
   `NV_RM_RPC_CONTROL(... NVA06C_CTRL_CMD_GPFIFO_SCHEDULE ...)`. The
   error returns from GSP firmware, propagated back through the RPC
   layer. So the printk must be at the RPC dispatch / return path
   (see `kernel/vgpu/rpc.c` around `NV_RM_RPC_CONTROL_INTERNAL` for
   the call site, and the response handler for GSP messages). A
   `NVKVM_SCHED_DIAG` printk inside the non-GSP branch of
   `kchangrpapiSchedule_IMPL` already exists but doesn't fire on
   RTX 3060. To get the error origin, patch the GSP-RPC path AND log
   the unpacked GSP response struct.
2. With that ground truth, decide which of:
   (a) a missing aux-output propagation (similar to the 97caf2f
       fix, but for a different ioctl);
   (b) the multiple-nvidiactl-open robustness fix (dedupe handle_id
       per (session, dev_id));
   (c) routing a specific RM_CONTROL through QEMU on its SCM_RIGHTS
       fd (per Path B') — but only if the printk shows a concrete
       cross-mm issue.
3. Validate test_ioctl_fwd stays 48/48 after each fix.
4. cumemalloc_test green = gate; then cuLaunchKernel; then small
   model; then 7B.

### The architectural reason (kept for context, now superseded by Path B')

The open driver was designed around the invariant that **one process owns
the file (`opener->mm`), the RM client tree (`pClient->pOSInfo`), and the
calling task's `current->mm` for every ioctl** — all the same. gVisor's
nvproxy preserves this trivially because the sentry IS that process. We
explicitly split:

- **Stub** owns the RM client tree (opens nvidiactl, allocates
  NV01_ROOT_CLIENT, runs RM ioctls).
- **QEMU** owns memory-side state (KVM region install, GPA placement,
  VMM_VA for mmap targets).

The kernel doesn't reject this split outright — most ioctls work. But for
channel scheduling, the GSP-side code paths read kernel-internal state
(per-channel USERD/pushbuffer/notifier physical addresses, runlist
entries) that was produced by the alloc sequence the *stub* ran. The
GPU's MMU then tries to resolve those addresses. If any piece of the
kernel-internal addressing was computed against the stub's `current->mm`
but the GPU actually needs a different mapping, schedule says
"NOT_READY".

`NV_CHANNEL_ALLOC_PARAMS_V570` (`src/abi/nvgpu.h`) carries SIX
`nv_memory_desc_params` substructures — `instance_mem`, `userd_mem`,
`ramfc_mem`, `mthdbuf_mem`, `error_notifier_mem`,
`ecc_error_notifier_mem` — each a physical-address descriptor the kernel
fills in. These are kernel-internal allocations, not user-visible memfds.
We currently forward the channel-alloc params straight to the stub and
the kernel does whatever it does inside the stub's process.

### Two paths forward (decision pending)

**Path A — gVisor-style parallel client graphs.** QEMU opens its own
nvidiactl, allocates its own NV01_ROOT_CLIENT, maintains a handle
translation table (libcuda's logical handle ↔ stub-side handle ↔
qemu-side handle). Memory-touching RM ops run from QEMU's client tree;
RM control runs from stub's. Hard work, multi-day, but the only way to
exactly mirror the contract the open driver was written against. Eats
the strict `rmclientValidate_IMPL` problem cleanly because each pClient
is owned by exactly one process.

**Path B — QEMU as proxy executor for memory-touching ops.** QEMU
opens its own nvidiactl, but uses *the stub's* NV01_ROOT_CLIENT handle
when running memory-touching ioctls. This requires bypassing or
satisfying the strict validate check (e.g., open driver patched to
match clientOSInfo lazily for memory-class allocs). Hacky on
unmodified open driver; only viable if we plan to ship a
debug-instrumented driver as part of the deployment. Rejected.

**Conclusion:** Path A is the right next milestone. Step 3 (open
ownership reversal) is the prerequisite that's now done; the next step
is parallel pClient + handle translation.

### Memory-region invariant (codified)

QEMU is the **authoritative orchestrator**. The stub is a sandboxed
syscall executor that QEMU dispatches to. The stub doesn't make policy
decisions about memory layout. Every ioctl that produces a GPU-visible
memory region or VA-space binding must, by the time it returns,
result in: (a) a kernel-side allocation that the stub's process holds
or that QEMU explicitly tracks, (b) a KVM memory region the guest can
reach for any portion the guest needs direct access to, (c) a record
in one of QEMU's four tables so cleanup is deterministic.

The mechanism for memory-touching RM ops (Path A) is that QEMU runs
the alloc directly through its own pClient and translates handles in
both directions. Mechanism for memory-touching mmaps already exists
(`nvkvm_req_mmap_on_isolate`).

---

## 7. Pointers to existing memory entries

- `event_os_event_paranoid_diag.md` — superseded by rmclient_validate_strict_fix.md
- `event_list_proof.md` — also superseded; root cause was strict-validate, not osUserHandleToKernelPtr
- `rmclient_validate_strict_fix.md` — confirmed 0x23 = NV_ERR_INVALID_CLIENT, the misnomer that misled us for days
- `multi_driver_support.md` — open driver is canonical; closed-driver compat is post-MVP
- `focus_open_driver_then_inference.md` — user's explicit direction: open driver, then inference, then closed driver

---

This document is the canonical plan. If anything in code disagrees with
this doc, the doc wins until updated.
