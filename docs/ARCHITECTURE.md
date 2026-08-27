# nvkvm Architecture

nvkvm forwards CUDA workloads from an **untrusted KVM guest VM** out to the
host's real NVIDIA GPU, WSL2-style, without disabling the host's own GPU use
and without a vendor SR-IOV/vGPU licence — the moat is *multi-tenant isolation
on commodity KVM*. This document records the *current* runtime shape of the
system. For the trust model and the default-deny security surface, read
[`SECURITY_MODEL.md`](SECURITY_MODEL.md) — this file is the data-flow/component
companion to it.

## Status (2026-05-30)

The goal is met: unmodified CUDA runs in the guest on the host GPU.
Empirically green through the forwarder:
- `cuInit` → `cuCtxCreate` → `cuMemAlloc`/HtoD/DtoH → `cuLaunchKernel`;
  `tests/integration/test_ioctl_fwd` 48/48; vector_add; 1024² fp32 matmul.
- **7B LLM inference** (Qwen2.5-7B-Instruct, all layers on GPU) at ~20 tok/s —
  `tests/integration/run_llm_7b.sh` (the headline milestone).
- `nvidia-smi` in the guest; the NVIDIA container toolkit (`docker --gpus`).
- Multiple concurrent CUDA processes in one VM; SIGKILL/exit cleanup; signal-
  interruptible forwarded ioctls (#73); cross-VM/host isolation proven
  (`tests/security/poc_cross_proc_dup.c`).

## Three-tier process model

```
  ┌─────────────────────────────────────────────────────────────────────┐
  │ guest VM (Linux + nvkvm-guest.ko + libcuda)            UNTRUSTED      │
  │   libcuda → /dev/nvidiactl /dev/nvidia0 /dev/nvidia-uvm               │
  │     │ ioctl/mmap                                                       │
  │     ▼                                                                  │
  │   nvkvm-guest.ko ── virtio-nvgpu (TX/RX queues, shm slots) ───┐       │
  └───────────────────────────────────────────────────────────────│──────┘
                                                                   │
  ┌────────────────────────────────────────────────────────────────┴─────┐
  │ host: QEMU process (one per VM)                        TRUSTED VMM      │
  │   virtio_nvgpu device → dispatch nvkvm_req_*                            │
  │   • owns the cross-VM/host policy (default-deny allowlists)             │
  │   • owns KVM (KVM_SET_USER_MEMORY_REGION — kvm->mm==current->mm)        │
  │   • owns the global handle table + UVM lifecycle                        │
  │     │  SOCK_SEQPACKET + SCM_RIGHTS, one socket per isolate              │
  └──────────────────────────────────────────────────────────────│────────┘
                                                                  │ per guest mm
  ┌────────────────────────────────────────────────────────────────┴─────┐
  │ host: stub / "isolate" process (one per guest mm)     SANDBOXED        │
  │   nvkvm_stub → ioctl/mmap on locally-opened nvidia fds                  │
  │   /dev/nvidiactl /dev/nvidia0 /dev/nvidia-uvm (real nvidia driver)      │
  │   VA layout mirrors the guest's userspace VAs so pointer fields in      │
  │   nvidia ioctl structs dereference correctly inside the driver.         │
  └────────────────────────────────────────────────────────────────────────┘
```

The stub is *the* talker to the nvidia kernel driver. QEMU is the trusted
boundary that owns KVM and orchestrates the stub. The guest never talks to
nvidia directly — it talks to a virtio device that looks like nvidia.

**Principal = the address space (`mm`)**, not the tgid: an isolate is keyed on
`current->mm`. nvidia keys access on tgid and a thread group has one mm, so
they are 1:1 for every normal process (see SECURITY_MODEL.md §1).

## Constraint table (verified)

| Operation | mm enforcement | Where it must run |
|-----------|----------------|---------------------|
| `KVM_SET_USER_MEMORY_REGION` | Strict `kvm->mm == current->mm` (`tests/integration/kvm_sparse_test.c`) | QEMU |
| RM ioctls (`nvidiactl`/`nvidia0` NV_ESC_*) | None observed | stub |
| RM mmap on `nvidia0` | None observed | stub (VA mirrors guest) |
| UVM_INITIALIZE | Binds `va_space.mm = current->mm` | one process for the whole UVM lifetime → **QEMU** |
| UVM VA-based ioctls (MAP_EXTERNAL_ALLOCATION, FREE, MIGRATE, REGISTER_*) | base/length interpreted in `va_space.mm` | same process as INITIALIZE → **QEMU** |
| UVM mmap | strict when MM tracking on; binds to `va_space.mm` | **QEMU** |

Hard constraints: KVM regions in QEMU's mm; the **entire UVM lifecycle in one
task** (QEMU). RM allocations run in the stub. The split is reconciled by the
REALIZE_UVM_MAPPING RPC (below) and a handle/fd-translation layer.

## Two KVM invariants (verified)

1. **KVM accepts sparse memory regions** — `kvm_sparse_test.c` maps 8 GiB
   `MAP_NORESERVE`, installs it as a region, and the host demand-faults each
   touched page. So a big sparse GPA window can be installed once and sliced.
2. **`kvm->mm == current->mm` is enforced with `-EIO`** — a `clone(CLONE_FILES)`
   child sharing the kvm_fd but with its own mm is rejected. So the stub cannot
   install KVM regions via a seccomp trap; region installs go through a
   stub→QEMU RPC.

## What QEMU does

- Listens on virtio queues; dispatches `NVKVM_REQ_*` (`virtio_nvgpu.c`).
- **Default-deny gate** on everything reaching the host driver — UVM schema,
  RM-control allowlist, frontend NR allowlist, alloc-class allowlist
  (`nvkvm_ctrl_allowlist.h`, `nvkvm_fe_alloc_allowlist.h`; see SECURITY_MODEL.md
  §2). Denials log `nvkvm: DENY …` and return NV_ERR_NOT_SUPPORTED/EACCES.
- **Global handle table** (`nvkvm_handle.c`): handle_id → the host fd QEMU holds
  (always a copy, via SCM_RIGHTS from the stub-opener). Lifetime = the guest
  struct-file refcount; closed via the CLOSE_HANDLE path. A *separate* per-
  isolate refcount tracks which isolates hold a handle; killing an isolate
  prunes per-isolate refs only and never touches the global table.
- **UVM lifecycle**: opens `/dev/nvidia-uvm`, runs the whole UVM ioctl sequence
  and UVM mmap in QEMU's own process/mm (it binds fd→mm at INITIALIZE). The
  stub does the RM allocations; `REALIZE_UVM_MAPPING` replays the recorded UVM
  state on a QEMU-side fd and installs the mapping.
- **GPA memory**: a single large sparse GPA window is pre-installed as one KVM
  memslot; per-mmap slices are placed with `MAP_FIXED` inside it (no per-mmap
  memslot). `nvkvm_mmap_host.c` / `nvkvm_sparse_gpa_alloc`.
- **Embedded-field translation**: every guest pointer/fd/pid/handle in a
  forwarded ioctl is sanitized or translated before the host driver sees it
  (info-lists, BUILD_VERSION strings, REGISTER_FD/OS_EVENT/MAP_MEMORY fds,
  GET_PIDS) — audit `docs/audits/embedded_field_translation.md`.
- Spawns/kills per-mm isolates; routes `NVKVM_REQ_INTERRUPT` to interrupt an
  in-flight forwarded ioctl (#73).

## What the stub (isolate) does

- One per guest mm, spawned by QEMU. **Sandboxed** (SECURITY_MODEL.md §4):
  freestanding static-PIE (no libc), `pivot_root` into a tmpfs holding only the
  bound `/dev/nvidia*` nodes, `CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS|NEWNS`,
  all caps dropped, `no_new_privs`, and a seccomp allow-list whose `mmap`/
  `mprotect` deny `PROT_EXEC` outright.
- Opens nvidia device nodes itself (so the file's owning mm is the stub's) and
  SCM_RIGHTS a copy *up* to QEMU for the global handle table.
- A reader thread frames commands; a worker pool runs the blocking `ioctl()`s.
  Workers translate embedded `handle_id`→local-fd, wire aux buffers, extract
  NvStatus, and reply with the echoed `txn_id`.
- On `ISOLATE_CMD_INTERRUPT` the reader posts `SIGUSR1` (no `SA_RESTART`) to the
  worker running that txn so its blocking ioctl returns `-EINTR` (#73).
- Verbose per-op tracing is gated behind `NVKVM_DEBUG` (`nvkvm_log.h`, QEMU
  side); the stub keeps only error diagnostics. Set `NVKVM_DEBUG=1` in the QEMU
  environment to re-enable QEMU traces.

## Request/response flow (an RM ioctl)

1. Guest libcuda issues `ioctl(/dev/nvidia0, NV_ESC_RM_CONTROL, &p)`.
2. `nvkvm-guest.ko` sanitizes embedded pointers into shm slots, allocates a
   `txn_id` + inflight record, sends `NVKVM_REQ_IOCTL_ON_ISOLATE` on VQ_TX and
   blocks (interruptibly) on completion.
3. QEMU validates against the allowlists, copies the slot blobs, and hands an
   `ISOLATE_CMD_IOCTL` to the owning isolate's socket.
4. The stub worker runs the real `ioctl()`, writes back params/aux/NvStatus.
5. QEMU's per-isolate reader thread matches the `txn_id`, writes the response
   into the guest's IN buffer, and returns the virtqueue descriptor.
6. The guest copies results back to userspace. A guest signal mid-flight routes
   `NVKVM_REQ_INTERRUPT` (step 3 in reverse) to cut the host ioctl short.

## Remaining work (tracked)

- **#55** — expose the GPA window as a 64-bit PCI BAR instead of squatting on
  fixed GPAs (the current single-window heap works but is not BAR-backed).
- **GET_PID_INFO per-process VRAM** — reads 0 from the stub's pid-ns; needs a
  QEMU init-ns admin-subdevice query (`docs/.../get_pid_info_findings`).
- **CUDA-IPC** export/import-fd control cmds are denied pending fd-translation.
- **M-2 / H-4** — stub aux-writeback tightening; adversarial-teardown reclamp
  (normal + SIGKILL teardown is verified). See SECURITY_MODEL.md §6.
- **HMM-mode UVM** — `UVM_CAN_USE_MMU_NOTIFIERS()` is false on this build; a
  rebuilt nvidia-uvm.ko would unlock UVM-on-any-mmap. Out of scope until needed.

## Files to know

- `src/abi/` — nvidia ABI structs (ioctl param types, status codes).
- `src/common/` — virtio + isolate protocol headers (`nvkvm_proto.h`,
  `nvkvm_isolate_proto.h`).
- `src/guest/` — `nvkvm-guest.ko` (virtio transport, sanitizers, session/mm
  keying, signal-interruptible waits).
- `src/qemu/` — the `virtio-nvgpu` QEMU device: dispatch, handle table, isolate
  manager, UVM realize, allowlists, mmap/GPA window, `nvkvm_log.h` trace gate.
- `src/stub/` — the sandboxed freestanding stub binary.
- `docs/SECURITY_MODEL.md` — trust boundaries + default-deny surface (read this).
- `docs/audits/` — per-surface justification (full ioctl surface, embedded-field
  translation, nvproxy gap analysis, control/frontend allowlists).
- `tests/integration/` — `test_ioctl_fwd.c`, `matmul_test.c`,
  `sig_interrupt_test.c` (#73), `run_llm_7b.sh` (#27), `kvm_sparse_test.c`.
- `tests/security/poc_cross_proc_dup.c` — cross-VM/host dup-denial proof.
- `scripts/run_remote_test.sh` — rebuild/test wrapper for the vast.ai host.

## Reference setup (vast.ai)

- Host: vast.ai instance, RTX 3060 + NVIDIA driver 575.51.03 (open kernel
  modules), recent 6.x kernel; exposes `/dev/kvm`.
- Guest: Ubuntu 24.04 cloud image; module rebuilt against the running kernel.
- 9p tag `nvkvm_src` exposes the repo to the guest. (Large model files must be
  guest-local — a 9p read of a multi-GB GGUF hits EIO.)
