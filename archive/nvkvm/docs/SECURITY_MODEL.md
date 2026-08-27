# nvkvm security model

How nvkvm isolates an **untrusted guest VM** while forwarding real NVIDIA GPU
ioctls to the host driver. This is the moat: WSL2-style GPU forwarding with
*multi-tenant* isolation on commodity KVM. Written 2026-05-29 after the
security-hardening pass; supersedes the security notes in the (older)
`ARCHITECTURE.md`.

## 1. Trust boundaries

```
 guest userspace (libcuda, CUDA apps)         UNTRUSTED
   │  /dev/nvidia* ioctls
 guest kernel module (nvkvm-guest.ko)         UNTRUSTED — owns intra-VM policy
   │  virtio-nvgpu (txn protocol)
 QEMU / virtio-nvgpu device (VMM)             TRUSTED — owns cross-VM/host policy
   │  per-isolate UNIX socket
 isolate "stub" (one per guest mm)            SANDBOXED — holds the real nvidia fds
   │  ioctl()/mmap() on /dev/nvidia*
 host NVIDIA kernel driver + GPU
```

Two distinct policy layers — **do not conflate them**:

- **QEMU = the cross-VM / host boundary.** Its only job: other VMs and host
  processes cannot touch this VM's GPU resources, and one isolate cannot reach
  another's, regardless of what a malicious guest says. Everything a guest can
  ask the host driver to do passes QEMU's allowlists first.
- **The guest kernel module = intra-VM access rights.** Which *guest process*
  may use which GPU handle is emulated entirely by the guest kernel, which owns
  the guest's pids/uids/namespaces/fds. QEMU does **not** police intra-VM access
  (that would break legitimate guest-commanded sharing, e.g. CUDA IPC). A
  compromised guest leaking between *its own* processes is one tenant's problem,
  not a cross-tenant breach.

**Principal = the address space (`mm`)**, not the tgid. An isolate is keyed on
`current->mm`. nvidia keys access on tgid and a thread group has exactly one mm,
so they're 1:1 for every normal process; the only divergence (CLONE_VM without
CLONE_THREAD) is tasks that already share all memory — one security domain — so
folding them into one mm-isolate is correct.

## 2. Default-deny ioctl surface (nvproxy parity, enforced in QEMU)

A guest can only reach host-driver operations on **explicit allowlists**. Four
categories, all default-deny, all in QEMU (the guest module is untrusted so the
gate must be host-side). Anything unlisted → `NV_ERR_NOT_SUPPORTED`/EACCES.

| Surface | Mechanism | Source |
|---|---|---|
| UVM ioctls | `nvkvm_uvm_schema[]` (33 cmds, sized) | isolate_handlers.c |
| RM control cmds | `nvkvm_ctrl_allowlist.h` (130 cmds) + GSP-mask/0x2081 passthrough + 1 MiB cap | #76 |
| Frontend NV_ESC ioctls | `nvkvm_fe_nr_allowlist[]` (24) | #76b |
| RM_ALLOC classes | `nvkvm_alloc_class_allowlist[]` (87) | #76b |

Each allowlist = the gVisor nvproxy 575-ABI set ∪ our empirically-captured
known-good set. Excluded by construction: reg-ops/HWPM/debug/fabric/power
controls, privileged frontend escapes (RM_CONFIG, I2C, SET_NUMA_STATUS), bare
NV01_EVENT/privileged-memory/OS-descriptor alloc classes, and the modeset/cap/
nvswitch device nodes (never registered in the guest → entire surface denied).
Audit: `docs/audits/full_ioctl_surface.md` (every denial justified, no gaps).

## 3. Cross-VM / host containment — the handle namespace

The core guarantee: **a foreign client cannot reach another client's GPU
objects.** nvidia handles are a global, access-gated namespace; a client can
only *resolve* (not just dup) objects it has reach to. Empirically proven:
`tests/security/poc_cross_proc_dup.c` — an unprivileged host neighbour, with its
own client + valid device parent, naming the exact live `(hClientSrc,hObjectSrc)`
of a guest VRAM object, is denied **`NV_ERR_OBJECT_NOT_FOUND` (0x57)** — the dup
fails at `clientGetResourceRef` *before* the share policy is consulted.

Crucially this holds **even under `RS_SHARE_TYPE_ALL`**: when the object is
granted DUP_OBJECT to *all* clients, the neighbour still can't reach it. So
containment is **reach-gating (namespace), not share-type** (#64 flip-test).
This is why the H-2 grant to UVM now uses `TYPE_ALL` with no hardcoded client
(the previous `0xc1d00001` was reboot-fragile and guarded a hole the reach-gate
already closes). Layered on top, defence-in-depth:
- **H-3** per-VM `client_allow[]` — every forwarded `'F'` ioctl's hClient must be
  one this VM allocated (recorded at alloc-time on the fds QEMU holds via
  SCM_RIGHTS — fd-anchored, unspoofable).
- **DUP_OBJECT gate** — source client of any dup vetted against this VM's set.

## 4. Stub (isolate) hardening — the most-privileged component

The stub holds the real nvidia fds, so it's the worst-case if compromised:
- **Freestanding** (no libc) static-PIE; minimal syscall surface.
- **seccomp allowlist** (applied before the main loop): only the ~25 syscalls it
  needs. mmap/mprotect **deny `PROT_EXEC` entirely** (not just W^X — W^X is
  bypassable by mmap-RW-then-mprotect-RX; the stub never needs executable
  runtime memory). Blocks execve/ptrace/fork/prctl/init_module etc.
- **mount-ns sandbox**: `pivot_root` into a tmpfs `/dev` containing *only* the
  bind-mounted nvidia nodes → openat of any host path fails by construction.
- **Namespaces**: CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS|NEWNS — no network,
  no host pid/ipc visibility, rootless.

## 5. Embedded-field translation (no raw guest pointers/pids/fds to the host)

Every guest pointer/pid/fd/handle embedded in a forwarded ioctl is translated or
sanitized so the host driver never derefs a guest VA or trusts a guest pid:
- **Pointers**: info-list family, BUILD_VERSION strings, FIFO channellist,
  SURFACE/GR/BIOS/FB/BUS_GET_INFO — copied into bounded aux + repointed by the
  stub; GET_ID_INFO szName zeroed. (Audit: `docs/audits/embedded_field_translation.md`.)
- **fds**: REGISTER_FD, ALLOC/FREE_OS_EVENT, RM_ALLOC_MEMORY/MAP_MEMORY, UVM
  rm_ctrl_fd — translated through QEMU's per-isolate handle table.
- **pids**: GET_PIDS / GET_PID_INFO — `pid_vnr` in the caller's ns (Docker-on-
  guest aware); QEMU validates queried pids against managed isolates.
- **Memory OOB**: C-1 — every slot access bounded to `[slot, slot+size)`.
- **Handle TOCTOU**: C-2 — the per-txn worker dups the fd under lock so a
  concurrent CLOSE+reuse can't race the in-flight ioctl.

## 6. Known residual (tracked)

- **M-2** — stub blind `param_buf+16` write for any aux ioctl.
- **H-4** — adversarial teardown (malicious guest skips cleanup → reclamp + GPA
  free-list); normal+SIGKILL cleanup is verified.
- **GET_PID_INFO memory value** — per-proc VRAM reads 0 from the stub's pidns;
  fix is to query from QEMU's init-ns admin subdevice (`get_pid_info_findings`).
- **CUDA-IPC** export/import-fd control cmds denied pending fd-translation.

## 7. What's empirically verified
Full CUDA path (cuInit/matmul/vec_add/big_memcpy/launch) + 2-concurrent +
`test_ioctl_fwd` 48/48 + nvidia-smi + container toolkit + llama inference, all
green through the forwarder; cross-VM/host dup denied (poc_cross_proc_dup);
no-leak + SIGKILL-cleanup verified. See the audit docs under `docs/audits/`.
