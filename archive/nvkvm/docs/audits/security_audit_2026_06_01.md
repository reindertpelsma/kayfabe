# nvkvm Security Audit — 2026-06-01 (Synthesis)

Synthesis of adversarially-verified findings. Each underlying finding was independently
confirmed real and reachable by a skeptic before inclusion here. This pass deduplicated
overlapping reports, grouped by escape class, and ranked by severity × threat value
(class-2 VM-escape and class-1 LPE weighted highest).

**Scope reminder / access model:** QEMU enforces ONLY the cross-VM / host boundary; the
guest kernel module emulates ALL intra-VM access rights and is itself attacker-controlled.
So "missing intra-VM access check in QEMU/stub" is by-design and not a finding — but
memory-unsafety / type-confusion / leak defects *in the module's own code* that an
unprivileged guest user can trip are valid class-1 LPE/DoS findings.

---

## Executive Summary

7 distinct issues after dedup (9 raw findings; #1≡#2 ring control-block, #3≡#5 guest aux-extend).

| Severity | Count |
|----------|-------|
| High     | 2     |
| Medium   | 2     |
| Low      | 3     |

No CRITICAL (no demonstrated VM-escape or host code-exec). The two HIGH items are the
"fix-first" priorities; both are reliable memory-corruption / OOB primitives reachable by
default from an unprivileged or kernel-level guest attacker.

### Fix-first (in order)

1. **FF-1 — Stub trusts guest-writable ring control block (`r->size`/`head`/`tail`).**
   OOB read/write inside the per-guest stub address space, armed by default, guest-triggerable.
   Class 3+4. The single most reliable corruption/disclosure primitive in this pass.
   *Fix:* pin `g_req_ring->size`/`g_resp_ring->size = ring_bytes` after mmap and use the
   stub-private `g_ring_bytes` (not `r->size`) as `N` in `peek`/`reserve`; snapshot
   `head`/`tail` once; bound `off+len` against the real mapped data length.

2. **FF-2 — Guest-kernel unbounded `aux_size` on the RM_CONTROL extend path.**
   ~512 KB OOB write/read into the VM-wide shared SHM slot region; corrupts other guest
   processes' in-flight slots (class-1 LPE primitive) or panics the guest kernel (DoS).
   *Fix:* clamp final `aux_size <= nvkvm.slot_size` after every extend (InfoList / FIFO_GET_CHANNELLIST /
   BUILD_VERSION), and defensively reject in `nvkvm_virtio_ioctl_on_isolate` before the slot memcpy.

3. **FF-3 (medium) — REALIZE_UVM_MAPPING leaks its GPA-window extent** (no tracking, no free,
   not reaped at teardown). Unprivileged guest can exhaust the 128 GiB window → VM-wide GPU DoS.

The remaining items (ENTER_LOOP lost-wakeup, stub stdio fd hygiene, guest fd type-confusion,
dmesg param dump) are correctness/hardening with confined intra-VM or post-compromise impact.

---

## Findings Table

| # | Sev | Class | Component | File:line | Title | Fix (short) |
|---|-----|-------|-----------|-----------|-------|-------------|
| FF-1 | High | 3,4 | stub | `nvkvm_ring.h:106,163,179`; `nvkvm_stub.c:1391,1415,2106` | Stub SPSC consumer/producer trusts guest-writable `r->size`/`head`/`tail` → OOB read/write in stub | Use stub-private `g_ring_bytes` as N; pin `r->size`; snapshot head/tail; bound vs real data length |
| FF-2 | High | 1 | guest | `nvkvm_main.c:1454-1489,1413-1452,1501-1529`; `nvkvm_virtio.c:982,1058` | Unbounded `aux_size` on RM_CONTROL extend → ~512 KB OOB write/read into VM-wide SHM slots | Clamp `aux_size <= nvkvm.slot_size` after each extend; reject before slot memcpy |
| FF-3 | Med | 1 | qemu | `nvkvm_isolate_handlers.c:2223,2243-2254` | REALIZE_UVM_MAPPING GPA-window extent leaked (no tracking/free/reap) | Record in `iso_mmap_tbl`, free on kill/error; or add unrealize → `sparse_gpa_free` |
| F-4 | Med→Low | 1 | guest | `nvkvm_ioctl.c:224-241,484-494,504-514`; `nvkvm_main.c:1786-1796` | `fget`+cast `f->private_data` as `nvkvm_fd_ctx*` w/o `f_op` check (type confusion 4-byte read) | Verify `f->f_op == &nvkvm_fops` (or drm/uvm fops) before touching `private_data` |
| F-5 | Med→Low | 1,3 | qemu | `nvkvm_isolate.c:1382-1430,603-618,655-684,912-1016` | ENTER_LOOP pool worker parks on shared sync state reused by KILL+CREATE → lost-wakeup / TX-thread wedge | Per-call wait object for ENTER_LOOP, or kill-rendezvous + identity/alive guard |
| F-6 | Low | 7,8 | stub | `nvkvm_isolate.c:757,767,802,810` | Stub inherits QEMU stdout/stderr (fd 1,2); `closefrom` floor=5 | `dup2(/dev/null)` over fds 1,2 in child before exec |
| F-7 | Low | 1 | guest | `nvkvm_ioctl.c:480-482,495-497`; `nvkvm_main.c:1777-1784,1799-1802` | Unconditional `print_hex_dump(KERN_INFO,...)` of guest ioctl params → dmesg leak / spam | Convert to `pr_debug` or remove |
| F-4 (handle) | Low | 4 | qemu | `nvkvm_handle.c:236-249`; `nvkvm_isolate_handlers.c:868-877` | Lock-free `nvkvm_handle_get` ptr; embedded-fd loop reads `hh->fd` w/o dup-under-lock (residual L-4) | Copy scalars under lock, or dup-under-lock for embedded fds (match C-2 invariant) |

---

## Per-Finding Detail

### FF-1 (HIGH, class 3+4) — Stub SPSC ring trusts guest-writable control block
**Files:** `src/common/nvkvm_ring.h` (`reserve` 106-152, `peek` 163-205), `src/stub/nvkvm_stub.c`
(`handle_setup_ring` 1356-1418/1391/1415, `ring_consumer_loop` 2106-2154, `ring_write_resp` 1940-1972,
`ring_exec_one` 2013-2071), `src/qemu/nvkvm_isolate.c` 1199/1207-1224/1251-1254.

*(Dedup: raw findings #1 and #2 are the same defect — one frames the read side, the other the
read+write sides. Merged here.)*

**Threat path.** QEMU mints a memfd and `MAP_FIXED`s it over the sparse GPA window so the single
pre-installed KVM memslot maps `[gpa, gpa+region)` onto those memfd pages
(`nvkvm_isolate.c:1207-1224`), returning the gpa to the guest. The `struct nvkvm_ring` control
block — `size` (offset 0), `tail`, `head` — sits at the base of each ring and is therefore
fully **guest-writable shared memory**. The same memfd is `MAP_SHARED` by the stub
(`nvkvm_stub.c:1391`). QEMU writes `req->size`/`resp->size = ring_bytes` exactly once at setup
(`nvkvm_isolate.c:1253-1254`) and never re-asserts them.

The stub captures the authoritative geometry in `g_ring_bytes` (`nvkvm_stub.c:1415`) but **never
reads it again** on the hot path. The SPSC helpers instead re-fetch `N = r->size` from the
guest-writable block on every call (`nvkvm_ring_peek` `nvkvm_ring.h:163`; `nvkvm_ring_reserve`
`:106`) and derive the in-region offset as `off = head & (N-1)` / `off = tail & (N-1)`. Every
bounds check (`avail > N`, `len > N`, `off+len > N`; `N-used < rec`) is computed against the same
attacker-supplied `N`/`head`/`tail`, so they are internally self-consistent for any forged
values. The `nvkvm_ring_size_ok()` helper exists (`:90`) but is **not** called in `peek`/`reserve`.

*Exploit (read):* guest sets `req->size = 0x80000000` (power-of-two, never re-checked),
`head = 0x100000`, `tail = head+16`. `avail=16` passes; `off = head & (N-1) = 0x100000`
(1 MiB past the real 64 KiB data region); `off+len <= N` passes. `peek` dereferences `data+off`
out of bounds (`h->len`/`h->type`), and the payload is copied into the resp ring the guest reads
back → **stub-memory disclosure**.

*Exploit (write):* on the resp producer (`ring_write_resp` → `nvkvm_ring_reserve`,
`nvkvm_stub.c:1950`) the guest sets `resp->head == resp->tail` (`used=0`, so `N-used >= rec`
always passes) and an arbitrary `tail`; `off = tail & (N-1)` becomes a guest-chosen offset where
the stub `memcpy`s the response record → **guest-positioned OOB write into stub memory**.

**Reachability.** The ring is set up unconditionally at isolate creation unless
`NVKVM_RING_DISABLE` is set (`nvkvm_isolate.c:872-873`); the consumer is armed by default. A
malicious guest kernel drives `NVKVM_REQ_SETUP_RING`/`NVKVM_REQ_ENTER_LOOP` straight from the
virtqueue (`virtio_nvgpu.c:780-798`), independent of the guest module's `ring_enable` default
(which only governs whether *legit* traffic routes over the ring, not whether the shared mapping +
consumer exist). `ring_exec_one`'s 256/4096-byte param/aux caps do **not** cover this — the OOB is
in the `peek`/`reserve` offset computation that runs *before* those caps.

**Why HIGH not CRITICAL.** The primitive is reliable and armed by default, but it lives entirely
inside the seccomp + ns + caps-dropped per-guest stub (which holds `handle_fds[]`, worker stacks,
`.text`). No cross-VM, cross-stub, or QEMU-side compromise is demonstrated; off-into-unmapped
degrades to SIGSEGV/self-DoS.

**Fix.** Treat the whole control block as hostile. Pass/assert the stub-private `g_ring_bytes`
(snapshotted at setup) as `N` in `peek`/`reserve`; pin `g_req_ring->size = g_resp_ring->size =
ring_bytes` after mmap; snapshot `head`/`tail` into locals once per call; validate `off+len`
against the real mapped data length (not `N`); add an `nvkvm_ring_size_ok(N)` guard. The producer
must not re-read its own `head`/`tail` from guest memory between reserve and commit.

---

### FF-2 (HIGH, class 1) — Unbounded `aux_size` on RM_CONTROL extend → SHM-slot OOB
**Files:** `src/guest/nvkvm_main.c` (InfoList 1454-1489, FIFO_GET_CHANNELLIST 1413-1452,
BUILD_VERSION 1501-1529, forward 1898-1902), `src/guest/nvkvm_virtio.c` (slot memcpy 982,
copy-back 1058, legacy path 487).

*(Dedup: raw findings #3 and #5 are the same defect. #5's class-2 "VM escape" framing was
downgraded by its own verifier — see below.)*

**Threat path.** `nvkvm_ioctl()`'s RM_CONTROL path extends the aux buffer to hold an embedded
info-list inline. For the InfoList family (`NV2080_CTRL_CMD_GR_GET_INFO` etc.,
`nvkvm_ctrl_list_entry_size` up to 8 B/entry), `list_size` is read straight from attacker
inner-params (`*(u32*)aux_buf`, `main.c:1459`) bounded only by `list_size <= 65536` → `list_bytes`
up to 512 KB, and `aux_size = ext = ctrl->params_size + list_bytes` (~576 KB). FIFO_GET_CHANNELLIST
(`num_channels<=4096` → 32 KB) and BUILD_VERSION likewise exceed the slot. The only size guards
bound the **base** params (`param_size<=64KB`, `ctrl->params_size<=64KB @1346`); the **extended**
`aux_size` is never re-checked. `nvkvm_virtio_ioctl_on_isolate` then does
`memcpy(slot_ptr, aux_buf, aux_size)` (`virtio.c:982`) into a single SHM slot of
`state->slot_size` (default 64 KB, can be negotiated as low as 4 KB) with no `aux_size <= slot_size`
guard. `nvkvm_slot_addr()` bounds only the slot index (<256), not `slot*slot_size + aux_size <= shm_size`.

The SHM region is a **VM-wide** 256-slot / 16 MB ioremap with a GLOBAL slot bitmap shared across
all sessions/processes. A 512 KB overflow stomps ~8 adjacent slots that concurrent guest processes
hold for their own in-flight params/aux → **cross-process corruption (class-1 LPE primitive):
attacker A overwrites victim B's forwarded RM params or response buffer**. For a high slot index
the write runs past the 16 MB ioremap entirely → guest-kernel corruption / panic (unprivileged
DoS). The copy-back `memcpy(aux_buf, slot_ptr, aux_size)` (`virtio.c:1058` → `main.c:2201`
`copy_to_user`) is the symmetric OOB **read**, disclosing adjacent slots' contents to the attacker.

**Reachability.** `/dev/nvidiactl` is mode 0666; RM_CONTROL is not privilege-gated. The SPSC ring
path that *would* bound aux (`main.c:442-446` punts when `aux_size > NVKVM_RING_MAX_AUX = 4096`) is
gated by `nvkvm_ring_enable` (default OFF), so the unbounded virtqueue isolate path is the live one.
The existence of `NVKVM_RING_MAX_AUX` on the ring path is itself evidence the equivalent bound is
missing here.

**Why HIGH but not class-2 VM escape.** The buggy `memcpy` runs in the **guest kernel** writing
into the guest's own ioremap'd shared BAR. On the QEMU side the C-1 fix already protects the VMM:
`virtio_nvgpu.c:434` rejects `aux_size > slot_size` before reading, and `slot_blob` re-bounds
`base+size <= shm_size`. So QEMU never over-reads and the guest cannot reach QEMU's private heap —
**no VM escape.** The genuine, reachable impact is class-1: unprivileged-user-triggerable corruption
of another guest process's in-flight GPU control buffers, cross-process disclosure, and guest-kernel
panic. High is justified on the class-1 primitives alone.

**Fix.** Bound the **final** `aux_size` against `nvkvm.slot_size` (the negotiated value, not the
compile-time 64 KB constant) after every extend (InfoList @~1465, FIFO @~1422, BUILD_VERSION @~1509):
`if (ext > nvkvm.slot_size) { kfree; return -EINVAL; }`. Defensively clamp/reject in
`nvkvm_virtio_ioctl_on_isolate` (`virtio.c:976`) before the slot memcpy and copy-back, and apply the
same to the param slot. Replace hard-coded 64 KB constants with `nvkvm.slot_size`.

---

### FF-3 (MEDIUM, class 1) — REALIZE_UVM_MAPPING leaks its GPA-window extent
**Files:** `src/qemu/nvkvm_isolate_handlers.c` (alloc 2223, gpa_base 2261, token 2263, error returns
2243-2254); freers `nvkvm_req_munmap_on_isolate` 1769/1824, `nvkvm_iso_mmap_reap_isolate` 1840-1869;
`src/qemu/virtio_nvgpu.c:118` (`nvkvm_session_destroy`).

**Threat path.** `nvkvm_req_realize_uvm_mapping()` allocates a GPA extent via
`nvkvm_sparse_gpa_alloc(nv, len)` (`:2223`) and returns it in `resp->gpa_base`. Unlike
`nvkvm_req_mmap_on_isolate` (which calls `iso_mmap_alloc` at `:1736`), the realize path **never**
inserts into `iso_mmap_tbl`. The only sparse-GPA freers — `munmap_on_isolate` and the kill-time
reaper `nvkvm_iso_mmap_reap_isolate` — iterate `iso_mmap_tbl` exclusively, so neither ever reclaims
a realize GPA. `nvkvm_session_destroy` closes fds and frees the RM client graph but never touches
sparse GPAs. So realize GPAs survive isolate kill **and** session destroy — leaked for the QEMU
process lifetime. The error returns at `2243-2254` also leak (they early-return after the successful
alloc at 2223 without freeing).

A guest libcuda process driving `NVKVM_UVM_REALIZE_MODE_SEM_POOL` (the implemented mode) in a
create-context / alloc-sem-pool / exit loop accumulates leaked extents (≥1 page each, permanently).
Enough iterations exhaust the 128 GiB window → `nvkvm_sparse_gpa_alloc` returns 0 and every
subsequent mmap/realize fails ENOMEM for **all** GPU users in that VM until QEMU restart.

**Why MEDIUM.** Real, unconditional, unbounded lifecycle leak with attacker-controlled
amplification, and a genuine residual not covered by the #80 H-1/M-E teardown work (which wired
`sparse_gpa_free` only into the `iso_mmap_tbl`-keyed paths). But the blast radius is a confined
single-VM DoS (each VM has its own QEMU + window — consistent with the access model); no escalation,
no memory-unsafety. The window is large relative to per-realize sizes, so exhaustion needs sustained
churn. (The raw finding's class-3 tag is a stretch — the stub is not the exhausted resource — class-1
holds.)

**Fix.** Record the realize mapping in `iso_mmap_tbl` (`kvm_slot = NVKVM_IN_WINDOW_SLOT`, gpa, len,
isolate_id) so the kill reaper frees it; and/or add an explicit unrealize request calling
`nvkvm_sparse_gpa_free`. At minimum free the GPA on the error returns at 2243-2254.

---

### F-4 (MEDIUM→LOW, class 1) — `fget`+cast `f->private_data` without `f_op` type check
**Files:** `src/guest/nvkvm_ioctl.c` (`guest_fd_to_handle_id` 224-241, ALLOC_OS_EVENT 484-494,
FREE_OS_EVENT 504-514), `src/guest/nvkvm_main.c:1786-1796` (NV01_EVENT_OS_EVENT nvos64 `ep->data`).

**Threat path.** Every embedded-fd translation takes a fully attacker-controlled fd out of an ioctl
field (UVM_MM_INITIALIZE.uvm_fd, REGISTER_GPU_VASPACE rm_ctrl_fd, REGISTER_FD, ALLOC/FREE_OS_EVENT.fd,
NV0005.data, etc.), does `fget(fd)`, then reads `((struct nvkvm_fd_ctx*)f->private_data)->handle_id`
(offset 0, 4 bytes) with **no** `f->f_op == &nvkvm_fops` check. Only the NULL case is guarded. For any
fd the caller owns that is not an nvkvm device (pipe, socket, eventfd, renderD128 drm_file…),
`f->private_data` points at a foreign subsystem's struct and its first 4 bytes are reinterpreted as a
handle_id. The in-tree comment at `nvkvm_main.c:1763` even notes the real driver verifies
`f->f_op == nv_frontend_fops` in `osUserHandleToKernelPtr` — the guard is a known upstream invariant
omitted here. An unprivileged process opening world-readable `/dev/nvidia-uvm` (0666) and pointing the
fd field at any fd it holds reaches this; `fget` resolves in the caller's own fd table so the attacker
steers which struct is read.

**Why LOW (downgraded).** It is a 4-byte **read** at offset 0 of a foreign `private_data` (not a write,
not a deref-as-pointer). For typical file types `private_data` is a valid pointer or NULL (NULL guarded),
so no oops and no attacker-chosen address. The 4 bytes are **not** returned to userspace — they become a
forged handle_id forwarded to QEMU, which re-validates handle ownership per-session (H-1, 2026-05-29),
blocking cross-session GPU-object reach. Real UB/robustness defect the upstream driver explicitly guards
against, and a weak info-influence primitive, but low practical escape value. Distinct from prior G-4
(stub-side untyped handle table) and G-5 (nvos21 raw-fd).

**Fix.** After `fget`, before touching `private_data`:
`if (f->f_op != &nvkvm_fops && f->f_op != &nvkvm_drm_fops) { fput(f); return -EBADF; }` at all four sites.

---

### F-5 (MEDIUM→LOW, class 1+3) — ENTER_LOOP pool worker parks on reused shared sync state
**Files:** `src/qemu/nvkvm_isolate.c` (enter_loop wait 1382-1430/1423-1424, reader_signal_sync/exit
603-618, kill 912-1016/956-959/1004, alloc_isolate_slot resets sync_done 655-684/669);
`src/qemu/virtio_nvgpu.c:785-799` (offload).

**Threat path.** `NVKVM_REQ_ENTER_LOOP` is offloaded to QEMU's aio thread pool (unlike all other sync
ops, which run on the serialized TX thread). The worker parks on **isolate-shared** state:
`while (!iso->sync_done) pthread_cond_wait(&iso->sync_cond, &iso->sync_lock)` (`:1423-1424`), predicate
checks only `!iso->sync_done` — no `iso->id`/`iso->alive` re-check. `nvkvm_isolate_kill` (TX thread)
closes the socket; the reader calls `reader_signal_sync` once (single `pthread_cond_signal`, no
rendezvous), then kill `pthread_join`s **only the reader** (`:956-959`), never the parked ENTER_LOOP
worker, and sets `in_use=false`. A following guest CREATE_ISOLATE reuses the slot and resets
`sync_done=false` (`:669`) under a different lock — an unsynchronized data race on `sync_done` and a slot
identity change the parked worker cannot detect.

Two failure modes: (a) the woken-but-not-rescheduled worker re-tests `sync_done` after CREATE reset it →
re-parks forever, stranding a pool thread (~64 strandings exhaust the pool → all virtio/GPU processing
wedges); (b) the stale worker and a NEW TX-thread sync op (SETUP_RING/OPEN_DEVICE) wait on the same
condvar — a single signal wakes the wrong one → TX thread hangs forever and/or the new isolate's
`sync_loop_head`/`sync_error` is returned to the guest (desync). Slot reuse is deterministic
(`NVKVM_ISOLATE_MAX=4096`, guest cycles create/kill). The design invariant at `nvkvm_isolate.c:6`
("non-IOCTL commands serialize via sync_lock+sync_cond, one at a time") is exactly what ENTER_LOOP's pool
offload breaks.

**Why LOW (downgraded from medium).** Genuine concurrency defect (the lost-wakeup can even fire
non-maliciously on a legit idle-timeout + teardown race). But the corruption is entirely confined to one
VM's isolate table inside its own single-tenant QEMU: a hung TX thread / stranded pool threads is a
self-DoS of the guest's own GPU forwarding, and any desync only returns one of the guest's-own isolate's
data to the same guest. No cross-VM / host / other-tenant / unpriv→priv boundary is crossed (the class-1
and class-3 tags are not actually achieved). The guest already fully controls its own VM and can trivially
DoS it. Worth fixing for reliability + durability of a QEMU-thread wedge, hence low not info.

**Fix.** Give ENTER_LOOP a per-call wait object (like the IOCTL `pending` mechanism) instead of the shared
`iso->sync_*` fields; or have kill rendezvous with any in-flight sync waiter before clearing `in_use`
(epoch/generation tag + in-flight drain), and add `iso->alive && iso->id == isolate_id` to the predicate.
Do not reset `sync_done` in `alloc_isolate_slot` while a prior waiter may be parked.

---

### F-6 (LOW, class 7+8) — Stub inherits QEMU's stdout/stderr (fd 1,2)
**Files:** `src/qemu/nvkvm_isolate.c` 757/802 (only STDIN reassigned), 767/810 (`closefrom` floor =
`NVKVM_DEV_DIRFD+1 = 5`); stub uses fd 2 at `nvkvm_stub.c:1018/1097/1180/2451/2464/2501`; seccomp allows
`write` at `nvkvm_stub.c:2302`.

**Threat path.** The child does only `dup2(sv[1], STDIN_FILENO)` (fd 0 = socketpair to QEMU), parks the
stub binary at fd 3 and the /dev O_PATH dirfd at fd 4, then `closefrom(5)`. fds 1 (QEMU stdout) and 2
(QEMU stderr) are never redirected/closed and survive into the exec'd stub — confirmed because the stub
writes to STDERR_FD=2 in non-debug error paths. seccomp allows `write()` unconditionally with no fd-arg
filter. **Reachable only under classes 7/8 (assume already-compromised stub).** A compromised stub can
`write(1/2,...)` attacker-chosen bytes into whatever QEMU's stdout/stderr were inherited from at launch
(host terminal, host log file, supervisor pipe e.g. vast.ai) — a weak, write-only spoofing/data channel
into a host resource the sandbox should not retain. M6 already closed the dangerous fds (KVM fd,
cross-stub sockets, memory-backend fds); this is a narrow residual of M6's `closefrom` floor.

**Why LOW.** Only reachable post-compromise; does not aid the initial compromise. The stub already keeps
fd 0 (socketpair) — a richer intended channel to the trusted VMM — so the marginal capability is small.
Write-only, no read, no KVM/cross-stub reach, no escalation. Worst case is log/terminal spoofing or
feeding a downstream host log parser.

**Fix.** `dup2(/dev/null)` over fds 1 and 2 in the child before exec (or extend the closefrom logic to
cover them). Production should not retain QEMU's stdio.

---

### F-7 (LOW, class 1) — Unconditional `print_hex_dump(KERN_INFO,...)` of guest ioctl params
**Files:** `src/guest/nvkvm_ioctl.c` 480-482 / 495-497 (ALLOC_OS_EVENT pre/post-translation),
`src/guest/nvkvm_main.c` 1777-1784 / 1799-1802 (NV01_EVENT_OS_EVENT nvos64 params + aux).

**Threat path.** Every ALLOC_OS_EVENT and NV01_EVENT_OS_EVENT RM_ALLOC unconditionally dumps the raw
param/aux buffers (guest-controlled bytes, translated fd/handle values) to the kernel ring buffer in the
steady-state forwarding hot path — not gated behind any debug flag/pr_debug/#ifdef. Any unprivileged guest
process allocating an OS event causes its bytes to hit dmesg; with `dmesg_restrict=0` another unprivileged
user can read them (intra-VM cross-process info leak) and it is a log-spam DoS vector.

**Why LOW.** Leaked content is small handle/fd integers (already translated), not bulk secrets. The
cross-user read requires `dmesg_restrict=0`, whereas modern guests (the targeted Ubuntu) default to 1, so
the reliable effect is log spam, not a high-value leak. Does not cross the VM boundary; it is a defect in
the module's own code, so a legitimate minor class-1 hygiene finding. No prior report covers it (distinct
from G-2's truncation bug).

**Fix.** Convert to `pr_debug` or remove; never dump translated handle/fd buffers at KERN_INFO in the
steady-state path.

---

### F-4(handle) (LOW, class 4) — Lock-free `nvkvm_handle_get` embedded-fd read (residual L-4)
**Files:** `src/qemu/nvkvm_handle.c` 236-249 (lock-free return); `src/qemu/nvkvm_isolate_handlers.c`
868-877 (embedded-fd loop reads `hh->fd` at 875, no dup-under-lock; cf. the C-2 fix
`nvkvm_handle_acquire_fd` at 883 which covers only `req->handle_id`); threading
`virtio_nvgpu.c` 764/855/860.

**Threat path.** `nvkvm_handle_get` returns the raw `struct nvkvm_handle*` after dropping the table lock;
callers dereference fields lock-free. `NVKVM_REQ_IOCTL_ON_ISOLATE` runs on QEMU's thread pool
(`virtio_nvgpu.c:855`, concurrent) while `NVKVM_REQ_CLOSE_HANDLE` runs synchronously on the TX thread
(`:764`). A pool worker translating an embedded UVM fd (reads `hh->fd` at `isolate_handlers.c:875`,
memcpy into param_buf) can race a TX-thread CLOSE_HANDLE that close()+recycles a slot/fd → translate
against a recycled fd. This is exactly the documented L-4 residual (security_audit_2026_05_30.md:110-112),
rated LOW.

**Why LOW (residual, not new).** Only UVM_REGISTER_GPU_VASPACE / UVM_MM_INITIALIZE carry embedded fds
(narrow surface); the recycled fd feeds only the kernel's own UVM fd-resolution within the SAME guest's
per-VM fd table (intra-VM, no cross-tenant/host reach); device handles carry `isolate_refcount` so CLOSE
returns -EBUSY while held, narrowing the window to memory handles the kernel would generally reject. The
mmap-path portion of the raw finding is **not** a real race — `nvkvm_req_mmap_on_isolate` is dispatched
synchronously on the TX thread (`virtio_nvgpu.c:860-865`), same thread as CLOSE_HANDLE.

**Fix.** Have `nvkvm_handle_get` copy needed scalars out under the lock (return a small value struct), or
dup-under-lock the embedded-fd fields too, matching the C-2 invariant.

---

## Residuals / Good-News Notes

- **QEMU/VMM host boundary held in this pass.** The two HIGH OOB primitives (FF-1, FF-2) and the
  type-confusion (F-4) all stop at the per-guest stub or the guest kernel; none reaches QEMU's private
  heap or another tenant. FF-2's tempting "OOB into shared MMIO → VM escape" framing was specifically
  disproven: the C-1 fix (`virtio_nvgpu.c:434` + `slot_blob` `base+size <= shm_size`) means QEMU never
  over-reads the overflowed bytes and the guest cannot write past its own 16 MB ioremap into QEMU memory.
- **The C-1 / G-8 / pass2 slot-bounding work on the QEMU/stub side is solid** — it is precisely why FF-2
  is class-1 (guest-internal) rather than class-2. The residual was the guest-kernel *producer* side, not
  the VMM *consumer* side.
- **M6 fd-hygiene is effective** — KVM fd, cross-stub sockets, and memory-backend fds are all closed; only
  the harmless-by-comparison stdio pair (F-6) leaked, and only matters post-compromise.
- **The C-2 handle-fd dup-under-lock invariant holds for the actual ioctl target fd**; the only gap is the
  embedded-fd translation loop (a documented LOW residual), and device handles' `isolate_refcount` already
  blocks close-while-referenced.
- **H-1 per-session handle-ownership validation backstops the guest type-confusion (F-4)** — a forged
  handle_id cannot reach another session's GPU objects, which is why F-4 stays LOW.
- **Teardown (#80) is robust for the iso_mmap path**; FF-3 is the single uncovered allocation type
  (REALIZE_UVM_MAPPING) — a focused fix (wire it into `iso_mmap_tbl` + the reaper) closes the gap without
  reworking the lifecycle.

No CRITICAL findings this pass. Prioritize FF-1 and FF-2 (reliable, default-armed memory corruption),
then FF-3 (DoS lifecycle leak); the remaining four are confined-impact hardening.
