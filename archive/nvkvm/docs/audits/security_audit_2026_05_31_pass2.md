# nvkvm security audit — 2026-05-31 (pass 2: verify G-fixes + new-bug hunt)

Scope: (1) verify the just-landed fixes for the first 2026-05-31 audit
(commits 2496f1c G-2/G-8, 4093851 G-1/G-3, 82d0ab7 G-5/G-7) are correct and not
bypassable; (2) hunt new bugs in src/{guest,qemu,stub}, emphasis on the
freestanding stub's hand-written futex sync primitives, size/count integer math
at the QEMU/stub boundary, embedded-pointer/fd translation, and memory safety.

Threat model (unchanged): guest fully malicious incl. its kernel module. QEMU +
the per-guest stub are the cross-VM/host trust boundary. Missing *intra-VM*
access checks are NOT findings. Host/cross-guest impact = high; guest-self =
low.

## Verdict on the 6 just-landed fixes

| Fix  | Status | One line |
|------|--------|----------|
| G-1 (NVKMS inner-cmdType allowlist) | **SOUND** | default-deny on inner cmdType, OOB-safe read, no alternate outer-cmd reaches modeset |
| G-2 (IDLE_CHANNELS NVOS30) | **BYPASSABLE (pointer-deref half still live)** | the boundary fix is in `nvkvm_dispatch.c`, which is DEAD CODE; the live IOCTL_ON_ISOLATE path forwards guest p_* pointers verbatim |
| G-3 (drop guest-VA DRM ioctls) | **SOUND** | 0x02/0x0a/0x0d/0x0e removed; no remaining allowed DRM nr carries an unmarshalled guest VA |
| G-5 (nvos21 OS_EVENT data-fd gate) | **SOUND for its LOW scope** (intra-VM only; bypass needs guest to forge its own param_size — self-harm) |
| G-7 (drop EXPORT_TO_DMABUF_FD 0x70) | **SOUND** | removed from the frontend allowlist; unreachable |
| G-8 (stub `max(param_size,_IOC_SIZE)` alloc) | **SOUND** | page-rounding/munmap stay consistent; tail is zero (MAP_ANONYMOUS); kills the OOB-*read* class |

## New / residual findings

| ID  | Sev | Title | Boundary |
|-----|-----|-------|----------|
| P2-1 | HIGH | G-2 fix is on DEAD CODE — IDLE_CHANNELS guest pointer fields still reach the host driver on the live path (kernel derefs guest-controlled VA in the stub mm) | guest + stub |
| P2-2 | MEDIUM | Double-fetch TOCTOU on the SHM param slot: every QEMU isolate-handler allowlist check (NVKMS cmdType, alloc-class, ctrl-cmd, DUP_OBJECT src, hClient) re-reads guest-mutable SHM AFTER the check, before `sock_send_full` copies it to the stub | QEMU |
| P2-3 | LOW | Stub fd-lookup→ioctl TOCTOU (C-2 analog, intra-VM): worker resolves `fd` under `fd_mutex`, then `ioctl(fd,…)` without the lock/dup; reader-thread CLOSE_FD can `close()`+recycle it mid-ioctl | stub |
| P2-4 | LOW | `interrupt_txn` spurious-EINTR race: reads `worker_inflight_txn[i]`, target txn may finish and slot start a new txn before `tgkill` lands → interrupts an unrelated in-flight ioctl | stub |

---

### P2-1 — G-2 fix lands on dead code; live path unchanged (HIGH)

**Location of the fix (dead):** `src/qemu/nvkvm_dispatch.c:353-381`
(`nvkvm_dispatch_ioctl` NVOS30 case) is reached only from
`handle_ioctl` (`src/qemu/virtio_nvgpu.c:359`, declared `static`), which is
**never referenced** in the TX request switch (`virtio_nvgpu.c:665-866` handles
`NVKVM_REQ_IOCTL_ON_ISOLATE` only — there is no synchronous `NVKVM_REQ_IOCTL`
case). `nvkvm_dispatch_ioctl` / `nvkvm_ioctl_expected_param_size` have no other
callers.

**Live path:** `NVKVM_REQ_IOCTL_ON_ISOLATE` → `nvkvm_req_ioctl_on_isolate`
(`nvkvm_isolate_handlers.c:750`) → `nvkvm_isolate_ioctl`
(`nvkvm_isolate.c:1207`, `sock_send_full(sfd, param_buf, param_size)` at
`:1265`) → stub `handle_ioctl_cmd` → `stub_ioctl(fd, cmd, param_buf)`
(`nvkvm_stub.c:1111`). On this path:
- The isolate handler applies the frontend-NR allowlist; `0x41` IDLE_CHANNELS is
  allowed (`nvkvm_fe_alloc_allowlist.h:26`). There is **no IDLE_CHANNELS-specific
  pointer handling anywhere on the live path** (confirmed: grep for
  `idle`/`p_clients`/`p_channels`/`num_channels`/`0x41`-as-NR finds nothing in
  the stub or the isolate handler — only the dead `nvkvm_dispatch.c` and
  unrelated `hClass==0x41` root-client checks).
- The stub's embedded-fd switch (`nvkvm_stub.c:1000-1023`) covers only
  0x4e/0x27/0xce/0xcf/0xc9 — not 0x41 — so it forwards `param_buf` verbatim.

**Consequence:** the ABI struct is now correctly 56 B and G-8 guarantees the
allocation ≥ `_IOC_SIZE`, so the **OOB-read half of G-2 is fixed**. But the
**guest-controlled-pointer-deref half is fully live**: a malicious guest module
sends `IOCTL_ON_ISOLATE(cmd=0xc0384641, param_size=56)` with `num_channels>0`
and attacker-chosen `p_clients/p_devices/p_channels`. QEMU forwards it; the stub
forwards it; the host nvidia kmd walks those NvP64 arrays as user pointers
**in the stub's address space**, dereferencing a guest-controlled 64-bit value
→ stub-memory disclosure into driver bookkeeping or a stub SIGSEGV
(`stub_exit`, self-DoS). The guest sanitizer (`nvkvm_ioctl.c:434-445`, forces
`num_channels=0`) is the ONLY thing preventing this and it runs in the
**untrusted guest** — i.e. no protection under the threat model.

**Why HIGH not CRITICAL:** the deref happens in the per-guest stub mm (no
cross-tenant reach), but the stub holds the cross-VM trust (its fds are the host
boundary) and the primitive is a host-driver pointer-deref of a guest value, so
it is the same severity the first audit assigned G-2.

**Fix:** move the IDLE_CHANNELS pointer overwrite onto the live path — either in
the QEMU isolate handler (mirror the `nvkvm_dispatch.c` logic for nr 0x41: zero
p_* when num_channels==0, else stage into aux with the 64-bit cap) or in the
stub (always force num_channels=0 and zero the three NvP64 slots for `'F'` nr
0x41 before `stub_ioctl`). The cleanest is the stub, since it already rewrites
embedded fields there; the dead `nvkvm_dispatch.c` path should be deleted or
wired.

---

### P2-2 — Double-fetch TOCTOU on the SHM param slot (MEDIUM)

**Location:** all QEMU allowlist/gate checks in `nvkvm_req_ioctl_on_isolate`
read directly from `param_buf`/`aux_buf`, which point into the guest-shared SHM
slot (`virtio_nvgpu.c:755 slot_blob(...)`), e.g. NVKMS cmdType
(`nvkvm_isolate_handlers.c:912`), alloc-class (`:994`), ctrl-cmd (`:1017`),
DUP_OBJECT src client (`:1042`), hClient allowlist (`:1080`). The bytes are then
re-read at `nvkvm_isolate.c:1265 sock_send_full(sfd, param_buf, param_size)`
when actually shipping to the stub. There is no snapshot/copy between check and
ship — both touch live SHM.

**Description:** the IOCTL_ON_ISOLATE worker runs on QEMU's thread pool
(`virtio_nvgpu.c:625`) concurrently with the guest vCPUs. The SHM slot is guest
RAM mapped into QEMU; a second guest vCPU can rewrite the slot between QEMU's
gate read and `sock_send_full`. Classic double-fetch: the guest presents an
allowed value during the check (e.g. NVKMS cmdType=17 REGISTER_SURFACE, or an
allowed alloc class / ctrl cmd / a self-owned `h_client_src`) and flips it to a
denied value (GRANT_PERMISSIONS, a denied class/ctrl cmd, or another VM's
`h_client_src`) before the send. The stub then forwards the denied value to the
host driver.

**Exploit sketch:** spin a vCPU writing `cmdType` (param_buf+0) in a tight loop
alternating 17↔GRANT_PERMISSIONS while another vCPU issues the NVKMS
IOCTL_ON_ISOLATE; on the race win, GRANT/ACQUIRE_PERMISSIONS reaches host NVKMS
(re-opening the exact cross-client display primitive G-1 was meant to close).
The same window defeats the alloc-class, ctrl-cmd, and — most seriously — the
DUP_OBJECT `h_client_src` cross-VM gate (`:1042`) and the hClient allowlist
(`:1080`), which are the cross-tenant defenses. Reliability is the only limiter;
on a multi-vCPU guest a flip-race is readily winnable over many attempts.

**Why MEDIUM (argue up to HIGH):** the DUP_OBJECT/hClient gates are explicitly
the cross-VM boundary, so a reliable win is cross-tenant (HIGH). Marked MEDIUM
pending confirmation that the host driver itself independently rejects a
DUP_OBJECT whose src client this stub never allocated (the per-VM handle table
already prevents naming *another stub's* fd, but DUP_OBJECT names RM client
*handles*, a global access-gated namespace per memory `hclient_not_fd_scoped`).
If the host accepts it, this is HIGH.

**Fix:** copy `param_buf`/`aux_buf` out of SHM into a worker-private buffer once,
run *all* gates against the private copy, and ship the private copy to the stub
(`sock_send_full` the copy, not the SHM). This is the standard single-fetch
remedy and also removes any future check/use divergence. (The dead synchronous
path likewise operated on SHM, so this bug predates the graphics work — but the
thread-pool offload made the concurrent-vCPU race practically exploitable.)

---

### P2-3 — Stub fd-lookup→ioctl TOCTOU (LOW, intra-VM)

**Location:** worker `nvkvm_stub.c:719-722` resolves `fd = handle_lookup(...)`
under `fd_mutex`, releases the lock, then issues `stub_ioctl(fd, …)` at
`:1111` (and the embedded-fd `handle_lookup`s at :788/:868/:971/:1033/:1082 also
run lock-free). The reader thread can process `ISOLATE_CMD_CLOSE_FD` →
`handle_close_fd` → `handle_remove` → `stub_close(fd)` (`:1410-1414`, `:463`)
concurrently, and `ISOLATE_CMD_RECEIVE_FD`/`OPEN_DEVICE` can install a new fd
that recycles the just-closed number.

**Description:** this is the stub-side analog of QEMU's C-2 (fixed there by
dup-under-lock at `nvkvm_isolate_handlers.c:838`). The stub never dups the target
fd, so an in-flight worker ioctl can land on a recycled fd pointing at a
different host object.

**Why LOW:** one stub serves one guest mm — no cross-tenant reach. The guest
fully controls its own close/ioctl ordering, so this is guest-self confusion of
its own isolate. No host/cross-guest impact. Worth a dup-under-lock for
robustness (and to match the QEMU C-2 invariant), but not a boundary break.

---

### P2-4 — `interrupt_txn` spurious-EINTR race (LOW, intra-VM)

**Location:** `nvkvm_stub.c:612-624`. The reader reads
`worker_inflight_txn[i]==target_txn`, reads `worker_tids[i]`, then `tgkill`s
SIGUSR1. The worker clears the txn at `:1112` and may publish a new txn at
`:1110` for the next job before the signal is delivered, so the SIGUSR1 can
interrupt an unrelated in-flight ioctl on that slot.

**Description:** SIGUSR1 makes the in-flight `stub_ioctl` return -EINTR (#73
design). A late/misrouted signal forces a spurious EINTR on a different
operation. `send_full`/`recv_full` already retry on EINTR so framing is safe;
the harm is an ioctl returning EINTR that the guest then surfaces as a spurious
failure of an unrelated op.

**Why LOW:** intra-VM only; only the guest's own txns are affected, and the
guest issued both. No host/cross-guest impact, no memory unsafety.

## Items verified clean

- **futex primitives** (`stub_freestanding.h:92-166`): the 3-state mutex is the
  standard Drepper algorithm (correct ACQUIRE/RELEASE, WAKE only on prev==2).
  The gen-counter condvar samples gen under the lock then `FUTEX_WAIT`s on it,
  so a signal in the unlock→wait window is caught by the gen mismatch (no lost
  wakeup). All `fs_cond_wait` callers loop on a predicate (`dequeue_job`
  `:693-702`). `fs_cond_signal` WAKE(1) paired with each enqueue + the
  predicate-loop means no job is stranded with all workers asleep. `fs_once`
  state machine is correct. No double-unlock, no missing barrier found.
- **G-8 munmap consistency** (`nvkvm_stub.c:1455-1487`): `_IOC_SIZE` capped at
  4096 = page size, so `blob_alloc`'s page-rounded size is unchanged for
  param_size ≤ 4096 and equals the param_size rounding for >4096; every munmap
  site rounds `param_size`, staying consistent. `blob_alloc` is MAP_ANONYMOUS so
  the widened tail is zero — no stale-heap disclosure.
- **slot bounding** (`virtio_nvgpu.c:196-227`): `slot_blob` uses 64-bit math,
  bounds size ≤ slot_size and [slot,slot+size) ≤ shm_size. No overflow.
- **GET_PID_INFO fixup** (`nvkvm_isolate_handlers.c:1119-1207`): count clamped to
  200, per-entry `off+STRIDE ≤ aux_size` check (H-A fix) truncates gpi_count;
  post-forward loop reuses the truncated count. No OOB.
- **stub InfoList / GET_CHANNELLIST / GET_BUILD_VERSION** aux pointer staging
  (`nvkvm_stub.c:834-925`): counts capped (4096 / 512), 64-bit `aux_size`
  bounds, list region ends exactly at aux end. No OOB.
- **G-1 cmdType read** (`nvkvm_isolate_handlers.c:912`): guarded by
  `req->param_size >= 4`; param_buf is slot_blob-bounded; default
  `0xffffffff` → denied. No OOB; no alternate outer cmd reaches modeset (the
  `!= 'F' && != NVKMS && != 'd'` default-deny at `:922-930` catches everything
  else, and the graphics-off gate at `:879` blocks 'd'/'m' entirely on
  compute VMs).
- **handle table bounds** (`nvkvm_stub.c:449-466`): all of lookup/store/remove
  bounds-check `id < MAX_HANDLES` (65536). No OOB index.

## Priorities

1. **P2-1** — wire the IDLE_CHANNELS pointer zeroing onto the LIVE path (stub or
   isolate handler); delete the dead `nvkvm_dispatch.c` synchronous path or mark
   why it is retained. The current G-2 commit gives a false sense of closure.
2. **P2-2** — single-fetch the param/aux slot in the worker (copy-then-check-
   then-ship); this is the cross-VM-relevant residual (DUP_OBJECT/hClient gate
   double-fetch).
3. **P2-3 / P2-4** — defense-in-depth / correctness; dup-under-lock in the stub
   and a txn-generation tag on the interrupt path.
