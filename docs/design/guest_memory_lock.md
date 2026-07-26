# The guest-shared-memory region lock — Layer 1, measured

**Status:** design, written against a bench round that landed **2026-07-26**. It is the
residual `l1_os_shell.md` §6.8 deliberately left open — *"the uffd design itself —
registration mode, the fault-handler thread's placement, and its interaction with the
`assert_lock_free` witness — is **not** designed here"* — plus the one shape §6.8.1 fixed in
advance (a plain mutex, and why).

**What this file is.** The design of the mechanism that makes **Layer 1** of the two-layer
trust model (`core_security_threat_model.md` §2.1) real: *while we hold a region, the guest
cannot move the bytes we are deciding on.* It names the syscall, the lock, the thread, the
seam the core sees, the tests, and the deployment requirement.

**What it is not.** It is not the trust model (that is §2.1, and it is normative over this
file, not the other way round). It is not the memslot strategy (`l1_os_shell.md` §6.7). It
is not a replacement for copy-once-then-validate — §2 says so in the strongest terms
available, because the available mistake is to read this document as one.

---

## 0. The one-paragraph version

A per-region **`Mutex`** serialises our threads against each other; while a thread holds it,
the region's pages are write-protected with **`UFFDIO_WRITEPROTECT`** on **our own window
VMA**, so a guest vCPU write *does not land* — it parks until we release. Passthrough (the
unarmed state) is a **pure opportunistic optimisation**: the trap path is first-class, runs
as a selectable mode, and the transitions between the two are what the tests attack. The lock
is affordable **only where the guest's write rate is low**, which is why it is confined to a
declared class of pages and **refuses** — loudly — everywhere else. Two agents are outside
its reach entirely and always will be: **the isolate** (different `mm`) and the **GPU**
(DMA does not walk our page tables).

---

## 1. Ground truth — everything this design is built on

Measured on the bench (2026-07-26), kernel **6.8.0-124**, unless tagged otherwise. Per §0.2's
discipline: **[measured]** = observed here, **[src]** = read from the kernel or `ogkm`,
**[inferred]** = a conclusion, marked so it can be attacked.

### 1.1 The mechanism works, and it was tested in the hard order

**[measured]** `UFFDIO_API` on this kernel reports all **17** features, including
`WP_HUGETLBFS_SHMEM`, `WP_UNPOPULATED` and `WP_ASYNC`. `UFFDIO_WRITEPROTECT` works on a
**populated `memfd` / `MAP_SHARED`** mapping — which is exactly what a window backing is
(`l1_os_shell.md` §4.4.1: the VM must be launched with a shareable backing anyway).

**[measured] ★ A guest vCPU write IS trapped**, verified in the order that could have failed:
the guest **writes first** — establishing a writable SPTE — and only *then* is the range
protected, so `mmu_notifier` is genuinely exercised rather than bypassed by a cold shadow
entry. KVM zaps the SPTE, the vCPU parks in our handler, **the write does not land**, and the
guest resumes correctly afterwards.

> **[inferred] The load-bearing consequence, stated because the whole design rests on it:**
> arming is **coherent against a vCPU that is already running**. There is no window in which a
> store issued before our `UFFDIO_WRITEPROTECT` returns can land after it — the SPTE zap plus
> its remote TLB flush is the barrier. So "arm, then read" needs no additional handshake with
> the guest, and *that* is what makes a 2.23 µs lock cycle a lock rather than a hope.

### 1.2 ★★ The privilege fact — `/dev/userfaultfd`, and why it changes the verdict

**[measured]** The trap fires only for a **full-mode** uffd. `UFFD_USER_MODE_ONLY` does
**not** trap guest writes, and the reason is structural rather than a quirk: **KVM's page walk
is a kernel-mode fault.** An unprivileged caller of the `userfaultfd(2)` *syscall* gets
`EPERM` for full mode by default.

**[measured] The fix is `/dev/userfaultfd`** (Linux 6.1+) plus a udev rule: an unprivileged
uid opening the device gets a **full-mode fd**, while the syscall form still returns `EPERM`
for the same uid. Verified both halves.

> **★ This is the fact that decides whether the feature exists at all.** Without it the
> requirement is `CAP_SYS_PTRACE` — root-equivalent, and **fatal to the unprivileged-host
> premise** this project is built on (`core_security_threat_model.md`: *an unprivileged host
> process*). With it, the requirement is a device node and a udev rule — **the same posture we
> already require for `/dev/kvm`**, and therefore not a new class of ask.

**[measured] It fails closed, and that is not as comforting as it sounds.** With a
`USER_MODE_ONLY` fd the guest write does not land *and* `KVM_RUN` returns `-EFAULT` — no
silent bypass, but the vCPU run loop is destroyed and the symptom will present as an
unrelated bug three layers away. Hence GL9 (§7.2): **detect it at startup, refuse loudly.**

### 1.3 ★★ Two hard boundaries — both measured, both permanent

| boundary | why | evidence |
|---|---|---|
| **The isolate is NOT covered** | different `mm`; its writes go through its own page tables and land unseen | **[measured]** |
| …and arming *its* side is forbidden anyway | NVIDIA's driver **rejects any range with `userfaultfd_armed(vma)`** | **[src]** `ogkm: kernel-open/nvidia-uvm/uvm_hmm.c:577-588` |
| **GPU DMA is NOT covered** | the GPU writes the physical page without walking our page tables | **[measured]** — same physical page written, no fault, write landed |

These are not gaps to be closed later. They are the **shape** of the mechanism, and §3 turns
them into a refusal rather than a caveat.

**[inferred] A third boundary follows from the mapping type**, and it is worth naming because
it looks like a tuning question and is not: uffd-WP applies to ordinary kernel-page backings
(anonymous / `memfd` / tmpfs). A **device mapping** — anything whose PTEs came from a driver's
own `mmap` handler — is not lockable, for the same reason it is not cacheable-by-request
(`../reference/memory_cacheability.md` §1, decider 1). Every lockable region is therefore, by
construction, a slice of a window we ourselves reserved and placed.

**★ And one collision we do not have, because the architecture already paid for it.**
`userfaultfd_armed()` on our window would be a live hazard if the *main process* also drove
UVM. It does not: nvidia descriptors never leave the isolate sandbox (`l1_os_shell.md` §3.5),
so the two facts in the table above cannot meet in one address space. This is a case of an
isolation decision made for a security reason turning out to be load-bearing for a
mechanical one — recorded rather than re-derived the next time someone proposes to open
`/dev/nvidia*` in the main process.

### 1.4 The costs — and the number that decides is a RATE

**[measured]** p50 unless stated.

| operation | cost |
|---|---|
| protect / unprotect, 1 page | **1.11 µs** |
| protect / unprotect, 4096 pages | **105 µs** (**≈25 ns/page** marginal) |
| TLB shootdown with 16 contending threads | **+0.6–0.9 µs, flat** — a constant, *not* a multiplier |
| uncontended full lock cycle (acquire → arm → disarm → release) | **2.23 µs** |
| trapped guest-write round trip | **24.8 µs** p50 / **63.6 µs** p99 — of which only ~6–8 µs is handler work; the rest is scheduler wakeup |
| `KVM_MEM_READONLY` sub-slot trap (the alternative) | **70.2 µs** — uffd-WP is **2.8× faster** |

Two readings, and the second is the one that matters.

- **Per-op, the lock is cheap.** 2.23 µs uncontended; the shootdown does not scale with
  contending threads, which is the result that could most easily have killed the design.
  **It also independently confirms `l1_os_shell.md` §6.7 item 5 on cost**, having been decided
  there purely on correctness (I-NOAMP): the struck alternative is also the slower one.
- **★ Per-workload, the rate decides, and it decides differently for different pages.**

| page class | **[measured]** guest write rate | cost if *every* access faults |
|---|---|---|
| GSP command queue | ~200–3000 /s | **0.5–7.4 % of one core** — comfortably affordable |
| page-table pages | ~18,000 /s | **~45 % of one core, plus vCPU stalls** — not affordable |

> **GL2 (normative). The lock/no-lock decision is made on the guest's WRITE RATE to the
> region, never on the per-operation cost of the lock.** A 2.23 µs primitive is cheap; a
> 2.23 µs primitive on an 18 kHz page is a design error, and it is the *same* primitive.

**★ And the 18 kHz row reproduces a known C failure from a new direction.** The C's C1
anti-pattern — the 0x110094 poll vmexit storm — was found as a *performance* bug and
diagnosed as a correctness-of-shape one (`C: mode2_execfwd_layer2`). Here the same page class
arrives at the same verdict from a pure cost calculation, with no storm and no bisect. Two
independent derivations of "do not put a trap on the page-table path" is the strongest form
this rule has ever had.

**⚠️ Caveat, inherited from `l1_os_shell.md` §6.7's own.** The bench host is itself a KVM
guest; absolute microsecond figures are inflated by nested virtualisation (**estimated 2–5×,
not measured**) and should be read as an upper bound. The **ratios** — uffd vs RO-memslot,
flat-vs-scaling shootdown, the marginal per-page cost — are the trustworthy part, and every
decision below rests on a ratio.

### 1.5 ★ A correction to our own cost model: a flags-only memslot flip is 1.49 µs

**[measured]** A memslot **flags-only** update is **1.49 µs** — *not* the 230–460 µs of a
DELETE/MOVE. **[inferred]** KVM takes a fast path that skips the invalidate-swap and the SRCU
grace periods; the mechanism is not kprobe-confirmed the way the two-swap DELETE claim is
(`l1_os_shell.md` §6.7), so it is tagged as inference from the timing.

This is corrected in `l1_os_shell.md` §6.7 as well as here, because **the error runs in the
direction that wrongly discourages a legitimate design**: it made the RO-memslot fallback
(§7.3) look two orders of magnitude more expensive than it is, and a fallback that is priced
out is a fallback nobody builds. Nothing about §6.7's *rule* changes — the rule is about
install/remove frequency and it stands.

### 1.6 Context: there is no C-era wall here, and R1 is retired

**The C never implemented uffd at all** — its refactor-plan PoC was written and never run.
So nothing below is fighting a known failure; this is new ground, and the risk register
(`C: mode2_rust_rewrite_architecture.md` R1 — *"if KVM-UFFD on 6.8 has gaps, switch to a
vCPU-pause path"*) is **retired empirically** by §1.1 rather than mitigated.

**What the C shipped instead was copy-once — and its known hole is exactly what this fills.**
The C's guest-side mmap path requires a manual writeback after every ioctl, and **nothing
detects a guest write to the original page** (`C: src/guest/nvkvm_mmap.c:429-436`, `:617-624`).
That is a TOCTOU window with a person standing in it, which is precisely what
`core_security_threat_model.md` §2.1 says Layer 1 exists to make *not exist*.

---

## 2. ★★ The layering — copy-once is primary, and the lock is the exception

The available misreading of this document is "we now have a lock, so use the lock". Stated
against that, in order of precedence:

> **GL1 (normative). Copy-once-then-validate is the PRIMARY layer and stays primary.**
> `l1_os_shell.md` §4.2.2's read split is unchanged: a **single value we decide on** →
> atomic/volatile, always; a **bulk payload** → copy once, validate the copy, never re-read
> the source. **The region lock is taken only where a decision needs the bytes to be stable
> across a read-then-act that copy-once cannot make atomic** — i.e. where we must read *and
> then act on the host* while the guest is forbidden to move the ground under us.

**Why copy-once is not merely "first", but sufficient nearly everywhere.** §4.2.2 already
establishes that a torn or hostile read introduces **no input the parser must not already
survive**, and that the dangerous consequence — the double fetch — is excluded *structurally*
by `MappedRegion` exposing no borrow. So for a payload we validate after copying, the lock
adds nothing: we were never going to re-read the source, and the copy is memory the guest
cannot touch.

**What the lock adds, precisely.** Not stability of *our copy* — we already have that. It is
stability of the **relationship between the copy and the world we then act on**: when we read
a descriptor, resolve it, and then issue a host operation derived from it, copy-once alone
lets the guest rewrite the descriptor in between and leaves us acting on behalf of a request
that no longer exists. Layer 1 removes that interval.

**And the third thing that follows, which is a limit rather than a feature:** the lock still
does not tell us the bytes are *finished*. That is Layer 2's job and no lock can do it
(`core_security_threat_model.md` §2.1, the "perfectly-locked wrong answer"). **A region hold
is taken at a Layer-2 edge, not instead of one.**

---

## 3. ★★ The page taxonomy is a RULE with a refusal, not a performance heuristic

### 3.1 The four classes

| class | pages | mechanism | lockable? |
|---|---|---|---|
| **Lock path** | GSP **command** queue (guest→GSP), instance blocks, and any dynamic faked-register page whose multi-word content we read as a unit | per-region `Mutex` + uffd-WP | **yes** — this is the only class that is |
| **Volatile / atomic** | completion semaphores, USERD, PTIMER — written by the **host or the GPU** | aligned atomics, `Relaxed`, with explicit fences at the two seams (`l1_os_shell.md` §4.3) | **no** — uffd is *blind* to these writers |
| **Copy-once / commit-point** | page-table pages, userspace pushbuffers | snapshot + validate; captured at the protocol's own edge (doorbell, CE release semaphore, TLB invalidate) | **no** — rate (§1.4) and doctrine (§2) both refuse |
| **Isolate-shared** | guest-RAM slices exported to isolates | an ownership protocol between us and the isolate | **no** — different `mm`, and UVM forbids arming (§1.3) |

### 3.2 The refusal is the point

> **GL3 (normative). Every lockable region declares its `PageClass` when it is REGISTERED,
> and `RegionLock::acquire` on a region that is not `PageClass::LockPath` is a LOUD FAULT —
> never a silent no-op, never a best-effort lock.**
>
> **GL4 (normative). Isolate-shared ranges and DMA-target ranges are unlockable by
> construction.** They are refused at registration, not at acquire, so the error arrives
> when someone declares the wrong thing rather than when someone relies on it.

The reason is one sentence and it is the whole argument: **a lock that appears held but
protects nothing is worse than no lock**, because the code above it is written as if the
guarantee exists. A no-op `lock_region` on an isolate-shared page would produce a system
that is *provably* correct in the mock, *observably* correct on the bench most of the time,
and silently wrong under exactly the concurrency it was built for.

This is `testing_doctrine.md` §6 rule 1 — *prefer a mechanism to a sentence* — applied to a
taxonomy: `PageClass` is an enum in the type system, so the classification exists in the
source rather than in this table.

### 3.3 ★ Three places I think the taxonomy as handed to me is wrong

Recorded here rather than silently implemented, because a miscategorisation is the most
expensive error available in this design.

1. **"GSP command/message queue" is two things, and only one of them is lock-path.** The
   **command** queue (guest→GSP) is guest-written and we read it — lock path, agreed. The
   **message/status** queue (GSP→guest) is written by **us** and read by the guest: we are
   the sole writer, so there is nothing to exclude. Its correctness comes from the transport's
   own publication discipline (write the payload, then publish the write pointer with a
   release), which `l1_os_shell.md` §6.5 already relies on for the completion tail. Locking it
   would put an arm/disarm pair on the **completion hot path** and buy exactly nothing.
   **Split the row.**
2. **"Dynamic faked registers" mostly needs OBSERVATION, not EXCLUSION — and we already have
   a mechanism for observation.** A page whose guest writes we merely need to *see* is served
   by the existing write-trap (`Vmm::map_read_native`, the RAM-backed-reads + write-subrange
   trap, or an RO window per §6.7 item 4). uffd is 2.8× faster (§1.4) but costs a privilege
   grant. The lock is warranted **only** for register-block pages we read as a coherent
   multi-word unit while the guest may be mid-write. The distinction to carry:
   **observation ≠ exclusion**, and only exclusion needs this document.
3. **"Instance blocks" is an open question, not a settled row.** Whether an instance block
   reaches us as a guest-RAM write or through the emulated **BAR2 aperture** decides which
   mechanism even applies — a BAR2 window write is trapped by the VMM's own path, not by
   uffd. The C has the relevant prior (`C: docs/design/mode2_bar2_mmu.md`).
   **OPEN QUESTION (bench experiment):** which aperture carries instance-block writes in the
   Mode-2 path we are building. Until it is answered the row is a *candidate*, and GL3 makes
   the wrong answer a refusal rather than a silent no-op.

The other three classes I believe are right, and two of them are right for reasons stronger
than classification: the volatile class is excluded by *who writes it* (§1.3), and the
pushbuffer is excluded by something better than a lock — **the host GPU executes from our
copy, not from the guest's buffer**, so a post-doorbell rewrite by the guest reaches nothing.

---

## 4. The lock protocol

### 4.1 The sequence

> **GL5 (normative).** **acquire mutex → arm → act → disarm → release**, in that order, on
> **one thread** — the thread already serving the guest's request (`l1_os_shell.md` §6.6:
> there is no memory-plane thread and no memory-plane queue) — with **every other lock of
> ours dropped**.

| step | what happens | failure disposition |
|---|---|---|
| 1. acquire | `assert_lock_free()`, then lock the region's `Mutex`; the watermark rises to the region rank (§4.2) | class refusal (GL3) is checked here and is a fault |
| 2. arm | `UFFDIO_WRITEPROTECT(mode=WP)` over the region's page range | **[src]** `ENOENT` if the range is not registered → **loud fault, never proceed to step 3** |
| 3. act | read the bytes (copy-once, per GL1), decide, issue the host op | ordinary refusal paths; the guard is `#[must_use]` and unwinds |
| 4. disarm | `UFFDIO_WRITEPROTECT(mode=0)` over the same range | a failed disarm is a fault: the region would stay armed and every guest write would take the trap path forever |
| 5. release | drop the guard → drop the mutex → pending faults resolve | — |

**Step 2's failure mode is the one to design against, not step 3's.** An arm that silently
did nothing is the GL3 hazard arriving through the back door, and there is a concrete way to
produce it — §4.4.

### 4.2 Where the region lock sits in the rank order — and the one R1 exception

`l1_concurrency.md` §3.3 declares device = 0, proc = 1, leaf = 2, and **R1 forbids a blocking
call under any lock**. Arming *is* a syscall under the region mutex, so this must be resolved
explicitly rather than by silence.

> **GL6 (normative). The region mutex is RANK 3 and is acquired only from a lock-free
> context.** `acquire` calls `assert_lock_free()` **first** (so no device/proc/leaf lock may
> be held), then raises the watermark to 3. Beneath it, **exactly two syscalls are legal** —
> `wp_arm_under_lock` and `wp_disarm_under_lock` — and nothing else, ever.

This uses the escape hatch `l1_os_shell.md` §4.5 already built rather than inventing one:
`*_under_lock` variants naming the rank they permit, enumerable by `grep -rn '_under_lock'`.
That grep is the property we want — *not* "there are no in-lock syscalls", but "there are
exactly these, and each was argued". After this change the set is: `raise_irq`'s descriptor
write, and these two.

**The argument for permitting them, stated so it can be attacked.** They are bounded
(1.11 µs for a page, ~25 ns/page marginal, shootdown flat in thread count — §1.4) and they do
not wait on a *peer* — unlike an RM verb, which queues on an uninterruptible foreign
`down_write` for up to 6 s (`../reference/rm_semantics_measured.md` §§1–2). **The honest
residual:** `UFFDIO_WRITEPROTECT` takes `mmap_lock` (read), and §6.7 measured `mmap_lock` as
genuinely contended in our process by every placement (+32 VMAs per CUDA process). So the arm
is bounded by whatever holds `mmap_lock` for **write** — a concurrent `MAP_FIXED` in the same
`mm`. That is per-`mm` and therefore I-NOAMP-benign, but it is not zero, and it is the reason
GL6 makes the region lock a **leaf**: nothing of ours may ever be waiting behind it.

### 4.3 The fault handler — and the assertion that would have been wrong

A fault can arrive in either state, and the handler must be correct in both. This is the
"passthrough is a pure optimisation" requirement made mechanical:

- **Fault while the region mutex is HELD.** Wait for the holder. Then unprotect the page and
  wake the faulting thread. The guest's write lands *after* our hold ends — which is the
  guarantee, delivered by making the vCPU wait, exactly as measured in §1.1.
- **Fault while the region mutex is NOT held.** Resolve immediately: unprotect, wake. **This
  is legal and must not assert.** It is the steady state of the always-trapped mode (§6.1),
  and it happens transiently in the opportunistic mode whenever a fault races our disarm.
  Nobody was relying on stability, so the guest wins, and that is correct.

> **★ GL7 (normative). "The mutex is held" is asserted on the READER side, never in the fault
> handler.** Every access by our threads to a `LockPath` region asserts it holds a
> `RegionAccess` covering the range. A handler-side assert would fire on its own correct
> steady state and would panic the fallback mode continuously.

This matters because the owner's phrasing — *"the trap handler … asserts the mutex is held"* —
reads naturally as a handler assert, and a handler assert is the one version of this that
cannot ship. The *intent* is right and lands on the reader side, where it is also the only
version a mock can check (§6.3).

**Resumption, and why the core is not on this path:**

> **GL8 (normative). Fault resolution NEVER depends on the core.** The handler resolves
> (unprotect + wake) and *then*, optionally, notifies. It never parks a vCPU behind the
> executor's inbox.

Two reasons, either sufficient. **Latency:** the executor is serialized, so parking a guest
store behind it prices one guest write at the inbox's current depth. **Deadlock:** the
executor is itself allowed to hold a region mutex (it runs the background work of §6.6), so
"resolve by asking the executor" is a cycle whose other end is a parked vCPU.

★ **This corrects a claim in the code's own rustdoc.** `CoreEvent::LockedRegionFault` today
says *"the accessing vCPU is parked until the core answers"* (`kayfabe-vmm`). Under GL8 that
is wrong: the vCPU is parked until the **holder** releases, and the core's notification is an
**observation after the fact**. The delivery seam stays exactly where §6.8 put it — arriving
on the serialized executor is a property of the core's entry discipline — but its meaning
changes from *decision point* to *observation*. **Owed edit to `kayfabe-vmm`'s rustdoc**
(not made here: another agent owns `crates/`).

**What the notification is good for, and its own trap.** A fault on the GSP command queue is
a genuine Layer-2-adjacent signal — *the guest just wrote a command* — and could replace a
poll. It is therefore **per-region opt-in and default off**, because in the always-trapped
mode it fires on every guest write and would convert a bounded observation into a rate.

### 4.4 ★ Registration is not arming — and every placement destroys it

`UFFDIO_REGISTER` is done **once per window, at install** (coarse tier), and `WRITEPROTECT`
is per page range (fine tier) — the same coarse/fine split as `l1_os_shell.md` §6.7's memslot
table, which is why registration never appears in a hot path. **Registration alone traps
nothing**; only `WRITEPROTECT` arms. Stating that separation explicitly is worth a paragraph
because the natural reading of "we registered the window" is "the window is protected", and
that reading is a silent no-op waiting to happen.

> **⚠️ [inferred] — OPEN QUESTION (bench experiment).** uffd registration is a property of the
> **VMA** (`vma->vm_userfaultfd_ctx`). A `MAP_FIXED` placement inside the window creates a
> **new VMA**, which by that reading carries **no** registration — so **every publication into
> a window silently un-registers that sub-range**, and a later arm over it fails `ENOENT`
> (loudly, per §4.1 step 2) or, worse, appears to succeed over a partially-registered range.
> I have not measured this; it needs one bench experiment and it is cheap.

**The conservative design, adopted now because it costs nothing if the inference is wrong:**

> **GL: re-registration is part of the placement operation, not a caller obligation.**
> `Reservation::map_fixed_in` and `Reservation::restore` re-register the affected sub-range as
> part of the same call. Re-registering an already-registered range is idempotent, so the
> unconditional form is safe, and the alternative — a rule in prose that every placement site
> must remember — is precisely the shape `testing_doctrine.md` §6 says has no reader.

### 4.5 The fault-handler thread — neither the reactor nor the executor

Somebody must `read(2)` the uffd, and that read blocks. `l1_os_shell.md` §6.6's queue
discipline says queues are drained by **existing** threads and anything wanting a thread of
its own must argue for it. The argument:

- **Not the reactor.** The reactor's whole contract is that it never blocks on anything but
  `epoll_wait` and touches zero core state (law 9, §3.2 — *it needs no table at all*).
  Resolving a fault means **waiting on the region mutex**; a blocking wait on the reactor
  thread stalls every completion source in the device. Fatal.
- **Not the executor.** GL8's deadlock, above: the executor may be the holder.
- **Therefore: one dedicated fault-handler thread per device** — the second long-lived thread
  we own, after the executor. It is a **source consumer**, not a queue drainer, so it does not
  increase the queue count §6.6 exists to bound: it owns no queue, has no schedule, and its
  work is strictly *one descriptor read → one wait → two ioctls*.
- **It never does core work.** Its entire vocabulary is `(address → RegionId → mutex →
  WRITEPROTECT)`. That is L1 shell state, not core state, so law 9's spirit survives the new
  thread intact.

---

## 5. The core-side seam — an opaque guard, and no OS vocabulary anywhere near the core

The core is `#![forbid(unsafe_code)]`, makes no syscalls, and must not learn the word
*userfaultfd* (the §6.2 vocabulary gate would fail the build if it did —
`eventfd|epoll|timerfd|rawfd|libc|O_NONBLOCK` absent from the 11 pure crates, *including
comments*). So what crosses is a **guard**, not a capability.

**The type and the trait, named:**

| item | home | what it is |
|---|---|---|
| `RegionAccess<'a>` | `kayfabe-util::region_lock` | ★ the **opaque guard** — *"I have exclusive access to this region"*. `#[must_use]`, `!Send` (§6.6: the acquiring thread is the acting thread), carries `RegionId` + length, exposes `covers(RegionId, offset, len) -> bool` and **nothing else** |
| `PageClass` | same | `LockPath` \| `Volatile` \| `CopyOnce` \| `IsolateShared` — §3's table as an enum |
| `RegionLock` (trait) | same | `register(RegionId, range, PageClass)`, `acquire(RegionId) -> Result<RegionAccess<'_>, RegionLockError>`, `mode() -> RegionMode` |
| `RegionLockError` | same | `NotLockable(PageClass)` \| `UnknownRegion(RegionId)` \| `ArmFailed` — **exact variants**, per `testing_doctrine.md` §2 |
| `UffdRegionLock` | `kayfabe-linux-raw` | the real implementation: the fd, the registration, the two `*_under_lock` ioctls, the handler thread |
| `MockRegionLock` | `kayfabe-mocks` | the deterministic one — records holds, injects faults on demand |

**Why `kayfabe-util` and not `kayfabe-vmm`.** It is the same argument §4.5 made for putting
the R1 lockwitness there: it is the bottom of the dependency graph, so the crate that cannot
depend on the runtime (`kayfabe-linux-raw`) and the crate the core depends on can both see
it. And it must **not** live on the `Vmm` trait — §6.8 removed it from there for reasons that
have not changed: this is something *we* do, to memory *we* own, with a syscall *we* make.
`kayfabe-util` has "zero GPU concepts", which is exactly true of a `RegionId` and a length.

**How it composes with plan/execute/commit (`l1_os_shell.md` §6.2).** The locked core phase
**emits** the intent; the shell **executes** it lock-free — acquire, arm, copy, disarm,
release — and the core's **commit** re-enters under re-acquired locks with **R5
re-validation**. The guard therefore lives entirely inside the execute phase and never
crosses a lock boundary, which is also why `!Send` costs nothing.

**Where the core sees it at all:** any core function that reads a `LockPath` region takes
`&RegionAccess<'_>` and asserts `covers(...)` before the read (GL7). The assertion is the
reader-side one, it is pure, and it is the only part of this design a mock can prove.

**Constructor discipline.** `RegionAccess` must be constructible only by a lock
implementation. Rust cannot express that across crates, so it is a named constructor
(`RegionAccess::held(...)`) plus a **CI grep** asserting it appears only in
`kayfabe-linux-raw` and `kayfabe-mocks` — same polarity trick and same honesty as gates A–C:
mechanical over what we write, and it says so.

---

## 6. Testing — the trap path is a mode, and the toggle is the test

Per `testing_doctrine.md` §7, which was written **from** this feature.

### 6.1 Two modes, both shipped, both run

```
RegionMode::Opportunistic   // arm at acquire, disarm at release; passthrough between holds
RegionMode::AlwaysTrapped   // armed at all times; every guest write takes the fault path
```

`AlwaysTrapped` is **not** a debug switch. It is the correct-but-slow half of the
optimisation, and per §7 rule 1 it is selected by the suite rather than reached by inducing a
failure. The precedent is `LockMode::{Degenerate, Sharded}`, which ships both configurations
from day one for exactly this reason.

**Rule 2's assertion:** the two modes agree on **everything the optimisation is not allowed
to change** — bit-identical end state, identical refusal variants, identical operation log.
Anything that differs between them is either a bug or a thing the optimisation was never
entitled to affect, and the test is where that question gets asked.

### 6.2 ★ Randomised toggling is the arm that finds the real bug

> A seeded, **irregular** flip between modes **while work is in flight**, with the same
> end-state assertions as either fixed run.

Irregular, not periodic — a regular period is a clock, and `testing_doctrine.md` §3 rule 3
rejects passing because of a timing coincidence. Two fixed runs prove each mode is
self-consistent and say **nothing** about the handoff, which is where a party acquires a
guarantee under one mode and acts on it after the switch. §6.8.1's rejected RW-lock
slip-through is the worked example: it exists *only* at the disarm edge.

**Non-vacuity (doctrine §1).** The toggle test asserts what it **observed**: at least one
fault resolved with the mutex **held**, at least one resolved with it **unheld**, and at least
one mode flip that landed **between** a region's acquire and its release. A toggle run in
which the fallback never engaged is a green instrument on an unexercised path.

### 6.3 What a mock CAN and CANNOT prove here — stated plainly

**Can:**

- that every `LockPath` read happens under a guard that covers it (GL7 is a pure assert);
- that `acquire` on `Volatile` / `CopyOnce` / `IsolateShared` refuses with the **exact**
  variant, with a non-vacuity arm that the same call on `LockPath` **succeeds** (doctrine §2);
- that no `acquire` happens while another lock of ours is held (the existing witness — GL6);
- that the two modes produce identical end states, and that toggling between them does too;
- that the fault-resolution path never re-enters the core (GL8) — a mock core that panics on
  re-entry from the handler thread is a one-line structural check;
- **the frequency invariant**: arm/disarm count grows with *holds*, not with guest writes —
  the same move as §6.7's mock slot-install gate and §3.4's wake-count assert, i.e. converting
  a cost only hardware can measure into a **quantity a mock can count**.

**Cannot — and these are §7.9-class rows, owed to the bench:**

- that `UFFDIO_WRITEPROTECT` actually stops a **guest vCPU** write. That is kernel behaviour;
  §1.1 measured it once, and it must be re-measured per target kernel, not assumed;
- that arming is coherent against an **in-flight** store (§1.1's inference);
- that `/dev/userfaultfd` yields a full-mode fd **on the deployment host** — a deployment
  fact, invisible to every type and gate we have (§7.2 is the only answer);
- whether a `MAP_FIXED` placement drops registration (§4.4's open question);
- any cost at all.

**★ And the sharpest limit, because it is the one a green suite will hide:** in a mock, the
"guest write" happens exactly where the test puts it, so **a mock cannot produce a
slip-through.** Randomised toggling in the mock tests *our state machine's* transitions; only
the bench tests whether the trap traps. The honest claim the mock earns is therefore narrow
and worth having: **we never rely on passthrough for correctness.** It is not: *the lock
works.*

---

## 7. Deployment

### 7.1 The udev rule

```
# /etc/udev/rules.d/60-kayfabe-userfaultfd.rules
KERNEL=="userfaultfd", MODE="0660", GROUP="kvm"
```

Group it with the `/dev/kvm` grant, because that is exactly the claim being made: **this adds
no new privilege class.** It is a device node an operator already has to reason about, and the
alternative it replaces — `CAP_SYS_PTRACE` — is root-equivalent and would end the
unprivileged-host thesis.

Requires **Linux 6.1+** for `/dev/userfaultfd`, and a kernel whose `UFFDIO_API` offers WP on
shmem (`WP_HUGETLBFS_SHMEM`; the bench's 6.8 offers all 17 features).

### 7.2 ★ The startup check — a functional probe, not a flag read

> **GL9 (normative). At device realize, prove the fd is FULL-mode by performing a KERNEL-mode
> write into a protected scratch page and requiring the fault. If it does not fault, refuse
> loudly (`VmmError::Unsupported`-shaped) and do not start.**

The probe, and why it is shaped this way: a `USER_MODE_ONLY` fd traps user-mode faults
perfectly well, so **any user-mode self-test passes on the broken configuration**. The
distinguishing input is a *kernel*-mode write — the same class of access KVM's page walk
makes. `read(2)` from a pipe into the protected page is one, and it needs no guest, no vCPU
and no GPU.

Also asserted at the same point, from `UFFDIO_API`'s negotiated feature set:

- **`WP_ASYNC` must not be enabled.** It auto-resolves write-protect faults and merely records
  them — a lock that looks armed and excludes nothing, which is GL3's hazard delivered by a
  feature flag.
- **`SIGBUS` mode must not be enabled**, for the same reason in a louder register.

This is `l1_os_shell.md` §4.4.1's pattern reused: a deployment precondition no type or CI
grep can observe, answered by **a loud refusal at realize** rather than by a `SIGBUS` or a
diverged page at first guest DMA.

### 7.3 The fallback: RO memslot — kept, documented, and honestly priced

If the udev grant is ever undeployable, the fallback is a **read-only memslot** covering the
lock-path pages: 70.2 µs per trapped write versus uffd's 24.8 µs (**2.8× slower**), and — its
one real advantage — **no privilege at all**.

**Its true cost is not the latency; it is a window.** KVM's read-only flag is a **slot**
property (`l1_os_shell.md` §6.7 item 4), and flipping a *shared* window to RO revokes writes
for **every proc in it** — an I-NOAMP violation by construction, and the same objection that
struck the memslot implementation in §6.7 item 5. So the fallback requires lock-path pages to
live in their **own RO-capable window**, which re-introduces window count and partitioning
that the primary design does not need.

★ **What §1.5's correction changes about it:** the flip itself is **1.49 µs** (flags-only),
not a grace-period-bearing update, so the fallback is *viable* — it is a latency and
partitioning cost, not a device-wide stall per lock. That is precisely why the wrong number
mattered: it made a legitimate fallback look impossible.

---

## 8. Honest residuals and named unknowns

1. **§4.4's registration-vs-placement question is unmeasured** and is the most likely way this
   design has a silent hole. One bench experiment closes it; the conservative design (§4.4)
   is adopted unconditionally in the meantime.
2. **§3.3 item 3 — instance blocks — is a candidate row, not a settled one.** Which aperture
   carries the writes decides whether uffd is even the right mechanism.
3. **The `mmap_lock` coupling (§4.2) is real and unbounded in the worst case.** The arm takes
   `mmap_lock` for read; a concurrent large `MAP_FIXED` holds it for write. Per-`mm`, so
   I-NOAMP-benign, but a bad interleaving makes a 1.11 µs arm arbitrarily long. Not measured
   adversarially. **The mitigation is structural, not a timeout:** GL6 makes the region lock a
   leaf so nothing of ours ever waits behind it.
4. **Every absolute number in §1.4 is from a nested-virt bench** and should be read as an
   upper bound (2–5×, estimated). Every decision here uses a ratio; if one ever uses an
   absolute, that is the line to re-measure first.
5. **The kernel-behaviour claims are per-kernel facts.** §1.1 holds on 6.8.0-124. GL9's probe
   is what makes a different kernel a loud startup failure rather than a subtle one, and it is
   the only mechanism in this document that survives a kernel change unattended.
6. **Multi-process contention on one region is unmeasured**, which is precisely the condition
   §6.8.1's mutex ruling is conditioned on. Its acceptance criteria for an RW upgrade are
   unchanged and still unmet: **(a)** measured contention on the region by threads of one
   proc, **(b)** a design where the disarm and the last reader's departure are one indivisible
   transition, **(c)** the §6.2 toggling test over the upgraded shape.
7. **The fault-handler thread is a new long-lived thread** — the first addition to the
   inventory since §6.6's audit. It is argued in §4.5 rather than waved through, and if the
   argument is wrong the consequence is a thread we did not need, not a correctness bug.
8. **What this design does NOT claim.** It does not make guest-writable memory
   authoritative — §2.1's rule stands unchanged, and a region hold is taken **at** a Layer-2
   edge, never instead of one. Nothing here licenses reading a shared table at an arbitrary
   instant and acting on it.

---

## 9. The normative rules, collected

| # | rule |
|---|---|
| **GL1** | Copy-once-then-validate stays the **primary** layer; the region lock is the exception, taken only where a decision needs stability across a read-then-act. |
| **GL2** | The lock/no-lock decision is made on the guest's **write RATE** to the region, never on the lock's per-op cost. |
| **GL3** | Every region declares a `PageClass` at registration; `acquire` on a non-`LockPath` class is a **loud fault**, never a silent no-op. |
| **GL4** | Isolate-shared and DMA-target ranges are **unlockable by construction** and are refused at registration. |
| **GL5** | The protocol is **acquire → arm → act → disarm → release**, on one thread, with every other lock of ours dropped. |
| **GL6** | The region mutex is **rank 3, acquired only from a lock-free context**; exactly two `*_under_lock` syscalls may run beneath it, and nothing else ever. |
| **GL7** | "The mutex is held" is asserted on the **reader** side; a fault with the mutex unheld is **legal** and is resolved, never asserted against. |
| **GL8** | Fault resolution **never depends on the core**: resolve first, notify second, never park a vCPU behind the executor. |
| **GL9** | A `USER_MODE_ONLY` uffd is a **startup refusal**, proven by a kernel-mode-write probe rather than by reading a flag; `WP_ASYNC` and `SIGBUS` modes are refused with it. |
| **GL10** | **Passthrough is never load-bearing.** The trap path is a selectable first-class mode, and the suite toggles between the two irregularly, under load, with non-vacuity assertions on both arms. |

---

## See also

- `core_security_threat_model.md` §2.1 — the two-layer trust model. **Normative over this
  file.** Layer 1 is what this document builds; Layer 2 is why building it is not sufficient.
- `l1_os_shell.md` §6.7 (memslot strategy, and §1.5's correction to its cost model), **§6.8**
  (why the capability left the `Vmm` trait — this file is that section's declared residual),
  **§6.8.1** (the plain-mutex ruling and its region-scope precondition), §4.2.2 (the read
  split), §4.3 (volatile vs atomic), §4.5 (the lockwitness and the `*_under_lock` set), §6.6
  (I-NOAMP, and who runs a memory-plane op).
- `l1_concurrency.md` §3.3 — R1 / R3 / R5, the invariants GL5 and GL6 are written to satisfy.
- `testing_doctrine.md` §7 (first-class fallback + randomised toggling — written from this
  feature), §1 (non-vacuity), §2 (exact variants).
- `../reference/rm_semantics_measured.md` §1 — why same-region contention by one proc's
  threads is rare: RM serializes per client anyway.
- `../reference/memory_cacheability.md` §1 — the four cacheability deciders; the same
  "who owns the PTE" logic that makes a device mapping unlockable.
