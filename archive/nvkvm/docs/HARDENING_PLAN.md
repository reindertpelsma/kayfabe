# nvkvm Isolate Lockdown + ioctl-Boundary Hardening — LIVING PLAN

> **READ THIS FIRST AFTER ANY CONTEXT COMPACTION.** This is the authoritative,
> live plan for the security-hardening work on branch `security-fixes`. Update
> the status markers as you go. Re-read before each phase.

## Why (the thesis)
The **isolate/stub is the security boundary**. Ioctls executed *in the stub*
inherit the nvidia kernel's per-RM-client / per-fd object isolation — a guest
can only touch objects its own stub created. The ioctl-verification work
(Phases 1–5) **relies on the stub actually being an isolate**. So Phase 0
(lockdown) comes first; if the stub isn't contained, the rest is futile.

The guest is untrusted. QEMU (vmm) is privileged. The stub runs guest-driven
nvidia ioctls. We must contain the stub so a stub RCE cannot escape to the host.

## Status legend: [ ] todo  [~] in progress  [x] done  [!] blocked

---

## Phase 0 — Isolate lockdown (FOUNDATION) — ✅ COMPLETE (0.0–0.5; 0.6 optional polish)
>
> DONE + verified on vast.ai + pushed (commits 1391b23, 759e35c, c92aad6): the stub
> runs rootless (userns 0->euid), in ALL SIX namespaces (user/pid/net/ipc/uts/mnt),
> PID 1 in its pid ns, with an EMPTY read-only tmpfs root (no host FS), zero
> capabilities (bounding+eff+amb), NoNewPrivs=1, seccomp filter mode, fail-closed
> (NVKVM_ISOLATE_NO_HARDEN to disable). single+2/4-concurrent matmul + nvidia-smi all
> pass. 0.6 (two-phase seccomp tighten + built-in self-test) is OPTIONAL polish —
> low marginal value since all privileged setup is in QEMU's child before the stub
> starts. NEXT: Phase 1 (inventory).

Target: a stub that is namespaced, capability-less, seccomp-filtered, and
chrooted into an empty read-only tmpfs — far stronger than today (seccomp was
effectively OFF in the libc test stub: "apply_seccomp defined but not used").

Split of responsibility (SYNTHESIZED — refined from the original split):
- **QEMU's forked child does ALL namespace + mount setup** (it has libc → easy
  uid_map writes, error handling, failure logging), then fexecve's the stub
  already-contained. Sequence in the child, before exec:
  1. `unshare(CLONE_NEWUSER)`; write `/proc/self/uid_map` `0 <vmm_uid> 1`,
     `setgroups=deny`, `/proc/self/gid_map` `0 <vmm_gid> 1` → **ns-root**
     (rootless: full caps inside the userns, unprivileged on host; NO ambient
     caps needed).
  2. `unshare(CLONE_NEWPID|NEWNET|NEWIPC|NEWUTS)`; `fork()` so the stub is PID 1
     in the new pid ns.
  3. Open `/dev` `O_PATH|O_DIRECTORY` dirfd; dup it to a known fd (e.g. 4) so
     the stub uses `openat(4, "nvidiaX", O_RDWR)` after the mount ns.
  4. `unshare(CLONE_NEWNS)`; mount tmpfs RO; `pivot_root`+`chroot`+`chdir` into
     empty root.
  5. `PR_SET_NO_NEW_PRIVS` (early — only blocks gaining privs).
  6. `fexecve(mfd)` → stub.
- **Stub (no-libc, raw syscalls) does only the tail**: `PR_SET_DUMPABLE=0`;
  **drop all caps LATE** (bounding set + capset) — after any privileged setup;
  **two-phase seccomp**: loose during its own init, then a TIGHT steady-state
  filter dropping unshare/mount/pivot_root/ptrace/socket/etc. (main loop needs
  only ~{ioctl,mmap,munmap,sendmsg,recvmsg,futex,close,openat,exit}).
- Devices: stub opens /dev/nvidia* on demand via `openat(dev_dirfd, ...)`.
  (Stub currently opens by path in OPEN_DEVICE; switch to openat(dev_dirfd).)
- **Fail closed**: lack of namespaces/seccomp ⇒ refuse to start unless explicit
  `NVKVM_STUB_NO_{NS,SECCOMP}=1` / QEMU flag.
- **Containment self-test** (debug build): after lockdown, attempt to (a) open a
  path outside the empty root, (b) see other PIDs, (c) open a socket, (d) regain
  a cap — assert each FAILS. Proves the isolate is real.

Sub-steps (each is a deploy+test cycle on vast.ai; test = single matmul + 2/4
concurrent + nvidia-smi must all still pass):

- [x] **0.0 Stub build decision.** DONE — no-libc seccomp-ON stub builds + passes single/2/4-conc/nvidia-smi end-to-end. The committed `security-fixes` stub is
  no-libc (C7). Recent integration testing used the *libc* stub (master-based
  /tmp/nvkvm-slot) with seccomp OFF. FIRST: build the security-fixes no-libc
  stub (`make -C src/stub`), deploy it, confirm single matmul + 2/4 concurrent
  + nvidia-smi pass with seccomp ON. This de-risks the foundation. If no-libc
  build/integration is broken, decide: fix it, or consolidate on libc stub.
  Update deploy scripts to build the chosen stub.
- [x] **0.1 seccomp ON + allowlist audit.** DONE — seccomp called unconditionally; no SIGSYS across full run, allowlist sufficient. Confirm apply_seccomp is actually
  called (not gated off). Run with strace/seccomp-log to enumerate every
  syscall the stub needs (ioctl, mmap, recvmsg/sendmsg, futex, clone3, openat,
  close_range, memfd, etc.) and the new lockdown syscalls (unshare, mount,
  pivot_root, capset, prctl, openat). Widen allowlist as needed. Verify pass.
- [x] **0.2+0.3+0.4(net/ipc/uts) DONE (A1, commit pending).** userns(rootless 0->euid)+net/ipc/uts ns+no_new_privs+dumpable=0+all-caps-dropped, in QEMU child, fail-closed. Verified: CapEff=0, NoNewPrivs=1, user/net/ipc/uts isolated; single/2/4-conc/nvidia-smi pass. REMAINING in 0.4: pid ns (needs fork-for-PID1 + host-pid reporting).
- [ ] **0.2b (superseded label) no_new_privs + dumpable=0 + drop all caps** (stub, after opening
  device handles). Test.
- [ ] **0.3 user namespace** (QEMU child: unshare(CLONE_NEWUSER), write
  uid_map/gid_map mapping 0→vmm_uid, deny setgroups). Test rootless spawn.
- [x] **0.4 pid/net/ipc/uts namespaces** DONE — via clone(CLONE_NEWUSER|NEWPID|NEWNET|NEWIPC|NEWUTS) from QEMU (no double-fork; clone returns the stub's host pid directly; parent writes the rootless single-line uid/gid map gated by a sync pipe). Verified: stub is NSpid 1, all 5 ns isolated, caps=0, seccomp=2. Single/2/4-conc/nvidia-smi pass.
- [ ] **0.4-old pid/net/ipc/uts namespaces** (QEMU: unshare(NEWPID|NEWNET|NEWIPC|
  NEWUTS) then fork so stub is PID 1). Bonus: nvidia host-kernel calls now see
  an empty pid ns → no other-process leakage. Test.
- [x] **0.5 empty mount namespace** DONE — clone adds CLONE_NEWNS; QEMU child captures /dev O_PATH @ fd4, mounts RO tmpfs (mode=000), pivot_root(.,.)+detach into empty root; both spawn paths now fexecve a PRE-OPENED binary fd (path vanishes after pivot). Stub opens devices via openat(dev_dirfd). Verified: all 6 ns isolated, root = empty RO tmpfs, single/2/4-conc/nvidia-smi pass.
- [ ] **0.5-old empty mount namespace** (stub: capture dev O_PATH dirfd; unshare
  NEWNS; mount tmpfs; pivot_root + chroot + chdir; switch OPEN_DEVICE to
  openat(dev_dirfd, ...)). Test.
- [ ] **0.6 fail-closed flags** + final re-test: single + 2/4 concurrent +
  nvidia-smi on the fully-hardened model. Commit milestone.

Relevant files: `src/qemu/nvkvm_isolate.c` (fork/exec spawn ~line 415-520),
`src/stub/nvkvm_stub.c` (main ~line 1700+, apply_seccomp ~1353/1374,
OPEN_DEVICE handler ~1150+), `src/stub/stub_freestanding.h` (syscall macros).
Refs memory: [[isolate_hardening_todo]], [[security_audit_2026_05_28]] (C6),
[[vast_host_setup]].

---

## Phase 1 — Inventory pass  [x] DONE
Classified by execution locus (nvkvm_isolate_handlers.c):

**STUB-executed (kernel per-RM-client scoped → contained by the now-real isolate):**
IOCTL_ON_ISOLATE non-UVM (nvkvm_isolate_ioctl @484/576), OPEN_NVIDIA_HANDLE,
OPEN_MEMORY_HANDLE, CLOSE_HANDLE, COPY_HANDLE_TO_ISOLATE, CLOSE_HANDLE_ON_ISOLATE,
POLL/UNPOLL, CREATE/KILL_ISOLATE, LIST_NVIDIA_DEVICES.

**QEMU-DIRECT (privileged, NOT client-scoped → danger surface):**
- **UVM ioctls** — dev_id==UVM runs `ioctl(h->fd, req->cmd, param_buf)` in QEMU
  (line 420); embedded fds partially translated, pids/VAs/access NOT generically
  validated. ← PRIMARY Phase-3 target.
- **MMAP/MUNMAP** — KVM_SET_USER_MEMORY_REGION + mmap (885/943); req->offset/prot/len.
- **REALIZE_UVM_MAPPING** — already §8a strict-validated. ✓
- **READ_HOST_FILE** — allowlist by file_id (nvkvm_hfile_path), not guest path. ✓
- **READ/WRITE_MEMORY_HANDLE** — by validated handle; bound-check sizes (low risk).

Conclusion: Phase-3 focus = UVM-ioctl field schema + MMAP prot/offset bounds.

## Phase 2 — nvidia-smi process list (guest-synthesized)  [x] CORE DONE
DONE: guest module intercepts NV2080_CTRL_CMD_GPU_GET_PIDS (0x2080018d) and
synthesizes the response from its session table (guest tgids) instead of
forwarding (host RM returns empty due to pid-ns). Verified: nvidia-smi lists the
guest's processes with correct GUEST pids + names (matmul shown). Guest-side per
your preference. FOLLOW-UPS: (a) GET_PID_INFO (0x2080018e) for per-pid memory —
needs per-session GPU-mem tracking (currently 0 MiB); (b) filter sessions that
haven't allocated GPU memory so nvidia-smi doesn't list itself. Neither blocks.

### (original investigation notes)
FINDING (2026-05-29): the process query is NOT NV2080_CTRL_CMD_GPU_GET_PIDS —
0x2080018d/018e never appear in the forwarded inner-cmd log, and nvidia-smi's
process table is empty even DURING a live matmul (148 MiB in use). strace shows
its tail is NV_ESC_RM_CONTROL (0x2a) + RM_FREE (0x29); the inner control cmd is
inside the NVOS54 param so strace can't show it. NEXT DIAGNOSTIC (resume here):
instrument the stub (or QEMU) to dump the inner control cmd + response bytes of
the controls nvidia-smi issues near exit, with a matmul running, to find which
control carries process info and WHY it returns empty. Hypotheses: (a) the RM
ties GPU allocations to the stub's host pid but the query client (nvidia-smi's
own stub = different RM client) isn't authorized to see other clients' pids;
(b) a control we stub/forge returns empty; (c) per-pid info needs a capability
(/dev/nvidia-caps) the stub doesn't open. Then translate stub-host-pid →
guest-tgid (needs guest tgid plumbed per session/isolate — not currently sent).
nvidia-smi enumerates processes via RM ioctl (NV2080_CTRL_CMD_GPU_GET_PIDS /
GET_PID_INFO) — confirmed NOT a /proc scan (strace showed zero /proc/<pid>
opens). Returned PIDs are HOST stub PIDs. Translate stub-host-pid → guest-tgid
in the response using the isolate↔session↔mm/tgid map (H2). Discard/scrub any
process the guest must not see. Same for per-process mem/util. Verify
nvidia-smi shows the guest's own PIDs.

## Phase 3 — Schema table + default-deny  [x] DONE (UVM)
DONE + verified on vast.ai (single + 2-concurrent matmul PASS, DENY=0): the
QEMU-direct UVM handler (nvkvm_isolate_handlers.c) now consults an allowlist
schema (nvkvm_uvm_schema[]) keyed on cmd, with EXACT per-cmd param sizes taken
from our ABI (src/abi/uvm.h, driver 575.51.03 — NOT gVisor's newer layouts;
several differ). DEFAULT-DENY: any UVM cmd absent from the table is refused,
never forwarded into privileged QEMU — this denies UVM_TOOLS_READ/WRITE_
PROCESS_MEMORY (62/63, a cross-process memory peek/poke) and any garbage cmd.
Embedded frontend-fd translation generalized to a schema field (kept to the two
cmds the prior code translated: MM_INITIALIZE@0, REGISTER_GPU_VASPACE@16).
Lessons: (a) min_size must come from OUR ABI, not gVisor (mis-denied REGISTER_GPU
40 vs real 32); (b) MAP_EXTERNAL_ALLOCATION (33) and MAP_DYNAMIC_PARALLELISM (65)
ALSO arrive via the generic path during cuCtxCreate, not only REALIZE — found via
DENY logs. Future refinement: per-field fd/VA validation for MAP_EXTERNAL on this
path (currently forwarded as-is, as before).

### (original)
Per-cmd descriptor table: `cmd → {runs_in, fields:[{off,size,kind}]}`,
kind ∈ {fd, handle, pid, gva_ptr, access_mask, count, opaque}. Validate/
translate per field. **No descriptor ⇒ not forwardable from QEMU** (stub-only).
Move every movable QEMU-direct cmd into the stub. Genuine exceptions (UVM binds
to mm; mmap/KVM-region installs) keep explicit schemas.

## Phase 4 — Access-model simulation  [~] IN PROGRESS

### STATUS 2026-05-29 (pushed 6788fe2):
- DONE: per-VM hClient allowlist + DUP_OBJECT gate (h_client_src must be VM-local);
  matmul green, gate inert for matmul (no guest dup).
- DONE: nvos55 ABI corrected to real 28-byte/7-field layout (575 SDK ground truth);
  gate reads h_client_src@12; stub status@24; abi_parity green.
- FINDING: handles are globally sequential/guessable (PoC: attacker client landed 12
  after victim). TYPE_ALL is the real exposure.
- BLOCKED: grant narrowing. TYPE_PID(QEMU pid) tried + reverted — broke cuCtxCreate
  (=800), proving the dup consumer is the UVM KERNEL-internal client, not QEMU's task.
  Correct fix = TYPE_CLIENT(uvm_kernel_client_handle), which must be DISCOVERED
  (kernel-internal; ~0xc1d00001 but drifts) via an open-driver printk in the dup
  access path (/root/open-gpu-kernel-modules on host). Grant stays TYPE_ALL w/ TODO.
- PoC (tests/security/poc_cross_proc_dup.c) stops at NV_ERR_INVALID_OBJECT_PARENT;
  turnkey exploit needs the attacker to alloc a device/subdevice parent first.

KEY INSIGHT (see memory [[hclient_not_fd_scoped]]): nvidia hClient/handle ids are
a GLOBAL access-gated namespace, NOT fd- or namespace-scoped. The real driver's
default RS_SHARE_TYPE_PID policy contains objects to the owning PID (global pid),
which incidentally isolates containers. Our split-process model breaks that PID
match, so Path-α grants RS_SHARE_TYPE_ALL — removing ALL containment (any host
process/VM can dup a guessed handle). 4-step fix:
  1. PoC: unprivileged host proc dups a live guest handle → prove the hole + get a
     regression oracle. [TODO]
  2. [DONE, matmul-green] QEMU per-VM hClient allowlist: VirtIONvgpu.client_allow[]
     records every hClient this VM's isolates use (RM_ALLOC/CONTROL/FREE/DUP/SHARE
     param[0]); DUP_OBJECT with a foreign h_client_src (offset 16, our 36B nvos55)
     is denied. matmul single+2conc PASS, 0 gate denials (matmul issues no guest
     DUP — its dups are kernel-internal). Allow-path correct by construction;
     allow/deny still need PoC + a CUDA-IPC test to exercise live.
  3. Narrow the Path-α TYPE_ALL grant → minimal consumer (the only layer that stops
     a host process bypassing our stack). [TODO]
  4. Guest-module hClient allowlist + ns-translated pids (intra-VM per-process). [TODO]

### (original)
Guest kernel module enforces guest-local /dev/nvidia* uid/gid/mode ("all" =
all-in-VM, never host-wide). RM-level sharing (RS_ACCESS/share-mask/DUP_OBJECT,
the ownMask machinery from cuctxcreate_800_pinned) brokered by QEMU within one
VM (same session↔mm); cross-VM denied.

## Phase 5 — UVM teardown audit  [ ]
Verify UVM_FREE / UVM_UNMAP_EXTERNAL translate the guest VA correctly and tear
down consistently: (a) VA→mapping translation, (b) KVM memslot removal,
(c) sparse-window anon-backing restore (recycling), (d) UVM kernel state free.
Suspected gap: we have REALIZE (create) but teardown may leak GPA windows.
Refs: [[gpa_window_design]], [[nvos56_fake_success]], [[state_machine_step_e]].

---

## Working rules (user directive)
- Do phases continuously: commit a milestone, immediately continue the next; no
  pause needed between milestones. Only stop when genuinely stuck.
- Tight timeouts on all remote commands; detect hangs fast.
- Test each Phase-0 sub-step (single + 2/4 concurrent + nvidia-smi) before moving on.
- Keep this doc live: update status markers + findings as you go.
