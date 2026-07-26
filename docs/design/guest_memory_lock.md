# The guest-shared-memory region lock — Layer 1, measured

**Status:** design. Written against the bench round of **2026-07-26**, then **substantially
revised on 2026-07-27** after an owner ruling and a second bench round. It is the residual
`l1_os_shell.md` §6.8 deliberately left open — *"the uffd design itself — registration mode,
the fault-handler thread's placement, and its interaction with the `assert_lock_free` witness —
is **not** designed here"* — plus the one shape §6.8.1 fixed in advance (a plain mutex, and why).

**What this file is.** The design of the mechanism that makes **Layer 1** of the two-layer
trust model (`core_security_threat_model.md` §2.1) real: *while we hold a region, the guest
cannot move the bytes we are deciding on.* It names the mechanism, the lock, the thread, the
seam the core sees, the tests, and the deployment posture.

**What it is not.** It is not the trust model (that is §2.1, and it is normative over this
file, not the other way round). It is not the memslot strategy (`l1_os_shell.md` §6.7). It
is not a replacement for copy-once-then-validate — §2 says so in the strongest terms
available, because the available mistake is to read this document as one.

> ### ★★ 2026-07-27 — THE MECHANISM CHANGED. Read §1.0 before anything else.
>
> This document previously specified **userfaultfd write-protect**, armed and disarmed around
> each hold, with a read-only memslot as a fallback. The owner ruled that a mechanism gated on
> a facility a stock host denies is not acceptable for an unprivileged-host project, and a
> candidate study was run: **`../reference/region_lock_mechanism_study.md`**.
>
> The study found that **three recorded facts this document rested on were wrong** (§1.0), that
> the fallback as written **silently discards guest writes on QEMU**, and that a different
> shape of the same fallback — **permanently read-only, never flipped** — is privilege-free,
> costs *nothing* per lock, and is **simpler than uffd in five separate places**. It also found
> that this shape is **unsound on arm64**, which splits the design by architecture.
>
> Where this file and the study disagree about *what the machine does*, **the study wins.**

---

## 0. The one-paragraph version

A per-region **`Mutex`** serialises our threads against each other. The region's pages live in
a **permanently read-only guest mapping**: guest **reads** are served from RAM at full speed
with no exit, and every guest **write** exits to our handler **on the vCPU thread that issued
it**, where the handler takes the region mutex — parking that vCPU for exactly as long as we
hold the region — and then applies the write itself. There is **no arming and no disarming**:
the exclusion is standing, so a lock cycle costs a mutex and nothing else. The price is paid
per *guest write*, which is why the lock is confined to a declared class of low-write-rate
pages and **refuses** — loudly — everywhere else. Three agents are outside its reach and always
will be: **the isolate** (different `mm`), the **GPU** (DMA does not walk our page tables), and
**arm64** (§7.4 — the mechanism is unsound there and the capability is refused at realize).

---

## 1. Ground truth — everything this design is built on

Measured on the bench, kernel **6.8.0-124** (rounds of 2026-07-26 and 2026-07-27), unless
tagged otherwise. Per §0.2's discipline: **[measured]** = observed here, **[src]** = read from
the kernel / QEMU / `ogkm`, **[inferred]** = a conclusion, marked so it can be attacked. Every
performance figure also carries a **layer**: **[KVM-direct]** or **[QEMU]**.

### 1.0 ★★ Three things this document previously asserted, and why each was wrong

Recorded first, at the top, because everything downstream was reasoned from them. Full detail
and citations: `../reference/region_lock_mechanism_study.md` §2.

1. **"A flags-only memslot flip is 1.49 µs."** There is **no flags-only read-only flip.**
   **[measured, KVM-direct]** setting `KVM_MEM_READONLY` on a live memslot returns
   **`-EINVAL`**; **[src]** `linux v7.1.0-rc6 virt/kvm/kvm_main.c:2079-2082` rejects any change
   to that bit on an existing slot, and **[src]** QEMU works around it explicitly
   (`v10.2.0 accel/kvm/kvm-all.c:373-386`, citing KVM commit `75d61fbc`).
2. **The measurement behind that number never toggled `KVM_MEM_READONLY`, and does not say
   1.49 µs.** **[src]** the bench binary's "flags" arm toggles `KVM_MEM_LOG_DIRTY_PAGES`
   (`kvmslotbench.c:293-294`); **[measured]** re-run today it reports **72.4 µs p50**, while
   the row whose magnitude is ~1.2 µs is the **no-op identical re-issue**. The figure was the
   wrong row of the right table.
3. **The RO-memslot fallback, spelled the obvious way on QEMU, is not a lock — it is silent
   data loss.** **[src]** `v10.2.0`: a plain RAM `MemoryRegion` keeps
   `mr->ops = &unassigned_mem_ops` (`system/memory.c:1302`); marking it read-only routes guest
   writes to `memory_region_dispatch_write`, which rejects them (`:1528-1531`) and calls
   `unassigned_mem_write` — **an empty function** (`:1344-1350`). The write is excluded and
   then **thrown away, with no callback and no fault.**

> **★ The methodological lesson, which is the durable part.** `l1_os_shell.md` §6.3.1 learned
> that *a named API in a design doc is a claim about a version, and it decays.* This is the
> same failure applied to a **number**: **a figure in a design doc is a claim about an
> experiment**, and it decays the same way — faster, in fact, because nobody re-reads the
> harness. Item 1 alone was enough to price a struck design back into the document.

### 1.1 The requirement, stated before any mechanism

Restated here because the candidate study eliminated whole families of options on this one
line, and it must not be glossed:

> **The mechanism must BLOCK the writer, not detect the write.** §2 below: the lock exists
> because *when we read a descriptor, resolve it, and then issue a host operation derived from
> it, copy-once alone lets the guest rewrite the descriptor in between.* A mechanism that
> reports the change afterwards reports it **after the host op has been issued**, and
> `l1_os_shell.md` §7.8's conservation ledger says a host RM verb is not generally undoable.

**[measured] This eliminates dirty logging and the dirty ring outright**, in both directions
at once: a guest store into a `KVM_MEM_LOG_DIRTY_PAGES` slot produces **no userspace exit**,
**the write lands**, *and* the bitmap bit is set. The instrument is live and it is a reporting
channel, not a lock. It also eliminates hardware watchpoints, whose x86 delivery is
*trap*-type — the store retires first.

The one honest qualification: where the "act" can be **deferred until after a revalidation**,
detect *is* sufficient, and that is the R5 shape `l1_concurrency.md` already has. That is a
restructuring of the caller, not a lock — and per §3.4 every candidate `LockPath` region has to
argue past it first.

### 1.2 ★★ The mechanism: permanent read-only, and it blocks — proven with the hold in the middle

**[measured, KVM-direct]** The guest runs a tight store loop against a permanently read-only
memslot. On the first trap the handler deliberately does nothing for 50 ms:

```
[E11] first store trapped; backing word = 0x0 (still the pre-store value)
[E11] after holding 51.4 ms without answering: backing word = 0x0
      -> UNCHANGED: the guest write did NOT land (BLOCKED)
[E11] after applying + resuming: exit=MMIO, backing word = 0x1122334455667788
```

**[measured, KVM-direct] Guest reads are not trapped** — a load from the same region is served
from RAM and never leaves the guest. That is what makes the mechanism affordable: only the
low-rate direction is priced.

**[measured, KVM-direct] Every x86 store form tested is trapped and applied correctly** —
`mov` (4 B and 8 B), `movsq`, `rep movsb`, `movdqu` (SSE), `vmovdqu` (AVX), `movnti`, and
`lock xchg`. Wide vector stores arrive as **≤ 8-byte** transactions; see §3.5 for the semantic
consequence, which is real.

> **[inferred] The load-bearing consequence, stated because the whole design rests on it:**
> the exclusion is **standing**, so there is no arming edge and therefore no window in which a
> store issued before we "arm" can land after it. The coherence problem that the previous
> design had to *measure its way out of* does not exist in this one. That is not a tuning
> improvement; it deletes a class of race.

### 1.3 ★★ The QEMU spelling — and it is a specific one, not "mark it read-only"

**[src] `v10.2.0`** The only correct spelling is a **ROM device region**:
`memory_region_init_rom_device_nomigrate()` (`include/system/memory.h:1629`) with a real
`MemoryRegionOps.write`. Reads go direct to RAM
(`include/system/memory.h:3153-3164`, `memory_access_is_direct` is true for reads); writes are
dispatched to **our** callback; and the KVM listener installs the region as a read-only memslot
(`accel/kvm/kvm-all.c:1511` `bool writable = !mr->readonly && !mr->rom_device;`, `:1517-1520`).

Two constraints come with it, both binding:

- **The handler must run BQL-free.** The region must also carry
  `memory_region_enable_lockless_io()` — otherwise a hold parks a vCPU **inside the BQL** and
  the region lock becomes a whole-VM stall. **★ This consumes a deployment requirement we have
  already paid for** (`qemu_102_facilities.md` row 1, decision `c3ec258`) rather than adding
  one, which is a large part of why this shape is affordable.
- **[src] There is no `_ptr` constructor.** QEMU allocates the `RAMBlock`. So a lock-path
  region is **not** a slice of our own reservation and `Reservation::map_fixed_in` does not
  reach it — a real change from the previous design's assumption, recorded in §8.

### 1.4 ★★ Three hard boundaries — all permanent

| boundary | why | evidence |
|---|---|---|
| **The isolate is NOT covered** | different `mm`; its writes go through its own page tables and land unseen | **[measured]** |
| **GPU DMA is NOT covered** | the GPU writes the physical page without walking our page tables | **[measured]** — same physical page written, no fault, write landed |
| **★ arm64 is NOT covered** | the mechanism traps by **emulating** the faulting store, and arm64 cannot decode the store forms `memcpy` emits | **[src]** — §7.4 |

These are not gaps to be closed later. They are the **shape** of the mechanism, and §3 and §7.4
turn them into refusals rather than caveats.

**[inferred] A fourth boundary follows from the mapping type**, worth naming because it looks
like a tuning question and is not: a **device mapping** — anything whose PTEs came from a
driver's own `mmap` handler — cannot be a lock-path region, for the same reason it is not
cacheable-by-request (`../reference/memory_cacheability.md` §1, decider 1).

**★ One collision the previous design had and this one does not.** uffd-WP would have been a
live hazard against NVIDIA's driver, which **rejects any range with `userfaultfd_armed(vma)`**
(**[src]** `ogkm: kernel-open/nvidia-uvm/uvm_hmm.c:577-588`). That constraint disappears
entirely: we no longer arm anything. It is recorded because it is the reason not to reintroduce
uffd casually.

### 1.5 The costs — and the number that decides is still a RATE

**[measured, KVM-direct]** p50 unless stated, on the 4 KiB region size the taxonomy actually
uses.

| operation | cost | layer |
|---|---|---|
| **a full lock cycle (acquire → act → release)** | **a `Mutex`, and nothing else** | — |
| trapped guest write: exit → handler → apply → resume | **55.6 µs**, sustained **17 973 writes/s** | [KVM-direct] |
| the same primitive, cross-checked | **50 µs**, **15 861 writes/s** | **[QEMU]** — `qemu_bql_spike.md` §6, a trapped BAR write dispatched to a device handler on QEMU 9.2 |
| guest read of a lock-path page | **free** — no exit | [KVM-direct] |
| *(struck alternative)* RO memslot flipped per lock | **124.5 µs** (0 vCPU) → **720.3 µs** (4 vCPU) per cycle, p99 3.6–7.2 ms, other vCPUs degraded **2.2–9.1×** | [KVM-direct] |
| *(displaced baseline)* uffd-WP arm + disarm | 6.5 µs, +27 ns/page | [KVM-direct] |

> **★ The cross-layer agreement in row 2 is worth one sentence of care**, because
> `qemu_102_facilities.md` §11.1 exists to stop exactly this move. The KVM-direct and QEMU
> figures agree to within **12 %** *for this primitive only*, and the reason is structural: the
> expensive QEMU additions live in the **memory-topology** path (BQL transaction, FlatView
> rebuild, listener walk), and **this design never touches the memory topology after realize.**
> The struck alternative is the one that does, and its QEMU cost is correspondingly **larger**
> than its KVM-direct cost, not equal to it.

**★ Per-workload, the rate decides, and it decides differently for different pages.**

| page class | **[measured]** guest write rate | cost at 55.6 µs/write |
|---|---|---|
| GSP command queue | ~200–3000 /s | **1.1 – 17 %** of one vCPU-thread-second — affordable |
| page-table pages | ~18 000 /s | **≈ 100 %** — and the mechanism's measured **ceiling is 17 973 /s** |

> **GL2 (normative, unchanged). The lock/no-lock decision is made on the guest's WRITE RATE to
> the region, never on the per-operation cost of the lock.**

> **★★ And the mechanism's capacity limit lands exactly on the taxonomy boundary the design
> already drew.** GL2 refuses the page-table class at 18 kHz; the mechanism saturates at
> 17 973 writes/s. That is the **third** independent derivation of the same line — the C's
> 0x110094 vmexit storm found it as a performance bug, §1.5's arithmetic found it as a cost
> calculation, and now the mechanism's own ceiling finds it as a hard wall. A misapplication to
> a high-rate page fails **immediately and obviously** rather than subtly, which is the best
> property a wrong classification can have.

**⚠️ Caveat, inherited.** The bench host is itself a KVM guest; absolute microsecond figures are
inflated by nested virtualisation (**estimated 2–5×, not measured**) and are an upper bound on
cost. **[inferred]** the GSP-queue row is therefore likely **3–8 %** of a vCPU on bare metal.

### 1.6 Context: there is no C-era wall here

**The C never implemented any of this.** So nothing below is fighting a known failure; this is
new ground, and the risk register (`C: mode2_rust_rewrite_architecture.md` R1 — *"if KVM-UFFD
on 6.8 has gaps, switch to a vCPU-pause path"*) is retired twice over: uffd was measured to
work, and it is no longer the mechanism. **[measured]** the vCPU-pause path it named as the
alternative was also measured and is worse than either: getting all vCPUs out of the guest
costs 15.6 µs at 1 vCPU and **6 ms at 16**, at whole-VM granularity.

**What the C shipped instead was copy-once — and its known hole is exactly what this fills.**
The C's guest-side mmap path requires a manual writeback after every ioctl, and **nothing
detects a guest write to the original page** (`C: src/guest/nvkvm_mmap.c:429-436`, `:617-624`).
That is a TOCTOU window with a person standing in it, which is precisely what
`core_security_threat_model.md` §2.1 says Layer 1 exists to make *not exist*.

---

## 2. ★★ The layering — copy-once is primary, and the lock is the exception

*(Unchanged by the mechanism revision: this section is about what a lock is for, not how it is
built.)*

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
| **Lock path** | GSP **command** queue (guest→GSP), and any dynamic faked-register page whose multi-word content we read as a unit *(instance blocks: candidate only — §3.3)* | permanently-RO region + per-region `Mutex` in the write handler | **yes** — the only class that is |
| **Volatile / atomic** | completion semaphores, USERD, PTIMER — written by the **host or the GPU** | aligned atomics, `Relaxed`, with explicit fences at the two seams (`l1_os_shell.md` §4.3) | **no** — the mechanism is *blind* to these writers |
| **Copy-once / commit-point** | page-table pages, userspace pushbuffers | snapshot + validate; captured at the protocol's own edge (doorbell, CE release semaphore, TLB invalidate) | **no** — rate (§1.5) and doctrine (§2) both refuse |
| **Isolate-shared** | guest-RAM slices exported to isolates | an ownership protocol between us and the isolate | **no** — different `mm` |

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
guarantee exists.

**★ And §1.0 item 3 is that hazard arriving from an unexpected direction**, which is why GL3
now has a companion at the *implementation* level: `memory_region_set_readonly()` on a RAM
region produces exactly "appears held, protects nothing" — except worse, because it also
destroys the guest's data. See GL9.

### 3.3 ★ Two rows that are still not settled

Recorded rather than silently implemented, because a miscategorisation is the most expensive
error available in this design.

1. **"GSP command/message queue" is two things, and only one is lock-path.** The **command**
   queue (guest→GSP) is guest-written and we read it — lock path. The **message/status** queue
   (GSP→guest) is written by **us** and read by the guest: we are the sole writer, so there is
   nothing to exclude, and its correctness comes from the transport's own publication
   discipline. Putting it in a read-only region would make **our own** writes trap. **Split the
   row.**
2. **"Instance blocks" is an open question, not a settled row.** Whether an instance block
   reaches us as a guest-RAM write or through the emulated **BAR2 aperture** decides which
   mechanism even applies — a BAR2 window write is trapped by the VMM's own path already.
   The C has the relevant prior (`C: docs/design/mode2_bar2_mmu.md`).
   **OPEN QUESTION (bench experiment):** which aperture carries instance-block writes in the
   Mode-2 path we are building. Until it is answered the row is a *candidate*, and GL3 makes
   the wrong answer a refusal rather than a silent no-op.

★ **What is no longer an open question:** "dynamic faked registers mostly need OBSERVATION, not
EXCLUSION". That distinction survives and is now cheaper to honour, because under this design
**observation and exclusion are the same mechanism** — a permanently-RO region delivers every
write to us whether or not anyone is holding the lock. A page that only needs observing is
registered `LockPath` and simply never acquired.

The other classes are right for reasons stronger than classification: the volatile class is
excluded by *who writes it*, and the pushbuffer is excluded by something better than a lock —
**the host GPU executes from our copy, not from the guest's buffer.**

### 3.4 ★ Every candidate region must argue past "no lock at all"

> **GL11 (normative). A region is registered `LockPath` only with a written argument that the
> decision cannot be restructured as copy → validate → **revalidate** → act.** That
> restructuring is `l1_concurrency.md`'s existing R5 shape; it costs nothing, works on every
> architecture, and needs no mechanism. The lock is for the case where the host operation's
> meaning depends on guest bytes we cannot carry forward — and today **no region has that
> argument written down.**

### 3.5 ★ What a lock-path page gives up, declared rather than discovered

**[measured, KVM-direct]** Because the mechanism traps by **emulating** the store:

- a guest `lock`-prefixed RMW on a lock-path page **is no longer atomic** against another vCPU;
- a 16- or 32-byte vector store is applied in **≤ 8-byte pieces**.

While the region is held, neither is observable — nothing else may write. Between holds, both
are real.

> **GL12 (normative). A `LockPath` region MUST NOT carry guest-side atomics, and no guest-
> visible invariant may depend on the atomicity of a wide store into it.** This is declared at
> registration alongside `PageClass`, because it is a property of the *page's protocol*, not of
> our code, and it is not discoverable by any test we can write on our own side.

---

## 4. The lock protocol

### 4.1 The sequence

> **GL5 (normative).** **acquire mutex → act → release**, in that order, on **one thread** —
> the thread already serving the guest's request (`l1_os_shell.md` §6.6: there is no
> memory-plane thread and no memory-plane queue) — with **every other lock of ours dropped**.

| step | what happens | failure disposition |
|---|---|---|
| 1. acquire | `assert_lock_free()`, then lock the region's `Mutex`; the watermark rises to the region rank (§4.2) | class refusal (GL3) is checked here and is a fault |
| 2. act | read the bytes (copy-once, per GL1), decide, issue the host op | ordinary refusal paths; the guard is `#[must_use]` and unwinds |
| 3. release | drop the guard → drop the mutex → any parked writer proceeds | — |

★ **Two steps are gone from the previous design — arm and disarm — and with them their failure
modes**, which were the two worst in the document: an arm that silently did nothing (the GL3
hazard through the back door) and a failed disarm that would leave a region trapped forever.
Neither is expressible now.

### 4.2 Where the region lock sits in the rank order

`l1_concurrency.md` §3.3 declares device = 0, proc = 1, leaf = 2, and **R1 forbids a blocking
call under any lock**.

> **GL6 (normative). The region mutex is RANK 3, is acquired only from a lock-free context,
> and NO syscall may be issued beneath it.** `acquire` calls `assert_lock_free()` **first** (so
> no device/proc/leaf lock may be held), then raises the watermark to 3.

★ **This is a strict improvement over the previous design and worth naming as one.** The uffd
version needed an explicit exception to R1 — two `*_under_lock` syscall variants, each argued —
because arming *is* a syscall under the region mutex. This version needs **none**: the only
thing beneath the mutex is a `memcpy` into a mapping we already own. The greppable
`*_under_lock` set shrinks back to exactly one member (`raise_irq`'s descriptor write), and the
`mmap_lock` coupling that §4.2 previously listed as an honest residual **disappears entirely**.

### 4.3 The write handler — and the assertion that would have been wrong

The handler runs **on the vCPU thread that issued the store**, dispatched from the region's
`MemoryRegionOps.write`. It must be correct in both states:

- **Write while the region mutex is HELD.** Block on the mutex. Then apply the write and
  return. The guest's write lands *after* our hold ends — which is the guarantee, delivered by
  making the vCPU wait, exactly as measured in §1.2.
- **Write while the region mutex is NOT held.** Acquire it uncontended, apply, return. **This
  is legal and must not assert.** It is the steady state — every guest write to a lock-path
  page takes this path — and nobody was relying on stability, so the guest wins.

> **★ GL7 (normative). "The mutex is held" is asserted on the READER side, never in the write
> handler.** Every access by our threads to a `LockPath` region asserts it holds a
> `RegionAccess` covering the range. A handler-side assert would fire on its own correct
> steady state and would panic continuously.

**Resumption, and why the core is not on this path:**

> **GL8 (normative). Write resolution NEVER depends on the core.** The handler applies the
> write and *then*, optionally, notifies. It never parks a vCPU behind the executor's inbox.

Two reasons, either sufficient. **Latency:** the executor is serialized, so parking a guest
store behind it prices one guest write at the inbox's current depth. **Deadlock:** the executor
is itself allowed to hold a region mutex (it runs the background work of §6.6), so "resolve by
asking the executor" is a cycle whose other end is a parked vCPU.

★ **This corrects a claim in the code's own rustdoc.** `CoreEvent::LockedRegionFault` today
says *"the accessing vCPU is parked until the core answers"*. Under GL8 that is wrong: the vCPU
is parked until the **holder** releases, and the core's notification is an **observation after
the fact**. The delivery seam stays where §6.8 put it, but its meaning changes from *decision
point* to *observation*. **Owed edit to `kayfabe-vmm`'s rustdoc** (not made here: another agent
owns `crates/`).

**What the notification is good for, and its own trap.** A write to the GSP command queue is a
genuine Layer-2-adjacent signal — *the guest just wrote a command* — and could replace a poll.
It is therefore **per-region opt-in and default off**, because it now fires on **every** guest
write and would convert a bounded observation into a rate.

### 4.4 ★ There is no fault-handler thread, and that is the fourth simplification

The previous design needed a **dedicated long-lived thread per device** to `read(2)` the uffd,
argued at length against §6.6's queue discipline because it could be neither the reactor (which
must never block) nor the executor (which may be the holder). **That thread is gone.** The trap
arrives on the vCPU thread that caused it, which is the only thread that could possibly be the
right one, and the thread inventory returns to what §6.6 audited.

### 4.5 ★★ The one hazard that survives, from the mechanism it replaces

Recorded because it is the reason not to reintroduce uffd casually, and because the question
was open and is now closed.

**[measured, KVM-direct]** `UFFDIO_REGISTER` is a property of the **VMA**. A `MAP_FIXED`
placement inside a registered window — either a memfd publication or the anonymous
`Reservation::restore` — **destroys the registration for that sub-range**, and a subsequent
arm fails with **`ENOENT`**. Re-registering restores it fully.

```
[E10] arm sub-range BEFORE any placement          : rc=0  (OK)
[E10] MAP_FIXED a fresh memfd backing over [1M,1M+64K)
[E10] arm the SAME sub-range AFTER the placement  : rc=-1 errno=2 (ENOENT)
[E10] arm after MAP_FIXED ANON restore            : rc=-1 errno=2 (ENOENT)
[E10] re-REGISTER the placed sub-range            : rc=0  (OK)
```

**§8's residual 1 — *"the most likely way this design has a silent hole"* — is closed: the
inference was right, and it fails loudly rather than silently.** It no longer applies to the
shipped design (we register nothing), and it is a precondition on §7.3's optional accelerator.

---

## 5. The core-side seam — an opaque guard, and no OS vocabulary anywhere near the core

*(Unchanged in shape; two rows change home.)*

The core is `#![forbid(unsafe_code)]`, makes no syscalls, and must not learn the word
*userfaultfd* or the word *memslot* (the §6.2 vocabulary gate would fail the build). So what
crosses is a **guard**, not a capability.

| item | home | what it is |
|---|---|---|
| `RegionAccess<'a>` | `kayfabe-util::region_lock` | ★ the **opaque guard** — *"I have exclusive access to this region"*. `#[must_use]`, `!Send`, carries `RegionId` + length, exposes `covers(RegionId, offset, len) -> bool` and **nothing else** |
| `PageClass` | same | `LockPath` \| `Volatile` \| `CopyOnce` \| `IsolateShared` — §3's table as an enum |
| `RegionLock` (trait) | same | `register(RegionId, range, PageClass, Atomicity)`, `acquire(RegionId) -> Result<RegionAccess<'_>, RegionLockError>` |
| `RegionLockError` | same | `NotLockable(PageClass)` \| `UnknownRegion(RegionId)` \| `Unsupported` — **exact variants**, per `testing_doctrine.md` §2 |
| `TrappedRegionLock` | `kayfabe-vmm-<backend>` | the real implementation: the read-only region, the write handler, the mutex map |
| `MockRegionLock` | `kayfabe-mocks` | the deterministic one — records holds, injects concurrent writes on demand |

★ **The implementation moves crates.** Under uffd it belonged in `kayfabe-linux-raw`, because
`UFFDIO_REGISTER` on our own VMA needed no VMM cooperation — that was §6.8's whole argument for
taking the capability *off* the `Vmm` trait. **That argument no longer holds:** creating a
read-only guest region and receiving its write callbacks is something **only the VMM can do**.

> **This does NOT put `lock_region` back on the `Vmm` trait.** §6.8's *second* and decisive
> objection is untouched: the removed method was **slot-granular**, and what the design needs
> is a **region** whose lifetime is the device's. What the adapter gains is a **realize-time**
> capability — *"create this range as a trapping region and call me on writes"* — which is
> installed once and never varies per lock. Coarse at realize, free at runtime: the same
> two-tier split `l1_os_shell.md` §6.7 already imposes on memslots.

**Why the guard type stays in `kayfabe-util`.** It is the bottom of the dependency graph, so
the crate that cannot depend on the runtime and the crate the core depends on can both see it.
`kayfabe-util` has "zero GPU concepts", which is exactly true of a `RegionId` and a length.

**How it composes with plan/execute/commit (`l1_os_shell.md` §6.2).** The locked core phase
**emits** the intent; the shell **executes** it lock-free — acquire, copy, act, release — and
the core's **commit** re-enters under re-acquired locks with **R5 re-validation**. The guard
lives entirely inside the execute phase and never crosses a lock boundary, which is also why
`!Send` costs nothing.

**Constructor discipline.** `RegionAccess` must be constructible only by a lock implementation:
a named constructor (`RegionAccess::held(...)`) plus a **CI grep** asserting it appears only in
the adapter crates and `kayfabe-mocks`.

---

## 6. Testing

Per `testing_doctrine.md` §7, which was written **from** this feature.

### 6.1 ★ There is only one mode now, and that is the strongest form of GL10

```
RegionMode::AlwaysTrapped   // the only shipped mode: every guest write takes the handler path
```

> **GL10 (normative, strengthened). Passthrough is not merely never load-bearing — on the
> shipped mechanism it DOES NOT EXIST.** There is no unarmed state, so there is no
> optimisation whose failure could be mistaken for correctness, and no transition between two
> modes for a test to have to catch.

★ **What this costs the test plan is the randomised-mode-toggling arm**, which §6.2 previously
called *"the arm that finds the real bug"*. That arm existed **because** the mechanism had two
states; it is not owed to a mechanism with one. The doctrine rule it motivated
(`testing_doctrine.md` §7: a first-class fallback plus irregular toggling) is **general and
stays** — it now has `LockMode::{Degenerate, Sharded}` as its live instance, and it becomes
owed again the day §7.3's optional accelerator is built.

**What replaces it as the sharp arm:** a seeded, irregular **concurrent-write** schedule — the
mock injects guest writes at unpredictable points relative to acquire/act/release — with
non-vacuity assertions that at least one write was **blocked by a held mutex** and at least one
was **applied with the mutex unheld**.

### 6.2 What a mock CAN and CANNOT prove here — stated plainly

**Can:**

- that every `LockPath` read happens under a guard that covers it (GL7 is a pure assert);
- that `acquire` on `Volatile` / `CopyOnce` / `IsolateShared` refuses with the **exact**
  variant, with a non-vacuity arm that the same call on `LockPath` **succeeds** (doctrine §2);
- that no `acquire` happens while another lock of ours is held (the existing witness — GL6);
- **that no syscall is issued beneath the region mutex** — now a *stronger* structural claim
  than before, because the legal set is empty rather than two;
- that write resolution never re-enters the core (GL8) — a mock core that panics on re-entry
  from a write handler is a one-line structural check;
- **the frequency invariant**: the number of trapping-region **installs** grows with devices,
  not with holds and not with guest writes — the same move as §6.7's mock slot-install gate,
  converting a cost only hardware can measure into a **quantity a mock can count**. Under this
  design the target is not "grows slowly" but **"exactly one per lock-path region, ever"**.

**Cannot — and these are §7.9-class rows, owed to the bench:**

- that a read-only region actually stops a **guest vCPU** write. §1.2 measured it once, and it
  must be re-measured per target kernel and per backend, not assumed;
- that the **QEMU** spelling delivers the write to our callback rather than discarding it —
  §1.0 item 3 is a source read, not a measurement (`../reference/region_lock_mechanism_study.md`
  §10 item 1 names the experiment);
- that the x86 emulator handles **every** store form the real guest driver emits — eight were
  measured (§10 item 2 of the study names the experiment: a full Mode-2 lifetime with zero
  `KVM_EXIT_INTERNAL_ERROR`);
- any cost at all.

**★ And the sharpest limit, unchanged in force and changed in target:** in a mock, the "guest
write" happens exactly where the test puts it, so **a mock cannot produce a slip-through.** The
honest claim the mock earns is narrow and worth having: **our state machine never reads a
lock-path region without a guard.** It is not: *the trap traps.*

---

## 7. Deployment

### 7.1 ★★ There is no deployment requirement. That is the point of the revision.

The mechanism needs **no device node beyond `/dev/kvm`, no udev rule, no sysctl, and no
capability.** It is a memory region created at realize by the VMM adapter and a callback we
already register. Its one dependency — BQL-free dispatch — is the **QEMU ≥ 10.2 floor we
already require** (`c3ec258`), so it consumes an existing requirement instead of adding one.

**★ The claim being retired, stated honestly rather than erased.** The previous §7.1 argued
that a udev rule on `/dev/userfaultfd` *"adds no new privilege class"* because it is the same
posture as `/dev/kvm`. **[measured, on two hosts, kernels 6.8.0-124 and 7.0.0-14]** that
argument was *correct on its own terms* — an unprivileged uid is denied **both** nodes with
`EACCES`, and **[src]** `linux v7.1.0-rc6 fs/userfaultfd.c:2192-2198` shows `/dev/userfaultfd`
performs **no capability check at all**, so uffd never required root at runtime. What it
required was a **second deployment requirement**, in a project that had just spent one. That is
a real, compounding cost, and it is the cost this revision removes. It is not the same thing as
"requires root", and the difference is recorded so the next reader does not re-derive a
stronger claim than the evidence supports.

### 7.2 ★ The startup check — a functional probe, not a flag read

> **GL9 (normative). At device realize, prove the region actually traps: perform a write to the
> region from the guest-visible side and require that it arrives at our handler. If it does
> not, refuse loudly (`VmmError::Unsupported`-shaped) and do not start.**

The probe is shaped this way because §1.0 item 3 is a **silent** failure: a region that is
read-only but wired to no callback discards writes and reports nothing. A flag read cannot see
the difference; only a round trip can. The realize-time probe additionally asserts:

- **`KVM_CAP_READONLY_MEM` is present** on the VM (it is a hard precondition, and its absence
  is silent otherwise);
- **the region carries lockless-IO** — a lock-path region without it parks a vCPU inside the
  BQL, which is a whole-VM stall wearing a per-region lock's clothes.

This is `l1_os_shell.md` §4.4.1's pattern reused: a precondition no type or CI grep can
observe, answered by **a loud refusal at realize** rather than by a diverged page later.

### 7.3 ★ userfaultfd survives as an OPTIONAL accelerator, unbuilt, with acceptance criteria

uffd-WP is a better *mechanism* on two axes and it should not be forgotten:

| | permanently-trapped (shipped) | uffd-WP (optional) |
|---|---|---|
| cost per lock | **0** | 6.5 µs [KVM-direct] |
| cost per guest write | **55.6 µs**, ceiling 17 973 /s | **0** when unarmed |
| how it stops the write | **emulate** — arch-dependent, decomposes atomics | **retry** — instruction-agnostic, preserves atomicity |
| arm64 | **unsound (§7.4)** | works [inferred] |
| deployment | nothing | **a udev rule** |
| mechanism it adds | none | a handler thread, an arm/disarm protocol, two in-lock syscalls, a re-registration obligation (§4.5) |

> **The acceptance criteria for building it, so it is not re-litigated from intuition:**
> **(a)** a measured workload in which the *trapped-write rate* on a `LockPath` region is the
> bottleneck — not an estimate, a profile; **and (b)** the `MAP_FIXED` re-registration
> obligation of §4.5 implemented as part of the placement call, never as a caller rule;
> **and (c)** `testing_doctrine.md` §7's irregular mode-toggling test running over the
> two-mode shape, because a slip-through is a *transition* bug and no steady-state test can
> see it.
>
> This is `l1_os_shell.md` §6.8.1's own discipline — *bounded and simple first; widen only on a
> measurement* — applied to the mechanism rather than to the lock granularity.

### 7.4 ★★ arm64: the capability is UNAVAILABLE, and is refused at realize

> **GL13 (normative). On arm64 the region lock does not exist. `register(..., PageClass::
> LockPath, ..)` returns `RegionLockError::Unsupported` at realize, and a device whose
> configuration requires a lock-path region refuses to start.**

**Why**, from source, because no arm64 host was available:

- **[src]** `linux v7.1.0-rc6 arch/arm64/kvm/mmu.c:2316-2318`, `:2358` — a write to a read-only
  memslot goes to `io_mem_abort()`, the same path as an access with no memslot at all.
- **[src]** `arch/arm64/kvm/mmio.c:173-188` — `io_mem_abort` first checks
  `kvm_vcpu_dabt_isvalid()`. Without a valid instruction syndrome it returns
  `KVM_EXIT_ARM_NISV` (or `-ENOSYS`), because — **[src]**
  `Documentation/virt/kvm/api.rst:7096-7112` — *"for certain classes of instructions, no
  instruction decode (direction, length of memory access) is provided, and fetching and
  decoding the instruction from the VM is overly complicated to live in the kernel."*
- **[inferred]** those classes include **`STP`/`LDP`, SIMD/FP load-stores, load-store
  exclusive, LSE atomics and `DC ZVA`** — i.e. what a compiler emits for `memcpy`/`memset`
  into a queue page. (Read from the Arm ARM's ISV definition and from why arm64 forbids
  `memcpy()` to device memory; **not measured**.)
- **[src]** `qemu v10.2.0 target/arm/kvm.c:560-565`, `:1379-1400`, `:1498-1501` — QEMU enables
  `KVM_CAP_ARM_NISV_TO_USER` and responds to the exit by setting
  `events.exception.ext_dabt_pending = 1`, **injecting an external data abort into the guest.**
  On Linux that is an oops, not a retried store.
- **[src]** A second, narrower divergence, documented by the kernel:
  `Documentation/virt/kvm/api.rst:1423-1430` — on arm64 a **page-table-walker** write (an A/D
  update) to a `KVM_MEM_READONLY` slot never produces `KVM_EXIT_MMIO`; KVM injects an abort.

> **So on arm64 a permanently-read-only guest-RAM page is not a lock; it is a page that kills
> the guest the first time the driver `memcpy`s into it.** The refusal is therefore not a
> deferral — it is the only correct behaviour until one of §8 item 5's three routes lands.

**arm64 without the region lock is not arm64 without correctness.** GL1 makes copy-once the
primary layer and §3.4 (GL11) requires every `LockPath` candidate to argue past a
copy → validate → revalidate restructuring first. An arm64 build runs on that restructuring
plus Layer 2, which is where the design says the guarantee actually lives.

---

## 8. Honest residuals and named unknowns

1. ~~**§4.4's registration-vs-placement question is unmeasured.**~~ ★ **CLOSED (2026-07-27):
   measured, the inference was right, and it fails loudly with `ENOENT`** — §4.5. It applies
   only to §7.3's optional accelerator now.
2. **§3.3 item 2 — instance blocks — is a candidate row, not a settled one.** Which aperture
   carries the writes decides whether this mechanism is even the right one.
3. **The QEMU half of the mechanism is a source read, not a measurement.** That
   `memory_region_init_rom_device_nomigrate` delivers the write to our callback, and that
   `memory_region_set_readonly` on a RAM region discards it, are both `[src]` at `v10.2.0`.
   **The experiment is named** in `../reference/region_lock_mechanism_study.md` §10 item 1 and
   it is a day's work; it should run before any code depends on it.
4. **The x86 emulator's coverage is evidence, not proof.** Eight store forms were measured;
   `KVM_CAP_EXIT_ON_EMULATION_FAILURE` exists because emulation can fail. The owed experiment
   is a complete Mode-2 guest-driver lifetime with the real GSP command queue in a trapping
   region, asserting **zero** `KVM_EXIT_INTERNAL_ERROR`.
5. **arm64 is refused on a source argument, and there are three routes back.** (a) the §7.3
   uffd accelerator, whose retry semantics are arch-agnostic — but it needs the udev rule and
   its arm64 behaviour is itself `[inferred]`; (b) our own arm64 store decoder behind
   `KVM_CAP_ARM_NISV_TO_USER` — the work the kernel declined to do, and it would be ours
   forever; (c) `KVM_MEM_USERFAULT`, which is exactly the right primitive and **[src]** is
   **absent from `linux v7.1.0-rc6`**. Route (c) is the one to watch.
6. **★ A fourth route, one upstream patch away.** **[measured]** `mprotect(PROT_READ)` on our
   own window VMA *does* stop a guest write, *is* resumable, is instruction-agnostic, costs
   6.1 µs and needs **no privilege at all** — but KVM signals it as a bare `-EFAULT`
   (**[src]** `arch/x86/kvm/mmu/mmu.c:3528`; no `KVM_EXIT_MEMORY_FAULT` caller is on this
   path), and **[src]** QEMU treats that as fatal (`v10.2.0 accel/kvm/kvm-all.c:3219`). If KVM
   ever reports this class of fault as `KVM_EXIT_MEMORY_FAULT` — the plumbing exists and QEMU
   already tolerates the combination — it becomes the best mechanism in the study.
7. **Every absolute number is from a nested-virt bench** and should be read as an upper bound
   (2–5×, estimated). Every decision here rests on a ratio or on a rate compared against the
   design's own declared band.
8. **Multi-process contention on one region is unmeasured**, which is the condition §6.8.1's
   mutex ruling is conditioned on. Its acceptance criteria for an RW upgrade are unchanged and
   still unmet. ★ Note the ruling is now *cheaper* to keep: with no disarm edge, the RW
   variant's slip-through hazard has no edge to live on — but the criteria stand, because the
   *reason* for a plain mutex (measured contention, not intuition) is unchanged.
9. **The store-atomicity loss (GL12) was demonstrated but not exploited.** `lock xchg` was
   measured arriving as an ordinary MMIO access; a two-vCPU race observing a torn RMW was not
   constructed. If a proposed `LockPath` page turns out to carry a guest-side atomic, that
   experiment becomes mandatory.
10. **What this design does NOT claim.** It does not make guest-writable memory authoritative —
    §2.1's rule stands unchanged, and a region hold is taken **at** a Layer-2 edge, never
    instead of one.

---

## 9. The normative rules, collected

| # | rule |
|---|---|
| **GL1** | Copy-once-then-validate stays the **primary** layer; the region lock is the exception, taken only where a decision needs stability across a read-then-act. |
| **GL2** | The lock/no-lock decision is made on the guest's **write RATE** to the region, never on the lock's per-op cost. |
| **GL3** | Every region declares a `PageClass` at registration; `acquire` on a non-`LockPath` class is a **loud fault**, never a silent no-op. |
| **GL4** | Isolate-shared and DMA-target ranges are **unlockable by construction** and are refused at registration. |
| **GL5** | The protocol is **acquire → act → release**, on one thread, with every other lock of ours dropped. There is no arm and no disarm. |
| **GL6** | The region mutex is **rank 3, acquired only from a lock-free context**, and **no syscall** may be issued beneath it. |
| **GL7** | "The mutex is held" is asserted on the **reader** side; a guest write arriving with the mutex unheld is the **steady state** and is applied, never asserted against. |
| **GL8** | Write resolution **never depends on the core**: apply first, notify second, never park a vCPU behind the executor. |
| **GL9** | At realize, **prove the region traps** with a functional write round trip, and refuse loudly otherwise; a read-only region wired to no callback **discards** guest writes silently. |
| **GL10** | **Passthrough does not exist.** The shipped mechanism has one mode; there is no unarmed state whose failure could be mistaken for correctness. |
| **GL11** | A region is registered `LockPath` only with a **written argument** that the decision cannot be restructured as copy → validate → revalidate → act. |
| **GL12** | A `LockPath` region **must not carry guest-side atomics**, and no guest-visible invariant may depend on wide-store atomicity into it. Declared at registration. |
| **GL13** | **On arm64 the region lock does not exist**; `LockPath` registration is refused at realize. |

---

## See also

- **`../reference/region_lock_mechanism_study.md`** — the candidate study this revision comes
  from: thirteen mechanisms, the measurements, the acceptance table, and what it does not
  establish. **Normative over this file on questions of fact.**
- `core_security_threat_model.md` §2.1 — the two-layer trust model. **Normative over this
  file.** Layer 1 is what this document builds; Layer 2 is why building it is not sufficient.
- `l1_os_shell.md` §6.7 (memslot strategy — and §1.0 of this file corrects its cost model),
  **§6.8** (why the capability left the `Vmm` trait — and §5 here explains what changes and
  what does not), **§6.8.1** (the plain-mutex ruling), §4.2.2 (the read split), §4.3 (volatile
  vs atomic), §4.5 (the lockwitness and the `*_under_lock` set), §6.6 (I-NOAMP).
- `../reference/qemu_102_facilities.md` row 1 (lockless IO — a hard precondition here), row 7
  (the RO-memslot reversal this file's §1.0 completes), §11.1 (the KVM-direct-vs-QEMU rule).
- `../reference/qemu_bql_spike.md` §6 — the only **[QEMU]**-layer number this design quotes.
- `l1_concurrency.md` §3.3 — R1 / R3 / R5, the invariants GL5 and GL6 are written to satisfy.
- `testing_doctrine.md` §7 (first-class fallback + randomised toggling — written from this
  feature, now owed only if §7.3 is built), §1 (non-vacuity), §2 (exact variants).
- `portability_arm64.md` — GL13 is the first hard architecture split in the design.
- `../reference/memory_cacheability.md` §1 — the same "who owns the PTE" logic that makes a
  device mapping unlockable.
