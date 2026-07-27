# uffd across the isolate boundary, across KVM, and the access matrix — measured

**What this file is.** Three questions the candidate study
(`region_lock_mechanism_study.md`) did **not** cover, asked because they can invalidate the
region-lock mechanism choice, and answered by **experiment**. Same authority rule as that file
and `qemu_102_facilities.md`: where this file and a design doc disagree about *what the machine
does*, **this file wins and the design doc gets amended.** It has no authority over what we
*should* do.

**Where it was run.** Measured **2026-07-27**.

| machine | kernel | arch | notes |
|---|---|---|---|
| `vh` (vast.ai bench) | **6.8.0-124-generic** | x86_64 | 21 cores, 49 GB, root, `/dev/kvm`; **itself a KVM guest** |
| dev box | **7.0.0-14-generic** | x86_64 | not used for these runs |

Source read at **Linux `v7.1.0-rc6`** (`research_clones/linux`, HEAD `6f3ed7fec`).
Harness: `isob.c` / `isob2.c` / `isob4.c` (subcommands `q1 q1b q1c q1b2 q1d q2 q2u q2x q2s priv`).
All KVM numbers are **[KVM-direct]** — a raw `/dev/kvm` VM with a 16-bit real-mode guest blob,
one 128 KiB main memslot and one 4 KiB target memslot. **No QEMU number is produced here.**

Tags: **[measured]** observed on the named machine, **[src]** read from the named file at the
named tag, **[inferred]** a conclusion.

> ### ★★ The one-paragraph version
>
> 1. **uffd-WP does NOT cross the isolate boundary.** A registration in process A is a property
>    of **A's `mm`**. Process B writing the same `MAP_SHARED` memfd page is **not trapped, not
>    blocked, and its write lands** — A's handler never fires. **[measured]**
> 2. **uffd-WP DOES trap a guest vCPU write inside a KVM memslot**, blocks it for as long as we
>    like, does **not** return to userspace, and resumes correctly — and the blocked vCPU is
>    **signal-interruptible**. **[measured]**
> 3. **The C artifact never blocked a write at all.** Its defence was **copy-once snapshot**
>    (audit item P2-2) — the honest zero row, `region_lock_mechanism_study.md` row L, shipped
>    and working on real hardware for two years. **[src]**
> 4. **★ New, and it changes the deployment cost:** the *only* uffd route that needs **no admin
>    action** — `UFFD_USER_MODE_ONLY` — **cannot see a guest write.** It turns the guest store
>    into a bare `-EFAULT` from `KVM_RUN`. The region lock genuinely requires a
>    **kernel-fault-capable** uffd, i.e. `CAP_SYS_PTRACE` **or** the sysctl **or** a udev rule.
>    **[measured] + [src]**

---

## 1. Q1 — does uffd-WP cross the isolate boundary? **NO.**

### 1.1 The control first: uffd-WP works fine on `MAP_SHARED` / memfd

The premise "uffd-WP may only work on private anonymous VMAs" is **false on this kernel.**
shmem write-protect landed in 5.19; 6.8 has it.

**[measured, `vh`, 6.8.0-124]** `isob q1c`:

```
UFFD_API features available = 0x1ffff   (PAGEFAULT_FLAG_WP=1, MINOR_SHMEM=1, EVENT_FORK=1)
REGISTER(WP) MAP_PRIVATE|ANON         : rc=0 ioctls=0x17c   arm WP: OK
REGISTER(WP) MAP_SHARED|ANON (shmem)  : rc=0 ioctls=0x17c   arm WP: OK
REGISTER(WP) MAP_SHARED memfd         : rc=0 ioctls=0x17c   arm WP: OK
SAME-PROCESS write to memfd mapping: blocked 300.4 ms, faults=1 flags=0x3 (WP bit=1)
  before=0x0 after=0xaaaabbbbccccdddd  -> TRAPPED AND BLOCKED
```

So the mechanism is real on exactly the VMA type an isolate would share. That makes the
cross-process result below a *boundary* result, not a *feature-support* result.

### 1.2 ★ The result: a write by another process is invisible and unblocked

Process A `mmap`s a 2-page memfd `MAP_SHARED`, registers **both** pages for
`UFFDIO_REGISTER_MODE_WP`, arms them, and runs a fault-handler thread. Process B — forked
**before** the uffd existed, with its **own** `mmap` of the same memfd, so its `mm` is genuinely
separate — then writes. Page 0 is pre-touched in B (PTE present); page 1 is cold (PTE absent),
to rule out "it only escapes because the PTE was already there".

**[measured, `vh`, 6.8.0-124]** `isob q1`:

```
[B] write page0 took 0.001 ms ; page1 (cold) took 0.001 ms
[B] readback page0=0xb0b0b0b0b0b0b0b0 page1=0xc0c0c0c0c0c0c0c0
[A] armed uffd-WP over BOTH pages of the shared memfd in A's mm
[A] before: page0=0x1111111111111111 page1=0x2222222222222222 faults=0
[A] child-done poll after 2000 ms: CHILD FINISHED ITS WRITES (NOT BLOCKED)
[A] after : page0=0xb0b0b0b0b0b0b0b0 page1=0xc0c0c0c0c0c0c0c0
[A] uffd faults seen by A's handler = 0
VERDICT: cross-process write LANDED (lock BYPASSED) ; A saw NO fault
[A] control: A's OWN write -> faults 0->1, 0.249 ms
```

**Three things at once, and all three matter.** B was **not blocked** (1 µs, i.e. an ordinary
store). The write **landed** and is visible in A. A's handler saw **zero** faults. The control
on the last line proves the registration was live the whole time.

> **The concern that prompted this study is confirmed exactly as stated.** uffd-WP registers on
> a **VMA within one `mm`**; the write-protect state is carried in **that `mm`'s** page tables.
> A second process mapping the same shmem pages walks **its own** page tables, which were never
> marked. `[inferred, from the measurement + the PTE-marker design]`

### 1.3 ★★ But this is NOT a discriminator between candidates

`region_lock_mechanism_study.md` §1.4 already records, **[measured]**, that the recommended
alternative — the permanently-read-only region (row B) — has the **same** boundary: *"the
isolate is NOT covered — different `mm`; its writes go through its own page tables and land
unseen."* Confirmed here for the composite case (`isob q2x`), where one memfd page is
simultaneously a KVM memslot, uffd-WP-armed by A, and mapped by an isolate:

**[measured, KVM-direct, `vh`]**

```
armed uffd-WP; memslot page is memfd-shared with the isolate
isolate finished within 2 s: YES (NOT blocked) ; word+8 = 0xfeedface
uffd saw a fault from the isolate write: NO
guest write -> uffd fault: YES ; vcpu done=0 ; word0=0x00000000
after resolve: exit=IO word0=0x12345678
```

**Same page, same instant: the guest is trapped and the isolate is not.**

> **[inferred] So Q1 does not choose between uffd and permanent-RO. It corrects a *claim* both
> designs must make identically:** the region lock covers **the guest**, which is the adversary,
> and does not cover **our own isolates**, which are not writers the lock was ever able to
> exclude. Any sentence of the form "while we hold a region, *nobody* can move the bytes" is
> wrong for both mechanisms and must be narrowed to *"no vCPU can"*.

### 1.4 What *would* cover another process — three routes, all measured

**(a) Register in each process — works, and the WP state is per-`mm`.**
**[measured]** `isob2 q1d`: A and B each create their own uffd and arm their own mapping of the
same memfd page. B's write blocks 300.4 ms in **B's** handler; **A's handler sees 0 faults**
during it; A's own write still blocks in A's handler. So coverage composes, but as *N
independent locks*, not one — the isolate becomes a **participant in the lock protocol**, with
its own handler thread and its own R1/R3 obligations, not a subject of it.

**(b) Passing the uffd over `SCM_RIGHTS` — the receiver operates on the CREATOR's `mm`.**
This is the subtle one and it is worth stating precisely, because the naive reading is dangerous.
**[measured]** `isob2 q1b2`, with B's own mapping placed at `0x300000000000`, an address A
provably does not have:

```
[B] REGISTER B-ONLY anon  0x300000000000 : rc=-1 errno=22 (Invalid argument)
[B] REGISTER B's memfd map 0x7e5e29d7a000 : rc=-1 errno=22 (Invalid argument)
[B] ARM WP on A's address 0x7e5e29db4000 from B : rc=0 (OK)
[B] B's own write took 0.000 ms (B is not covered)
[A] A's write after B armed A's range: 0.307 ms, A-handler faults=1
```

A passed uffd is **fully usable** by the receiver — B successfully armed a range — but every
range it names is resolved in **A's** address space. **[src] `v7.1.0-rc6 fs/userfaultfd.c:2146`**
`ctx->mm = current->mm` is bound at creation, and register/writeprotect operate on `ctx->mm`.

> **★ The trap to avoid:** B registering "its own" address can *appear to succeed* when B's
> address happens to collide with a valid VMA in A — which is exactly what happens when B is a
> `fork()` of A, and it produced a false positive in the first version of this experiment. The
> de-confounded run above is the one to cite. **A passed uffd is a remote-control for A's
> address space, not coverage of B's.**
> **[inferred] The one genuinely useful thing this enables:** the *handler* may live in another
> process. Coverage stays A's; only the policy thread moves.

**(c) `UFFD_FEATURE_EVENT_FORK` — covers a `fork()`ed child, and is destroyed by `execve`.**
**[measured]** `isob4`: with `EVENT_FORK` negotiated, `fork()` delivers `UFFD_EVENT_FORK`
carrying a **new fd for the child's `mm`**, and the child's write blocks 400.4 ms against that
new ctx. **[measured]** `isob4 exec`: if the child then `execve`s and re-maps the same memfd at
the same VA, the write takes **0.0 ms and lands** — *"registration DESTROYED by execve"*.

Two consequences, both operational:

- **[measured] `EVENT_FORK` deadlocks a single-threaded manager.** `fork()` does not return
  until the event is read; if the forking thread *is* the manager, the process hangs in `fork()`.
  A separate manager thread is mandatory. (Cost us one experiment.)
- **[inferred] For kayfabe this route is dead anyway**: isolates are `fexecve`'d, and `execve`
  replaces the `mm`. Only a fork-without-exec child inherits coverage.

### 1.5 The residual honest gap

**Nothing measured here covers GPU DMA**, and `region_lock_mechanism_study.md` §1.4 already
records **[measured]** that it does not: the GPU writes the physical page without walking
anyone's page tables. That boundary is common to every candidate in the table.

---

## 2. Q2 — does uffd interact badly with KVM? **NO — it works, and better than feared.**

The prior worth testing was that post-copy migration proves `MISSING` works but **WP is a
different feature**. It was tested. WP works too.

### 2.1 A guest vCPU write to a uffd-WP'd memslot page is trapped and blocked

**[measured, KVM-direct, `vh`, 6.8.0-124]** `isob q2 500 0` and `isob q2 500 1` — identical
results for an **anonymous** and a **memfd `MAP_SHARED`** memslot backing:

```
warm run rc=0 exit=IO target=0x12345678 (expect 0x12345678)
registered uffd-WP on the memslot page (ioctls=0x17c)
uffd readable after 1000 ms: YES (a fault is pending)
fault: event=18 addr=0x71de709b1000 flags=0x3 (WP=1 WRITE=1)
vcpu thread done=0  (0 => STILL INSIDE KVM_RUN)
backing word during hold = 0x00000000 (0 => write BLOCKED)
after holding 500 ms: done=0 backing=0x00000000
after resolve: KVM_RUN rc=0 errno=0 exit=IO backing=0x12345678  (blocked 513.4 ms)
```

Every part of that is load-bearing:

| observation | consequence |
|---|---|
| a real `UFFD_EVENT_PAGEFAULT` with `FLAG_WP\|FLAG_WRITE` | KVM's stage-2 fault **is** routed through `handle_userfault` |
| backing word unchanged for the whole hold | **it blocks, it does not detect** — §1 requirement 1 of the study is met |
| `done=0` for 500 ms | the vCPU **does not** return to userspace: no `KVM_EXIT_MEMORY_FAULT`, no `-EFAULT`, no exit at all |
| `rc=0 exit=IO` after resolve, correct value | the store is **retried**, not emulated — so no instruction decoder is involved |
| identical on memfd-backed memslot | the shared-backing shape we actually use is not special |

> **★ The retry-not-emulate property is the reason row A survives on arm64 where row B does
> not** (`region_lock_mechanism_study.md` §7.4/§7.5): retry never asks what the instruction was,
> so the ISV=0 problem cannot arise. That remains **[inferred]** — no arm64 host was involved
> here either, and §10 item 3 of the study stays open.

### 2.2 ★ The R1 question: the blocked vCPU **is signal-interruptible**

The concern was "a vCPU blocking inside `KVM_RUN` while we hold a lock has R1 consequences."
It is a real block — but a **cooperative** one.

**[measured, KVM-direct, `vh`]** `isob2 q2s` — vCPU blocked on an unresolved uffd-WP fault, then
sent `SIGUSR1`:

```
vcpu done=0 before signal (0 = blocked in KVM_RUN)
after 5 x SIGUSR1: done=1 sigs_delivered=1 rc=-1 errno=4 exit=INTR
backing=0x00000000
-> KVM_RUN RETURNED on signal (INTERRUPTIBLE)
final: rc=-1 errno=4 exit=INTR backing=0x00000000 blocked 300.2 ms
```

`KVM_RUN` returns `-EINTR` with `exit_reason = KVM_EXIT_INTR`, **and the guest write still has
not landed.** So the standard VMM vCPU-kick works: a hold can always be broken from outside,
the vCPU is never wedged uninterruptibly, and the exclusion is not lost by interrupting it.

**[inferred]** This is materially better than the R1 posture of the permanently-RO alternative,
whose handler runs **on the vCPU thread itself** and must therefore take the region mutex from
inside the guest's own execution context. Here the vCPU is parked in a wait queue and the
handler is elsewhere — which is also why row A pays a cross-thread handoff (the study's §8
measured 244–389 µs trap latency, handler-shape-dependent) that row B does not.

---

## 3. Q3 — what did the C research artifact do? **It never blocked a write. It copied once.**

Searched `/workspace/nvidia-gpu-passthrough` (`src/qemu/`, `src/stub/`, `src/guest/`,
`docs/design/`). Findings, all **[src]** at that repo:

### 3.1 `userfaultfd` in the C artifact: zero implementation

Not one occurrence in `src/`, `tests/`, or `tools/`. Every hit is aspirational prose about the
Rust rewrite — `docs/design/mode2_rust_rewrite_architecture.md:555-556`
(*"ASSUMPTION — verify"*), `docs/design/mode2_rewrite_consistency_audit.md:90` (marked **✖**
missing), `docs/REFACTOR_PLAN.md:303` (refers to a `tests/poc/uffd_kvm.c` that **does not
exist**). **It was planned four times and never built.**

### 3.2 "demand-fault" does not mean uffd — it means an `-EFAULT` retry loop, and it is dead code

The isolate blurb *"memfd migration, double-mmap, demand-fault"* decomposes as:

- **`SIGSEGV` handler, but as a crash reporter** — `src/stub/nvkvm_stub.c:2661-2667` installs
  `SA_SIGINFO`; `:668-686` records `si_addr` into `worker_fault_addr[]` and then calls
  `stub_exit(139)`. **It never returns**, so `resp.fault_addr` (`:1425`, the only producer) is
  **always 0**.
- **The retry loop it fed** — `src/guest/nvkvm_main.c:2003-2018`,
  `NVKVM_MAX_EFAULT_RETRIES 128`, `if (ret != -EFAULT || !fault_addr) break;` — therefore
  **breaks on iteration 0, every time**. Commit `3c23db9` ("security audit round 2: fix R2-H1")
  changed the handler from return-and-retry to exit, to kill an infinite-SIGSEGV DoS; the
  demand-fault trigger was collateral. What actually runs is **eager** migration before the
  ioctl is forwarded.
- **`mprotect` to revoke access: not in the product.** `nvkvm_stub.c:2551` is a seccomp
  allowlist entry with no call site. The only real `mprotect(PROT_NONE)`+`SIGSEGV` trap in the
  repo is a standalone LD_PRELOAD debug tool, `tools/nvtrap.c:118`.
- **KVM write-trapping: never.** `KVM_MEM_READONLY` appears once
  (`src/qemu/nvkvm_mmap_host.c:469`) and **both callers pass `false`** (`:209`, `:560`). No
  memslot revoke/restore. `KVM_EXIT_MMIO`: zero occurrences.
- **memfd seals: never.** `MFD_ALLOW_SEALING` / `F_SEAL_WRITE` / `F_ADD_SEALS` — **zero
  occurrences in the whole repo.** No memfd is ever made unwritable.
- **What "demand-fault" genuinely refers to** is the host kernel's own lazy population of a
  `MAP_NORESERVE` region — `src/qemu/nvkvm_mmap_host.c:136-139`, *"Host kernel demand-faults
  pages on first access by either side"*. Transparent kernel paging, not a trap nvkvm can
  observe.

### 3.3 ★★ How the C actually defended the descriptor — copy-once, named and deliberate

This is the most valuable finding in this study, because it is a **working implementation on
real hardware** of `region_lock_mechanism_study.md`'s row L.

**[src] `src/qemu/virtio_nvgpu.c:626-663`** — audit item **P2-2**, with the reasoning written out:

> *"param_buf/aux_buf point into the guest-shared SHM slot, and this worker runs on the thread
> pool CONCURRENTLY with the guest vCPUs. The handler's allowlist gates … read these bytes, then
> the very same bytes are shipped to the stub — a second vCPU can flip an allowed value to a
> denied one in the window between (a classic double-fetch that defeats the cross-tenant gates).
> **Snapshot the slot into a worker-private buffer ONCE up front** so every gate checks, and the
> stub receives, the SAME bytes."*

The same discipline appears twice more:

- **[src] `src/stub/nvkvm_stub.c:2244-2270`** `ring_exec_one()` — *"the payload was already
  peeked (pointer into the hostile ring); we copy it into private scratch, validate, run …"*.
  This one matters because the SPSC ring is **direct guest↔isolate shared memory with QEMU out
  of the datapath**.
- **[src] `src/common/nvkvm_ring.h:106-111`** — the companion pattern: trust a **private
  snapshot of the geometry**, never the shared control word: *"`cap` is the caller's TRUSTED
  ring capacity (snapshotted at setup), NOT `r->size` — that control word lives in shared memory
  the peer can forge."*

**And there is no lock of any kind between QEMU and an isolate over a shared region.** The ring
header is explicit (`nvkvm_ring.h:5-11`): *"Lock-free SPSC across a shared mmap … No mutex, no
per-record atomics."* The stub's `fs_mutex` (`src/stub/stub_freestanding.h:92-114`) is a futex
over stub-internal thread state, never placed in shared memory.

**What the C did not solve.** Bulk data buffers are unprotected: after
`nvkvm_cpu_pages_migrate_range()` (`src/guest/nvkvm_mmap.c:782`, `:910-941`) `remap_pfn_range`s
the guest VMA onto the memfd, guest userspace and the isolate share the same live pages with no
revocation, no seal, no snapshot. The implicit position — these are the tenant's *own* data
pages, so mutation is self-harm, not a cross-tenant break — **is nowhere written down as an
invariant.** Writing it down is cheap and it is exactly the argument
`region_lock_mechanism_study.md` §3.4 demands each candidate `LockPath` region make.

> **[inferred] The lesson to carry.** The C shipped Mode-1 at host parity, ran 22 real GPU apps,
> and passed two security audits **with copy-once and no lock at all.** That does not prove a
> lock is unnecessary — the C never had to satisfy §2's resolve-then-issue shape — but it does
> mean the burden of proof sits on each region that claims to need one, not on row L.

---

## 4. ★ The access matrix, measured by actually dropping privilege

The briefed cost was *"a udev rule on `/dev/userfaultfd`"*. The refinement offered was that
`userfaultfd(2)` + the sysctl might make it **free** on some distros. Both were tested. The
answer is **more constrained than either**, because of §4.2.

### 4.1 The routes

**[measured, `vh`, 6.8.0-124]** `isob priv`. Baseline: `vm.unprivileged_userfaultfd = 0`,
`/dev/userfaultfd` mode `0600 uid=0 gid=0`. The sysctl was set to `1` for row C and
**restored to `0`**; the device mode was widened to `0666` for row D and **restored to `0600`**.
Both restorations were verified after the run.

| # | route | uid | sysctl | dev mode | result |
|---|---|---|---|---|---|
| A | `userfaultfd(2)` full mode | 0 | 0 | 0600 | **OK** (`CAP_SYS_PTRACE`); WP register on shmem OK |
| A′ | `/dev/userfaultfd` + `USERFAULTFD_IOC_NEW` | 0 | 0 | 0600 | **OK**; WP register on shmem OK |
| B | `userfaultfd(2)` full mode | 65534 | 0 | 0600 | **`EPERM`** |
| B′ | `open("/dev/userfaultfd")` | 65534 | 0 | 0600 | **`EACCES`** |
| **C** | `userfaultfd(2)` full mode | 65534 | **1** | 0600 | **OK**; WP register on shmem OK |
| **D** | `/dev/userfaultfd` + `IOC_NEW` | 65534 | 0 | **0666** | **OK**; WP register on shmem OK |
| E | `userfaultfd(2)` **`UFFD_USER_MODE_ONLY`** | 65534 | 0 | 0600 | **OK — no admin action at all** |
| — | `open("/dev/kvm")` *(control)* | 65534 | — | 0660 root:kvm | **`EACCES`** |

**[src] `v7.1.0-rc6 fs/userfaultfd.c:2167-2182`** explains rows B/C/E exactly:

```c
static inline bool userfaultfd_syscall_allowed(int flags)
{
	/* Userspace-only page faults are always allowed */
	if (flags & UFFD_USER_MODE_ONLY)
		return true;
	if (capable(CAP_SYS_PTRACE))
		return true;
	return sysctl_unprivileged_userfaultfd;
}
```

and **[src] `:2192-2198`** the device path performs **no capability check whatsoever** — row D
is a pure file-permission grant. Both facts reconfirm study §8.1 claim 13 on a third run.

### 4.2 ★★ The finding that decides it: row E cannot see a guest write

Row E is the "free" route. It does not work for this purpose, and the failure is silent in the
sense that registration and arming both **succeed**.

**[measured, KVM-direct, `vh`]** `isob q2u` — identical to §2.1 except the uffd was created with
`UFFD_USER_MODE_ONLY`:

```
warm run rc=0 exit=IO target=0x12345678
uffd created with UFFD_USER_MODE_ONLY: fd=6
registered uffd-WP on the memslot page (ioctls=0x17c)      <- REGISTER SUCCEEDS
uffd readable after 1000 ms: NO                            <- no fault is ever delivered
vcpu thread done=1                                         <- KVM_RUN returned immediately
after resolve: KVM_RUN rc=-1 errno=14 exit=other backing=0x00000000
```

`errno=14` is **`EFAULT`**. And **[measured]** the same `USER_MODE_ONLY` uffd still traps and
blocks an ordinary **same-process** userspace write for the full 300 ms — so the flag is not
broken, it is doing precisely what it says.

**[src] `v7.1.0-rc6 fs/userfaultfd.c:413-414`** — the mechanism:

```c
	if (!(vmf->flags & FAULT_FLAG_USER) && (ctx->flags & UFFD_USER_MODE_ONLY))
		goto out;                       /* -> VM_FAULT_SIGBUS */
```

A guest write is resolved by KVM through `get_user_pages`, which is a **remote** fault, not a
`FAULT_FLAG_USER` one. So `handle_userfault` bails, GUP fails, and KVM turns it into plain
`-EFAULT`.

> **★ This is the same failure mode the study struck row F (`mprotect`) for**, and for the same
> reason: **[src]** `mmu.c:3528` yields bare `-EFAULT`, **not** `KVM_EXIT_MEMORY_FAULT`, and
> **[src]** QEMU `v10.2.0 accel/kvm/kvm-all.c:3212-3233` treats a non-`MEMORY_FAULT` `-EFAULT`
> as *"error: kvm run failed"* and **aborts the VM**. Row E does not merely fail to lock — on
> QEMU it would **kill the guest on the first protected store.**

### 4.3 The corrected deployment statement

> **The region lock requires a *kernel-fault-capable* uffd.** There are exactly three ways to
> get one, and **[measured]** all three yield a ctx that registers WP on shmem and traps guest
> writes: **`CAP_SYS_PTRACE`**, **`vm.unprivileged_userfaultfd=1`**, or **file permissions on
> `/dev/userfaultfd`**. The zero-admin-action route does not exist for this use.

So the honest cost is **"a one-line sysctl **or** a udev rule"** — the refinement is correct and
does widen the options — but **not** "possibly nothing at all", unless the host already ships
the sysctl at `1`.

**[src] It does not, by default.** `v7.1.0-rc6 fs/userfaultfd.c:36` —
`static int sysctl_unprivileged_userfaultfd __read_mostly;` — **no initialiser, so the upstream
kernel default is `0`.** A distro shipping `1` would have to override upstream deliberately.
**[measured]** both hosts available here (`vh` 6.8.0-124 and the dev box 7.0.0-14) report `0`.
**[inferred]** Debian/Fedora/RHEL/Arch as shipped: not checked, no config file cited — do not
quote a number for them.

**And the control line from study §8.1 still stands and is still the strongest argument in row
A's favour:** **[measured]** the same unprivileged uid is denied **`/dev/kvm`** on the same host.
A project that already requires the operator to grant `/dev/kvm` access is not made
qualitatively less deployable by a second device-node grant.

---

## 5. Acceptance table

| # | claim | how established | verdict |
|---|---|---|---|
| 1 | uffd-WP registers and blocks on `MAP_SHARED` memfd + shmem on 6.8 | [measured] `q1c`, 300 ms hold | **settled** |
| 2 | **A uffd-WP registration in A does not trap, block, or observe a write by B to the same shared pages** | [measured] `q1`, both warm and cold PTE | **settled** |
| 3 | The same is true with the page simultaneously a KVM memslot | [measured] `q2x` | **settled** |
| 4 | The permanent-RO alternative has the identical boundary | [measured] `region_lock_mechanism_study.md` §1.4 + `q2x` | **settled — Q1 is not a discriminator** |
| 5 | Per-process registration composes; WP state is per-`mm` | [measured] `q1d` | **settled** |
| 6 | A `SCM_RIGHTS`-passed uffd operates on the **creator's** `mm` | [measured] `q1b2` (de-confounded); [src] `fs/userfaultfd.c:2146` | **settled** |
| 7 | `EVENT_FORK` covers a forked child via a new ctx; `execve` destroys it | [measured] `isob4`, `isob4 exec` | **settled** |
| 8 | `EVENT_FORK` deadlocks a single-threaded manager inside `fork()` | [measured] | **settled** |
| 9 | **uffd-WP traps a guest vCPU write in a KVM memslot and blocks it** | [measured, KVM-direct] anon + memfd, 400–500 ms holds | **settled for x86** |
| 10 | The vCPU stays inside `KVM_RUN`; no userspace exit, no `MEMORY_FAULT` | [measured] `done=0` throughout | **settled** |
| 11 | The store is **retried** and lands correctly on resolve | [measured] `rc=0 exit=IO`, correct value | **settled** |
| 12 | **The blocked vCPU is signal-interruptible** (`-EINTR` / `KVM_EXIT_INTR`), and the write still has not landed | [measured] `q2s` | **settled** |
| 13 | **`UFFD_USER_MODE_ONLY` cannot see a guest write; `KVM_RUN` returns bare `-EFAULT`** | [measured] `q2u`; [src] `fs/userfaultfd.c:413-414` | **settled** |
| 14 | Three routes yield a kernel-fault-capable uffd: `CAP_SYS_PTRACE`, sysctl=1, dev permissions | [measured] `priv` rows A/A′/C/D | **settled** |
| 15 | Upstream kernel default for the sysctl is `0` | [src] `fs/userfaultfd.c:36` (no initialiser); [measured] two hosts | **settled** |
| 16 | **The C artifact never blocked a write; its defence was copy-once (P2-2)** | [src] `virtio_nvgpu.c:626-663`, `nvkvm_stub.c:2244-2270`, `nvkvm_ring.h:106-111` | **settled** |
| 17 | The C's "demand-fault" is an `-EFAULT` retry loop, dead since commit `3c23db9` | [src] `nvkvm_stub.c:668-686`, `nvkvm_main.c:2003-2018` | **settled** |

---

## 6. What this study does **NOT** establish

1. **No QEMU number, and no QEMU behaviour, was produced here.** Every KVM result is
   KVM-direct. In particular, **how QEMU's vCPU loop behaves when a uffd fault parks a vCPU
   under the BQL is untested** — §2.2 shows the kernel-level primitive is interruptible, not
   that QEMU's `kvm_cpu_exec` does the right thing with it. That is the same trap
   `qemu_102_facilities.md` §11.1 exists to prevent, and it is open.
2. **No arm64 host was involved.** §2.1's retry-not-emulate property is the reason row A is
   believed sound on arm64, but *"uffd-WP traps a guest vCPU write on arm64 at all"* remains
   **[inferred]** — study §10 item 3 is unchanged by this file.
3. **`vh` is itself a KVM guest**, so absolute microseconds are inflated (estimated 2–5×, not
   measured). Nothing in this file rests on an absolute latency; every conclusion is a
   yes/no or a `[src]` citation.
4. **The NVIDIA-driver collision is not re-examined.** `region_lock_mechanism_study.md` §1.4
   records **[src]** `ogkm: kernel-open/nvidia-uvm/uvm_hmm.c:577-588` — UVM **rejects any range
   with `userfaultfd_armed(vma)`**. That is a live constraint on *which* regions may be armed
   and it is untouched by anything measured here.
5. **Distro sysctl defaults were not surveyed.** §4.3 cites the upstream kernel default and two
   measured hosts, and nothing more.
