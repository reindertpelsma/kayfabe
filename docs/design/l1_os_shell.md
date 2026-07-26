# L1-M2 — the OS shell: reactor, raw module, VMM seam, and the reclamation lifecycle (decision #38)

**Status:** design for review, 2026-07-25 · Written **before any L1-M2 code**, per the
standing method (design-first on the highest-risk seam; decision #34 did this for L1-M1
and §12 of `l1_concurrency.md` is the receipt). Scope: the **OS half** of L1 — the real
reactor loop under the core's pure completion-source port, the one audited `unsafe`
module, the real `Vmm`, and — elevated by the owner to a co-equal pillar — **cancellation
and the whole-lifecycle reclamation invariant**.

**Why a separate doc rather than a §13 of `l1_concurrency.md`:** that doc is the design of
*record for concurrency* — threads, locks, completion flow — and its §12 contact log is
now the most-cited artefact in the repo. This doc's subject is orthogonal and larger: it
is about **what happens when the thing underneath is a real kernel** — descriptors,
page sizes, un-killable threads, and resources that must be given back. Folding it in
would bury both. `l1_os_shell.md` is named for the layer, not the milestone, so it does
not rot when L1-M3 exists. Cross-references are two-way: every amendment this doc makes to
`l1_concurrency.md`'s normative text is marked **★ AMENDS §x** and must be applied there
when the corresponding stage lands.

**Companion docs:** `l1_concurrency.md` (§1 inherited law, §3.3 R1–R5, §5.4 the interrupt
seam this doc builds, §6 the reactor port, §7.2/§7.3 the pool, §9 the contract table, §12
the contact log), `core_state_and_consolidation.md` (the port surface), `execution_plane.md`
(§2.4 completion patterns a–e), `portability_arm64.md` (§"L1 — the one real pressure
point": the binding page-size rule this doc must *enforce*, not restate),
`core_security_threat_model.md` (§1: memory-safety/breakout is deferred to "when L1 is
written" — it is being written now), `multi_gpu_and_mig.md` (MG-5/MG-6: per-`(Proc, GpuId)`
isolates and per-target windows, which is also the reclamation geometry). C-repo cites are
prefixed `C:`.

**★ This doc is written against the POST-FIX core.** A parallel read-only audit of the
core's teardown/reclamation completeness (2026-07-25) found that reclamation could not be
*designed* against today's signatures, and four fixes are landing in the core now (G1, G4,
G3b, G3 — §0.1). Everything below assumes them. Where a design here needs a shape the fixes
do not provide, it says so explicitly rather than assuming today's signatures.

**★ File ownership.** This doc is the only file this design touches. The amendments it
implies for `l1_concurrency.md` (now **seven**) are collected — not applied — in **§15**,
because that file is being appended to concurrently by the implementation work.

**How to read this doc:** §0 is the new failure ledger — what a mock structurally cannot
prove — and §0.1 is the core-side precondition set. §1 restates the inherited law as L1-M2
obligations. §2 is the architecture in one picture. §3–§7 are the five decisions, each
recommendation → rationale → alternatives → what's open. §7 is the big one (cancellation +
lifecycle + the conservation ledger). §8–§9 are contracts and testing. §10 is the staged
plan. §11 is security. §12 is the ledger of decisions. §13 is the honesty section. §14 is
the contact log (§14.1 = M2-a's findings; §14.2 = the owner-directed amendment round of
2026-07-26, which added §4.1.1, §4.2.1, §6.3's whole-surface BQL contract, §6.6 and §6.7).
§15 is the proposed amendments to `l1_concurrency.md`.

---

## 0. Why this doc exists — the failure ledger of *contact with an OS*

`l1_concurrency.md` §0 tabulates the C's threading bugs (F1–F6) and turns each into a
rule. This doc needs its own table, because L1-M1's whole achievement was building a layer
that **never touches an OS**, and every class below is invisible to that layer *by
construction*. The unifying sentence:

> **A mock never blocks, never leaks, never dies, and never recycles a name. A kernel does
> all four.**

| # | Class | Why L1-M1 could not see it | The L1-M2 rule it produces |
|---|---|---|---|
| **O1** | **The mock never blocks.** §12.6 recorded this exactly once, for `RmBackend`: *"stage 2's backends are mocks that never block: with a real host verb it would be a live R1 violation with no assert firing."* Stage 3 fixed it **for `RmBackend` only**. | `MockVmm` is a `BTreeMap`. `map_guest` is an insert. Nothing in the suite can distinguish a memcpy from `KVM_SET_USER_MEMORY_REGION`. | R1 gets its **second** reckoning: the syscall-shaped `Vmm` methods are classified in-lock-illegal and the assert moves to the syscall itself (§6.2). |
| **O2** | **The mock never leaks.** A `HostHandle` that is never freed costs a `BTreeSet` entry. | No test asserts release, only refusal. `worker_death_retires_the_proc_loudly` was green *only because it never issued an `apply` afterwards* (§12.13) — isolated tests test what you thought of. | **Conservation of host resources** as a ledger property, composed into the mean run (§7.8). |
| **O3** | **The kernel recycles fd numbers; we recycle nothing.** The core's `CompletionSource` mint is monotonic and never reused — a deliberate anti-ABA property. The moment L1 keys readiness on an **fd number**, the ABA class walks back in through the adapter. | There are no fds in L1-M1. | The reactor keys epoll on the **`CompletionSource` value, never the fd** (§3.2), and the deregister→close order is a stated rule (§3.3). |
| **O4** | **Level-triggered readiness spins.** An armed nvidia os-event fd stays readable until the event is *consumed by an RM ioctl* — `C: mode1_poll_relay_plan` says so in as many words ("Avoid a busy spin… suppress re-notify for that handle until the guest re-polls"). A naive `epoll_wait` over it returns instantly, forever. | Mock signals are function calls. | F1 (**every poll must be provably bounded** — `l1_concurrency.md` §4.2) is rebuilt at the epoll layer: **every source the reactor watches is a counter-shaped primitive the loop drains**, and the loop's wake count is *asserted* against the signal count (§3.4). That assertion IS the bound — one wake per signal — which is why the rule is stated as a quantity rather than as "no spinning": a level-triggered fd that never stops being readable fails a wake-count assertion loudly, while it satisfies any absence-shaped rule vacuously. |
| **O5** | **The host page size is not 4096.** arm64 hosts run 16 KiB / 64 KiB base pages. | No mmap exists. `portability_arm64.md` states the rule and admits it is only a document: *"This doc is the record until the L1 mmap design exists to carry the rule."* This is that design. | The page size becomes a **type and a test axis**, not a constant (§5). |
| **O6** | **A host thread can be un-killable.** D-state ioctls are the C's proven wedge class (F5); §11 B3 is the open owner decision about them. | A mock verb returns. | Cancellation is **advisory**, the reply is never abandoned *unless the slot is condemned in the same act*, and a two-stage watchdog converts unbounded stall into bounded, loud failure (§7.5). |
| **O7** | **A real VMM has its own global lock and its own syscalls.** QEMU's BQL is a lock we do not rank; `msix_notify` takes it; memslot updates take it and the KVM SRCU. | `MockVmm::raise_irq` pushes to a `Vec`. | `raise_irq` is contractually **irqfd-shaped** — implementable with no VMM-global lock — and the BQL inversion is named before it can happen (§6.3). |
| **O8** | **Real memory is written concurrently by hardware.** Semaphores and USERD are written by the GPU while we read them; guest RAM is written by the guest while we parse it. | `Vec<u8>` is not concurrently mutated. | The raw module hands out **no borrow into shared memory, ever** — copy-then-parse (kills double-fetch) and aligned ≤8-byte accesses only (kills tearing) — §4.3. |

The core turned the C's *logical* bug families into structure. L1-M1 turned the
*concurrency* families into asserted invariants. What is left — and what this doc designs
— is the part only an OS layer can get wrong: **who owns a descriptor, who blocks in a
syscall, and who gives the memory back.**

### 0.1 ★ The core-side preconditions (the reclamation audit, G1–G10)

A read-only audit of the core's reclamation completeness landed while this doc was being
written, and its verdict reframes the milestone: **reclamation could not be designed
against today's core signatures, because several of them make the correct reclaim
unwritable.** Four fixes land in the core first; this doc designs against them and treats
the rest as first-class lifecycle work.

| # | Finding | What it means for this doc |
|---|---|---|
| **G1** | A published backing's host *memory* handle is unrecoverable: `commit_publish` stores only `host_va` into `Binding`, so a reclaim path can unmap but never `free(memory)`. | **Precondition.** `Binding` carries the backing identity; every reclaim call site in §7 keys off it. Without it, §7.8's ledger cannot balance even in principle. |
| **G4** | No cancellation vocabulary (`RmError::Interrupted`, `FwdFault::Cancelled` absent), and `Worker::execute` returns a bare `Err(RmError)` — so if the worker **died mid-chain** the internal unwind cannot run and everything already allocated is in no `Orphans` and in no core state. | **Precondition.** `execute -> Result<VerbReply, VerbFailure>` with `VerbFailure { err, orphans }`. This *corrects* §7.4 below: cancellation does **not** inherit all-or-nothing "for free". |
| **G3b** | `reap_retired(&mut self) -> usize` drops procs in place, under the caller's rank-0 write lock — so a real isolate's `Drop` (waitpid + namespace teardown) blocks under a lock, a live R1 violation with **no assert**, because `assert_lock_free` guards verbs, not drops. | **Precondition, and a shape rule for all of §7:** every reclamation path **returns the reclaimed objects to be dropped lock-free**. §7.6 is written that way throughout. |
| **G3** | "Quiesce" is undefined and unchecked: the reap can legally run while a `Box<dyn RmBackend>` is checked out on a foreign thread with all locks released ⇒ the isolate is torn down under a live connection and the orphan disposal runs against a dead sandbox. | **Precondition.** `Isolate::is_quiesced()`; a non-quiesced proc goes *back* on the retired list. §7.6 T3's predicate is built on it. |
| **G2** | `refresh` drops **live** host state with no release: `p.vases.retain(…)` and `p.channels.retain(…)` discard `host_vas` / bound `host_va`s / `host_channel` / `host_engine_objects` while **the proc is still alive**. Guest-reachable. | **★ A first-class lifecycle trigger this doc did not have.** It is the one path where §7.0's "the process boundary is the garbage collector" backstop **does not apply** — nothing dies. New trigger **T0** (§7.6). |
| **G5** | There is no device reset: no reset event, no `Spine::reset`. On `rmmod`/`modprobe`, guest panic, or VM reset the graph, live `Proc`s, `condemned`, `retired`, `sources`, routing maps and mints all survive into the new driver life, and the new life derives a second set of components beside the corpses. The C's WPR2 limitation is **inherited by omission**. | **Promotes T4 from "wire the trigger" to "build the mechanism".** `Spine::device_reset(...) -> Vec<Proc>` (G3b's shape), clearing graph + condemned while **keeping the mints monotone** — which is exactly the property §7.7 leans on. |
| **G6** | ★ **FIXED** (`l1_concurrency.md` §12.20). `GpaArena` was bump-only: no intra-proc free, so a long-lived map/unmap process exhausted its arena (measured: cycle 128 on 512 KiB). | Now a coalescing free list plus a move-only `GpaBlock` token (`free` by value ⇒ a double free does not compile) and `kayfabe_fwd::unpublish_backing`, which returns the GPA *with* the host `Orphans`. A free list driven by declared graph facts, not a collector. The residual case — a `Vas` dropped by `refresh` — is named and deferred with the host-side reclaim it must travel with. |
| **G7** | ★ **FIXED** (§12.19). The window check was a `debug_assert!` (compiled out in release) and a missing target silently dropped the arena. | Now a loud `Result<(), ForeignArena>` that hands the arena back; arenas carry their owning target, so `reap_retired` routes home by the arena's **own** owner and there is no key to get wrong; `GpaArena` lost `Clone`; the unroutable arena is reported on `Reclaimed::orphaned()`. Under disjoint windows the symptom is a leak + cross-aperture GPAs; the two-live-procs overlap needs a non-disjoint (MIG-shaped) geometry, and the guard is now independent of the geometry. |
| **G8** | `SourceKind::OsEvent` carries no channel/`Vas` identity, so per-channel deregistration cannot be written. | Bites T0 and T5 (a freed channel's sources must go without retiring the proc). Free now, a migration later — recommend now (§3.8). |
| **G9** | ★ **FIXED** (§12.21). `GpuId` derived from a **guest-supplied** `device_instance` and first touch minted a window + `DeliveryPlane`, uncapped and unvalidated. | Capped to the **entitlement** (`Gpu::realize`'s roster), refused at the `Device` alloc as `RmGraphError::InvalidDeviceInstance` — RM's `NV_ERR_INVALID_CLASS`. Deliberately not `NV_MAX_DEVICES`: RM already bounds the field to `< 32`, so that cap is theatre. The `unwrap_or(0)` default-to-GPU-0 guess went with it. |
| **G10** | ★ **FIXED** (§12.22), and worse than reported: the *carry-forward* was O(n² log n) per apply (55 s at the cap), not merely the scan. | Named caps on both lists; the scan is an index and the carry-forward is union-find (55 s → 3.8 s). The refusal lands on deriving a **new `Proc`** — never on the condemnation, which would un-condemn a component whose isolate is already dead. |

**Doc contradictions the audit surfaced that this doc must not repeat:**
`core_state_and_consolidation.md` §4's "eager host-side reclaim is fine for correctness,
wire later" is false in both clauses (G1/G2). `kayfabe-fwd`'s "both dispositions are
decided, neither is a leak" misses that the namespace dies at **reap**, not at `retire()`,
and that a mid-chain worker death is an unnamed third disposition (G4). `gpa.rs`'s "safe by
construction" is a claim about an adapter `Drop` the core neither performs nor checks.
`kayfabe-gsp`'s "resettable in-process" is true of the GSP FSM and false of the `Spine`
(G5) — which matters, because §7.6 T4 leans on that sentence.

### 0.2 ★ Ground truth — where each behavioural claim comes from

Standing rule: **do not invent NVIDIA/RM/GSP behaviour.** The C artifact at
`/workspace/nvidia-gpu-passthrough` is a *working* Mode-2 implementation that ran real
CUDA, PyTorch and a 7B LLM on real GA106 at host parity; where this doc asserts what the
guest driver does, the citation is to the C's code or its measured notes, and where the C
does not settle it, this doc says **OPEN QUESTION (bench experiment)** rather than guessing.

**★ Two limits on that oracle, both measured after this table was first written, both
load-bearing for how citations here should be read:**

1. **The C is a SINGLE-PROCESS Mode-2 oracle.** It runs exactly one CUDA process per QEMU
   lifetime; the second fails `cuInit` → 999 regardless of how the first ended
   (`../reference/mode2_bench_lifecycle.md` §1). The parity results above are unaffected (they
   are in-process), but **no citation here may be read as "this is what the C does with two
   Mode-2 processes"** — nobody has observed that. Mode-1, with its per-`mm` isolates and 22
   real apps at host parity, remains a valid multi-process oracle and is the one to use.
2. **Several rows below cite the C's *comments*.** Two of them were checked against the C's
   *behaviour* and turned out to be false. A comment in the C is a strong prior, not a
   measurement; `../reference/mode2_bench_lifecycle.md` is where the difference gets recorded.

| Claim used below | Source |
|---|---|
| ~~the guest emits `UNLOADING_GUEST_DRIVER` (RPC fn 47) on **both** a real driver unload **and** a GPU-idle release~~ ★ **FALSIFIED (bench, 2026-07-26).** `rmmod` emits **no** fn-47 — the idle release at process exit already consumed it. The C's comment claiming "TWO distinct triggers" is factually wrong, so there is no second RPC to disambiguate and a reset armed only on fn-47 never runs on a true driver restart | comment: `C: src/qemu/nvkvm_gpu_emul.c:2452–2456`; measurement + consequences: `../reference/mode2_bench_lifecycle.md` §2 |
| **the GSP queue re-handshake IS the quiesce point** — "the re-handshake = the quiesced point (GPU was idle-released; next context boots)" — and the C's deferred reap runs exactly there | `C: nvkvm_gpu_emul.c:3458–3460` (`nvkvm_m2_reap_dead`), `:1988–1994`, `:2123` |
| reaping resolution/backing state **at** the client-root free hangs the dying context's residual polls (bench-proven, `cupctx2_min` CTX2 destroy) — hence deferral, not eagerness | `C: nvkvm_gpu_emul.c:1988–1993` |
| what must be reset at fn-47: WPR2 down + `gsp_suspended` + `bootargs_dumped` + `q_ready`; **not** the queue counters | `C: nvkvm_gpu_emul.c:2471–2477`, with the failure mode for each stated in place |
| at the re-handshake: reset the status-queue **write position only**, and **PRESERVE the seqNums**, because the driver's `MESSAGE_QUEUE_INFO` is built in `kgspConstructEngine` and destroyed only in `kgspDestruct` (module unload) — *not* on idle-release — while the per-boot `GspStatusQueueInit`→`msgqRxLink` resets only `rxReadPtr` | `C: nvkvm_gpu_emul.c:3462–3487` (#12 L3, 2026-06-20), citing `message_queue_cpu.c:762,768` |
| SEC2 Booter Load raises WPR2 and Booter Unload lowers it; a post-teardown STARTCPU must be disambiguated from a genuine re-boot or the 2nd context hangs forever waiting for a `GSP_INIT_DONE` that never comes | `C: nvkvm_gpu_emul.c:4208–4262`, citing `kernel_gsp_booter_tu102.c` / `osinit.c:2363` |
| host-side reclaim (#80): a per-VM sparse-window **free list, first-fit + tail/adjacent coalesce**, freed from `MUNMAP_ON_ISOLATE` *and* from the kill reaper; plus a session destroy that force-closes host fds and releases RM objects | `C: src/qemu/nvkvm_mmap_host.c:172`, `nvkvm_isolate_handlers.c:1954/2009/2054/2431/2439`, `C: teardown_hardening_done` |
| a guest that goes silent without `KILL_ISOLATE` was the C's **residual** — bounded to its own per-VM QEMU and fully reclaimed at VM stop (process exit) | `C: teardown_hardening_done` ("Residual") |
| signal-interruptible forwarded ioctls (#73): no-`SA_RESTART` handler, per-txn tid safeguard, **never abandon the reply buffer**; the C's "~3.4–3.5 s bounded EINTR-unwind measured on RTX 3060" is **re-read by `l1_concurrency.md` §12.26 as RM's own 4 s RPC timeout elapsing**, not an unwind — RM's waits are uninterruptible (`ogkm: .../gpu/gsp/kernel_gsp.c:2963-3060`, `.../resserv/src/rs_server.c:3164-3168`) | `C: docs/design/signal_interrupt_delivery.md`, `C: signal_interrupt_delivery_done`, `ogkm: .../os/os.c:2136-2139` |
| the os-event relay must extend the stub's existing wait set and must **not** re-notify until the guest re-polls (busy-spin hazard) | `C: docs/design/mode1_poll_relay_plan.md` |

★ **Every `ogkm:` citation above is consolidated, with its driver-version caveat, in
`../reference/rm_semantics_measured.md`** — cite that file when a design needs the fact, so a
wrong fact gets corrected in one place instead of five. The C-behaviour rows have the same
treatment in `../reference/mode2_bench_lifecycle.md`.

**What the audit confirms is sound, and this doc builds on rather than re-litigates:**
`CompletionSource`'s monotone never-reused mint (§3.2, §7.7); `deregister_proc`/`belongs_to`
covering both ends of a seam; the condemned-component mechanism and its client-set key
(§7.5); `Refusal.retry`'s converging/divergent split — **cancellation slots in as a third
shape** (non-retryable, orphan-carrying), which is evidence the abstraction was cut in the
right place; the `Worker` ownership shape (checkout moves the backend out), which is the
only reason G3 is fixable at all; and per-`(Proc, GpuId)` arenas + isolates disjoint by
construction.

---

## 1. Inherited law — restated as L1-M2 obligations

Everything in `l1_concurrency.md` §1 still binds. Four items acquire teeth here, plus one
new law:

1. **Law 9 (the reactor loop touches ZERO core state)** stops being a discipline and
   becomes a *type* fact: the loop thread owns an epoll descriptor and an `InboxSender`,
   and `InboxSender` structurally holds nothing but the queue (`inbox.rs` already encodes
   this). §3.2 goes further — the loop needs **no table at all** on its hot path, so there
   is not even a shared structure to be tempted by.
2. **Law 10 (purity + `forbid(unsafe_code)`)** becomes a build-system fact: exactly one
   crate omits `[lints] workspace = true`, and CI asserts that it is exactly one and names
   it (§4.1). The §6.2 vocabulary gate — `eventfd|epoll|timerfd|rawfd|libc|O_NONBLOCK`
   absent from the **11 pure crates**, in code *and comments* — is not weakened, not
   reworded, and not scoped down by one crate. Every mechanism in this doc lives on the
   `kayfabe-rt` / `kayfabe-linux-raw` side of it, and where the core needs a new concept
   (§3.5's isolate-exit source class) it is named in the core's own abstract vocabulary.
3. **Law 8 (retire-eager / reap-deferred)** is found to be **half a law**: L1-M1 has the
   deferral and no bound. A deferral with no bound is a leak wearing a lesson's clothes.
   §7.6 gives the quiesce point a predicate, a re-arm policy, and an escalation.
4. **Law 2 (MISS = FAULT)** extends to descriptors and to lifetimes: a stale source, a
   stale reply, a stale mapping, and a verb that outlives its device are all loud faults,
   and — critically — they are loud *because every identity in the system is a
   never-recycled mint*. §7.7 shows that this one property is what makes device reset
   correct without inventing an epoch counter.
5. **★ NEW — law 11: conservation.** *Every host resource acquired on behalf of a guest is
   released exactly once — not zero times, not twice — on every path, including the violent
   ones.* This is the owner's elevated requirement, stated as an invariant so it can be
   tested rather than reviewed (§7.8).

---

## 2. The architecture, in one picture

L1-M1's picture (`l1_concurrency.md` §2) is unchanged above the dashed line. Everything
below it is new.

```text
 vCPU threads (N)                executor thread            reactor loop thread
 ────────────────                ───────────────            ───────────────────
 trap → device lock (rk0)        drains the inbox;          epoll_wait over:
      → proc lock  (rk1)         runs dispatch +              · per-os-event counters
      → plan / checkout          Device::event under           · per-worker channel HUPs
      ↓ (ALL LOCKS DROPPED)      the SAME locks + R1–R5        · per-isolate exit fds
   verb round-trip on the           │                          · the notify fd
   checked-out worker               │                          · the deadline timer
      ↓                             │                        maps  data.u64 → CompletionSource
   re-lock, RE-VALIDATE (R5),       │                        (NO table, NO core state)
   commit, check in                 │                        pushes CoreEvent, drains counter
 ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┼ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
                                    │
   POST-ENTRY DRAIN (§3.6): after every core entry, with ZERO locks held, the caller
   discharges three latches with bounded non-blocking syscalls —
        pending WAKE   → write the notify fd
        pending TIMER  → re-arm the deadline
        pending CANCEL → signal the worker(s) named (§7.1)
   ...which is why NO other syscall is ever made under a lock (§6.2).

   kayfabe-linux-raw  — the ONE unsafe crate   (unsafe ONLY in *_unsafe.rs — §4.1.1)
   ─────────────────────────────────────────
   HostPageSize::query()            mmap/munmap, Reservation + map_fixed_in + restore
   geometry(page_size) : pure       MappedRegion  (copy in/out; NO borrow escapes)
   Epoll/EventFd/TimerFd/PidFd      VolatileRegion(aligned ≤8B atomic views only)
   KVM ioctls (harness backend)     every entry asserts the lockwitness (§4.5)
   ★ NO host CPU address leaves this crate, in any representation (§4.2.1)
   ★ memslots at WINDOW granularity only; publication = MAP_FIXED inside one (§6.7)

   isolate process (1 per (Proc, GpuId))       ★ the reclamation boundary
   ────────────────────────────────────────────────────────────────────
   N workers, each 1-deep req/reply + a control pipe (cancel)
   1 relay thread: polls the host os-event fds, writes our counters (§3.5)
   1 pidfd held by us: readable on exit → a reactor source → GC (§7.6)
   ★ the process's death frees its ENTIRE RM object tree — the kernel is the
     garbage collector of last resort, and that is what makes §7 tractable.
```

Three threads per device (vCPUs excluded), one crate of `unsafe`, one place where wall
time exists, one place where a descriptor is closed.

---

## 3. Decision 8 — the real reactor loop

The core owns the model (`kayfabe_core::reactor`: opaque `CompletionSource`,
`SourceRegistry`, `Dispatch`, `WakeRequest`). L1 owns everything below.

### 3.1 The source primitives

| Core `SourceKind` | Host primitive | Signalled by |
|---|---|---|
| `OsEvent{proc,gpu,ev}` | one **counter fd** we create, one per armed guest os-event | the isolate's relay thread (§3.5) |
| `Worker{proc,gpu,worker}` | the worker's control channel; readiness means **HUP** | kernel, on worker death |
| `CrossIsolate{from,to}` | a counter fd (unwired; the class exists so landing it is a list entry) | future |
| `Notify` | one counter fd per device | any thread, via the post-entry drain |
| **★ NEW `IsolateExit{proc,gpu}`** | a **process descriptor**, readable on exit | kernel |
| *(not a source)* deadline timer | one timer fd per device | kernel |

`IsolateExit` is the one core-side addition this decision requires: a new `SourceKind`
variant, a new `Dispatch::IsolateExited`, and a match arm. That is exactly what §6.1
promised ("a source *class* now precisely so that landing it is a match arm, not a
reshaping of the port") and it is the first time the promise is cashed. It costs the core
nothing impure — the core learns that "an isolate ended", not what a process descriptor is.

**Why a process descriptor and not `SIGCHLD`:** we do not own the process's signal
disposition. In the QEMU adapter we are a plugin inside a process that already handles
`SIGCHLD` for its own children; installing a competing handler is a fight we would lose
intermittently and debug forever. A per-child descriptor slots into the model we already
have, is race-free against pid reuse (the classic `waitpid` footgun), and is epollable.
Honest cost: it does **not** reap the zombie — an explicit `waitpid` still runs, on the
executor, at the point the exit is dispatched (§7.6). Requires Linux ≥ 5.3; that is inside
the target envelope and is stated as a floor, not assumed.

### 3.2 ★ The key mechanic: the loop needs no table

The naive shell keeps a `HashMap<fd, CompletionSource>` and looks up every readiness. Do
not build it. The readiness API carries **64 bits of caller-chosen data per registration**,
and `CompletionSource` is a `u64` from a monotonic, never-recycled mint. So:

> **Register the source with `data = the CompletionSource's value`. The loop's "table
> lookup" is the identity function.**

Three consequences, all load-bearing:

1. **O3 dies.** A descriptor number can be recycled by the kernel the instant it is closed;
   a `CompletionSource` never can. Keying readiness on the descriptor would let a *new*
   source inherit a *stale* readiness report — the C's F4 use-after-retire, rebuilt in the
   adapter, under a design that had explicitly killed it in the core. Keying on the handle
   means a stale report resolves to a deregistered handle and dispatch answers
   `SourceFault`: loud, mutation-free, already implemented, already tested.
2. **Law 9 becomes structural, not merely enforced.** The loop holds an epoll descriptor
   and an `InboxSender`. It has no map, no lock, no `Arc<SharedDevice>` — there is
   *nothing shared* it could touch. The reactor loop is ~40 lines with zero shared mutable
   state, which is the smallest the §8.1 thin-waist rule has ever managed for a thread.
3. **A reverse table is still needed — but only for teardown** (`CompletionSource` →
   descriptor, so deregistration can remove and close). It lives with the *registrar*
   (a rank-2 leaf structure beside the inbox), is touched at arm/disarm only, and the loop
   never sees it.

### 3.3 Descriptor lifetime — the two rules

- **Deregister then close, never close then deregister.** Closing a descriptor removes it
  from the readiness set *only if it was the last reference*; a duplicated one silently
  stays and keeps firing. The order is a rule, not an optimisation.
- **A readiness report in flight across a deregistration is normal and safe.** The loop may
  hold a batch of reports naming a handle that the executor deregistered microseconds ago;
  it pushes them anyway, and dispatch faults. `SourceFault` is not an error path we are
  tolerating — it is the *designed* answer, and §12.13's condemnation work already proved
  the shape (a signal after retire "resolves to nothing rather than to dead state").

### 3.4 Level vs edge — the F1 gate, rebuilt

Class O4 is where a busy-poll is *accidentally* reintroduced. Three options:

- **(a) Edge-triggered.** Fires once per readiness transition. Risky in exactly the way
  that matters: whether a second arrival while already-readable produces a second edge is a
  per-source-type property, and a missed edge is a hang, not a slowdown.
- **(b) One-shot + explicit re-arm.** Correct, and it makes the re-arm a testable core-driven
  edge — but a forgotten re-arm is a lost wakeup, i.e. the F3 hang class we exist to prevent,
  and it costs one syscall per completion.
- **(c) ★ Level-triggered over primitives the loop *drains*.** Recommended. Every source
  the reactor watches is a **counter** we own (§3.5 makes even the nvidia os-event one),
  the loop reads it (which resets it to zero), and readiness therefore self-clears. No
  re-arm to forget, no edge to miss, no spin.

**The coalescing question, answered exactly.** A counter coalesces: N writes then one read
returns N. The core's `Dispatch::Observe{ev}` is *per event*. Coalescing would silently
drop completions — except that `SourceKind::OsEvent{proc, gpu, ev}` binds **one source to
one `OsEventRef`**, so all N firings are the same event and the count is exactly how many
`SourceSignal`s to push. One source == one guest os-event == one counter. The mismatch is
resolved by the port's own keying, not by a convention.

**The F1 gate, as an assert rather than a hope.** The loop counts its own wakes. A test
drives K signals and asserts the loop woke exactly K times (± the notify wakes it was told
about). A level-triggered spin shows up as an unbounded wake count — structurally, with no
timing dependence, on any machine. This is the same trick as the mean test's
progress-under-pending: measure the *structure*, never the clock.

### 3.5 ★ Who holds the nvidia descriptor: the isolate relays

The guest's blocking-sync path waits on an `NV01_EVENT_OS_EVENT`, whose readiness lives on
a real nvidia descriptor. Two ways to get it into our reactor:

- **(a) Pass the descriptor to us** (over the isolate's control channel). One hop fewer.
  But it puts an nvidia descriptor — an ioctl-capable capability — into the process that
  parses hostile guest bytes. That is the security posture *inverted*: the threat model's
  load-bearing claim is that "unprivilege, not keying, is the host security boundary", and
  the isolate exists precisely so RM capability lives only inside the sandbox.
- **(b) ★ The isolate relays.** Recommended. A relay thread inside the isolate waits on its
  own nvidia descriptors and writes our counter when one fires. Every nvidia descriptor
  stays inside the sandbox; the reactor watches only descriptors *we* created, which is
  also what makes §3.4(c)'s drain semantics ours to define. This is the shape the C
  actually built and validated (`C: mode1_poll_relay_plan`: stub extends its own wait set,
  sends `isolate_resp_poll_event`; the QEMU side pushes it to the guest).

Honest costs of (b): one extra thread per isolate and one hop (~µs, off every hot path);
and the relay thread is itself a thing that must be shut down cleanly (§7.6 covers it — it
dies with the process, which is the whole reclamation argument). The C's second-order
lesson also carries over verbatim: **the relay must never be a polling producer.** The C's
`m2_poll_kick` doorbell-replay is the anti-pattern; the relay blocks in a wait, always.

### 3.6 The wake — and what §6.1's "re-read the set" actually becomes

**★ AMENDS `l1_concurrency.md` §6.1.** That section models the reactor as *"the main loop
joins whatever the list currently holds… signal the notifiable source, the loop re-joins."*
That is a `poll(2)`-shaped model: the set is a userspace array the loop re-reads. With an
epoll-shaped set the set lives **in the kernel**, registration from another thread is
immediately effective against a concurrent wait, and *no wake is needed to add a source at
all*. The wake's real jobs are narrower and should be named:

1. a **removal barrier** (make the loop finish its current batch so a deregistered
   descriptor can be closed safely);
2. **injected work** and shutdown;
3. **timer re-arm** when the earliest deadline moves.

Keep the `WakeRequest`/latch API exactly as the core has it — it is conservative and
correct, and a spurious wake costs one loop iteration. Only the *meaning* changes, and it
must be written down, because "signal the loop so it re-reads the set" would send an
implementer to build the array the kernel already has.

**The post-entry drain.** `defer` must not call a timer syscall while a lock is held, and
`register_source` must not call a registration syscall while a lock is held (§6.2 forbids
all in-lock syscalls). So all three latches — wake, timer, cancel — are set under the lock
as **plain data** and discharged after the guards drop:

```text
    let receipt = device.some_core_entry(...);   // locks taken and released inside
    shell.discharge(receipt);                    // zero locks held; bounded syscalls
```

`take_pending_wake()` already exists and already has this exact shape; timer and cancel
join it. The receipt is `#[must_use]`, which is the trick the core already uses for
`WakeRequest` and which the workspace's `-D warnings` turns into a compile error. **One
mechanism, three needs, and the pattern was already invented — that is the argument for
it.**

### 3.7 The executor is its own thread — and `ExecutorWaker` is why L2 is not a retrofit

The loop cannot also drain the inbox: draining runs `dispatch` and `Device::event`, which
is core state, which law 9 forbids from the loop thread. So the executor is a second
thread parked on the inbox. Cost: one futex wake per completion. Accepted — the alternative
is deleting the one law that keeps the readiness path unable to corrupt anything.

But in QEMU the executor is *not our thread*: it is the main loop / bottom-half context.
So the inbox's wake is abstracted **now**, not later:

```text
    trait ExecutorWaker: Send + Sync { fn wake(&self); }
      · harness impl:  condvar notify
      · QEMU impl:     schedule a bottom half
```

Four lines, and it is the difference between L2 being an adapter and L2 being a re-plumb of
the inbox. This is the cheapest anti-bolt-on in the doc; take it.

### 3.8 ★ The source cap — where a documented deferral comes due

`reactor.rs` states its own deferral honestly: *"there is no cap on the number of
registered sources. Nothing guest-reachable registers one today… When arming becomes
guest-driven it needs the `MAX_OUTSTANDING_COMPLETIONS` treatment — a named constant and a
loud refusal — and `register` grows a `Result`."*

**L1-M2 is when arming becomes guest-driven.** One armed os-event now costs: a core
registry entry, a descriptor in our process, a descriptor and a wait-set slot in the
isolate. A hostile guest arming in a loop exhausts `RLIMIT_NOFILE` in the *QEMU* process —
which is not a contained refusal, it is a device-wide DoS (I4 violated) and quite possibly
a QEMU-wide one. So the cap lands in this milestone, in both places:

- **core:** `SourceRegistry::register` grows a `Result`, with a per-`(proc, gpu)` bound and
  a device-global bound, both named constants, refusal loud and contained (the guest's arm
  fails; nothing else notices);
- **shell:** the descriptor budget is derived from the *actual* `RLIMIT_NOFILE` at startup,
  with a reserve, so the core's constant cannot be set optimistically past reality.

The refusal path must be exercised by the security suite (a proptest that arms to the bound
and past it, asserting containment), because an unexercised refusal path is where the C's
worst behaviour lived.

**★ And it has two siblings the audit found, in the same family and with the same fix
shape:**

- **G9 — `targets` is guest-driven and uncapped.** ★ **CLOSED** (`l1_concurrency.md`
  §12.21). Validated against the realized roster exactly as recommended — a `Device` naming
  an instance the device does not have is a loud refusal at the alloc, not a mint. Two
  refinements the bench findings forced: the cap is the **entitlement**, *not*
  `NV_MAX_DEVICES` (RM already enforces `< 32`, so that bound would still permit 31 windows
  on a single-GPU box), and the refusal mirrors RM's `NV_ERR_INVALID_CLASS` so a guest
  cannot fingerprint us by probing. Pruning at device reset stays part of T4.
- **G10 — `condemned` and `retired` are unbounded.** ★ **CLOSED** (§12.22). Both bounded, the
  scan replaced by an index, and — the part the audit had not seen — the **carry-forward**,
  which was O(n² log n) per apply and dominated everything (55 s at the cap), replaced by
  union-find over entry indices. **The overflow question is answered: refuse the guest's next
  derivation.** Refusing the condemnation would un-condemn a component whose isolate is
  already dead (silently serving a zeroed backing, §12.13's corruption path); condemning
  device-wide would be a self-inflicted brick. Refusing new `Proc` derivation leaves every
  live proc serving and lets the guest recover by freeing the dead client roots.

### 3.9 Shutdown

The loop exits on a shutdown flag delivered through the notify source. **No lock may be
held across the join** (R1 — a thread join is a blocking call; the existing
`BlockingSection` is exactly the marker for it). Order matters and is stated in §7.9.

---

## 4. Decision 9 — `kayfabe-linux-raw`, the one audited `unsafe` crate

### 4.1 ★ The `forbid` mechanics — a finding, not a formality

The workspace declares `[workspace.lints.rust] unsafe_code = "forbid"`, and **13 of 13
crates opt in** with `[lints] workspace = true`. `forbid` cannot be relaxed by an `allow`
inside the crate — that is the point of `forbid` — so `kayfabe-linux-raw` must **omit** the
inheritance block and declare its own lints:

```toml
# crates/kayfabe-linux-raw/Cargo.toml   — deliberately NOT `[lints] workspace = true`
[lints.rust]
unsafe_code = "allow"        # the ONE crate; see the CI gate below
missing_docs = "warn"        # not lost just because the block was replaced
[lints.clippy]
undocumented_unsafe_blocks = "deny"   # every `unsafe` carries a `// SAFETY:` or the build fails
```

This creates a silent opt-out mechanism for the *whole workspace*: any future crate that
simply forgets (or quietly drops) `[lints] workspace = true` loses `forbid` with no
diagnostic. So the gate is two-sided and both sides are cheap:

- **CI gate A (inheritance):** every `crates/*/Cargo.toml` **except** `kayfabe-linux-raw`
  must contain `[lints]` + `workspace = true`. A new crate that forgets fails the push.
- **CI gate B (containment):** `unsafe` appears **nowhere** outside
  `crates/kayfabe-linux-raw/src/`. Same polarity trick as the existing boundary gate (a
  `grep` hit is the failure).
- **Ratchet (soft):** the count of `unsafe` blocks inside the crate is printed and compared
  against a committed number, like the mutation threshold. Raising it is a reviewed commit
  with an argument; drifting upward during a debugging session is not possible.
  `undocumented_unsafe_blocks = deny` makes every one of them carry its own justification,
  which is what makes the §11 review tractable at all.

### 4.1.1 ★ `unsafe` lives only in files named `*_unsafe.rs`

Containment to one *crate* (gate B) makes the audit finite. Containment to named *files*
makes it **enumerable by `ls`**, which is a different and stronger property: an auditor who
has never seen this repo must be able to list the entire unsound surface without reading a
line of code, and must be able to tell from a diff's **filenames alone** whether a change
touched it.

> **Standing rule: an `unsafe` block or `unsafe fn` may appear only in a file whose basename
> ends `_unsafe.rs`. Every other file in the workspace — `kayfabe-linux-raw` included — is
> safe Rust.**

So the raw crate is itself two-layered: `mmap_unsafe.rs`, `epoll_unsafe.rs`,
`region_unsafe.rs`, `kvm_unsafe.rs` hold the syscalls and the pointer work;
`page_size.rs`, `geometry.rs`, `region.rs` hold the newtypes, the arithmetic and the bounds
checks and are ordinary safe Rust. That split is not cosmetic — §5.2 already requires the
geometry to be pure and parameterised so the determinism suite can run it at three page
sizes, and §4.7 already requires the crate to hold no business logic. **The file rule is the
mechanical form of both**, which is why it costs nothing to adopt: the layering it demands
was already required for two other reasons.

- **CI gate C (file naming):** `unsafe` appears only in `**/*_unsafe.rs`. (Being built
  separately; this section is the specification it implements.) Same polarity trick as
  gates A and B — a `grep` hit is the failure.
- The §4.1 ratchet counts blocks **per `*_unsafe.rs` file**, so a *new* unsafe file is a
  visible, reviewed event rather than a line inside a diff.
- `undocumented_unsafe_blocks = "deny"` still applies, so every block in those files carries
  its own `// SAFETY:`.

Consequence worth stating: `*_unsafe.rs` files should be **small and boring**. A safety
argument that needs a paragraph of GPU or guest semantics to hold is a sign a decision leaked
down a layer — §4.7's rule, arriving from the other direction.

### 4.2 The bounded-object API (decision #16, applied)

No raw pointer ever escapes. Decision #16 states that as a general principle; **§4.2.1
sharpens it into a rule about *which* surface needs it hardest and *why*** — because a
general principle is exactly the thing that gets traded away at the one seam where it is
load-bearing.

Three region types, distinguished by *who else writes the memory* — the distinction is the
API, because it is the thing that determines what access is sound:

| Type | Backing | Other writer | Access surface |
|---|---|---|---|
| `MappedRegion` | our own mapping (guest RAM view, the RPC queue, pushbuffer pages) | **the guest**, concurrently | `read_into(off, &mut [u8])`, `write_from(off, &[u8])` — **copies only** |
| `VolatileRegion` | a page the **GPU** writes (semaphores, USERD, fences) | **hardware** | `load_u32/u64(off)`, `store_u32/u64(off, v)` — naturally aligned, ≤ 8 bytes, and *nothing else* |
| `Reservation` | `PROT_NONE` address-space reservation | nobody | `map_fixed_in(offset, len, fd, foff, prot)` → a `MappedRegion` **inside** it |

Every offset is a `HostOffset` newtype; every guest-physical address is `Gpa` (already in
`kayfabe-arch`); every length is checked (`off + len` via `checked_add`, `len == 0` is a
loud refusal, `usize`/`u64` conversions are `try_into`). No `as_slice`, no `as_ptr`, no
`Deref<Target = [u8]>`, on any of the three — asserted by `trybuild` (§4.6).

**Why copy-only on `MappedRegion` is a security property, not a style choice:** anything the
core reads from guest RAM (pushbuffer bytes, RPC queue entries, page-table entries) is
mutable by the guest *while we look at it*. A borrow into that memory invites a
double-fetch: validate a length field, then re-read it — the classic. `read_into` copies
once; the parser then works on memory the guest cannot touch. The core is already shaped
for this (its fuzz harness feeds it a `&[u8]` it owns), so the rule costs nothing and
closes a whole vulnerability class by construction.

#### 4.2.1 ★★ Two kinds of out-of-range — and only one of them is a fault

The design already has bounded objects everywhere it thought to put them. This section says
where the rule is *not negotiable*, and the reason is that **the same arithmetic mistake has
two categorically different consequences depending on which address space it happens in.**

- **`Gpa` / BAR offset / GPGA / host-GPU VA are guest- or device-address spaces.**
  Out-of-range there is a **guest-visible fault**. MISS = FAULT already covers it, the
  answer is a loud refusal, and the blast radius is the guest's own aperture (or, for a host
  GPU VA, the *isolate's own* host VAS — which is the #14 boundary doing exactly its job).
  These may be plain integers with checked arithmetic, and they are.
- **A host CPU address is different in kind.** Out-of-range there is not a fault; it is a
  **hypervisor escape**. There is no aperture to be confined to, no MISS to report, and no
  guest to fault. Different consequence ⇒ different treatment, and the treatment has to be
  structural because the failure has no detection story of its own.

**And the half that catches people: `#![forbid(unsafe_code)]` does not save us here.**
Creating a raw pointer, casting it to an integer, and doing arithmetic on it are all *safe*
operations — only the dereference is `unsafe`. So a boundary that mints a host address
(a `*mut u8`, a `NonNull`, a `usize`, a `u64` field called `host_va`) and hands it upward has
already lost: the safe code above does unchecked address math with a clean conscience, the
`unsafe` block that eventually trusts it is three crates away, and gate B, gate C, the
ratchet and every `// SAFETY:` comment all still pass. **The unsoundness happened where the
pointer was minted; everything after it is safe code being wrong.** That is why the rule is
about *what crosses the seam*, not about who writes `unsafe`.

> **THE RULE. A host CPU address never crosses a crate boundary, in any representation.**
> Not `*mut u8`, not `*const u8`, not `NonNull`, not `usize`, not a `u64` field. What crosses
> is a **bounded object** — a region that carries its own length and offers only checked
> accessors — or an **opaque token** that only the raw layer can resolve. Offsets are
> domain-typed (`HostOffset`), guest addresses are domain-typed (`Gpa`), and **a length is
> never passed separately from the base it belongs to.**

**What the API must refuse** (each is a `trybuild` row — §4.6 rows 7–10):

1. any accessor returning `&[u8]`, `&mut [u8]`, `*const T`, `*mut T` or `NonNull<T>` over a
   region — the direct route;
2. any `Deref` / `AsRef` / `Borrow` / `Index` impl on a region type — the sideways route to
   (1), and the one a helpful `impl` block adds by accident;
3. any public field, constructor or getter exposing a region's or reservation's base address
   as an integer — an integer host address is a pointer with the checks filed off;
4. `Gpa` ⇄ `HostOffset` conversion by `From`, `as`, or arithmetic, in either direction —
   these are different address spaces with different consequences for being wrong, and the
   compiler should be the one that says so;
5. an address-taking placement API — `map_fixed_in(offset, len, …)` on `&mut Reservation`
   exists; `map_fixed(addr, len, …)` does not (§4.4 is this same rule's other half).

**★ The exemption, written down because the naming actively misleads.**
`RmBackend::map_gpu_va` returns a `u64` that the core stores as `host_va` and that `Binding`
carries (G1). **That is not a host CPU address.** It is an address in the *host GPU's*
virtual address space, minted by RM inside the owning isolate; out-of-range there faults the
GPU MMU inside one isolate's own host VAS. It stays a plain `u64`, and the bounded-object
rule does not touch it. Both available mistakes are live: an implementer who bans it on a
name match makes the design worse for nothing, and an implementer who waves through a real
host pointer because "the other `host_va` was fine" makes it unsound. Hence the distinction
is stated here rather than left to inference.

**★ Where this binds hardest — and it is not the raw crate.** The raw crate is the one place
everybody already knows to be careful. The dangerous seam is **the real `Vmm` implementation
and whatever L1 shell code sits between it and the raw crate**, because the obvious QEMU
`gpa_read` resolves a GPA to a host pointer once and memcpys, and the obvious next step is to
cache that pointer and hand it upward "just for the hot path". That step is the entire bug,
and it is a *performance-motivated* step, which is the kind that gets made during a
measurement session rather than a design one.

The good news, stated so the rule's real job is clear: **the abstract `Vmm` trait is already
clean.** `gpa_read`/`gpa_write` take a `&mut [u8]`/`&[u8]` the *caller* owns; `HostRegion` is
`{ id, offset }` with a backend-scoped opaque id; `RamHandle` is a token plus a guest-physical
range. Nothing pointer-shaped is on that trait today. So the obligation here is
**conservation, not construction**: the trait must not *acquire* a pointer-shaped method
later, and the impl must not grow a pointer-shaped internal helper that anything outside a
`*_unsafe.rs` file can call. A rule whose job is to prevent a future addition needs to be
written before the addition is tempting, which is why it lands now and not at M2-c.

**How it is tested, and where the honesty is.** The compile-fail half is trybuild rows 7–10.
The "someone added a new module" half is subsumed by gate C (§4.1.1): a `*mut`/`*const`/
`NonNull`/`transmute` outside a `*_unsafe.rs` file is already a gate failure, so this costs
nothing new. What neither can see is a *semantically* unbounded bounded object — a region
whose length field is right but whose backing was mapped shorter. That is a **review
obligation**, named as one: every new public item in `kayfabe-linux-raw` or in the real `Vmm`
impl is checked against the five refusals above, as part of §11's exit gate. Types close four
of the five; the fifth is a human, and pretending otherwise would be the exact failure mode
this doc keeps cataloguing.

### 4.3 ★ Volatile vs atomic — where the existing wording is wrong

`l1_concurrency.md` §9.1 says the raw module owns *"volatile access to concurrently-
GPU-written pages (VolatileSlice-style — semas/USERD are written by real hardware;
non-volatile access is UB-adjacent tearing)"*. The *intent* is right and the *mechanism* is
imprecise, and the imprecision matters because it is what someone will implement:

- A concurrent write by an agent outside the Rust abstract machine is a **data race**;
  `read_volatile` is not defined to be atomic and carries no non-tearing guarantee in the
  abstract machine. It is the right tool for **MMIO registers**, where the *side effect* of
  the access is the point.
- For **shared memory concurrently written by another agent**, the defined primitive is an
  **atomic access with `Relaxed` ordering**. It lowers to exactly the same instruction on
  the targets we care about, it is defined under concurrent modification, and it cannot be
  split or duplicated by the optimiser.

**Recommendation:** `VolatileRegion`'s accessors are implemented as `&AtomicU32` /
`&AtomicU64` views constructed once over the mapping (alignment checked at construction,
lifetime tied to the region), loaded/stored `Relaxed`. Keep `read_volatile` for a future
MMIO-register type, where it belongs, and say which is which. Ordering against *other*
memory is a separate, explicit act: a `Release` fence before ringing a doorbell, an
`Acquire` fence after observing a semaphore — named at the two seams that need them rather
than smeared into every access.

This is the doc's clearest "I think the existing design is wrong" item. It is a small
correction and worth making before the code exists, because the alternative is discovering
it during a heisenbug hunt against a real GPU at L3.

### 4.4 `MAP_FIXED` is the breakout surface — reserve first

`MAP_FIXED` silently unmaps whatever is already at the target address. In a process that
also contains QEMU, "whatever is already there" can be QEMU's heap, our stack, or our own
text. **The double-mmap of guest RAM into an isolate is the single most dangerous call in
the codebase**, and it is born here.

Rule, shaped so violation does not typecheck: `map_fixed_in` is a method on
`&mut Reservation` taking an **offset within the reservation**, never an address. To place
a mapping you must first own the address space. Belt and braces: the underlying call uses
`MAP_FIXED_NOREPLACE` and treats a collision as a loud fault, so even a reservation-
accounting bug fails loudly rather than clobbering.

### 4.4.1 ★ The precondition the double-mmap has always had, and never stated

The design above assumes a slice of guest RAM can be mapped into a second process. **That
is not a property of guest RAM; it is a property of how the VM was launched.**

> **Guest RAM belongs to the VMM, and is shareable with an isolate only if the VM was
> started with a *shareable* memory backing.** On cloud-hypervisor that is
> `--memory shared=on`; on QEMU it is `memory-backend-file`/`memory-backend-memfd` with
> `share=on`. With a private backing (the default on both), the region is `MAP_PRIVATE`
> anonymous memory: `Vmm::export_ram` has nothing to hand out, and any handle it invented
> would produce copy-on-write pages — an isolate writing a completion the guest never sees.

Three things worth saying about it, and they pull in different directions:

- **It is portability-neutral.** The requirement is *identical* on both backends, and both
  express it as a launch flag. So this is not a CH-vs-QEMU difference and it does not
  belong in the adapter comparison; it belongs here, with the design that depends on it.
- **No code gate can catch it.** It is a deployment fact. There is no type, no lint and no
  CI grep that can observe how the operator started the VM. That is precisely why it is
  written into the design rather than left as folklore: the only available mechanism is a
  **loud refusal at the first export** — `VmmError::Unsupported`, at device realize —
  rather than a `SIGBUS` or a silently-diverged page at first guest DMA, which is what an
  unstated precondition buys you.
- **It is load-bearing for the *whole* isolate memory design**, not one method. The
  `m2_stub_ram_base` `MAP_FIXED` share, Mode-1's double-mmap, the per-proc GPA arena
  slices, §11 item 2's least-privilege export scoping — every one of them is downstream of
  a shareable backing existing at all.

Pinned by `tests/tests/vmm_portability.rs`
(`export_ram_without_a_shared_backing_refuses_at_the_first_export`): the exact
`Unsupported` variant, for both the sliced and the whole-RAM export.

### 4.5 ★ Every raw entry point asserts the lockwitness

The R1 witness already lives at the bottom of the dependency graph
(`kayfabe_util::lockwitness`) precisely so a crate that cannot depend on `kayfabe-rt` can
still assert it (§12.8). `kayfabe-linux-raw` is such a crate. So:

> **Every syscall-performing function in `kayfabe-linux-raw` calls `assert_lock_free`
> first.** Since the raw crate is the *only* place a syscall can happen, "no syscall under
> any lock" stops being a rule and becomes a mechanism.

Two escape hatches, both greppable and both reviewed: the deliberately non-blocking
discharges (`raise_irq`'s descriptor write, if a future path needs it in-lock) get
`*_under_lock` variants with the lock rank they permit named in the signature. A grep for
`_under_lock` enumerates the entire set of in-lock syscalls in the codebase, which is the
property we actually want — not "there are none", but "there are exactly these and each was
argued".

### 4.6 The `trybuild` compile-fail matrix

Each row is a dangerous pattern that must **not compile**:

| # | Pattern | Why |
|---|---|---|
| 1 | `region.as_slice()` / `as_ptr()` on any region type | no borrow escapes into shared memory (O8, double-fetch) |
| 2 | `VolatileRegion::load` of 16 bytes, or at an unaligned offset | tearing |
| 3 | `map_fixed` at a bare address (no `Reservation`) | §4.4 breakout |
| 4 | `HostPageSize(4096)` / `HostPageSize::from_bytes(4096)` outside tests | §5 |
| 5 | a `MappedRegion` read whose result outlives the region | use-after-munmap |
| 6 | `Worker`/region types crossing a thread boundary they may not | the `Send`/`Sync` contract |
| **7** | a `Deref`/`AsRef`/`Borrow`/`Index` impl on any region type | §4.2.1(2) — the sideways route to a borrow, added by a helpful `impl` block |
| **8** | reading a region's or reservation's base address as an integer (`.base()`, `.addr()`, a public field) | §4.2.1(3) — an integer host address is a pointer with the checks filed off |
| **9** | `Gpa → HostOffset` or `HostOffset → Gpa` by `From`/`as`/arithmetic | §4.2.1(4) — different address spaces, different consequence of being wrong |
| **10** | a host CPU address (`*mut u8`/`*const u8`/`NonNull`/`usize`-as-address) in the signature of any item reachable from outside `kayfabe-linux-raw` | §4.2.1's crate-boundary rule, in compile-fail form |

### 4.7 What the crate must **not** contain

No business logic. No decisions. If a function in `kayfabe-linux-raw` has an `if` that
depends on GPU or guest semantics, it is in the wrong crate. This is the same rule as §8.1's
thin waist, and it is the only thing that keeps the audit surface reviewable by a human in
one sitting — which is the actual acceptance criterion for the one crate that can be
unsound.

---

## 5. ★ Decision 10 — the host page size: designed in, and ENFORCED

`portability_arm64.md` states the binding rule (`sysconf(_SC_PAGESIZE)`, never a hardcoded
`4096`) and admits it is currently only prose. Making it stick is this decision.

### 5.1 First, the distinction that makes a naive grep theatre

There are **two** page sizes in this system and they are unrelated:

- the **GPU MMU** page size (4 KiB / 64 KiB / 2 MiB leaves) — `kayfabe_arch::PageSize`,
  a genuine architectural constant of the *GPU*, correct on every host CPU;
- the **host CPU** page size — 4 KiB on x86-64, 16 KiB or 64 KiB on arm64.

A workspace-wide grep for `4096`/`0x1000` therefore fires constantly on legitimate GPU
constants and on test addresses (there are dozens today). A gate that fires constantly gets
muted, and a muted gate is worse than none. So the enforcement must be **typed and
targeted**, with grep as a narrow backstop only.

### 5.2 The recommendation, in strength order

1. **★ A newtype with no literal constructor.** `HostPageSize` lives in
   `kayfabe-linux-raw`, has a private field, and exactly two constructors:
   `HostPageSize::query()` (one `sysconf`, cached in a `OnceLock`, asserting the result is
   a power of two in `[4 KiB, 64 KiB]` — an absurd value is a loud startup fault, never a
   silent misalignment) and `HostPageSize::forced(n)`, which is `#[cfg(test)]` /
   feature-gated. Every alignment, window-granularity, slice-bound and mmap-length API
   takes a `HostPageSize`. **A literal `4096` cannot flow into the alignment path because
   it does not typecheck** — trybuild row 4 pins both polarities.

   > **★ And the exact reach of that sentence, because it is narrower than it reads (added
   > by the portability round, §14.4).** The typed rule **stops at the adapter**. The core's
   > geometry constructors take plain integers — `GpaSpace::new(window: Range<u64>,
   > arena_len: u64)`, `TargetGeom { window_len, arena_len, .. }` — and they *cannot* take a
   > `HostPageSize`, because that type lives in `kayfabe-linux-raw` and a pure crate may not
   > depend on it (that is the hexagonal boundary working, not a defect). So a literal `4096`
   > **does** typecheck as an `arena_len`, and on a 64 KiB host that is a misaligned window.
   >
   > The obligation this creates is on the composition root, and it is the only place it can
   > live: **the adapter validates every core-supplied geometry against `HostPageSize::query()`
   > at construction and refuses loudly** — a startup fault, in the same spirit as `query()`'s
   > own power-of-two assert, never a silent round. Say it this way rather than widening the
   > newtype's claim: "no literal reaches the alignment path" is true of the raw crate and
   > false of the core, and a rule quoted one crate beyond where it holds is how an untrue
   > invariant gets believed.
   >
   > The same seam appears on the `Vmm` port: `map_read_native`'s `write_trap` sub-range is
   > rounded to whole **host** pages by the backend, so a caller that reasons in 4 KiB gets
   > more pages trapped than it asked for on arm64 (correct, quietly slower). That rustdoc
   > now says so.
2. **★ Geometry as pure functions of the page size.** Every derived quantity —
   round-up/round-down, window granularity, arena alignment, the double-mmap slice bounds,
   the memslot geometry — lives in a `geometry` submodule that performs **no syscalls** and
   takes `HostPageSize` as a parameter. This is what makes (3) possible at all, and it
   keeps the arithmetic inside the thin waist where the determinism suite can reach it.
3. **★★ The page size as a TEST AXIS, not a constant — the headline.** Every geometry test
   runs over `[4 KiB, 16 KiB, 64 KiB]`, and the integration suites gain a
   `KAYFABE_FORCE_HOST_PAGE_SIZE` knob (honoured only in test builds) so the *whole* shell
   suite — including the mean run — can be executed once at 64 KiB on an x86 runner. This
   is not a new idea in this repo: it is exactly what `LockMode` already does. Both lock
   configurations are tested from day one specifically so the late granularity flip is
   never the untested mode (§8.2, review item P5). **The page size deserves the same
   treatment for the same reason**, and arguing it from the project's own precedent is why
   I expect it to survive.
4. **A narrow grep, as a backstop.** CI greps `crates/kayfabe-linux-raw/src` and
   `crates/kayfabe-rt/src` (only — the GPU constants live in `kayfabe-arch`, outside the
   scope) for bare `4096` / `0x1000` / `>> 12`, excluding the geometry tests. Narrow enough
   not to be noise; it exists to catch a *new module* that bypasses the newtype entirely,
   which is the one thing (1) cannot prevent.
5. **The existing aarch64 `cargo check` job stays** and now covers a crate that actually
   contains arch-sensitive code, which it did not before.

### 5.3 What this does NOT prove — stated so nobody claims it

A forced-64 KiB run tests our **geometry**, not the **kernel**. On an x86 host the real
`mmap` still wants 4 KiB alignment, so the forced runs must not perform real mappings (the
harness's forced mode drives the pure geometry and the mock-backed paths). Whether a real
16 KiB/64 KiB kernel agrees with our arithmetic is verified only on real arm64 hardware.
That is an L3 item, listed in §9.4, and it is genuinely residual — this design reduces the
arm64 bring-up to "run the suite on the hardware", which is the whole goal, but it does not
eliminate it.

---

## 6. Decision 11 — the real `Vmm`

### 6.0 ★★ The portability contract — a founding requirement that was written down nowhere

**The finding that produced this section is structural rather than technical.** The word
*"agnostic"* appears **zero times** in this document and zero times in `l1_concurrency.md`.
Hypervisor-agnosticism is a founding requirement of the `Vmm` port — the architecture doc's
§4.1 says it in as many words (*"count is not the invariant; hypervisor-agnosticism is"*) —
and below that one line it is restated in no normative text, guarded by no gate, and tested
by nothing. A requirement in that state does not survive its first adapter; it decays one
identifier at a time, and the decay is only visible when someone attempts the second one.

**The contract, stated normatively so it can be cited:**

> **A second hypervisor backend costs exactly one adapter crate: no trait change, no core
> change.** The core and the L1 shell name hypervisor **capabilities**; only an adapter
> names a hypervisor's **API**.

**What holds it — three mechanisms, in descending strength:**

1. **★ A CI gate, in the shape of the hexagonal one (decision #39).** The 11 pure crates
   **plus `kayfabe-rt`** are grepped for hypervisor API vocabulary:
   `BQL_LOCK_GUARD`, `bql_lock`, `qemu_mutex_lock_iothread`, `MemoryRegion`,
   `memory_region_`, `QEMUBH`/`qemu_bh`/`aio_bh`, `qdev_`, `VMStateDescription`, `iothread`,
   and the two English idioms *"bottom half"* and *"main loop"*.
   ★ **It matches API identifiers, never the vendor's name** — so naming the adapter crates
   (`kayfabe-vmm-qemu`, `kayfabe-vmm-ch`), citing a QEMU source file, or writing "QEMU's is
   the whole-machine one" all stay writable. That is what makes it an **allowlist-free**
   gate, and a gate with an allowlist is a gate with a negotiation. Same exit-code polarity
   caveat as its two siblings (`grep` exits 0 on a hit; a hit is the failure).
   *`kayfabe-rt` is in scope even though it is outside the hexagonal list*: it is an adapter
   for threads and locks, not for a hypervisor, and the `&self`-plus-ranked-locks shape it
   encodes is exactly what a second backend must be able to reuse.
2. **The trait names capabilities.** Where a capability genuinely *is* backend-conditional,
   the trait says so per item rather than pretending uniformity — `IrqSpec::IntxLevel` is
   the only such item today, and its rustdoc names the refusal.
3. **The class rule of §6.3**, which is the sharpest instance and gets its own section: a
   foreign lock is a *class*, not one hypervisor's mechanism.

**What this is NOT.** It is not a claim that a CH port is free — it is a claim about *where*
the cost lands. The audit that produced this section dry-ran the gate over the tree and
found exactly **two** violations, both prose in `kayfabe-vmm`'s own rustdoc, both one-word
rewordings. That is the whole point of landing it now: at zero adapters the gate is a
rewording, at two adapters it is a migration.

### 6.1 ★ The classification: which capability groups may be called under a lock

The `Vmm` trait's capability groups (eight when this table was written; **seven** since §6.8
struck the memory-lock pair) are not one kind of thing. Some are memcpys into
memory we already mapped; some are syscalls that take the kernel's memslot machinery and,
in QEMU, a global lock. **The trait does not say which, and that is the gap.** It must:

| `Vmm` method | Real cost | In-lock legal? |
|---|---|---|
| `gpa_read` / `gpa_write` | memcpy into an already-installed mapping | **yes** — and it must stay so, because the pushbuffer parse legitimately runs under the proc lock |
| `now` | one clock read (vDSO, no syscall) | **yes** |
| `defer` | push onto a heap + latch the timer | **yes** (the syscall is the post-entry drain's, §3.6) |
| `raise_irq` | one descriptor write on an irqfd | **yes** — the single named exception (§6.3) |
| `map_guest` / `unmap_guest` | **memslot syscall**; KVM SRCU; QEMU: BQL | **NO** |
| `map_read_native` | as above | **NO** |
| `set_trap` | as above | **NO** |
| `export_ram` | memfd + mmap syscalls | **NO** |
| ~~`lock_region` / `unlock_region`~~ | ~~memslot revoke / userfaultfd~~ | **removed from the trait — §6.8** |

This table becomes rustdoc on every method and, more importantly, becomes an **assert**:
the real `Vmm` impl calls `assert_lock_free` at the top of every "NO" row, using the same
witness as `Worker::execute`. One witness, three enforcement sites (worker verbs, raw
syscalls, VMM syscalls), zero drift.

**★ Two amendments to this table, made in §6.3 and §6.7 and flagged here so the table is not
read alone.** (i) The "yes" rows are legal **only because their implementations cannot reach
VMM-global API** — that is a binding requirement on the adapter, not a property of the method
name, and §6.3 makes it the contract. (ii) The `map_guest`/`unmap_guest` row silently covers
**two operations** with different costs and frequencies (a memslot update vs a `MAP_FIXED`
placement inside an installed window); §6.7 splits it, and the split is what keeps the data
plane off the memslot path entirely. Both remain "NO"; what changes is the cost model and the
frequency, and §6.2's cost estimate is corrected in §6.7.

### 6.2 ★★ The finding: R1's second reckoning, and the biggest thing L1-M1 will not survive

Turning that assert on will **panic on the existing code**, and the reason is §12.6's
lesson repeating one layer up.

`publish_backing`'s commit phase carves a GPA from the proc's arena and forward-populates
the address table **under the proc lock**. The moment that publication must become
guest-visible, it needs a memslot installed — a `map_guest`. Same story for the read-native
overlays and for any trap re-registration. The mock's `map_guest` is a `BTreeMap::insert`,
so L1-M1's suite is *structurally incapable* of noticing that a real one is a slow syscall
under a kernel-global lock, held under our rank-1 lock, with a rank-0 read guard above it.

§12.6 wrote the general form of this already: *"That is correct only because stage 2's
backends are mocks that never block: with a real host verb it would be a live R1 violation
**with no assert firing**."* The sentence was about `RmBackend`. It is equally true of
`Vmm`, and stage 3 fixed only the first one.

**The fix is the shape the codebase already has.** `RmBackend` got `plan → execute →
commit` with a typed `VerbPlan`. `Vmm`'s syscall-shaped methods get the same treatment:
the locked commit phase **emits** the memory-plane instructions it needs (a small typed
`MemPlan`: install this backing at this GPA, revoke this slot, export this slice), and the
shell executes them lock-free before or after the locked phase, with R5 re-validation
across the gap exactly as the verb path does. §12.12 already established the precedent that
the split is worth checking per site rather than assuming, and it established the property
that makes it cheap: *no site needs to consult core state between two instructions.* The
memory-plane instructions look the same — they are data-dependent on each other and on the
plan, not on state read in between.

**Honest risk:** I do not know how many sites this is. Grepping the current tree, the core
touches `Vmm` in only three places (`raise_irq` ×2, `gpa_read` ×1) — all of which are
in-lock **legal** by the table above — because the memory-plane installation *does not
exist yet*: `publish_backing` today carves the GPA and populates the table without ever
installing a memslot. So the honest statement is not "there are N sites to convert" but:

> **The `Vmm` memory plane is largely unbuilt, and L1-M2 is where it gets built. It must be
> built in the plan/execute/commit shape from the first line, because building it in the
> obvious shape (call the VMM from the commit) reproduces exactly the violation stage 3
> spent a milestone removing from the verb path.**

That is a far better place to be than discovering N sites — and it is precisely why this is
a design task. If it is built the obvious way first, the retrofit touches the commit path,
the lock discipline, and the R5 re-validation sites simultaneously: the shape this project
keeps refusing.

### 6.3 ★★ The foreign-lock contract — a lock we do not own, an inversion R3 cannot see

> **★ Restated as a CLASS by the portability round (§14.4). What follows below was written
> against QEMU's mechanism; everything it says is retained, and it now sits underneath the
> portable rule rather than standing in for it.**
>
> ## **No lock the VMM owns — one we neither construct nor rank — may be acquired beneath one of our locks; and our entry paths may arrive with one already held.**
>
> **The instances, named, so the class is not abstract:**
>
> | Backend | The foreign lock | Granularity | Held across |
> |---|---|---|---|
> | QEMU | the **BQL** (big QEMU lock) | **whole machine** | trap dispatch, and the serialized-executor context |
> | cloud-hypervisor | the **per-device `Mutex`** from the `impl<B: BusDevice> BusDeviceSync for Mutex<B>` blanket impl (`vm-device/src/bus.rs:41`) | **one device** | the **entire** MMIO callback — and it *panics* on poisoning |
>
> **Why the class matters more than either instance — the concrete cost of having written
> the mechanism.** The rule as originally phrased — *"nothing beneath one of our locks may
> take the BQL"* — is **vacuously true on cloud-hypervisor**, which has no BQL. Read
> literally it says a CH adapter is unconditionally compliant, while saying nothing about
> the lock CH actually imposes. Same hazard, different granularity, invisible to the rule
> that was supposed to cover it. That is the general failure mode of writing a mechanism
> where an invariant belongs, and it is worth noticing that the wrong version was *careful*
> and correctly scoped to L2: care does not substitute for generality.
>
> **★ The CH adapter's escape, recorded now because it is free and will not be obvious
> later.** CH's foreign lock is imposed by the *blanket impl*, not by the bus: a device
> registered as `Mutex<B>` is exclusively locked for its whole callback. So —
>
> > **Do not register through the `Mutex<B>` blanket impl.** Implement `BusDeviceSync`
> > **directly** on a thin wrapper over `kayfabe_rt::SharedDevice`, which is already
> > `Send + Sync`, exposes `&self` methods, and does its own ranked locking.
>
> Under that shape **CH imposes zero foreign locks**: every lock on the path is one we
> constructed and ranked, so R3 sees all of them and the inversion below cannot be
> constructed at all. Worth stating explicitly that this is the *unusual* path —
> `PvmemcontrolBusDevice` is the only in-tree device that implements `BusDeviceSync`
> directly — i.e. it is supported but off the ergonomic road, and the ergonomic road is the
> one that silently reintroduces a foreign lock. Naming it before someone reaches for
> `Mutex<Device>` costs one paragraph; discovering it costs an adapter rewrite.
>
> This is also why `Device` takes **`&self`** (see the trait's own rustdoc and §14.4 item 2):
> a `&mut self` port would have forced a CH adapter through a whole-device `Mutex` anyway,
> reintroducing by our own hand exactly the lock this escape avoids.

This started as a note about `raise_irq`. It is a whole-surface **correctness** contract, and
it is elevated here because it is one of the few problems in this milestone that
**measurement cannot help with**: it is not a slow path, it is a cycle. It is either absent
or it deadlocks, and how fast the call is has nothing to do with which.

**The inversion, spelled out.** QEMU takes the BQL *before* dispatching into
`Device::mmio_write`, which then takes our device lock. So the arrival order on a vCPU thread
is fixed and not ours to change:

```text
  vCPU thread   : BQL  →  device(rk0)  →  proc(rk1)          [QEMU's order, imposed on us]
  our thread    : device(rk0)  →  … → BQL                     [ABBA. Deadlock.]
```

Thread 1 (vCPU) holds the BQL and blocks acquiring our device lock; thread 2 (executor or
reactor) holds our device lock and blocks acquiring the BQL. Neither proceeds. Note what is
*not* required for this: no slow syscall, no contention, no unlucky timing beyond the
interleaving itself. **A fast BQL-taking call under our lock is exactly as deadlocked as a
slow one.**

**Why R3 structurally cannot catch it.** R3 is a per-thread rank watermark maintained by *our*
guard wrappers. The BQL has no rank because it is not ours: we never construct it, we do not
control its acquisition sites, and QEMU functions take it *internally* (`msix_notify`,
`memory_region_add_subregion`, `memory_region_set_enabled`) where no wrapper of ours is
involved. R3 is a complete discipline over the locks it can see and is *blind by construction*
to this one. That is not a gap to be closed by a better assert; it is the boundary of what the
mechanism is.

**The contract, as a normative rule over the whole `Vmm` surface** (the class rule above,
instantiated for QEMU — the same sentence holds with "CH's per-device `Mutex`" substituted,
and holds *emptily* if the adapter takes the direct-`BusDeviceSync` escape):

> **Nothing beneath one of our locks may take the BQL.** Every `Vmm` method classified
> **in-lock legal** in §6.1 MUST be implementable as a primitive that cannot reach VMM-global
> API: a memcpy into an already-installed mapping, a descriptor write, a clock read, or a push
> onto a structure we own. `Vmm::raise_irq` in particular MUST be irqfd / event-notifier
> shaped — **never** a BQL-taking `msix_notify()` on the calling thread. Every method that can
> reach VMM-global API is classified **NO** and asserts lock-free.

**The resolution already reached, and why it is one fix rather than two.**
**Drop the lock.** A thread that holds no lock of ours may take the BQL in any order it likes
and no cycle can form — the BQL is simply the outermost lock on every path, and a total order
with one lock at the top is not an inversion. The *same* act fixes the stall: a BQL-taking
op that is slow costs the VM only while it runs, not while a device lock is queued behind it.
**One rule, two problems**, which is the argument for making it the general rule (§6.6) rather
than an exception list.

**How a violation is caught — honestly, in descending order of strength.**

1. **Classification + assert (primary, and indirect).** A BQL-taking capability is a "NO" row
   in §6.1's table, and "NO" rows call `assert_lock_free` at the top. So the assert *does*
   fire on the dangerous pattern — but it fires because of how the method was **classified**,
   not because anything detected the BQL. The mechanism is real; its premise is a judgement.
2. **One acquisition site (mechanical, but only over what we own).** The L2 adapter must have
   **exactly one** function that takes the BQL, it calls `assert_lock_free` first, and CI
   greps that `bql_lock|qemu_mutex_lock_iothread|BQL_LOCK_GUARD` appears in the adapter
   exactly once. Under that shape the BQL *can* be given a rank — **rank −1**, entered but
   never acquired — and R3's watermark then rejects "rank −1 while holding rank 0" like any
   other inversion. This is worth doing because it is nearly free, and worth not overselling:
   it enforces the rule only at the acquisitions we write.
3. **The residual, which is a review obligation and is named as one.** A QEMU function that
   takes the BQL *internally* is invisible to (2). So the adapter carries a **written list of
   the QEMU functions called from each in-lock-legal `Vmm` method**, and that list — not the
   code — is the review artifact at §11's exit gate. There is no mechanism here. Saying there
   is one would be the exact failure this doc keeps cataloguing.
4. **QEMU's own asserts fire the other way.** Many BQL-requiring QEMU functions
   `assert(bql_locked())`, so calling one from a thread that *lacks* the BQL aborts loudly.
   That is the failure mode we prefer, and it is a genuine safety net for the drop-the-lock
   rule — but it says nothing about the inversion, which happens with the BQL held.

**★ And the finding this contract exposes: in QEMU we are GIVEN the BQL, so "lock-free" is
not enough.**

> **★★ EVERYTHING IN THIS SUBSECTION IS QEMU-CONDITIONAL** (tagged by the portability round,
> §14.4 item 3). Every claim below — that our entry paths arrive with a machine-global lock
> held, that a verb round-trip stalls **every vCPU**, and that I-NOAMP *cannot* be met on the
> trap path without `memory_region_clear_global_locking()` — is **true for QEMU and false for
> cloud-hypervisor**, where MMIO dispatch is a synchronous `VmOps` call **on the vCPU thread**
> with no VM-wide lock. On CH the entry path holds at most the per-device lock of the class
> table above, and nothing at all under the direct-`BusDeviceSync` escape. Do not read the
> conclusions here as properties of "a VMM"; read them as properties of *this* VMM, which is
> exactly what §6.0's gate exists to keep visible.
>
> **★ And CH has an affordance aimed at precisely this problem, which QEMU lacks.**
> `BusDevice::write` returns `Option<Arc<Barrier>>`, and the vCPU blocks on `barrier.wait()`
> **after the device lock has been released** (`vmm/src/vm.rs:491-495`). That is a clean
> primitive for parking *one* vCPU across an isolate round-trip **with no lock of anyone's
> held** — the thing §7.1's backpressure argument wants to be true and which, under a held
> BQL, is not. It is recorded here rather than designed against, because no adapter exists
> yet; but it is the strongest single reason to expect the CH answer to this problem to be
> structurally better than the QEMU one, and it is worth knowing before the first adapter
> chooses its shape.

Both of our entry paths arrive with the BQL already held — a trapped MMIO
access (QEMU re-takes the BQL after `KVM_RUN` returns, before dispatch) and a main-loop
bottom half (that context runs under it). Consequences the doc does not currently state:

- **A verb round-trip issued from either context stalls every vCPU in the VM, with every lock
  of ours correctly dropped.** §7.1 of `l1_concurrency.md` says the calling thread does the
  round-trip itself and that backpressure is inherent because "the caller blocks, the guest's
  RPC stalls, the guest slows down". Under a held BQL that sentence is false in its most
  important clause: it is not *the caller* that stalls, it is **the machine**. This is the
  sharpest violation of I-NOAMP (§6.6) available anywhere in the design, and it is caused by
  a lock we neither hold deliberately nor can rank.
- **Therefore the L2 adapter has a named, load-bearing obligation:** our trapped BAR regions
  must be dispatched **without** the BQL. QEMU provides `memory_region_clear_global_locking()`
  for exactly this case (VFIO and virtio use it); the adapter must apply it to our MMIO
  regions and **verify it exists and behaves as expected on the target QEMU version** — this
  is an API-availability check, not an assumption, and it is listed in §7.9's not-mock-testable
  table. Correspondingly, any deferred work that can block must not run on the main-loop BH
  under the BQL.
- **If that mechanism is unavailable, the drop-the-lock rule is necessary but not
  sufficient**, and I-NOAMP cannot be met on the trap path at all. That would be an **L2
  blocker, not a tuning item**, and it should be discovered by a deliberate bench measurement
  early rather than by a wedge under load.
- **The C never solved this and said so in writing** — `C: src/qemu/virtio_nvgpu.c:16-18`:
  *"Currently we hold the QEMU BQL across dispatch for simplicity and will relax this
  later."* Its Mode-2 doorbell path likewise instrumented wall-vs-thread-CPU time precisely
  because the trap ran on "the BQL/vCPU thread" (`C: src/qemu/nvkvm_gpu_emul.c:877-878`). So
  this is not a hypothetical hazard we invented; it is an unpaid debt we are inheriting with
  its receipt attached.

Stated now, in the trait's docs and the adapter's shape, all of this costs nothing.
Discovered at L2 it is a deadlock in a nested-virt bench at 3 a.m.

A pleasing consequence of §6.5: on the completion path `raise_irq` ends up **outside** the
locks anyway, so the in-lock exception is unused there. Keep it in the contract (a future
path may want it), but note that the shipping completion path does not rely on it — "we have
one exception and currently exercise it nowhere" is a much stronger position than "we have one
exception on the hottest path".

### 6.4 `defer` — share the queue with the mock, do not re-implement it

The requirement is "deadline-ordered and deterministic, matching the semantics `MockVmm`'s
virtual clock already pins". The weakest way to meet it is to write a second heap and a
comment claiming it matches. The strongest is to **make it the same code**: factor
`MockVmm`'s `BinaryHeap<Reverse<(Instant, seq, CoreEvent)>>` into a pure `DeferQueue` in
`kayfabe-util` (no clock, no descriptor: `push(deadline, ev)`, `due(now) -> impl Iterator`,
`earliest() -> Option<Instant>`), and have both the mock and the real `Vmm` own one.

Then "matches the mock" is not a claim, it is a tautology; the tie-break rule (equal
deadlines resolve by insertion sequence) is tested once; and the queue is mutation-tested
along with everything else. The real `Vmm` adds exactly two things the mock does not have:
`now()` reads `CLOCK_MONOTONIC` (**the only wall-clock read in the entire system** — §8.3's
rule, finally instantiated), and `earliest()` moving re-arms the deadline timer via the
post-entry drain.

### 6.5 §12.7's edge, closed: observe → pump → encode → write → IRQ

§12.7 recorded that stage 2's executor observes but does not pump, because pumping opens a
drain-gated batch that only a real deliverer can close, and there was no `Vmm`. With one,
the edge completes:

```text
  reactor → SourceSignal → executor:
      dispatch (device read; observe into the OWNER's queue)        [locks held]
      ── drop every guard ──
      pump_completions(gpu)                                          [device WRITE lock]
      ── drop the guard ──
      encode the batch onto the GSP queue        (pure, kayfabe-gsp)
      gpa_write the encoded batch                (memcpy)            [NO locks]
      raise_irq(SWGEN0)                          (descriptor write)  [NO locks]
```

**Why the tail is safe with no lock held** — and this is the interesting part. Normally
"compose a batch, then publish it" outside a lock is a race. Here it is not, and the
argument is the drain gate rather than a lock: the per-target gate admits **one outstanding
batch**, and only the guest's own IRQSCLR (`completions_drained`) reopens it. So between
`pump` returning a batch and that batch being delivered, no second batch can exist for that
target. The gate is doing the mutual exclusion, which is what lets the delivery tail —
including a `gpa_write` whose real cost we do not control in a foreign VMM — run lock-free.

Two obligations fall out and must be honoured:

- **An undelivered batch must not be dropped.** `Effect::Redelivered` already surfaces
  batches "because an undelivered batch must be fed back… exactly as a real drain would".
  If encode or `gpa_write` fails, the honest response is a loud device fault, not a silent
  drop: a dropped batch is a completion the guest waits on forever — the F3 hang, rebuilt.
- **§12.5's gap comes due.** `CoreEventKind::CompletionRedeliver` carries no `GpuId`, and
  delivery is per-target since MG-6. §12.5 says explicitly: *"this must become a
  target-carrying payload when the defer plumbing lands"*. The defer plumbing lands in this
  milestone. Fix it here.

### 6.6 ★ Who RUNS a memory-plane op — the calling thread, with the lock dropped

§6.2 says the memory plane is built in the plan/execute/commit shape. It does not say *which
thread executes the middle phase*, and the tempting answer — a dedicated memory-plane thread
fed by a queue — is wrong for the same reason §7.1 of `l1_concurrency.md` already rejected
relay threads. Rule:

> **A memory-plane instruction requested by the guest executes on the thread that is serving
> the guest's request, with every lock of ours dropped. There is no memory-plane thread and
> no memory-plane queue.**
>
> **Background work with no caller to bill it to runs on the executor** — the
> `pending_release` drain (§7.6 T0), the deferred reap (T3), T0's backstop sweep. The
> executor already owns that role, is already serialized, and is already bounded.

**Rationale — the property a queue would throw away.** §7.1's sentence transfers verbatim:
*"Backpressure is inherent: the caller blocks (lock-free), the guest's RPC stalls, the guest
slows down. No queue to size, no overflow policy to invent."* A vCPU thread that blocks in an
`mmap` is **self-limiting**, and — the part that matters — it limits *precisely the process
that caused it*. A guest that issues a map storm throttles itself and nobody else. A
dedicated thread plus a queue converts that free property into unbounded queue growth, and
then needs three new mechanisms to get back to where it started: a bound, an overflow policy,
and per-proc fairness. Worse, a queue **reorders**, so R5's re-validation would have to defend
a gap that no longer has a caller standing in it to be refused.

The one thing a memory-plane thread would buy is decoupling the vCPU from a slow syscall —
and §6.7 shows the expensive case (a memslot update) costs every vCPU *regardless of which
thread issues it*. So the thread buys nothing on the exact path that motivated it.

#### ★ I-NOAMP — the intra-VM invariant, named

> **★ No cross-process amplification: process A's activity must never make process B's vCPU
> wait through anything WE introduced.**

This is #14's twin. #14 is about A's *addresses* colliding with B's; I-NOAMP is about A's
*latency* landing on B. Both are intra-VM properties, both are free on passthrough hardware,
and both are things a forwarder can lose without any test noticing.

**What bounds it, and what does not — the honest baseline first.** The guest driver already
serializes on its own locks across every GSP RPC: `_kgspRpcRecvPoll` runs with the GPU
(subdevice-group) lock held (`ogkm: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2848`, the
`rmGpuGroupLockIsOwner` assert at `:2954`), with the RM API lock above it
(`ogkm: .../resserv/src/rs_server.c:3164-3168`, §12.26). So on **real hardware**, process A's
slow RM call already makes process B's RM call wait. **We do not create this property and we
cannot delete it. We can only amplify it.** Any claim that our design "gives B independent
progress" would be claiming something the hardware path does not have either — and would fail
the moment it met a real driver.

So the obligation is two-part and testable rather than a slogan:

- **(a) added latency comparable to hardware's** — a forwarded verb's added cost is the
  isolate round trip (µs), not a queueing discipline of our own invention;
- **(b) no amplification we introduced** — no lock, queue or thread of ours may make B wait
  on A where the guest driver's own locks would not have.

Four concrete consequences, three of them already satisfied:

- **R1/R2 already deliver (b) at the lock layer.** Verbs run lock-free, and a device *writer*
  excludes bookkeeping but never waits for in-flight verbs — R2, verbatim: *"'write lock' now
  means 'no torn bookkeeping', deliberately not 'no outstanding host work'."* That is I-NOAMP
  for locks, written down before it had a name.
- **The worker pool is per-`(Proc, GpuId)`**, so a pool-full wait is intra-proc by
  construction: A cannot exhaust B's workers.
- **The `pending_release` drain and the reap run on the executor and are `is_quiesced`-gated**
  (§7.6 T0/T3), so background reclamation for A never parks a thread B needs.
- **The two places we can still amplify are named rather than argued away:** the **BQL**
  (§6.3 — a lock we do not own, and the reason the trap path must not run under it) and the
  **memslot** (§6.7 — an SRCU grace period charged to every vCPU no matter who issues it).
  Neither is fixable by threading. The first is fixed by not holding it; the second only by
  frequency.

**Test shape.** The mean run's existing progress-under-pending assertion **is** the I-NOAMP
canary and should be named as one: proc B makes progress while proc A's verb is parked, both
lock modes, no clock. §6.7 adds its sibling — B makes progress across A's arena grant and
release.

#### ★★ QUEUE DISCIPLINE — the standing rule, and the distinction that keeps the count down

§6.6 above answers *"which thread runs a memory-plane op"*. The general question behind it
is *"how many queues may this system have"*, and the answer has to be normative or the count
only ever goes up: every new asynchronous need arrives looking like it deserves its own
queue, and each one that lands is individually defensible.

The owner's constraint, and it is the right one:

> *"I am worried that we get too many queues in the codebase for tasks to happen — the
> isolate/main code should have no more queues than threads, and a shared queue system
> that's maybe abstract for scheduled tasks."*

The distinction that makes the count enforceable is between two things that look alike and
are not:

> - **A SCHEDULER owns work and needs a thread.** Something must decide *when* its contents
>   run, so something must be running to decide. **There is exactly ONE: the executor
>   inbox** (`kayfabe_rt::inbox`, rank 2, leaf).
> - **An ACCUMULATOR holds work for whoever next passes through, and needs NO thread.** It
>   has no schedule of its own; it is drained opportunistically by a thread that was going
>   to be there anyway. `Proc::pending_release` is the canonical one — taken by whoever next
>   checks a worker out of that isolate (§7.6 T0).
>
> **The rule: queues are drained by EXISTING threads, never by new ones; and there is one
> scheduled-task abstraction, not several.** Anything that wants a thread of its own must
> first argue against `l1_concurrency.md` §7.1's rejection of relay threads — *"backpressure
> is inherent: the caller blocks (lock-free), the guest's RPC stalls, the guest slows down.
> No queue to size, no overflow policy to invent"* — and must say which of those three
> properties it is giving up.

**Why the split is not word-play: an accumulator costs nothing that a scheduler costs.** A
scheduler needs a bound, an overflow policy, a fairness rule and a wake protocol, because it
stands between the producer and *time*. An accumulator needs none of those: it is drained by
a thread already committed to doing work for that same owner, so its depth is bounded by the
owner's own progress and its latency is the owner's own latency. Every one of §6.6's
arguments against a memory-plane thread is really an argument for preferring the accumulator
form, and stating it once means the next such case does not have to be re-litigated.

**The inventory, audited (2026-07-26). One scheduler, one thread of our own.**

| # | structure | class | drained by | bound |
|---|---|---|---|---|
| 1 | `kayfabe_rt::inbox::Inbox` — `VecDeque<CoreEvent>` | ★ **SCHEDULER** | the **executor** thread (the only long-lived thread we own) | unbounded by design and justified in its own docs: entries are per-source-signal / per-deadline, never per-guest-byte |
| 2 | `Proc::pending_release` — `BTreeMap<GpuId, Orphans>` | accumulator | whoever next checks a worker out of that isolate (`checkout_and_drain`), the executor's backstop sweep (`SharedDevice::drain_pending_releases`), and the thread that destroys the corpse (`Proc::drop`, §12.35) | bounded by the proc's own live object count |
| 3 | `Spine::retired` — `Vec<Proc>` | accumulator | whichever thread calls `reap_retired` (in L1: the executor, at the adapter's quiesce point) | `MAX_RETIRED_PROCS` (G10) |
| 4 | `Spine::condemned` — `Vec<BTreeSet<HClient>>` | retention set, not work | re-derived inside `refresh` on the applying thread; cleared when the guest frees the client root | `MAX_CONDEMNED_COMPONENTS` (G10) |
| 5 | `DeferQueue` (§6.4) — the deadline heap shared by `MockVmm` and the real `Vmm` | accumulator **with an ordering** | the timer's post-entry drain, whose product is an inbox event — so it feeds #1 rather than competing with it | one entry per outstanding `defer` |
| 6 | `CompletionQueue` (`pending` / `in_flight` / `drained`) | the **guest's** mailbox, on the far side of the seam | the pump/poll on whichever thread runs it | `MAX_OUTSTANDING_COMPLETIONS`, and refusal is loud |
| 7 | `PoolGate` waiters | not a queue — a condvar generation + counters | n/a: threads park, nothing is enqueued | pool size |
| 8 | `Orphans` inside a `Refusal` / `VerbFailure` | a `#[must_use]` **value in flight** | the thread holding it, before its check-in | per-op |

**Verdict: the rule holds today, and #5 is the one to watch.** Rows 2–4 are accumulators,
rows 6–8 are not ours to count (a guest mailbox, a lock gate, a value). The `DeferQueue` is
the only structure that *orders work by time*, which is a scheduler's defining property — it
stays legal precisely because it owns no thread and its output is an inbox event. The day
something proposes to give it a drain thread of its own, that is the day the count goes to
two, and this table is where the argument has to be had.

### 6.7 ★★ Memslot strategy — COARSE REGIONS WITH AN ALLOCATOR INSIDE, never per-object

**The cost, from the source, because the number is the whole argument.** A
`KVM_SET_USER_MEMORY_REGION` swaps the memslot array and then calls
`synchronize_srcu_expedited(&kvm->srcu)` (`linux: virt/kvm/kvm_main.c:1599-1633`) — it waits
for **every** vCPU to leave its SRCU read section, and before that it waits out any in-flight
MMU-notifier invalidation (`:1614-1622`). A CREATE is one such swap. A **DELETE or MOVE is
two**: `kvm_invalidate_memslot` swaps, `kvm_arch_flush_shadow_memslot` zaps the slot's shadow
/ EPT entries with a remote TLB flush across all vCPUs, then `kvm_activate_memslot` swaps
again (`:1798-1846`, `:1929-1941`). Every update in the VM also serializes on
`kvm->slots_lock`.

So the cost is **milliseconds, scaling with vCPU count, charged to every vCPU** — and it is
charged **no matter which of our threads issues the ioctl and no matter what locks that
thread holds**. Deferring it moves it; threading it moves it; dropping a lock does not touch
it. **The only lever is frequency.** That makes this the sharpest instance of I-NOAMP in the
design: a memslot update caused by process A briefly costs process B, we cannot make that cost
zero, and so the design's entire job is to make it *rare*.

**The rule.**

> **One memslot per window (or per arena grant) — never one per published object.**
> Making a backing guest-visible is a **`MAP_FIXED` placement inside an already-installed
> window** and performs **no KVM ioctl at all**. Memslot churn happens at window-install and
> arena grant/release frequency; it never scales with the data plane.

**This is not a new idea; it is the shape the C converged on after paying for the
alternative.** A single `cuCtxCreate` performs >1500 tiny 4 KiB device mmaps; with a memslot
each, the C exhausted its slot allocator and regressed *single-process* matmul
(`C: multiproc_collision_blocker`, `realize_kvm_slot_regression`). The fix was one
`MAP_ANONYMOUS|MAP_NORESERVE` window in QEMU's mm (`C: src/qemu/nvkvm_mmap_host.c:143-176`),
exactly **one** raw memslot install at the BAR-assigned base (`nvkvm_sparse_ensure`,
`:180-215`), and every GPU mmap becoming a `MAP_FIXED` slice inside it, tagged with a sentinel
whose whole meaning is *there is no memslot here*: `NVKVM_IN_WINDOW_SLOT = -2`
(`C: src/qemu/nvkvm_isolate_handlers.c:1730-1738`, used at `:1839`, `:1869`) — "zero per-mmap
KVM ioctls" (`C: gpa_window_design`).

**And the core already has the geometry.** `GpaSpace` is a per-target window; `GpaArena` is a
per-proc disjoint sub-range inside it; since `e50f7da` an arena carries a coalescing intra-arena
free list and a move-only `GpaBlock` token, and `GpaSpace` recycles whole arenas LIFO with a
never-reused generation. That is *already* "a coarse region with an allocator inside". The
memslot rule simply says: **the memslot boundary is the window (or the arena grant), the
allocator inside it is `GpaArena`, and `GpaArena::alloc`/`free` must never touch KVM.**

**What the rule forbids, explicitly:**

1. **No `map_guest` per publication.** `publish_backing`'s commit must not mint a `SlotId`.
   ★ This contradicts §6.2 as written — corrected below.
2. **No memslot per host mmap** in the isolate-mapping path. That is the C's exact regression,
   with a measured failure and a bisect.
3. **No slot delete/recreate to change a mapping.** A DELETE is two grace periods plus a
   shadow zap. Re-`MAP_FIXED` the window's backing instead — and an *unmap* inside a window is
   `mmap(MAP_FIXED|MAP_ANONYMOUS|MAP_NORESERVE)` restoring anonymous backing, **never a plain
   `munmap`**, which punches a hole in the window's VMA and leaves the live memslot pointing
   at a gap (`C: nvkvm_isolate_handlers.c:1899-1906`, `:1930-1936`). **This is a raw-crate API
   shape, not a convention:** `Reservation::restore(offset, len)` exists, and `munmap` of a
   sub-range of a live reservation does not.
4. **No per-object protection change via the memslot.** KVM's read-only flag is a *slot*
   property, so `Prot::ReadOnly` cannot be expressed per object inside a shared read-write
   window. **Protection is a window property:** partition into an RW window and an RO window
   and place the object in the right one. Never mint a slot to make one page read-only.
5. **No per-object `lock_region` via memslot revoke.** §6.1's docs offer "userfaultfd /
   memslot revoke-restore" as interchangeable implementations of decision #6. Under this rule
   they are **not** interchangeable: revoking a shared window's slot is a device-wide DELETE
   (two grace periods) in order to lock one object, and it revokes access for *every* proc
   sharing the window. **`lock_region`/`unlock_region` are userfaultfd-on-the-window,
   per-page; the memslot alternative is struck.** That resolves decision #6's remaining
   ambiguity in the only direction I-NOAMP permits.
   ★ **And striking the alternative moved the capability off the trait entirely — §6.8.**

**★ The correction to §6.2, stated plainly because it is load-bearing.** §6.2 says:
*"The moment that publication must become guest-visible, it needs a memslot installed — a
`map_guest`."* **That sentence is wrong, and deleting it is half of why this amendment
exists.** Publication needs a *placement inside an installed window*, which is an
`mmap(MAP_FIXED)`: still a syscall, still forbidden under a lock, so **§6.2's plan/execute/
commit conclusion survives unchanged** — but it does not take `kvm->slots_lock`, does not
synchronise SRCU, and does not scale with vCPU count. §6.2's *architecture* was right and its
*cost model* was wrong by orders of magnitude, in the direction that would eventually invite
someone to "optimise away" the plan/execute/commit split as overkill for a memcpy-adjacent
op. Both halves matter: **keep the shape, fix the number.**

Consequently §6.1's table row `map_guest` / `unmap_guest` covers **two operations with
different costs and frequencies**, and the trait must say so. The memory plane's instruction
set is two-tier:

| Tier | Op | Cost | Frequency | In-lock legal? |
|---|---|---|---|---|
| **coarse** | install / remove a window (or an arena grant, if that is the chosen boundary) | memslot: `slots_lock` + 1–2 SRCU grace periods + shadow zap; QEMU: BQL | per window, or per proc create/destroy | **NO** |
| **fine** | place / restore a backing inside a window | `mmap(MAP_FIXED)`: `mmap_lock`, no KVM ioctl, no BQL | per publish / unpublish | **NO** (still a syscall — §6.2's shape stands) |

#### ★★ The measurement (2026-07-26) — the rule was right, and the cost has RELOCATED

The bench measurement this section was written to accommodate has landed. It changes no
constant and no shape; what it does is turn three of the paragraphs above from *arguments*
into *evidence*, and it moves the thing to worry about.

**[measured] The per-update cost, with running vCPUs.** `KVM_SET_USER_MEMORY_REGION`
DELETE/MOVE: **230–460 µs p50, 1–4 ms tail.** The two-swap claim above is confirmed
mechanically rather than by reading: kprobes on `kvm_swap_active_memslots` count **two** calls
per DELETE/MOVE against **one** for ADD, **965 of 965 samples**. So `:1798-1846` /
`:1929-1941` is not a plausible reading of the source, it is the observed behaviour.

**[measured] The cost is charged per vCPU, and the ioctl latency hides it.** Aggregate damage
is **≈135 µs × nvCPU per update**. Note the shape carefully, because it is exactly the shape
that makes this easy to under-count: **the ioctl's own latency is flat in vCPU count**, so a
naive benchmark on a 2-vCPU box reports the same number as a 64-vCPU box while the *cost* is
32× larger — every vCPU stalls, and only the issuing thread is timed. This is I-NOAMP's
sharpest instance measured rather than argued.

**[measured] ★ A real workload provokes ZERO memslot updates in steady state.**

| workload | memslot updates |
|---|---|
| full C Mode-2 lifetime: guest driver load, `cuInit`, `cuCtxCreate`, 400 kernel launches, 600 alloc/free | **0** |
| Mode-1 with the real **128 GiB** GPA window: probe | **1** (once, at probe) |
| Mode-1: a complete CUDA process on top of that window | **0** |

**The C is already at the theoretical floor**, and the rule above is therefore not an
aspiration to reach but an invariant to *hold*. The mock gate (below) is what holds it.

**★★ [measured] The per-object cost did not vanish — it RELOCATED, to host VMA churn under
`mmap_lock`.** A CUDA process adds **+32 VMAs**. That is the true price of "publication is a
`MAP_FIXED` placement": every placement splits or merges a VMA in QEMU's address space, under
`mmap_lock`. Two things follow, and they point in opposite directions:

- **It is much less bad than a memslot**, and structurally so: `mmap_lock` is **per-`mm`**. It
  does not stall other vCPUs, so it does not violate I-NOAMP the way a memslot update does.
- **It is directly relevant to R1**, and more so than the memslot ever was: `mmap_lock` is
  contended by *every* mapping operation in the same process, including ones issued on behalf
  of other guest procs sharing that QEMU. A `MAP_FIXED` under one of our locks is a blocking
  call under a lock whose blocking is now measured, not hypothetical. §6.2's plan/execute/
  commit conclusion survives its wrong premise (§13 item 9) **for this reason instead**.

**[measured] A no-op re-issue is free (~1.2 µs)** — KVM early-returns on an identical region
before any SRCU synchronisation. Consequence for API shape: an **idempotent "ensure this
window is installed" call costs nothing**, so the shell may call it unconditionally rather
than tracking installed-ness itself. Cheap idempotence beats a cache that can go stale.

**⚠️ Caveats, stated so the numbers are not over-quoted.**

1. **The bench host is itself a KVM guest.** Absolute numbers are inflated by nested
   virtualisation — **estimated 2–5×, not measured**. The *ratios* (DELETE vs ADD, flat ioctl
   latency vs linear aggregate cost, zero-in-steady-state) are the trustworthy part; the
   microsecond figures are an upper bound on real hardware.
2. **Multi-process is not covered.** Every number above is a single guest process. The
   interesting adversarial case — many procs churning arenas concurrently — is unmeasured.

**★ The watch item this creates.** `C: docs/design/mode2_bar1_memslot_perf.md` proposes
memslot-backing host-written read-mostly BAR1 pages. Whatever its merit for BAR1 latency,
**a per-page or per-object memslot boundary converts the measured zero into a rate** — the
exact conversion §6.7 exists to forbid, and one that would arrive wearing a performance
justification. If that idea ports, it must answer the three-part test below (slot-level
attribute, non-guest-controlled frequency, measured hostile bound) before a slot is minted,
and the honest answer is more likely "a second window with that attribute".

**What a measurement can and cannot change.** Written before the measurement above landed, and
left standing because it is what made that measurement cheap to act on: measurement can only
tune **constants**, never the shape — the shape is fixed by a correctness/DoS argument
(below), not by a performance one. The constants it can tune:

- **`ARENA_LEN`** — per-proc arena size. Larger = fewer grants for a churning process;
  smaller = more procs per window. The measurement supplies the distribution of per-proc peak
  GPA footprint. (Note this only affects memslot traffic if the memslot boundary is the
  *arena*; if it is the *window*, arena grants are free and `ARENA_LEN` is a pure
  address-space parameter.)
- **`WINDOW_LEN` and the window count** — how many arenas fit before a second window (and
  therefore a second memslot) is required. The C chose 128 GiB `MAP_NORESERVE`; the only
  thing it costs is virtual address space.
- **`ARENA_RETENTION`** — how long a released arena stays on `GpaSpace::free` before its
  window could be considered for teardown. Recycling is already LIFO and already implemented;
  the measurement decides only whether a window is ever torn down at all. **The honest
  default is never**, until address space is shown to matter.
- **The memslot boundary itself — window vs arena grant.** Both satisfy the rule. Window is
  strictly fewer updates; arena grant is strictly tighter guest-visible exposure (an arena
  belonging to no live proc is not addressable at all). **This is the one place the
  measurement can legitimately move the design, and it moves it between two allowed points**,
  not out of the shape.

**What would have to be true to justify a finer slot** — written down so that a future "let's
just add a slot here" has to answer it. A per-object memslot is justified only if **all three**
hold: (a) the object needs a *slot-level* attribute — KVM read-only, dirty logging,
`KVM_MEM_GUEST_MEMFD` — that cannot be obtained by placing it in a window that already has
that attribute; **and** (b) its install/remove frequency is bounded by something that is not
guest-controlled; **and** (c) the worst-case update rate under a *hostile* guest is bounded
and measured. Absent all three it is a **guest-reachable device-wide DoS**: a guest that can
provoke a memslot update per operation can stall every vCPU in the VM at will, from an
unprivileged process, with no ioctl of its own. **That is the security framing, and it is why
this is a rule rather than a performance note** — it lands on §11's checklist as item 11.

**The structural gate, available today with no OS code.** `MockVmm` already carries a
monotonic `next_slot` and a public `slots` map, so the *cumulative* install count is directly
observable — no new instrumentation, no clock, no OS. The mean run asserts the shape:
**slot installs grow with arena grants, not with publications** — `O(procs)` against
publications that the existing workloads already drive orders of magnitude higher. A
per-object regression fails it immediately and structurally, on any machine, in both lock
modes. It joins §9.3's gate table. *This is the same move as the reactor's wake-count assert
(§3.4): convert a cost that only real hardware can measure into a **quantity** a mock can
count.* ★ And the measurement gives the gate a **target rather than a trend**: the C's real
workloads provoke **zero** updates in steady state, so the assertion to hold is not "grows
slowly" but "does not grow at all once the windows are installed".

*What the measurement did **not** supply, so `ARENA_LEN` stays unpinned:* the distribution of
per-proc peak GPA footprint, and any multi-process arena-churn rate. Those remain the open
inputs to the window-vs-arena boundary.

### 6.8 ★★ `lock_region`/`unlock_region` leave the trait — a capability wearing a port's clothes

**The chain that removes them is entirely §6.7's, one step further.** §6.7 item 5 struck the
memslot implementation of decision #6, which leaves exactly one implementation:
**userfaultfd**. And the region it must be registered on is **our own `mmap`** — the window
`kayfabe-linux-raw` reserves and places backings inside (§4.4). `UFFDIO_REGISTER` on a VMA in
our own address space needs **no cooperation from the hypervisor on either backend**.

So the method fails the only test that decides trait membership:

> **A `Vmm` method must name something only the VMM can do.** `lock_region` names something
> *we* do, to memory *we* own, with a syscall *we* make.

Leaving it on the trait has a concrete price rather than a stylistic one: **every adapter
would have to contain the identical userfaultfd code**, and the second one would either
duplicate it or reach across into the first — which is the exact failure mode §6.0's contract
exists to prevent, arriving from the inside rather than from vocabulary drift.

**A second, independent defect at the same seam, and it is the decisive one.** The signature
is **slot-granular** — `lock_region(&mut self, slot: SlotId, on_fault: CoreEventKind)` — while
§6.7 requires **per-page** locking inside a *shared* window. The method as declared therefore
**cannot express what the design now demands**: the only thing it can say is "lock this whole
window", which is precisely the device-wide, every-proc revocation §6.7 item 5 struck. This is
why the choice is not between "move it" and "keep it with a page-range parameter": a corrected
signature would be a `kayfabe-linux-raw` call spelled through a hypervisor port, i.e. the same
mistake with better arguments.

**Decision.** ★ **Remove both methods from `Vmm`.** The trait drops from eight capability
groups to seven; arch §4.1's own *"count is not the invariant; hypervisor-agnosticism is"* is
the licence, and this is the first time it has been drawn on. The capability is documented as
`kayfabe-linux-raw`'s remit (the crate does not exist yet, so this is a trait **removal** plus
a design note, not a move of code).

**What deliberately stays on the `Vmm` seam, and why the split is exactly here:**

| Kept | Removed |
|---|---|
| `CoreEvent::LockedRegionFault { slot, gpa }` — the fault **delivery** | `Vmm::lock_region` — the arming |
| `CoreEventKind::RegionFault` — the tag a deferred re-entry carries | `Vmm::unlock_region` — the release |

The delivery stays because the core must observe the fault **on the same serialized executor
as every other event** — that is a property of the core's entry discipline, not of
userfaultfd, and it would be identical if the mechanism changed again. `slot` keeps naming the
containing **window**, never the granularity; `gpa` names the faulting page. Both rustdocs now
say so, so the per-page/per-window distinction is not re-lost.

**What this costs in tests:** `MockVmm` loses two method impls and `SlotRecord::locked` (which
nothing asserted on — the capability had **zero core call sites**, which is also why this was
free today and would not have been after L2 wired one).

**Residual, stated:** the uffd design itself — registration mode, the fault-handler thread's
placement, and its interaction with the `assert_lock_free` witness — is **not** designed here.
It belongs with `kayfabe-linux-raw`, and this section's only claim is where it belongs.

---

## 7. ★★ Decision 12 — cancellation and the reclamation lifecycle

> Owner, verbatim: *"cancelling operations is really important, also test that. this
> genuinely must not be a bolt on. clean cleanup on gpu getting idle, restart driver,
> process killed, isolate can be gc collected, etc etc. no leaks, safe."*

Cancellation is not a feature; it is **one trigger of a reclamation lifecycle** that has
eight triggers and eleven resource classes. Designing the cancel API alone would be the
bolt-on the owner is warning against. So: §7.1–§7.5 design the cancel seam, §7.6 enumerates
the eight triggers as one lifecycle, §7.7 states the property that makes them all correct,
and §7.8 makes the whole thing a *measurable ledger invariant* rather than a review
checklist.

### 7.0 ★ The fact that makes the whole section tractable

> **The isolate process boundary is the garbage collector.**

An isolate is one host process per `(Proc, GpuId)` holding one RM client connection. When
that process dies, the kernel closes its descriptors, and RM frees the **entire object tree
under that client** — every VAS, channel, engine object, mapping, and surface, whether or
not we knew about it. Its mmaps go with its address space. Its memory is unmapped.

This is why the blast-radius boundary (#14's isolation, MG-5's per-target split) is *also*
the reclamation boundary, and it is why "reclaim everything on every path" is achievable
rather than aspirational. Per-object reclamation is an **optimisation** (return resources
promptly, keep the host's handle count low); process death is the **correctness backstop**.
Every path in §7.6 says which of the two it uses.

**★ And in Mode-2 there are TWO collectors, one at each boundary — measured, not assumed.**
Killing a guest CUDA process makes the *guest kernel* free the guest's client tree with no
application cooperation: **178 `fn=10` RM-FREE RPCs, then fn-47**, while the host stub stays
alive and the host GPU returns to its exact baseline in ~11 s
(`../reference/mode2_bench_lifecycle.md` §5). So the forwarder sits between two working
garbage collectors and its only obligation is to **refuse to paper over the gap between
them** — which is exactly what the condemned-component rule does
(`l1_concurrency.md` §12.17). Two caveats bound this: the host-side observation is a *memory*
proxy (`nvidia-smi`), not an object count, and the kill was not proven to land strictly inside
an ioctl — so the G4 case (an alloc whose reply never arrived) is still open, with a strong
prior that the alloc **completed** (`../reference/rm_semantics_measured.md` §2).

The corollary — stated because it is the load-bearing constraint on everything else: **an
isolate must never be shared between procs, and its lifetime must never exceed its proc's.**
If either is violated, the backstop is gone and every reclamation obligation becomes a
promise that a human keeps.

**★ And the exception, which is the sharpest thing the audit found.** There is exactly one
path where nothing dies and the backstop therefore does not apply: **the guest frees a
*subset* of its objects while the process keeps running** (G2 — `refresh`'s `retain` drops
`Vas`es and `Channel`s holding live host state, with the proc still alive and its isolate
still healthy). On that path, per-object reclamation is not an optimisation — **it is the
only reclamation there is.** A long-running guest process that allocates and frees in a
loop is not an exotic case; it is a training job, an inference server, and every workload
this project exists for. So T0 (§7.6) is the one trigger whose correctness rests entirely
on the completeness of a cleanup list, and it is therefore the one that most needs the
ledger (§7.8) rather than an argument.

### 7.1 The cancel seam — where the API lives

The thread that could cancel is never the thread that holds the worker: the holder is
blocked inside the verb. So the cancel capability must be *separable from the `&mut
Worker`* without reintroducing a shared reference to the backend (which §12.8 deliberately
made unrepresentable).

```text
  Isolate::checkout()  -> Worker            (moves the backend OUT, as today)
                       +  CancelHandle      (stays in the pool slot, under the proc lock)

  CancelHandle:  Send + Sync, holds NO reference to the Worker or the backend.
                 Identifies (isolate, worker, txn) and nothing else.
```

- The **handle stays in core state**, reachable under the proc lock, so `Proc::retire`, the
  watchdog, and the device-teardown path can all reach it without touching the worker.
- **Firing is a two-step, reusing §3.6's post-entry drain.** `Isolate::request_cancel(worker,
  reason)` under the proc lock sets a flag and latches a `CancelRequest` (`#[must_use]`);
  the shell discharges the real signal after the guards drop. This is not ceremony: firing
  a cancel is a syscall, and §6.2 forbids syscalls under locks. The mechanism already
  exists for wake and timer; cancel is the third user, which is the argument that it is the
  right mechanism rather than a third one.
- **`txn` ids exist only for this** — §7.2 of `l1_concurrency.md` says so, and this is the
  only place they appear. A `CancelHandle` is armed for one txn; a request naming a stale
  txn is dropped. That is the C's refinement 4 verbatim (*"main thread only signals the
  worker if it is still on that txn_id"*), and the C needed it because without it a cancel
  races the completion and lands on an unrelated later operation.

### 7.2 The per-worker interrupt handshake

Ported from `C: signal_interrupt_delivery` / `signal_interrupt_delivery_done` (#73), whose
four refinements transfer without modification:

1. The isolate installs its break-signal handler **without `SA_RESTART`**, before the
   sandbox locks down. With `SA_RESTART` the *host* kernel silently restarts the ioctl, it
   never returns `EINTR`, and we never learn we interrupted anything — the failure mode is
   "cancellation appears to work and does nothing".
2. Cancellation is delivered **out of band** — a byte on the worker's control pipe (the
   channel itself is 1-deep request/reply and must not be desynchronised), and the isolate's
   control thread signals the worker thread.
3. The handler body sets a flag and returns. No syscalls beyond the allowlist; nothing that
   can block. The seccomp policy gains exactly `tgkill`, `rt_sigaction`, `rt_sigreturn` —
   the C's list, and nothing more (§11).
4. **Cancellation is armed for exactly one ioctl.** Once `EINTR` is observed it disarms, so
   the unwind's `free`s (§7.4) are not themselves interrupted. A cancel arriving during the
   unwind is a no-op; if the situation is that bad, the escalation is isolate death, whose
   backstop is §7.0.

And the one refinement the C learned the hard way, restated as the strongest rule in this
section:

> **★ The requester NEVER abandons the reply.** After firing a cancel, the requesting
> thread stays in `Worker::execute` until a reply arrives — `Interrupted{txn}` or the real
> one. It holds **no lock** while it waits (R1), so the wait costs nothing but that guest
> thread. Abandoning the reply desynchronises the channel: the *next* checkout of that
> worker reads the previous transaction's reply as its own. The C's version of this was a
> use-after-free (`C:` #73: *"it must not abandon the virtqueue descriptor (UAF) — it sends
> INTERRUPT, then waits uninterruptibly to reclaim the descriptor, then returns
> -ERESTARTSYS"*). Ours would be worse: silent cross-transaction corruption.

The single exception is §7.5's wedge escape, and it is safe for exactly one reason: it
retires the slot in the same act, so no next reader exists.

### 7.3 What a cancelled verb returns, and how commit tells three things apart

Per G4, `Worker::execute` returns `Result<VerbReply, VerbFailure>` where
`VerbFailure { err, orphans }`, and `RmError` gains `Interrupted` (no payload — the txn is
L1's business, not the core's). The commit path then distinguishes **four** outcomes, where
L1-M1 distinguishes two:

| Outcome | Truth | Surfaced fault |
|---|---|---|
| `Err(VerbFailure{ err: Interrupted, orphans })` | **we** cancelled it | **`FwdFault::Cancelled { reason }`** *(new)* — `reason ∈ {ProcExit, DeviceReset, Watchdog, GuestSignal}` — and `orphans` are disposed of before the fault surfaces |
| `Err(VerbFailure{ err: other, .. })`, proc live | genuine host failure | `FwdFault::Rm(e)` *(unchanged)* |
| `Err(VerbFailure{ .. })`, proc gone | divergent staleness | `FwdFault::Stale(Proc)` *(unchanged — §12.10)* |
| `Ok(reply)`, cancel lost the race | the verb finished first | **commit normally**, then R5 decides: if the proc vanished, refuse divergently and hand the fresh handles to `Orphans` |

**Cancellation is the third shape in §12.9's staleness table**, and that it *fits* is the
evidence the table was cut correctly:

| shape | example | resolution |
|---|---|---|
| converging | a sibling materialised the same host VAS first | re-plan from the top (bounded) |
| divergent | proc retired, channel torn down, route rewritten | refuse loudly — MISS = FAULT |
| **★ cancelled** *(new)* | the requester was interrupted, or its proc is going away | **non-retryable and orphan-carrying**: dispose, then refuse with the truth |

The fourth row is the subtle one and it is where a naive cancel implementation leaks: a
cancel that arrives after the verb completed must not cause the reply to be discarded,
because the reply names **host objects that now exist**. Discarding it leaks them silently —
the ledger (§7.8) is what catches this, and it is exactly the class of bug that no
refusal-shaped test finds.

§12.10's lesson generalises here too: **a fault must name the truth, not the symptom.** A
cancelled verb that surfaced `Rm(Other)` would be "the host failed" when the truth is "we
killed it" — and a canary asserting only "it refused" would pass for the wrong reason. The
mean test's cancel canary asserts the *variant*.

### 7.4 Disposition of already-allocated host objects — `Orphans`, reused

The requirement is explicit: reuse the `Orphans` mechanism, do not invent a parallel path.
It turns out to require **no new mechanism at all**, which is the best possible outcome and
worth spelling out:

- **Mid-chain, worker alive.** `Worker::execute` already unwinds: a chain that allocated a
  host VAS and a memory object and then failed at `map_gpu_va` frees both on the same worker
  before returning. `Interrupted` is just an `RmError` from a chain step, so cancellation
  inherits those semantics.
- **★ Mid-chain, worker DEAD — and this is where "for free" was wrong.** My first draft
  claimed cancellation inherits all-or-nothing for free. G4 refutes it: the unwind runs
  *inside* `execute`, on the worker, so a worker that died mid-chain cannot run it, and the
  handles it already minted are in no `Orphans` and in no core state. That is the exact
  premise of cancellation-under-teardown, so the "free" case is the one case that does not
  apply. **`VerbFailure { err, orphans }` (G4) is the fix**, and with it the disposal is
  still the existing path, just reached from one more place.
- **At commit.** A refused commit already returns `Refusal { fault, orphans, retry }`, and
  `verb_op` already runs `refusal.orphans.release_plan()` on the same worker, lock-free,
  before check-in. A cancelled-then-refused commit takes the identical path.
- **What must be added:** `Cancelled`'s refusal is constructed non-retryable (the world did
  not converge, it ended — §7.3's third shape); `commit_*` populate `orphans` on the
  cancellation path as carefully as on the retire path; and **`Refusal` and `Orphans` become
  `#[must_use]`** (G4), because *dropping a `Refusal` silently leaks its orphans* — the one
  place in this design where the compiler can do the reviewing.

So: **the cancel seam is small because stage 3 built the right shape — but it is not free,
and the un-free part is precisely the violent case.** The reason cancellation is at bolt-on
risk is that the *vocabulary* does not exist, not that the machinery does not.

### 7.5 ★ The D-state case — bounded, loud, and honest

A host thread in uninterruptible sleep cannot be signalled awake. Then `execute` never
returns, and per §7.2 the requester never abandons the reply — so that guest thread is
stuck. §11 B3 already accepts a residual here; this makes it bounded.

**A two-stage watchdog, on the virtual clock so it is testable:**

1. **At checkout**, arm `Vmm::defer(VERB_BUDGET, VerbWatchdog{proc, gpu, worker, txn})`.
   (Bounded by construction: armed only while a verb is outstanding, disarmed at check-in —
   F1's "never periodic-forever" satisfied.)
2. **First expiry** (executor): fire the cancel. The overwhelmingly common case is that the
   verb was merely slow and the interrupt lands.
   ★ **`VERB_BUDGET` is sized against RM's OWN timeouts, not against a measured unwind**
   (`l1_concurrency.md` §12.26): a GSP RPC times out at **6 s** (4 s `defaultus`,
   `ogkm: .../os/os.c:2136-2139`, x 1.5 at `.../gpu/gsp/kernel_gsp.c:2927`), and that wait
   can itself be queued behind *another client's* hold of the global API lock. So the budget
   must exceed 6 s plus queueing, and the second budget much more. Sizing it against the C's
   "~3.5 s unwind" would have made every ordinary GSP-RPC timeout look like a wedge — the
   number was RM's 4 s timeout, and RM's waits are uninterruptible
   (`.../gpu/gsp/kernel_gsp.c:2963-3060`), which also means **an interrupted alloc probably
   completed** (the G4 open question, still owed a bench measurement).
3. **Second expiry:** declare the worker **wedged**. Then, in one act:
   - `worker_died(slot)` — the slot is permanently dead, never respawned (existing);
   - **condemn the component** — the §12.13 mechanism, keyed on the client set, already
     built and already tested (existing);
   - **abandon the reply** and release the requesting guest thread with
     `FwdFault::Wedged`;
   - `SIGKILL` the isolate process, and never `join`/`waitpid`-block on the wedged thread —
     joining a D-state thread is itself unbounded, and reaping happens via the exit
     descriptor whenever the kernel gets around to it (§7.6).

**Why abandoning is safe here and nowhere else:** the desync hazard of §7.2 is that a
*future* reader of that channel misreads the stale reply. There is no future reader,
because the slot is dead in the same act and slots are never resurrected. The safety of the
escape is *conditional on the condemnation*, and the two must therefore be one operation,
not two steps someone can reorder.

**The requester's wait must be interruptible by this** — and it must not be a timeout, or
the mean test acquires a clock dependency, which §8.4 forbids. So the wait is
`reply_ready || abandoned`, where `abandoned` is signalled by the watchdog. In the mock,
"the socket" is a condvar and the abandon signal is the same condvar: the D-state case
becomes **deterministically testable with no sleeps at all**.

**The honest residual, stated plainly:** the D-state host thread and its RM objects leak
until the kernel finishes the ioctl. `SIGKILL` does not reap a task in uninterruptible
sleep. What we convert is *unbounded silent stall* → *bounded loud failure plus a leak we
can name, count, and report*. One guest process dies; the device, the other procs, and the
host stay healthy. That is strictly better than the C, which wedged the whole GPU on these
(F5), and it is the best available: no user-space design can kill a D-state thread.

### 7.6 ★ The eight teardown triggers — one lifecycle

Resource classes tracked on every path: **(R1)** host RM objects · **(R2)** host VAS +
GPU mappings · **(R3)** isolate processes (and zombies) · **(R4)** descriptors · **(R5)**
mmaps / double-mmaps · **(R6)** GPA arenas (whole, and **intra-arena** — G6) · **(R7)**
completion sources · **(R8)** pool workers + channels · **(R9)** queued inbox events naming
dead procs · **(R10)** per-`GpuId` targets, windows and delivery planes (G9) · **(R11)**
condemned-component entries (G10).

**Every path returns its reclaimed objects to be dropped lock-free** (G3b's shape). No path
drops an isolate, a mapping, or a worker while any rank is held; that is a live R1 violation
with no assert, because `assert_lock_free` guards verbs and not `Drop`.

---

**T0 — ★ The guest frees a subset while the process keeps running (G2).**
*The path with no backstop (§7.0), and the most frequent one in a real workload.*
`refresh`'s `p.vases.retain(…)` / `p.channels.retain(…)` discard `host_vas`, bound
`host_va`s, `host_channel`, `host_token` and `host_engine_objects` while the proc is alive
and its isolate is healthy. Nothing dies, so nothing reclaims. Design:

- **Fill before you drop.** Immediately before each `retain`, the dropped values' host
  identities are moved into a **`pending_release` queue on the `Proc`** — reusing `Orphans`
  as the payload type, so there is exactly one disposal vocabulary in the system.
  G1's `Binding`-carries-the-backing-identity fix is what makes this expressible at all:
  without it the queue could hold a `host_va` to unmap and no `memory` handle to free.
- **Drain lock-free.** `refresh` runs under the device write lock, so R1 forbids issuing
  the verbs there. The queue is drained on a checked-out worker via
  `Orphans::release_plan()` — the same mechanism the refused-commit path already uses, on
  the same worker discipline (§7.4). Draining is a *plan-and-execute* op like any other,
  so it obeys R5: a proc that retired before its queue drained simply hands the queue to
  the retire path, which is already a bulk-release site.
- **When to drain.** Opportunistically at the next verb-issuing op for that proc (the
  worker is checked out anyway — near-zero marginal cost), with the executor as the
  backstop for a proc that goes quiet. Never inline in `refresh`. **★ §6.6 makes the thread
  placement a rule rather than a preference:** the opportunistic drain rides a caller who is
  already there (so it is billed to the proc that created the garbage), and the backstop runs
  on the **executor** because it has no caller to bill — which is also what keeps it from
  violating I-NOAMP by parking a thread another proc needs.
- **The related half:** `sync_proc_rpc_bindings` only unbinds VAs in `vas.rpc_bound`, so a
  `publish_backing` binding is never unbound by *any* path today. The publish path must
  register its bindings where the unbind path can see them, or T0 leaks exactly the
  allocations the data plane makes most of.
- **Ledger:** this is the trigger the conservation invariant exists for. ~~A long-running
  process that maps and unmaps in a loop is the mean test's `ctl_workload` — so T0 is
  already being exercised thousands of times per run, and is today silently leaking every
  time.~~ **★ MEASURED AND WRONG (§14.1, `l1_concurrency.md` §12.32).** `ctl_workload`'s
  churn adds and removes *RPC* bindings, whose `Binding::host` is `None`: they own no host
  object, so dropping them leaks nothing. The mean run's other two subset-frees target
  channels the script deliberately leaves virgin. **T0 was not exercised at all**, and the
  first conservation census over the unmodified script correctly reported zero. Reaching it
  needed a new phase (`t0_churn`) that materializes host state *before* it frees — which is
  the general lesson: a reclamation test that does not first allocate proves nothing.
- **★ And the drain needs a precondition this section did not have.** "At the next
  verb-issuing op" is not sufficient: freeing a host object underneath a verb that is still
  *using* it is a use-after-free, and no lock can exclude an in-flight verb because verbs run
  lock-free by construction. The drain fires only when that `(proc, gpu)` isolate is
  otherwise **idle** — `Isolate::is_quiesced`, the same predicate T3's reap uses for the same
  reason. Consequence, named in §14.1: T0 is lazier than this section describes, and a
  never-idle proc needs the backstop sweep to be armed by a real trigger (M2-f).

**T1 — Verb cancelled mid-flight** (§7.1–§7.5).
Cancel latched → discharged lock-free → `EINTR` → `Interrupted` → the chain's own unwind
frees what it allocated (**R1, R2** — per-object) → `FwdFault::Cancelled` → worker checked
in (**R8**) → watchdog disarmed. Nothing was adopted, so no arena was carved (**R6**
untouched — carving happens in commit) and no source was registered (**R7** untouched).
*Backstop if any of this fails: none needed — the proc is still alive and healthy.*

**T2 — Guest process exits** (normally, or killed, or killed *while a verb is pending*).
The guest kernel frees the process's client root on fd close, so all three cases are the
**same path**: an `RmEvent` free → `refresh` finds no boundary → the **graph-driven** retire
(which, per `the_graph_driven_retire_paths_never_condemn`, condemns nothing — a guest
tearing itself down must never DoS itself). Then:
`Proc::retire()` → every per-target isolate retires (refuses new checkouts, **R8**) → **for
every checked-out worker, `request_cancel(ProcExit)`** (this is the new part; today retire
does not cancel, so a pending verb runs to completion against a dead proc) → each such verb
returns and its commit R5-refuses divergently, handing its fresh handles to `Orphans`
(**R1, R2**) → `deregister_proc` clears every source (**R7**) → the proc lands on
`spine.retired` awaiting T3.
*Backstop: the isolate is killed at T3/T6 and the process boundary frees anything missed.*

**T3 — ★ GPU idle / the quiesce point. The guest TELLS us, and the C already listened.**

Law 8 is currently half a law: `reap_retired` exists, `CoreEvent::Deferred(DeferredReap)`
exists, and **nothing arms it**. But the fix is not primarily a timer — I was designing one,
and the C's answer is better and is measured:

> **The guest driver emits `UNLOADING_GUEST_DRIVER` (RPC fn 47) both on a real driver
> unload AND on a GPU-idle release when the last client/context exits with the module still
> loaded**, and it then re-runs the queue handshake. The C names the resulting moment
> exactly: *"the re-handshake = the quiesced point (GPU was idle-released; next context
> boots)"* — and runs its deferred reap there (`C: nvkvm_gpu_emul.c:3458`, `:1988–1994`).

So the quiesce trigger is **an observed protocol event, not a guess about idleness**, which
is the same doctrine as the address table (forward-populated by declared facts, never
reverse-inferred). Design:

- **Primary trigger:** the GSP queue re-handshake (the tx-header write). This is what
  `l1_concurrency.md` §7.3 already gestures at ("the quiesce point the adapter declares —
  GSP re-handshake / idle") and it is now cited, not hoped for.
- **Why deferred and not eager, in the C's own words:** reaping resolution/backing state
  *at* the client-root free **hangs the dying context's own residual polls** —
  bench-proven on `cupctx2_min`'s CTX2 destroy (`C: nvkvm_gpu_emul.c:1988–1993`). That is
  lesson L10's actual evidence, and it is why "just free it immediately" is not on the table.
- **Predicate** (G3's `is_quiesced()`, checked on the executor under one device write guard):
  (a) every target's completion drain gate is closed/empty; (b) **no worker of any retired
  proc is checked out** — this is the G3 hazard, where the reap would otherwise tear an
  isolate down under a live connection held by a foreign thread; (c) no verb is in flight
  against a retired proc. **A non-quiesced proc goes back on the retired list** rather than
  being dropped.
- **Shape:** `reap_retired` returns `Vec<Proc>` (G3b); the caller drops them **with no lock
  held**, so a real isolate's `Drop` (waitpid, namespace teardown) cannot block under rank 0.
- **Secondary trigger:** a `DeferredReap` deadline armed whenever `spine.retired` becomes
  non-empty — and only then, never periodic (F1). This is the backstop for a guest that
  simply keeps running and never idles.
- **★ Escalation — the part that makes it a law again:** after `MAX_REAP_DEFERRALS` the reap
  escalates: cancel the blocking verbs (T1), and on the second failure declare them wedged
  (§7.5) and reap anyway. **A wedged verb must not pin a reap forever**, or "reap-deferred"
  is a leak with a lesson's name on it. The C shipped the un-escalated version and recorded
  the consequence honestly as its residual: *"no idle/timeout reaper for a guest that goes
  silent… fully reclaimed at VM stop (process exit)"* (`C: teardown_hardening_done`). We can
  do better than "reclaimed when QEMU exits", and this is where.

**T4 — ★ Guest driver restart / GSP re-handshake (G5). Two flavours, and they are not the
same event.**

There is no device reset in the core today (G5): on `rmmod`/`modprobe`, guest panic, or VM
reset the guest sends **no `Free` events**, so the graph, every live `Proc` with its
isolates/arenas/host handles, `condemned`, `retired`, `sources`, routing maps and `targets`
all survive into the new driver life, and the new life derives a *second* set of components
beside the corpses. The C's WPR2 limitation is inherited by omission.

**What the C actually does, and what it teaches** (`C: nvkvm_gpu_emul.c:2450–2478`,
`:3462–3487`, `:4208–4262`) — this is the section where invention is most tempting and most
dangerous, so every rule below is cited:

1. ~~**One RPC, two triggers.** fn-47 is emitted for a real unload *and* for an idle release.
   The emulator cannot tell them apart from the RPC alone.~~
   ★★ **FALSIFIED ON THE BENCH (2026-07-26) — and the correction is worse than the claim.**
   `rmmod` emits **no fn-47 at all**: the idle release at process exit already consumed it
   (`../reference/mode2_bench_lifecycle.md` §2). So the problem is not that two triggers are
   indistinguishable — it is that **the unload has no trigger**. Anything armed on fn-47 (the
   quiesce point in T3, a future `Spine::device_reset`) simply does not fire on a real driver
   restart. A restart-time reset needs its own observable signal; finding one is M2-f work and
   is not settled here.
2. **Reset at fn-47 only the boot-gating state:** WPR2 down (`fwsec_ran = false`),
   `gsp_suspended = true`, `bootargs_dumped = false`, `q_ready = false`. Each has a named
   failure mode for omitting it: leaving WPR2 up makes `_kgspBootGspRm` bail
   `NV_ERR_INVALID_STATE` ("unexpected WPR2 already up"), a false cascade that masks the
   real failure **and forces a full VM/QEMU restart**; leaving `bootargs_dumped`/`q_ready`
   set means the tx header is never re-detected and `GspStatusQueueInit`→`msgqRxLink` times
   out (`kernel_gsp_tu102.c:570`) — "the original reload bug".
3. **Do NOT reset the queue counters at fn-47.** The guest sent fn-47 at the current
   `rxSeqNum` and polls for its ack at that seqNum; zeroing first sends the ack at seqNum 0
   → "Bad sequence number" → Xid 119 → **teardown corrupts, and the NEXT context inherits a
   broken GPU** (the sequential/multi-process #12 hang).
4. **At the re-handshake, reset the write POSITION and PRESERVE the seqNums.** The driver's
   `MESSAGE_QUEUE_INFO` (with its rx/txSeqNum) is built in `kgspConstructEngine` and torn
   down only in `kgspDestruct` (**module unload**) — *not* on the cuCtxDestroy-of-last-ctx
   idle release — so it persists across the re-boot, while the per-boot
   `GspStatusQueueInit`→`msgqRxLink` resets only `rxReadPtr`. Reset the seqNums here (as an
   older one-shot boot did) and the re-posted `INIT_DONE` arrives at seqNum 0 ≪ N, msgq
   treats it as an old package and ignores it (`message_queue_cpu.c:762,768`), and the 2nd
   context hangs in `kgspWaitForRmInitDone`.
5. **Disambiguate the post-teardown STARTCPU.** SEC2 Booter Load raises WPR2 and Booter
   Unload lowers it; from BAR0 they differ only by a mailbox value. A trailing-teardown
   STARTCPU must not re-raise WPR2, while a genuine re-boot must — or the 2nd context hangs
   forever waiting for a `GSP_INIT_DONE` that never comes.

**★ The honest verdict on "do we inherit the C's fresh-boot limitation?"** Partly designed
out, and one part is a **named unknown**:

- **WPR2 itself: designed out, and the C already did it.** fn-47 clears it in-process
  precisely so a re-insmod does not need a QEMU restart. `kayfabe-gsp`'s "resettable
  in-process (lesson L12)" is therefore true *of the GSP FSM* — and false of the `Spine`,
  which has no reset at all (G5). That gap is this milestone's work, and it is the thing
  that would silently re-impose the fresh-boot tax if left.
- **★★ THE EXPERIMENT RAN (2026-07-26), and it moved the blocker.** The question was whether
  a true `rmmod`/`insmod` restarts `rxSeqNum` at 0 (because `kgspDestruct` destroys
  `MESSAGE_QUEUE_INFO`) while the idle release preserves it, and therefore whether
  `Spine::device_reset` needs a *flavour* parameter. **The seqNum question is real but
  downstream: the run does not survive far enough to reach it.** Full account in
  `../reference/mode2_bench_lifecycle.md` §3. What was measured:
  - **WPR2 is correctly lowered.** That half of the C works, and the framing this bullet
    inherited — "WPR2 is what forces the fresh boot" — is **wrong**.
  - **The blocker is a latch/stale-queue chain.** The teardown `STARTCPU` arrives with
    `was_suspended == true`, is misclassified as a **re-acquire**, re-latches
    `bootargs_dumped` / `q_ready`, and the next driver life is left pointing at the **dead
    queue's GPA**. The failure is a **`msgqRxLink` timeout** — **not** Xid 119 and **not** the
    #12 site, which are the failure modes for zeroing the counters at fn-47 (rule 3), a thing
    the C correctly does not do. Debugging this by hunting Xid 119 goes to the wrong subsystem.
  - ★ **Design consequence, concrete: a Rust `device_reset` that models only WPR2 would not
    fix this.** It must also clear the latches, invalidate the queue-GPA binding, and classify
    the trailing-teardown STARTCPU on something other than `was_suspended`. That is a
    requirement on `Spine::device_reset` + `kayfabe-gsp`, not a tuning note.
  - **Still open, and now correctly ordered:** (a) what observable signal marks a true driver
    restart, given rule 1's falsification — fn-47 is not it; (b) *then* the seqNum flavour
    question, once a restart gets far enough to ask it.
  - **And the C's stale state is guest-reachable** — arbitrary guest RAM parsed as GSP RPC and
    answered `NV_OK` (508 lines per failed bring-up), plus an unguarded `% s->q_msgcount`
    SIGFPE (`C: nvkvm_gpu_emul.c:1615`). Both are C-only; the shapes they forbid are recorded
    in `../reference/mode2_bench_lifecycle.md` §4 and belong on §11's checklist as a
    do-not-reproduce.
- **Obligations once triggered (device-scoped, G3b's shape):**
  `Spine::device_reset(...) -> Vec<Proc>` — every proc retired and returned for lock-free
  drop; every isolate killed and reaped (T6); every source deregistered (**R7**); every
  guest-visible memslot removed (**R5**, via §6.2's memory-plane instructions); every arena
  back in its window (**R6**); `targets` pruned (**R10**); the graph and `condemned` cleared
  (**R11**); **and the mints kept monotone** — `next_proc`, the `CompletionSource` counter
  and `geom.next_base` must never rewind, or a stale handle re-binds across the reset and
  §7.7's whole argument collapses.
- **★ Nothing may be re-adopted — and structurally it cannot be.** A plan in flight across
  a reset holds a `ProcId`, and `commit_phase` keys on that `ProcId` (deliberately: *"a proc
  that vanished in the gap is itself the loudest staleness answer"*). Every identity is a
  monotonic mint, so no stale reference can alias a new-life object, and R5 refuses it with
  `Stale::Proc` on the existing path. **No epoch counter is needed** — I considered one and
  it is redundant machinery, *conditional on the mints staying monotone across the reset*,
  which is why that is called out as an obligation above rather than assumed.
  The one place this could go wrong is worth naming: the *guest's* keys (`Pdb`, `VChid`)
  **are** recycled across a restart, so a commit that re-resolved through `by_pdb`/`by_vchid`
  could bind to a new-life channel with the same numeric key. It does not, because commit
  keys on `ProcId` first. That is a property, not an accident, and it gets a canary: *a verb
  in flight across a device reset must never commit into the new life.*

**T5 — Isolate death (worker HUP / sandbox failure).** The routing story is done (§12.13:
condemned components, keyed on the client set, `FwdFault::Condemned`). The **reclamation**
story has four gaps to close here:
- **★ the mid-chain orphans (G4).** `Worker::execute` promises all-or-nothing by unwinding
  internally — but if the worker **died mid-chain**, which is the entire premise of this
  trigger, the unwind cannot run, and everything allocated before the failure is in no
  `Orphans` and in no core state. It is unrecoverable, and no test today can see it. G4's
  `VerbFailure { err, orphans }` is what carries those handles out to a path that can
  dispose of them — on a *sibling* worker if the isolate still lives, or by isolate death
  below. This is the single most leak-prone moment in the design and it currently has no
  vocabulary at all.
- the dead worker's channel descriptors must be closed and its source deregistered
  (**R4, R7**) — deregistration rides `deregister_proc`; the close is new. **G8 bites here
  too:** `SourceKind::OsEvent` carries no channel/`Vas` identity, so a *per-channel*
  deregistration (T0's case, and a partially-failing isolate's) cannot be written — only
  the whole-proc sledgehammer. Add the identity now; it is a field, and later it is a
  migration.
- the isolate **process** may still be alive with N−1 workers. ★ **The design answer is to
  escalate worker death to isolate death**: `SIGKILL` the isolate. A worker that died
  mid-chain left allocations that are in no `Orphans` and in no core state (G4 above), so
  no path can enumerate them; per §7.0, killing the process is precisely how you reclaim
  state you cannot enumerate — the kernel closes the RM connection and frees the lot.
  Attempting per-object cleanup through a sibling worker would mean reasoning about
  exactly the set we just said is unenumerable. Note this is a *reclamation* argument and
  it stands on its own; it is **not** the reason the component is never resurrected —
  that reason is that the guest's data died with the isolate's RM client
  (`l1_concurrency.md` §7.3, "Why a condemned component is never resurrected").
- the component stays condemned until the guest frees its client root (existing, tested).

**T6 — Isolate garbage collection.** Triggered by T2's reap, T4, T5, or T7.
`SIGKILL` → the process's **exit descriptor** becomes readable → reactor → `IsolateExited`
→ executor: `waitpid` (non-blocking; we know it exited, so no zombie and no blocking wait —
**R3**), close every channel and control descriptor (**R4**), tear down every double-mmap
of guest RAM into it (**R5**), release its arenas (**R6**), deregister its remaining sources
(**R7**), drop its worker slots (**R8**). *The kernel has already released **R1** and **R2**
by closing the RM connection* — §7.0. **The exit descriptor makes this race-free**: the
alternative (`kill` then `waitpid` keyed on a pid) races pid reuse, which is a
kill-the-wrong-process bug in a design whose whole premise is isolation.

**T7 — Whole-device teardown (VM shutdown or reset).** An **explicit, ordered `shutdown()`**,
not a `Drop`:
1. stop accepting traps (the adapter's job — after this, no new work enters);
2. `request_cancel(DeviceReset)` for **every** checked-out worker, discharged lock-free;
3. wait bounded for replies; wedged ones take §7.5's escape;
4. retire every proc; run the reap unconditionally (the predicate is satisfied by 1–3, and
   the escalation covers what is not);
5. kill and reap every isolate (T6);
6. **drain the inbox** and assert every remaining event resolves to a fault, never a
   mutation (**R9**) — a queued `SourceSignal` naming a dead proc is exactly the F4 shape,
   and it is safe only because handles are never recycled, which is worth *asserting* rather
   than believing;
7. signal shutdown to the reactor and executor and **join** them — no lock held across a
   join (R1);
8. close every remaining descriptor, unmap every region;
9. assert the ledger (§7.8) balances.

`Drop` is a **tripwire, not a teardown**: if `shutdown()` was not called, it logs loudly and
does best-effort cleanup. Tests assert the tripwire never fires. This mirrors the
retire/reap two-stage discipline the project already uses per-proc, lifted to device scope —
and it avoids the trap of doing fallible, blocking work in a destructor, where errors have
nowhere to go and a panic during unwind aborts.

### 7.7 The property that makes all eight paths correct

Every one of the paths above relies on the same two facts, and it is worth stating them
together because together they are the whole argument:

> **(i) Every identity in the system is a monotonic, never-recycled mint** — `ProcId`,
> `CompletionSource`, `IsolateId`, `HostHandle`, `ChanId`, worker slots. So *nothing stale
> can ever alias anything fresh*, on any path, and MISS = FAULT does the rest. This is why
> the violent paths need no extra machinery (no epoch, no generation counter, no
> revalidation token).
>
> **(ii) The process boundary reclaims what per-object cleanup missed.** So correctness
> never depends on the completeness of a cleanup list — only promptness does.

The one thing (i) does **not** cover is a host-side descriptor number, which the kernel
*does* recycle. §3.2 handles it, and that is why §3.2 is a load-bearing decision and not an
optimisation.

### 7.8 ★★ CONSERVATION OF HOST RESOURCES — the ledger

> **For any scripted lifecycle — however violent, in any interleaving — every host resource
> acquired is eventually released exactly once: never zero times, never twice.**

Stated as a ledger property so it is *measured*, not reviewed.

**Where it lives.** `kayfabe-mocks::HostLedger`, behind the existing `RmRecorder`. This is
the crucial placement decision: every mock verb already funnels through
`MockRmBackend::record`, so **a verb that is not in the ledger does not exist** — a new verb
cannot escape accounting by being forgotten. Isolate lifecycle (spawn, kill, exit, reap)
funnels through `MockIsolate`/`MockIsolateFactory` the same way. Nothing is opt-in.

**What it tracks.**

| Class | Acquire | Release | Notes |
|---|---|---|---|
| **R1** host objects | every handle-minting verb | `free`, or **namespace death** (bulk) | namespace death is a *modelled* bulk release, per §7.0 |
| **R2** mappings | `map_gpu_va` | `unmap_gpu_va`, or namespace death | keyed `(vas, host_va)`; a mapping may not outlive its VAS |
| **R3** processes | `spawn` | `exit` + `reap` | a spawn with no reap is a zombie |
| **R4** descriptors | raw-crate open/create | close | counted at the raw seam, harness-side |
| **R5** mmaps | raw-crate map | unmap | " |
| **R6** arenas | `carve` (whole) + `alloc` (intra) | `release` (whole) + intra-free (**G6: does not exist**) | the C's fix is instructive: a per-VM free list, first-fit + tail/adjacent coalesce (`C: nvkvm_mmap_host.c:172`) |
| **R7** sources | `register` | `deregister`/`deregister_proc` | core-side, already observable |
| **R8** workers | `checkout` | `checkin`/`worker_died` | `idle == pool_size − dead` at rest |
| **R10** targets | `ensure_target` (guest-driven! G9) | device reset / prune | today: minted from a **guest-supplied** id, uncapped, never pruned |
| **R11** condemned | `retire_proc` | the guest frees its client root | a retention mechanism is also a leak class (G10) |

**The end-of-run assertion** — one call, seven properties:

```text
  ledger.assert_conserved_at_quiesce()
    L1  BALANCE      every acquired resource has exactly one release (direct or bulk)
    L2  NO DOUBLE    no resource released twice — including "freed, then freed by
                     namespace death", which is the shape a leaky T5 produces
    L3  NO FOREIGN   no free of a handle from another isolate's namespace   (blast radius)
    L4  ORDERING     no mapping outlives its VAS; no object outlives its client
    L5  CORE SIDE    every retired proc's arena is back in ITS target's free list exactly
                     once; every source deregistered; workers all accounted
    L6  INBOX        every event still queued resolves to a fault, never a mutation
    L7  RIGHT WINDOW every arena is released into the window it was carved from
                     (G7: today only a `debug_assert!` — compiled out in release, so
                     releasing into the WRONG window is representable, and the symptom
                     is overlapping recycled arenas = the #14 collision class returning)
```

**★ G6 is the one class the ledger will report as unfixable without a core change.** There
is no intra-arena free, so a long-running process's arena monotonically fills; L1 will
balance at *proc* granularity and the ledger will still show intra-arena occupancy that
never returns. That is not a false positive — it is the C's #80 leak reproduced at a finer
granularity, and it is exactly the "clean cleanup on the GPU going idle" case the owner
named. The C's fix is the template: a first-fit free list with tail/adjacent coalescing,
freed from the unmap path *and* from the reaper (`C: nvkvm_isolate_handlers.c:1954/2009/
2054/2431`). Recommend porting that shape rather than inventing an allocator.

**How it composes into the mean test — not beside it.** This is the requirement, and §12.13
is the reason for it: `worker_death_retires_the_proc_loudly_and_never_resurrects` was green
*only because it never issued an `apply` afterwards*. **Isolated tests test what you thought
of.** So:

- `mean_run` returns the ledger, and the existing `sweep_conservation` grows a seventh
  section that calls `assert_conserved_at_quiesce()` — under **both** lock modes, as
  everything there already runs.
- The mean run's existing "process that tears down mid-flight" thread becomes a
  **lifecycle-chaos thread**: on each iteration it picks a `(trigger, KillPoint)` from a
  seeded permutation and executes it, while the alloc/map-heavy, doorbell-heavy and
  poll-heavy threads keep running against the same two GPUs.
- **The assertion runs after the run's final reap**, never in a `Drop` — §12.14's lesson
  (*"a held latch must be released on unwind, or a failed mean assert reads as a hang"*)
  says loudly that assertions inside teardown machinery present as wedges, and an
  assert-in-`Drop` is that trap with a bow on it.

**★ The adversarial angle — the `KillPoint` matrix.** Cancellation at a *convenient* moment
proves nothing. Enumerate the inconvenient ones:

```text
  KillPoint ∈ { BeforePlan, AfterPlan_BeforeCheckout, AfterCheckout_BeforeVerb,
                MidChain(k)  — after the k-th verb of a multi-verb chain,
                AfterVerb_BeforeCommit,
                DuringCommit_Contended   — a sibling wins the same CAS (§12.9),
                DuringReplanRetry        — inside the bounded retry loop,
                AfterCommit_BeforeCheckin }

  Trigger   ∈ { PartialFree(T0), VerbCancel(T1), ProcExit(T2), IdleRelease(T3),
                DriverRestart(T4), WorkerHUP(T5), IsolateKill(T6),
                DeviceShutdown(T7), ReapEscalation }
```

The cross product is a table-driven test; each cell runs a small scenario and asserts the
ledger balances **and** the surfaced fault names the truth (§7.3). `MidChain(k)` is the one
that finds real bugs: a `Publish` chain that has allocated a host VAS and a memory object
but not yet mapped is the state where "release what you allocated" is neither empty nor
total. `DuringCommit_Contended` is the second: it composes cancellation with §12.9's
converging-staleness retry, which is the only place in the design where an op legitimately
loops.

**Falsification, as §12.14 did it.** The assertion's teeth must be *demonstrated*, not
assumed: deliberately drop one `Orphans` release and confirm L1 fails; deliberately free
twice and confirm L2 fails; skip the arena release and confirm L5 fails. Run all three, and
record the result in the contact log. A conservation assertion that has never failed is a
conservation assertion nobody has checked.

### 7.9 What is **not** mock-testable — and therefore waits for L3

Stated so we never claim more than we verify:

| Claim | Why the mock cannot settle it | The L3 measurement |
|---|---|---|
| the **host driver** actually released the objects | our ledger records our own verbs, not RM's accounting | RM memory/handle accounting deltas across a full lifecycle run on real hardware; must return to baseline |
| a real RM ioctl is actually interruptible | the mock's `EINTR` is instant and total. ★ §12.26: the source says **mostly not** — the API lock is a `down_write`, the GSP RPC busy-polls with no signal check, the client drain is a bare refcount spin. The C's "3.4–3.5 s, bounded and consistent" is the signature of a **timeout**, not of an unwind | block in a long op, signal, and measure: if the latency tracks RM's 4 s `defaultus` rather than the signal, it is a timeout — then check whether the object was created anyway (the G4 question) |
| the D-state escape behaves as designed | you cannot mock uninterruptible sleep | induce a genuinely wedged ioctl; assert bounded loud failure and a healthy device afterwards |
| a real 16/64 KiB host page size works | forced runs test our geometry, not the kernel | run the suite on Grace-Hopper / Jetson (§5.3) |
| memslot/BQL interaction | `MockVmm` has neither | the QEMU adapter at L2, under the nested-virt bench |
| descriptor exhaustion under a real `RLIMIT` | mock descriptors are integers | arm past the cap on a real host; assert contained refusal |
| **★★ the trap path holds NO FOREIGN LOCK** (§6.3's class rule — stated backend-independently, because the QEMU answer must never be mistaken for the general one) | `MockVmm` owns no lock we did not construct, and the harness owns its own threads — so *no* mock can distinguish "we hold none" from "this backend imposes none" | measure it the same way on either backend: park a verb on one vCPU and assert another vCPU's unrelated MMIO still completes. **Two expected outcomes, and they differ:** **QEMU — conditional pass:** requires `memory_region_clear_global_locking()` to exist on the target version and to be applied to our BAR regions (an API-availability *check*, not an assumption). If it is unavailable, the drop-the-lock rule is necessary but not sufficient and I-NOAMP cannot be met on the trap path — **an L2 blocker, not a tuning item**. **CH — unconditional pass expected:** MMIO dispatch is a synchronous `VmOps` call on the vCPU thread with no VM-wide lock, and with the direct-`BusDeviceSync` registration (§6.3) not even a per-device one. A CH failure here would mean the escape was not taken |
| **★ the real memslot cost and update rate** (§6.7) | mock `map_guest` is a `BTreeMap` insert | measure `KVM_SET_USER_MEMORY_REGION` latency vs vCPU count, and count updates provoked by a real workload. Tunes the §6.7 constants; the mock-side *frequency* gate already pins the shape |
| the guest driver really frees its client root on process death | the mock guest is a script | a real guest process killed mid-op; assert full reclamation |
| tearing on GPU-written pages | no hardware writer | high-rate semaphore observation under real load |

---

## 8. The thread-safety contract — the new rows

Extending `l1_concurrency.md` §9's normative table; each becomes a rustdoc header.

| Interface | Send/Sync | Who calls, from where | Contract |
|---|---|---|---|
| reactor loop thread | owns its readiness set exclusively | nobody calls it | touches ZERO core state — and now holds **no shared structure at all** (§3.2). Produces inbox events only |
| `ExecutorWaker` | `Send + Sync` | the inbox's producers | `wake()` must be non-blocking and safe from any thread, including under locks (it is a descriptor write / BH schedule) |
| `Vmm` (real) | `Send`, not `Sync` | `&mut dyn` from inside a core entry | **per-method in-lock legality is normative (§6.1)** and asserted; `raise_irq` must be irqfd-shaped (§6.3); `defer` shares the pure `DeferQueue` with the mock (§6.4) |
| `CancelHandle` | `Send + Sync` | proc-lock holders (retire, watchdog, shutdown) | holds no reference to the `Worker` or backend; arming is per-`txn`; **firing is latched, discharged lock-free** |
| `kayfabe-linux-raw` (all) | per-item | adapter crates only | the only `unsafe`; **every syscall entry asserts the lockwitness** except the named `*_under_lock` set; no pointer escapes; all offsets/lengths checked |
| `HostPageSize` | `Copy + Send + Sync` | geometry + the syscall layer | constructible only by `query()` (or a test-gated `forced`); every geometry function takes it as a parameter |
| isolate relay thread | n/a (process boundary) | nobody | blocks in a wait; **never polls**; writes counters only; dies with its process |
| **memory plane (§6.6/§6.7)** | n/a — no thread of its own | the thread serving the guest's request; the **executor** for work with no caller | guest-requested ops execute on the calling thread with **every lock of ours dropped**; there is no memory-plane thread and no queue. Coarse tier (memslot) at window/arena-grant frequency only; fine tier (`MAP_FIXED` placement) per publish. **Neither is in-lock legal** |
| **the VMM's own locks — the FOREIGN-LOCK CLASS** | not ours: neither constructed nor rankable | the VMM takes one *before* entering us. QEMU: the whole-machine BQL, on both entry paths. CH: the per-device `Mutex` of the `BusDeviceSync` blanket impl, across the whole MMIO callback — **avoidable**, by registering a direct `BusDeviceSync` impl over `SharedDevice` (§6.3) | **no lock the VMM owns may be acquired beneath one of ours, and our entry paths may arrive with one already held.** Rank −1: entered, never acquired. In-lock-legal `Vmm` methods must be implementable without reaching VMM-global API; the adapter has **exactly one** VMM-global-lock acquisition site and it asserts lock-free (§6.3) |

---

## 9. Testing strategy

### 9.1 The tiers, extended

- **T1 (deterministic, single-threaded)** — unchanged and still the majority. Grows: the
  `geometry` suite over three page sizes; the `DeferQueue` semantics suite (shared with the
  mock, so it tests both at once); the `KillPoint` × `Trigger` matrix, which is
  single-threaded and deterministic despite being about violence.
- **T2 (real threads, mock ports)** — grows the cancel/wedge scripts and the ledger. Still
  runs **both lock configurations**.
- **T2-OS (new)** — real descriptors, mock core: the reactor loop driven by real counters
  and a real timer, asserting the wake-count property (§3.4), registration/deregistration
  races, and the deregister→close ordering. Small, and the only tier that can fail for a
  reason the rest cannot see.
- **T3 (`loom`)** — still not applicable by rule (§4.2 forbids lock-free structures). The
  ledger and cancel paths introduce none: the cancel flag is a plain `Mutex`, not an atomic.
- **TSan** stays the race ceiling and stays a standing nightly gate, now covering the new
  threaded targets.

### 9.2 ★ What the mocks must grow

The mock recorder currently injects **one-shot** host errors and one-shot holds. Four
changes, each with its own reason:

1. **`fail_next` is racy by construction under concurrency — replace it.** It is a single
   `Option<RmError>` on the *shared* recorder consumed by "the next verb on ANY isolate".
   In a multi-threaded run, which verb consumes it is nondeterministic. Replace with
   `fail: Vec<(HoldSpec, RmError)>`, matched and consumed exactly like `holds` — so failure
   injection gains the precise `(isolate, gpu, worker, verb)` targeting the holds already
   have. *This is a real defect in the current harness, not just a missing feature: any
   existing test that relies on `fail_next` inside a concurrent run is passing for a reason
   it did not intend.*
2. **Cancellable holds.** `VerbHold::enter_and_park()` becomes
   `park_until(released || cancelled)`, and a cancelled hold returns
   `Err(RmError::Interrupted)`. That single change makes the entire cancel path testable:
   hold A's verb pending, fire the trigger, assert the fault variant, assert B (same proc)
   made progress throughout.
3. **A `never_cancels` hold — the D-state simulation.** Ignores cancellation entirely, so
   the watchdog's second stage runs. The requester's release must be the **abandon signal**,
   not a timeout, so the test stays clock-free (§7.5).
4. **A cancel observer.** Counts delivered cancels per `(isolate, worker, txn)` so tests can
   assert the handshake happened, that a **stale-txn cancel is ignored** (the C's refinement
   4 — the safeguard with no other observable effect), and that no cancel is delivered
   twice.

5. **A mid-chain worker death.** A hold that, when released, makes the worker *die* rather
   than return — the G4 case where the internal unwind cannot run. Without this the
   `VerbFailure.orphans` path has no test, and it is the leakiest moment in the design.
6. **`MockIsolate::is_quiesced()`** (G3) with a scriptable answer, so T3's predicate can be
   made to fail and the re-queue + escalation path can be driven deterministically.
7. **A scriptable `device_reset`** on the mock world (G5), including the two flavours of
   T4 once the bench experiment settles which signal distinguishes them.

Plus the `HostLedger` itself (§7.8) and `MockIsolate::request_cancel`.

### 9.3 The new standing gates

| Gate | Where | Fails when |
|---|---|---|
| lint inheritance | push | a crate other than `kayfabe-linux-raw` drops `[lints] workspace = true` |
| `unsafe` containment | push | `unsafe` appears outside the raw crate |
| **`unsafe` file naming (gate C, §4.1.1)** | push | `unsafe` appears in a file not named `*_unsafe.rs` — or a host pointer type (`*mut`/`*const`/`NonNull`/`transmute`) appears outside one |
| `unsafe` ratchet | push | the block count (**per `*_unsafe.rs` file**) exceeds the committed number |
| **memslot frequency (§6.7)** | push | the mean run's slot-install count grows with publications rather than with arena grants — a per-object memslot, caught structurally and without a clock |
| **★ VMM vocabulary (§6.0, decision #39)** | push | hypervisor **API** vocabulary (`BQL_LOCK_GUARD`, `memory_region_*`, `qdev_*`, `QEMUBH`, "bottom half", "main loop", …) appears in the 11 pure crates **or `kayfabe-rt`**. Matches identifiers, never the vendor's name, so adapter-crate names never trip it and no allowlist exists |
| **single VMM-global-lock acquisition site (§6.3)** | push (L2) | the adapter contains more than one acquisition site for **the foreign lock of its backend** — QEMU: `bql_lock`/`qemu_mutex_lock_iothread`/`BQL_LOCK_GUARD`; CH: any `Mutex<Device>`-shaped registration that reintroduces the per-device lock the direct-`BusDeviceSync` escape removes — or that site does not `assert_lock_free`. **Zero sites is a pass**, and is the expected CH result |
| narrow page-size grep | push | a bare `4096`/`0x1000`/`>>12` in the raw or rt crates |
| page-size axis | push | the geometry suite fails at 16 KiB or 64 KiB |
| forced-page-size integration | nightly | the shell suite fails at 64 KiB |
| **conservation** | push | the ledger does not balance in the mean run, either lock mode |
| kill-point matrix | push | any cell leaks, double-frees, or misnames its fault |
| reactor wake-count | T2-OS | the loop wakes more than the signals it was given (an **unbounded** poll — F1's testable form, `l1_concurrency.md` §4.2) |
| TSan / mutants | nightly / weekly | as today, over the enlarged surface |

Existing gates that must stay untouched and un-weakened: the §6.2 hexagonal vocabulary
grep (11 crates, code and comments), the aarch64 cross-check, the mutation threshold.

### 9.4 The arbiter is still the mean test

Everything above composes into `l1_mean.rs`. New machinery that sits *beside* the mean test
does not count — that is §12.13's finding as a policy. The mean run after L1-M2 is:
multi-proc × multi-thread × multi-GPU × multi-workload × **multi-page-size** ×
**multi-lifecycle-trigger**, with progress-under-pending, the R1/R3/R5 asserts, the
condemnation sweep, and the conservation ledger all running hot in the same run.

**Pass = the design survived contact. Fail = the doc changes, not the assert.**

---

## 10. The staged plan

Seven stages, each independently gatekeepable and green, sized like L1-M1's four. The plan
is restructured around the lifecycle pillar: the ledger comes first (it is the measuring
instrument), the OS layers follow, and the two structural lifecycle gaps the audit found
(G2, G5) get their own stage rather than riding along.

**M2-0 — the core preconditions.** *Not this design's work — landing in the core now, and
everything below assumes it:* G1 (`Binding` carries the backing identity), G4
(`VerbFailure { err, orphans }`, `RmError::Interrupted`, `FwdFault::Cancelled`, `#[must_use]`
on `Refusal`/`Orphans`), G3b (`reap_retired -> Vec<Proc>`), G3 (`Isolate::is_quiesced()`;
non-quiesced procs re-queued).
**Gate:** the existing suite green with the new signatures. **If any of these lands in a
different shape than described, §7 needs reconciling — say so rather than working around it.**

**M2-a — the conservation ledger, and an honest baseline.** *Mock-only; zero OS code.*
`HostLedger` behind the recorder, the six assertions, the `fail_next` replacement,
composition into the existing mean run, and the falsification runs that prove the
assertions have teeth.
*Rationale for going first: the ledger is a measuring instrument, and you build the
instrument before the thing it measures. It will report what the current tree already
leaks — and that number, whatever it is, is the honest baseline every later stage is gated
against.*
**Gate:** the ledger balances in both lock modes, or every imbalance is a named finding in
§14.

**M2-b — `kayfabe-linux-raw`.** The crate, the lint gates, `HostPageSize` + pure geometry,
the three region types, `Reservation`/`map_fixed_in`/**`restore`** (§6.7 item 3), the
readiness/timer/exit-descriptor wrappers, the KVM ioctls for the harness backend,
`assert_lock_free` at every syscall entry, the `trybuild` matrix, and the **`*_unsafe.rs`
file layering** (§4.1.1) from the first commit — retrofitting a file split is a rename storm
across every `// SAFETY:` review.
**Gate:** trybuild green (**all ten rows** — §4.6), geometry green at 4/16/64 KiB, `unsafe`
gates A/B/**C** green, aarch64 cross-check green, existing tests untouched.

**M2-c — the real `Vmm` + the in-lock-syscall reckoning.** The shared `DeferQueue`; the
per-method in-lock classification asserted; the memory plane built **in the
plan/execute/commit shape from the first line** (§6.2) and **two-tier from the first line**
(§6.7 — coarse window install vs fine `MAP_FIXED` placement, so a publication never touches
KVM); guest-requested ops on the calling thread with the lock dropped, background work on the
executor (§6.6); §12.7's observe→pump→encode→write→IRQ edge; §12.5's target-carrying redeliver
payload.
**Gate:** `rt_shell` + `l1_mean` green against the real `Vmm` in both lock modes; a test
asserting no syscall-shaped `Vmm` method is ever invoked with a rank held; **the memslot
frequency gate** (§6.7 — installs `O(procs)`, not `O(publishes)`); the ledger still balances.

**M2-d — the real reactor loop.** The epoll thread keyed on `CompletionSource`; counter,
timer and exit-descriptor sources; the isolate relay; the executor thread +
`ExecutorWaker`; registration plumbing and the deregister→close rule; the source cap (core
`register` grows a `Result`).
**Gate:** T2-OS — a real counter fires and a real IRQ descriptor is observed end to end; the
wake-count assert; the cap's refusal is contained.

**M2-e — ★ cancellation.** `RmError::Interrupted`, `FwdFault::Cancelled`/`Wedged`,
`CancelHandle` + the latched `CancelRequest`, the interrupt handshake, retire firing cancels,
the two-stage watchdog, the abandon-with-condemnation escape. Mock growth (cancellable
holds, `never_cancels`, the cancel observer).
**Gate:** the mean run grows a cancel phase and a wedge phase, composed into the same run;
the fault variants are asserted, not just "it refused"; TSan and mutants over the new
surface.

**M2-f — ★ the reclamation lifecycle.** All eight triggers wired: **T0's `pending_release`
queue (G2)** and the `sync_proc_rpc_bindings` half; T3's fn-47/re-handshake quiesce with
`is_quiesced()` and the bounded escalation; **T4's `Spine::device_reset` (G5)**, built to
the C's measured rules (§7.6 T4 items 1–5) with the flavour question resolved by the bench
experiment or explicitly deferred with the conservative behaviour chosen and named; T6's
isolate GC via the exit descriptor; T7's ordered `shutdown()` + the `Drop` tripwire.
**Gate:** the `KillPoint` × `Trigger` matrix green; the ledger balances after every trigger
in every interleaving; the T4 canary (a verb in flight across a reset never commits into the
new life) green; **and the fresh-boot tax measured, not assumed** — a re-handshake and a
driver restart both survive in-process, or the residual is written into §14.

**M2-g — the exit gate.** The §11 security review of the raw module's API; the threat-model
appendix for the double-mmap boundary; the caps from G9 (targets) and G10 (condemned/retired
growth) with their refusal paths exercised; the ratchets re-measured.
**Gate:** the review signed off **as part of this milestone, not as later cleanup**, and
every cap's refusal path exercised by the security suite.

---

## 11. Security — the exit gate, not a later cleanup

`core_security_threat_model.md` §1 defers memory-safety and host-breakout explicitly:
*"These are born at L1/L2 (the mmap/isolate/trap/VMM adapters), not in pure logic… The
bounded-memory type and its `trybuild` compile-fail assertions belong to L1 and are written
when L1 is."* They are being written now. So the review is part of M2-f's gate.

**What the review must check — the checklist:**

1. **No `MAP_FIXED` outside a `Reservation`** (§4.4). Clobbering our own or QEMU's mappings
   is *the* breakout. Verify the type shape makes an address-taking variant unrepresentable,
   and that `MAP_FIXED_NOREPLACE` is used underneath.
2. **★ Least privilege of the guest-RAM export — and a defaults finding.** `RamHandle`
   currently declares `covers: Option<Range<u64>>` where **`None` means all of guest RAM**.
   The maximally-privileged value is the ergonomic default, which is backwards.
   **Recommend inverting it: make the range mandatory.** An isolate should receive only its
   own proc's GPA arena; exporting everything should require saying so, loudly, in one
   reviewed place. One-line ABI change, real security value, and only cheap *now* — after
   L2 it is a migration.
   ★ **And its unstated precondition, now stated (§4.4.1):** an export is only possible at
   all if the VM was **launched** with a shareable memory backing (`--memory shared=on` /
   `memory-backend-*,share=on`). Identical on both backends, catchable by no code gate, and
   load-bearing for the entire double-mmap design — so the review must confirm the adapter
   **refuses loudly at the first export** rather than discovering it at first guest DMA.
3. **Write protection.** Pages an isolate only reads are mapped read-only in the isolate.
4. **Descriptor hygiene.** `O_CLOEXEC` everywhere; the shared memory object sealed against
   shrink/grow (an unsealed shrink under the isolate's feet is a `SIGBUS` on access — a
   guest-triggerable isolate crash, i.e. a DoS with extra steps); no descriptor reaches the
   sandbox beyond the intended set (the Mode-1 posture: cleared env and descriptors,
   namespaces, `pivot_root`, seccomp, unprivileged uid).
5. **Arithmetic.** Every `off + len` checked; every `u64 → usize` fallible; `len == 0`
   refused; page rounding done in checked arithmetic. A rounding overflow is the classic
   way a bounded object stops being bounded.
6. **Access shape on shared pages.** No borrow into guest- or GPU-written memory ever
   escapes (§4.2/§4.3); GPU-written pages accessed only via aligned ≤8-byte atomic views.
7. **Double-fetch.** Everything parsed from guest memory is copied first, then parsed from
   the copy. Verify no parser takes a region reference.
8. **Lifetime/unmap ordering.** A region cannot be unmapped while a derived reference lives
   (lifetimes); an isolate's mappings are torn down before the backing object's last
   reference drops.
9. **Seccomp delta.** The cancellation path adds exactly `tgkill`, `rt_sigaction`,
   `rt_sigreturn` (handler installed pre-lockdown); the relay adds its wait syscall and a
   write. Nothing else. Review the diff of the allowlist, not the allowlist.
10. **Resource caps.** The source cap (§3.8) exists, is derived from the real descriptor
    limit, and its refusal path is exercised by the security suite.
11. **★ No guest-reachable memslot churn (§6.7).** No guest-issued operation causes a
    `KVM_SET_USER_MEMORY_REGION` on a per-operation basis. A memslot update costs an SRCU
    grace period charged to **every** vCPU, so a guest that can provoke one per op is a
    device-wide DoS from an unprivileged process — and, unlike most DoS surfaces, it is
    *contained by nothing*, because the cost lands outside our own blast-radius boundary.
    Verify against §6.7's three-part justification test, and verify the two-tier split (a
    publication takes the fine tier only).
12. **★ No host CPU address crosses a crate boundary (§4.2.1).** Verify the five refusals by
    type where types can carry them (trybuild rows 7–10) and by review where they cannot.
    Note explicitly that `forbid(unsafe_code)` is **not** evidence here: pointer minting and
    pointer arithmetic are safe operations, so the audit must look at what the boundary
    *hands out*, not at where `unsafe` is written.

**Threat-model deltas to record** in `core_security_threat_model.md` when M2-f lands:
boundary 1 (guest ↔ host memory safety) is now *live*, with the raw crate as its entire
attack surface; boundary 2 (cross-isolate) gains the export-scoping property from item 2;
and the DoS surface gains descriptor exhaustion, which is contained by the cap and asserted.

---

## 12. Decision ledger (#38)

1. **The reactor keys on the `CompletionSource`, never the descriptor (§3.2)** — kills the
   descriptor-number ABA, and makes law 9 structural (the loop holds no shared state at
   all). *Cost: a reverse table for teardown only.*
2. **Level-triggered over counter-shaped primitives the loop drains (§3.4)** — makes F1
   structural rather than hoped-for; coalescing is sound because one source binds one
   `OsEventRef`. *Cost: a drain per wake; the wake-count assert is the gate.*
3. **The isolate relays os-events; nvidia descriptors never leave the sandbox (§3.5)** —
   security posture over one hop. *Cost: a thread and a hop per isolate.*
4. **★ AMENDS §6.1: the wake is a removal barrier + injected work + timer re-arm, not
   "re-read the set" (§3.6)** — the set lives in the kernel. *Cost: none; the API is
   unchanged, only its documented meaning.*
5. **The post-entry drain: wake, timer and cancel are latched under the lock and discharged
   lock-free (§3.6/§7.1)** — one mechanism, three needs, already invented for `WakeRequest`.
6. **A separate executor thread + an abstract `ExecutorWaker` (§3.7)** — law 9 kept; L2 is an
   adapter, not a re-plumb. *Cost: one futex wake per completion.*
7. **The source cap lands now (§3.8)** — arming becomes guest-reachable in this milestone;
   `SourceRegistry::register` grows a `Result`. *Cost: a core API change and a refusal path
   to test.*
8. **`kayfabe-linux-raw` is the one crate without inherited `forbid`, and CI gates both
   sides of that (§4.1)** — containment and inheritance.
9. **★ Volatile → aligned atomics with `Relaxed` for hardware-written pages (§4.3)** —
   corrects §9.1's mechanism; volatile is reserved for MMIO registers. *Cost: none at
   runtime; explicit fences at the two seams that need ordering.*
10. **Every raw syscall entry asserts the lockwitness (§4.5)** — "no syscall under a lock"
    becomes a mechanism, with a greppable `*_under_lock` exception set.
11. **★ The host page size is a type and a test axis, not a constant (§5)** —
    `HostPageSize` with no literal constructor + pure geometry + runs at 4/16/64 KiB, with a
    narrow grep as backstop. *Cost: every geometry function grows a parameter.* **Residual:
    the forced runs test our arithmetic, not a real kernel.**
12. **The `Vmm`'s capability groups are classified in-lock-legal vs not, normatively
    and by assert (§6.1)** — the trait currently does not distinguish a memcpy from a
    memslot syscall.
13. **★★ The `Vmm` memory plane is built in plan/execute/commit from the first line
    (§6.2)** — the alternative reproduces exactly the violation stage 3 spent a milestone
    removing from the verb path.
14. **`raise_irq` is contractually irqfd-shaped; no VMM-global API under our locks (§6.3)** —
    the foreign-lock inversion is unrankable and therefore must be designed out, not
    detected. ★ **Generalised by the portability round (§14.4) from QEMU's BQL to the
    CLASS** — *any* lock the VMM owns, which we neither construct nor rank. QEMU's is
    whole-machine; CH's is a per-device `Mutex` held across the whole MMIO callback, and is
    avoidable outright.
15. **`defer` shares one pure `DeferQueue` with `MockVmm` (§6.4)** — "matches the mock"
    becomes a tautology instead of a claim.
16. **The completion delivery tail runs lock-free, protected by the drain gate (§6.5)** —
    and §12.5's missing `GpuId` is fixed here.
17. **★ Cancellation is advisory; the reply is never abandoned except in the same act that
    condemns the slot (§7.2/§7.5)** — the C's #73 UAF lesson, ported with its four
    refinements.
18. **Cancelled verbs reuse `Orphans`; no parallel disposal path exists (§7.4)** — and it
    needs no new machinery, because stage 3 built the right shape.
19. **★ Worker death escalates to isolate death; the process boundary is the garbage
    collector (§7.0/§7.5)** — per-object cleanup is promptness, not correctness.
20. **★ Law 8 gains a bound: the quiesce point has a predicate, a bounded re-arm, and an
    escalation (§7.6 T3)** — deferral without a bound is a leak.
21. **Isolate GC via a process exit descriptor, not `SIGCHLD` (§7.6 T6)** — race-free
    against pid reuse, and it lands as a new `SourceKind`, exactly as §6.1 promised.
22. **Device teardown is an explicit ordered `shutdown()`; `Drop` is a tripwire (§7.6 T7)** —
    fallible blocking work does not belong in a destructor.
23. **No epoch counter for device reset (§7.6 T4/§7.7)** — considered and rejected as
    redundant: monotonic mints plus commit-keyed-on-`ProcId` already make re-adoption
    unrepresentable. Pinned by a canary rather than by machinery.
24. **★★ Conservation is a ledger property fed by the mocks' existing single funnel, asserted
    at quiesce, composed into the mean run, and attacked by a `KillPoint` × `Trigger`
    matrix (§7.8)** — with falsification runs to prove the assertion has teeth.
25. **`RamHandle::covers` should become mandatory (§11 item 2)** — the maximally-privileged
    value is currently the default. **Open: owner call, but cheap only now.**
26. **★ T0 (partial free) is a first-class trigger with a `pending_release` queue (§7.6 T0,
    G2)** — the one path with no process-boundary backstop, and the most frequent one in a
    real workload. *Cost: a queue on `Proc` and an opportunistic drain site.*
27. **★ The quiesce point is the observed fn-47 / GSP re-handshake, not a timer (§7.6 T3)** —
    the guest tells us the GPU went idle, and the C already listens
    (`C: nvkvm_gpu_emul.c:3458`). The timer is a backstop, and the escalation is what makes
    law 8 a law. *Cost: none — it deletes a heuristic.*
28. **★ `Spine::device_reset -> Vec<Proc>`, built to the C's five measured rules (§7.6 T4,
    G5)** — reset the boot-gating state at fn-47, the write position at the re-handshake,
    preserve the seqNums, disambiguate the trailing STARTCPU, keep every mint monotone.
    **One sub-question is a named unknown requiring a bench experiment** (the
    idle-release vs true-reload seqNum divergence) and is *not* guessed here.
29. **Every reclamation path returns its objects for a lock-free drop (G3b's shape)** —
    a `Drop` that blocks under rank 0 is an R1 violation that `assert_lock_free` cannot see,
    because it guards verbs, not destructors.
30. **G6's intra-arena free should be ported from the C, not invented (§7.8)** — first-fit
    with tail/adjacent coalescing, freed from the unmap path *and* the reaper. **Open:**
    it is a core change, so it is the owner's call whether it rides M2-f or follows.
31. **G7's window check becomes a hard fault, not a `debug_assert!`** — a release into the
    wrong window is the #14 collision class returning, and it is currently representable in
    release builds. Ledger check L7 catches it in tests; the fault catches it in production.
32. **G8: `SourceKind::OsEvent` gains channel/`Vas` identity now** — without it, per-channel
    deregistration (T0, T5) cannot be written and only the whole-proc sledgehammer exists.
    A field now; a migration later.

**★ Owner-directed amendments, 2026-07-26 (pre-M2-b) — see §14.2.**

33. **★★ A host CPU address never crosses a crate boundary, in any representation (§4.2.1)** —
    because out-of-range in a *guest* address space is a guest-visible fault (MISS = FAULT
    already covers it) while out-of-range in a *host* address space is a VM escape, and
    because `forbid(unsafe_code)` does not help: minting and offsetting a pointer are safe
    operations, so the boundary that hands one out has already lost. Five refusals, four of
    them compile-fail (trybuild 7–10), the fifth a named review obligation. **Includes the
    exemption that `host_va` from `map_gpu_va` is a host *GPU* VA, not a host pointer.**
34. **★ `unsafe` lives only in files named `*_unsafe.rs` (§4.1.1)** — containment to a crate
    makes the audit finite; containment to named files makes it enumerable by `ls` and visible
    in a diff's filenames. CI gate C; the ratchet becomes per-file. Costs nothing, because
    §4.7 and §5.2 already required the same layering for other reasons.
35. **★★ Nothing beneath one of our locks may take the BQL (§6.3)** — ★ **superseded in
    scope by decision #40 (§14.4): the rule is now the CLASS — no lock the VMM owns.**
    QEMU's BQL is one instance; CH's per-device `Mutex` is another, at a different
    granularity, and the BQL-shaped phrasing was **vacuously true** on it. Everything below
    is retained as the QEMU instantiation, and its last paragraph in particular is
    **QEMU-conditional**: extended from
    `raise_irq` to the whole `Vmm` surface, because the BQL is unrankable (it is QEMU's) and
    R3 is blind to it **by construction**, not by omission. Enforcement is honest and layered:
    classification-plus-assert (indirect), one grep-gated BQL acquisition site that can carry
    rank −1 (mechanical but only over what we write), and a written list of QEMU functions
    called from in-lock-legal methods (**a review obligation, not a mechanism**).
    **Drop-the-lock fixes both the inversion and the stall**, which is why it is the general
    rule rather than an exception list. *And the finding it exposes:* QEMU **gives** us the
    BQL on both entry paths, so "lock-free" is not sufficient — the trap path must be
    dispatched without the BQL (`memory_region_clear_global_locking`, verified at L2) or
    I-NOAMP fails on the hottest path. The C recorded this as an unpaid debt
    (`C: virtio_nvgpu.c:16-18`).
36. **★ Guest-requested memory ops run on the calling thread with the lock dropped; there is
    no memory-plane thread and no memory-plane queue (§6.6)** — the caller blocking is
    self-limiting backpressure aimed at exactly the process that caused it; a thread plus a
    queue replaces one free mechanism with three invented ones (bound, overflow policy,
    fairness) and adds reordering for R5 to defend. Background work with no caller to bill —
    the `pending_release` drain, deferred reap, T0's sweep — belongs on the **executor**, which
    already owns that role and is already bounded. **Names I-NOAMP** (no cross-process
    amplification) and states its honest baseline: the guest driver already serializes on its
    own GPU/API locks across every GSP RPC (`ogkm: kernel_gsp.c:2848,2954`), so we do not
    create that property — we may only fail to amplify it.
37. **★★ One memslot per window (or per arena grant), never per published object (§6.7)** —
    a memslot update costs `synchronize_srcu_expedited` over every vCPU
    (`linux: virt/kvm/kvm_main.c:1599-1633`; a DELETE/MOVE is *two* plus a shadow zap), so
    process A's update costs process B regardless of thread or lock. **Threading cannot fix
    it; frequency is the only lever.** The C converged here after >1500 per-mmap slots per
    `cuCtxCreate` regressed single-process matmul; `GpaSpace`/`GpaArena` already have the
    shape. Forbids: per-publication `map_guest`, per-mmap slots, delete/recreate to remap
    (use `Reservation::restore`, never `munmap` inside a live window), per-object `Prot`
    (protection is a **window** property), and per-object `lock_region` via memslot revoke
    (**userfaultfd only** — decision #6's ambiguity resolved). **Corrects §6.2's claim that
    publication needs a memslot**: it needs a `MAP_FIXED` placement, so §6.2's shape survives
    and its cost model does not. Pinned by a mock-only, clock-free frequency gate; the bench
    measurement tunes `ARENA_LEN`, `WINDOW_LEN`, `ARENA_RETENTION` and the window-vs-arena
    boundary, and can move the design only *between two allowed points*.

**★ Portability round, 2026-07-26 (owner-directed) — see §14.4.**

38. **★★ Hypervisor-agnosticism is a contract with a gate, not a sentence in the arch doc
    (§6.0)** — *"a second backend costs exactly one adapter crate: no trait change, no core
    change."* The tell that it had gone unwritten is structural: "agnostic" appeared **zero**
    times in this doc and in `l1_concurrency.md`. *(Numbering note: this is §12's local ledger;
    the repo-wide CI-gate number for the grep below is #39.)*
39. **★ A VMM-vocabulary CI gate over the 11 pure crates plus `kayfabe-rt` (§6.0/§9.3)** —
    the hexagonal gate's sibling for the other portability axis. **Gates API identifiers, not
    the vendor's name**, so adapter-crate names never trip it and **no allowlist is ever
    needed**. Dry-run at introduction: exactly two violations, both prose in `kayfabe-vmm`'s
    own rustdoc, both one-word rewordings — which is the argument for landing it at zero
    adapters rather than at two.
40. **★★ The foreign-lock rule is a CLASS, not QEMU's mechanism (§6.3)** — *no lock the VMM
    owns, which we neither construct nor rank, may be acquired beneath one of ours; and our
    entry paths may arrive with one already held.* The BQL-shaped phrasing was **vacuously
    true on CH**, whose `impl<B: BusDevice> BusDeviceSync for Mutex<B>` blanket impl holds a
    per-device `Mutex` across the entire MMIO callback (and panics on poisoning). **The escape
    is recorded now because it will not be obvious later:** implement `BusDeviceSync`
    *directly* on a wrapper over `kayfabe_rt::SharedDevice` — then CH imposes **zero** foreign
    locks. Supersedes #35's scope; #35's QEMU-only findings are tagged as such.
41. **★★ `lock_region`/`unlock_region` leave the `Vmm` trait (§6.8)** — with §6.7 item 5
    striking the memslot alternative, the only implementation is userfaultfd **on our own
    window VMA**, which needs no VMM cooperation on either backend; and the declared
    signature is slot-granular where the design requires **per page**, so it cannot express
    what it is for. Seven capability groups, not eight. Fault *delivery*
    (`CoreEvent::LockedRegionFault`) stays — that is the core's entry discipline, not
    userfaultfd. Zero core call sites today; a migration once L2 wires one.
42. **★ `Device` takes `&self`, and is `Send + Sync` (`kayfabe-vmm` rustdoc, §6.3)** —
    `&mut self` forces whole-device exclusivity per trapped access and makes the core's
    per-`Proc` sharding unreachable *through the declared port*, while
    `kayfabe_rt::SharedDevice` already offers the opposite. The backend that pays is the one
    with the **better** concurrency story (a `&self` bus callback), not the one that
    serializes everything anyway. Zero implementors today; free now, a signature break later.

**Genuinely open:**

- **§10.6 (B3, the system-proc stall)** — materially improved again by §7.5's watchdog, which
  is the "stronger story" §10.6 asked whether to demand. Owner may now be able to close it.
- **Decision 25** — an ABI change to `kayfabe-vmm`; trivial today, a migration after L2.
- **`VERB_BUDGET` and `MAX_REAP_DEFERRALS`** — tuning constants, not design questions, but
  the first cannot be honestly set until L3 measures real RM latency distributions. Until
  then they are set generously and the *shape* is what is tested.

---

## 13. Honesty — the bets, and where I think the existing design is wrong

**Bets, named:**

- **B7 — epoll + counter descriptors, deliberately not `io_uring`.** A shared submission
  ring is a lock-free structure, which §4.2 rules out by name, and it is a large kernel
  attack surface for a component whose whole claim is containment. The C's lesson set
  contains zero "not enough I/O throughput" bugs and six "concurrency wrongly shaped" ones.
  If a measured workload ever shows the reactor is the bottleneck, that is a reviewed design
  change, never a debugging-session substitution.
- **B8 — the isolate relay beats descriptor passing.** Betting one hop and one thread per
  isolate against keeping every RM capability inside the sandbox. Strongly held; the C built
  and validated this exact shape.
- **B9 — advisory cancellation plus condemn-and-abandon beats any abort path.** The bet is
  that "always wait for a reply, except when the slot dies in the same act" is strictly
  safer than any design that can abandon a live channel. Cost when it bites: a wedged
  D-state verb costs a whole guest process. Accepted, loudly.
- **B10 — the page-size test axis is a faithful proxy for arm64.** Honest blind spot: it
  tests geometry, not the kernel. It reduces arm64 bring-up to "run the suite on the
  hardware", which is the goal, but it does not eliminate the bring-up.
- **B11 — the mock ledger is a faithful proxy for host reclamation.** It proves *we* asked
  for every release. It cannot prove *RM performed* them. The L3 measurement in §7.9 is the
  only thing that can, and until it runs, "no leaks" means "no leaks in our accounting".
  Say it that way in the milestone log.
- **B12 — the process boundary really does reclaim everything (§7.0).** This is the load-
  bearing assumption of the entire reclamation design. It is true for RM objects, mappings
  and descriptors by construction (fd close → client teardown). It has **two known
  exceptions**: a D-state thread's in-flight kernel work (§7.5's residual), and — the one
  the audit found — **T0, where nothing dies at all** (G2), so the backstop is simply
  absent and per-object reclamation is the only reclamation. If the assumption were ever
  false for a further resource class, the whole §7 argument would need rebuilding, which is
  why it is checked explicitly at L3 rather than assumed forever.
- **B13 — six stages will not turn into one.** M2-c (the memory plane) is the stage most
  likely to want to absorb M2-d, because the observe→pump edge and the reactor loop are
  tempting to build together. Resist: the mean test can drive the pump edge without a real
  reactor (it already does), so the stages are genuinely separable.

**Where I think the existing design is wrong (beyond the amendments already listed):**

1. **§9.1's "volatile" is the wrong mechanism** for hardware-written shared memory (§4.3).
   Small correction, made cheaply now.
2. **§6.1's reactor model is `poll(2)`-shaped** and would send an implementer to rebuild a
   set the kernel already owns (§3.6).
3. **Law 8 is half a law** (§7.6 T3) — *confirmed independently by G3/G3b*. "Reap-deferred"
   with no trigger, no predicate and no bound is indistinguishable from "reap-never" in a
   run that never quiesces; and the reap that does exist can legally tear an isolate down
   under a live connection. The C's #80 leak is what the first looks like from outside.
4. **§7.3's retire does not cancel** — *confirmed by G4*. `Proc::retire` retires the isolate
   and refuses new checkouts, but a verb already in flight runs to completion against a dead
   proc, and there is no vocabulary to say otherwise, so it surfaces as
   `FwdFault::Rm(Other)` — §12.10's wrong-reason conflation, again.
5. **`RamHandle::covers`'s default is the privileged one** (§11 item 2).
6. **`fail_next` is racy under concurrency** (§9.2 item 1) — a harness defect, so any test
   relying on it inside a threaded run passes for a reason it did not intend.
7. **★ My own first draft was wrong twice, and both corrections came from references rather
   than reasoning.** (a) I wrote that cancellation "inherits all-or-nothing chain semantics
   for free"; G4 shows the unwind runs *on the worker*, so the dead-worker case — the whole
   premise — is exactly the case it does not cover (§7.4). (b) I wrote that the C's WPR2
   limitation was "designed out, not inherited" on the strength of a crate doc; the C's
   actual code shows WPR2 *is* reset in-process at fn-47, that the reset is **two-phase with
   a measured ordering constraint**, and that the `Spine` has no reset at all — so the real
   answer is "partly designed out, one part is a named unknown" (§7.6 T4). The lesson is the
   project's standing rule working: **the C knew, and reasoning from first principles got it
   wrong in both directions.**
8. **`core_state_and_consolidation.md` §4's "eager host-side reclaim is fine for
   correctness, wire later"** is false in both clauses (G1/G2), and it is the sentence that
   most needs deleting, because it is the one that would make a reviewer wave this milestone
   through.

**★ Added 2026-07-26 by the amendment round (§14.2) — four more places this doc had drifted:**

9. **★ §6.2's "publication … needs a memslot installed — a `map_guest`" is wrong** (§6.7).
   Publication needs a `MAP_FIXED` placement inside an installed window. The conclusion
   §6.2 drew from it (plan/execute/commit, never call the VMM from a locked commit) is
   **correct and unchanged**; the cost model behind it was wrong by orders of magnitude, in
   the direction that invites a later "this is overkill for a memcpy" simplification. This is
   the most consequential drift found in this round precisely because the *recommendation*
   was right — a wrong premise supporting a right conclusion is the hardest kind to notice.
10. **★ §6.1's classification table is read as if method names determined cost.** Two of its
    rows are amended in place (§6.1's new note): the "yes" rows are legal only because their
    *implementations* cannot reach VMM-global API — a requirement on the adapter, not a
    property of the trait — and the `map_guest` row silently covers two operations with
    different costs and frequencies.
11. **★ "No blocking call under ANY lock" (R1) is not sufficient in QEMU** (§6.3).
    ★★ **QEMU-CONDITIONAL — tagged by the portability round (§14.4). True on QEMU, FALSE on
    cloud-hypervisor**, where MMIO dispatch is a synchronous `VmOps` call on the vCPU thread
    with no VM-wide lock; the worst CH imposes is a per-device `Mutex`, and the direct-
    `BusDeviceSync` registration removes even that. Read the following as a property of one
    backend, never of "a VMM". On QEMU: both of
    our entry paths arrive with the BQL held, so a verb round-trip with all of *our* locks
    correctly dropped still stalls every vCPU in the VM. `l1_concurrency.md` §7.1's
    backpressure argument — *"the caller blocks, the guest's RPC stalls, the guest slows
    down"* — is false in its most important clause under a held BQL: it is not the caller
    that stalls, it is the machine. The argument is *sound for the L1 harness and for a
    BQL-free trap path*, which is exactly why the adapter obligation must be written down
    rather than assumed. Proposed as amendment 6 to `l1_concurrency.md` (§15).
    ★ **And CH has an affordance aimed at this exact problem that QEMU lacks:**
    `BusDevice::write -> Option<Arc<Barrier>>`, with the vCPU blocking on `barrier.wait()`
    **after the device lock is released** (`vmm/src/vm.rs:491-495`) — a clean way to park one
    vCPU across an isolate round-trip with no lock of anyone's held. So the honest form of
    this item is not "the design has a problem" but "**this problem is one backend's, and the
    other backend ships a primitive for it**".
12. **★ §6.1's `lock_region` docs treat "userfaultfd / memslot revoke-restore" as
    interchangeable.** They are not, once I-NOAMP is named: memslot revoke of a shared window
    is a device-wide two-grace-period DELETE that revokes access for every proc sharing the
    window, in order to lock one object. Struck in §6.7 item 5. Decision #6 said "never host
    `mprotect`" and left the remaining pair open; this closes it.

**And one thing I checked and found already right, recorded because a negative result is
also information:** the abstract `Vmm` trait is already pointer-free — `gpa_read`/`gpa_write`
take caller-owned slices, `HostRegion` is `{ id, offset }` over a backend-scoped opaque id,
`RamHandle` is a token. §4.2.1's job at this seam is therefore **conservation** — preventing a
future pointer-shaped method or helper — not repair. The `RamHandle::covers` default
(decision 25) remains the one genuinely wrong thing on that trait.

**The single biggest risk, stated plainly.** L1-M1's design was validated by a harsh mock
harness, and the harness's fidelity is exactly the property this milestone destroys: **a
mock cannot be slow, cannot leak, cannot die, and cannot recycle a name.** Every decision
in this doc is an attempt to put a *structural* assert where the mock's fidelity ends — the
lockwitness at the syscall, the ledger at the verb funnel, the wake-count at the loop, the
page size as a type. Where I could not find a structural assert, I said so (§7.9's eight
rows, §5.3, B11). The specific way I expect this to bite: **the memory plane (§6.2).** It is
the largest genuinely-new surface in the milestone, it is the one the mocks have never
exercised, and it is the one where building it the obvious way produces a violation that
does not fail any existing test. If L1-M2 goes wrong, it goes wrong there, and it goes wrong
silently — which is why M2-c's gate is an assert about lock state and not a passing test.

---

## 14. Contact log — what building L1-M2 changed in this design

> Empty by construction. `l1_concurrency.md` §12 has fifteen entries because the build
> found the design wrong fifteen times, and the doc's stance is that this is the *expected*
> outcome, not a failure of the design phase. Entries are appended as stages land; nothing
> here is ever a plan, all of it is a finding.
>
> Three specific places I expect to be wrong, recorded now so the log can confirm or refute
> them rather than quietly rediscover them:
>
> 1. **The post-entry drain will not be as tidy as §3.6 draws it.** Some core entry will
>    want to latch something from a path that does not return through the shell's discharge
>    point, and the answer will either be a third latch or a shape change.
> 2. **The conservation ledger's "namespace death is a bulk release" model will be too
>    coarse** the first time a partially-torn-down isolate is observed, and L2's "no double
>    release" will need a tri-state (`released` / `released-by-death` / `unknown`).
> 3. **`MAX_COMMIT_RETRIES` and the cancellation retry will interact.** §12.9's converging
>    staleness re-plans; a cancel arriving mid-retry is `DuringReplanRetry` in the matrix,
>    and I do not fully believe the current answer ("the retry sees `Cancelled`, which is
>    divergent, so it stops") survives the case where the cancel lands between the release of
>    the orphans and the re-plan.
> 4. **T0's opportunistic drain will be too lazy for some workload**, and the first symptom
>    will be an arena filling (G6) rather than a leak report — i.e. it will present as
>    exhaustion, not as an accounting failure, which is the harder thing to read.

### 14.1 M2-a — the ledger, the baseline, and T0/G2

Full account in `l1_concurrency.md` §12.32. What belongs here is the **gate** ("the ledger
balances in both lock modes, or every imbalance is a named finding") and the residue.

**The baseline, measured before anything was fixed** (per mean run, identical in both lock
modes): **24 host objects** (6 host VAS + 6 sysmem + 6 channel + 6 engine object), **6 host
mappings** and **24 576 GPA bytes** outstanding on *live* procs and nameable by nothing —
exactly 4 objects + 1 mapping + one 4 KiB block per subset-free, linear in the number of
frees. Zero dangling, zero double-frees, zero cross-namespace frees.

**★ And the first census read zero, which was a true negative.** The mean script's two
existing subset-frees target channels phase 0 deliberately leaves virgin, so `retain` was
dropping `Channel`s with no host state. §7.6 T0's claim that *"T0 is already being exercised
thousands of times per run by `ctl_workload`"* was **wrong**: that workload's RM map churn
only adds and removes *RPC* bindings (`Binding::host == None`), which own nothing host-side.
Reaching T0 required a new `t0_churn` phase. Corrected here rather than left to imply
coverage that did not exist.

**Post-fix the ledger balances: 0 / 0 / 0**, both modes. Three residues are named rather
than zeroed:

1. **Namespace-death residue — 6 objects, 2 mappings.** The two out-of-band-retired procs'
   host state. A retired isolate refuses every verb, disposal included, so §7.0's process
   boundary is the disposition of record. Pinned at its exact value, so it cannot drift
   silently.
2. **★ T0's drain is LAZIER than §7.6 T0 describes, and deliberately.** The design says
   "opportunistically at the next verb-issuing op"; the implementation says "at the next
   verb-issuing op **that finds the isolate otherwise idle**". Draining unconditionally is a
   use-after-free — it freed a host VAS underneath a parked mapping verb, measured, §12.32 —
   and no lock can exclude an in-flight verb, only `is_quiesced` can. **This is prediction 4
   above, arriving early and for a different reason than predicted:** the laziness is not a
   tuning choice, it is forced. A continuously-saturated multi-threaded proc holds its queue
   until an idle moment or a quiesce-edge sweep. Today the sweep is called explicitly
   (`SharedDevice::drain_pending_releases`); **M2-f owes it a trigger** — the GSP
   re-handshake / fn-47 idle edge that T3 already uses is the obvious one, and it is the same
   edge, so this is a wiring debt rather than a design gap. Until then, the exhaustion
   symptom prediction 4 describes is reachable in principle for a proc that never goes idle.
3. **R6 beyond the T0 path is untouched.** The GPA census now *measures* intra-arena
   occupancy that no `Vas::blocks` token can name, and T0 returns its blocks — but nothing
   else does. A publication never unpublished still holds its block for the life of the proc,
   which is correct (it is reachable) and is not what G6 warns about.

**Not done in M2-a, from §10's stage description:** the `fail_next` replacement (the mock's
single global one-shot injection point becoming per-verb targeted) and §7.8's L4–L7 —
ordering, core-side arena/source/worker accounting, the inbox and the right-window check.
L4's *mapping never outlives its VAS* half is enforced structurally by the release order and
observed by `unmap_of_unknown`; the rest need machinery later stages build (R3/R4/R5 have no
raw seam yet, R7/R8/R10/R11 are core-side counters). None of them is blocked; they were
simply not this stage's product, which was the instrument and the baseline.

### 14.2 ★ Owner-directed amendment round, 2026-07-26 — before M2-b

**Provenance, stated because this section's discipline requires it.** §14 opens *"nothing
here is ever a plan, all of it is a finding"*, and these four are **owner directives**, not
build findings — they arrived from the project owner between M2-a landing and M2-b starting,
each with its reasoning already established. They are logged here rather than silently folded
into §4/§6 so that a reader can tell which parts of this doc were argued from the design and
which were directed, and so the exception to §14's rule is visible instead of quietly taken.
Where a directive *contradicted* existing text, the contradiction is recorded in §13 items
9–12, which is the part of this round with the most value.

| # | Directive | Landed in | Ledger |
|---|---|---|---|
| A1 | The `Vmm`/raw seam exposes **bounded objects, never raw pointers** — because guest-address out-of-range is a fault while host-address out-of-range is a VM escape, and because holding a raw pointer is unsafe *even in safe code* | §4.2.1, §4.6 rows 7–10, §11 item 12 | #33 |
| A2 | The **BQL contract**: nothing beneath one of our locks may take the BQL; R3 cannot see it, so say honestly what does. ★ **Generalised to the foreign-lock CLASS by the §14.4 round (#40); the assertions it makes about held locks and stalled vCPUs are QEMU-conditional** | §6.3 | #35 → #40 |
| A3 | Guest-requested memory ops: **drop-the-lock on the calling thread**, no memory-plane thread; background work on the **executor**; name the **I-NOAMP** invariant and its honest baseline | §6.6 | #36 |
| A4 | **Memslot strategy: coarse regions with an allocator inside**, never per-object; write it so measurement tunes constants, not shape | §6.7, §9.3, §11 item 11 | #37 |
| A5 | `unsafe` lives only in `*_unsafe.rs`, so an auditor can enumerate the unsafe surface by `ls` | §4.1.1, gate C | #34 |

**What this round changed about the doc's own claims**, beyond §13 items 9–12: §6.2's stage
M2-c gate grows the memslot-frequency assertion, which is notable because it is a **structural,
mock-only, clock-free gate for a cost that is otherwise measurable only on real hardware** —
the same trick as the reactor's wake-count assert (§3.4). The bench measurement of memslot
cost, running separately, can therefore change constants and the window-vs-arena boundary and
nothing else. That was an explicit design requirement of A4 and it is the property to check if
the measurement comes back surprising: if a number would force a *shape* change, the shape
argument was a DoS-containment argument (§11 item 11) and the number does not reach it.

**Not amended, and worth saying so.** A3 asked whether a dedicated memory-plane thread was
wanted; the answer is no, and the existing design already had it right — §7.1 of
`l1_concurrency.md` had rejected the identical shape for relay threads, and this doc's §3.7
already puts background work on the executor. A3's contribution is therefore the *invariant*
(I-NOAMP) and its honest baseline, not a structural change. The baseline matters more than it
looks: without it, "process B never waits on process A" reads as achievable, and it is not —
the guest driver's own GPU and API locks impose it on real hardware too.

**What this round owes the build.** Nothing here has met contact yet. The three specific
places I expect these amendments to be wrong, recorded in §14's own spirit so the log can
confirm or refute rather than rediscover:

1. **The `*_unsafe.rs` split will be awkward somewhere** — most likely `Reservation`, whose
   safe half (offset arithmetic, occupancy accounting) and unsafe half (`mmap(MAP_FIXED)`,
   `restore`) want to share a private invariant. The answer is a private safe type the unsafe
   file consumes, not a relaxation of the rule; if it turns out to be a relaxation, that is a
   finding and it belongs here.
2. **The two-tier memory plane will want a third tier** the first time a backing needs a
   slot-level attribute (§6.7's justification test). I expect this to arrive as dirty logging
   for live migration and to be genuinely justified — at which point the answer is *another
   window with that attribute*, not a slot per object, and the test in §6.7 is what forces
   that answer to be argued.
3. **I-NOAMP will fail its first honest measurement**, and the cause will be the BQL rather
   than anything in our lock discipline. §6.3 predicts this; the value of predicting it is
   that the fix is then an adapter change rather than a redesign of §3.
   ★★ **BACKEND-CONDITIONAL (amended by §14.4).** This prediction is about **QEMU**, and it
   should not be scored against the design as a whole. On cloud-hypervisor there is no
   VM-wide lock on the MMIO path at all, so the same measurement is expected to **pass
   unconditionally** — and if it does not, the cause is that the adapter registered through
   the `Mutex<B>` blanket impl rather than implementing `BusDeviceSync` directly (§6.3).
   Stated this way the prediction becomes falsifiable per backend instead of globally, which
   is the only form in which it can teach anything: *"I-NOAMP fails, therefore the lock
   discipline is wrong"* would be the wrong inference on either backend, and on CH it would
   also be the wrong diagnosis.

### 14.3 ★★ Bench round, 2026-07-26 — the memslot measurement landed, and two cited C comments were false

**Provenance:** measurements on the serialized vast.ai bench, run against the C artifact and
against KVM directly, while M2-b was being designed. Unlike §14.2 these are findings, not
directives, so they belong here by §14's own rule. Full write-ups:
`../reference/mode2_bench_lifecycle.md` (C lifecycle) and
`../reference/rm_semantics_measured.md` (host RM/UVM semantics).

| finding | effect on this doc |
|---|---|
| memslot DELETE/MOVE = 230–460 µs p50, 1–4 ms tail; two `kvm_swap_active_memslots` vs one for ADD (965/965 kprobe samples); **≈135 µs × nvCPU** aggregate, with the ioctl latency **flat** in vCPU count | §6.7 gains its evidence. **No constant and no shape changed** — which was A4's explicit design requirement and is the thing to notice |
| a real workload provokes **zero** memslot updates in steady state (C Mode-2 full lifetime = 0; Mode-1's real 128 GiB window = 1 at probe, 0 thereafter) | §6.7's mock gate gets a **target** (zero growth) instead of a trend. The C is already at the floor |
| ★ **the per-object cost relocated to host VMA churn under `mmap_lock`** (+32 VMAs per CUDA process) | §6.7. Less harmful than a memslot (per-`mm`, so I-NOAMP-benign) but **more** relevant to R1 than the memslot ever was. §6.2's conclusion now stands on a *measured* premise, having lost its original one to §13 item 9 |
| a no-op memslot re-issue is ~1.2 µs (KVM early-returns pre-SRCU) | §6.7. Idempotent "ensure installed" is free; prefer it to shell-side installed-ness tracking |
| ★★ `rmmod` emits **no** fn-47 — the C's "TWO distinct triggers" comment is false | §0.2 row 1 and §7.6 T4 rule 1 struck. The unload has **no** trigger, so nothing armed on fn-47 fires on a driver restart |
| ★★ the driver-restart blocker is the **latch/stale-queue chain**, not WPR2; failure is a `msgqRxLink` timeout, not Xid 119 | §7.6 T4's OPEN QUESTION re-scoped and **re-ordered**. A `Spine::device_reset` modelling only WPR2 does not fix it |
| the C runs **one** CUDA process per QEMU lifetime (`cuInit` → 999 for the second, three boots, either exit mode) | §0.2 preamble. The C is a **single-process** Mode-2 oracle; Mode-1 is the multi-process one. Prior parity/#12/#13/#14 results are on a different axis and stand |
| the guest kernel frees 178 objects then fn-47 on process death, host stub alive | §7.0 gains its measured basis — two collectors, one at each boundary |
| RM serializes every ioctl per client; its waits are uninterruptible; **an interrupted alloc almost certainly completed** | already folded into §7.5 / `l1_concurrency.md` §12.26 by the source pass; the reference doc is now the citable home, and it carries the **version caveat** (`ogkm` 610.43.02 vs the bench's 580.159.04) that nothing else recorded |

**★ The methodological finding, worth more than any single row.** Rows 5 and 6 were both
*cited* in this doc, both from the C's own comments, and both false. The C's comments are a
strong prior — repeatedly the strongest available — but §0.2's discipline was "cite the C
rather than guess", and that is one notch weaker than it reads: **citing a comment is still
citing a belief.** Where a claim is load-bearing for a mechanism (a reset trigger, a quiesce
edge), the standard is a *behavioural* check, and §0.2 now says so.

### 14.4 ★★ Portability round, 2026-07-26 — the second-backend audit, acted on while it was free

**Provenance.** Like §14.2 these are **owner directives**, not build findings, arriving from
a portability audit against **cloud-hypervisor** as the hypothetical second backend. §14's
rule ("nothing here is ever a plan") is therefore bent the same way and for the same reason,
and it is logged rather than folded in silently so a reader can tell directed text from
argued text.

**The audit's headline is a negative result and deserves to be recorded as one:** the
boundary **holds**. A CH port costs one adapter crate; **no trait change and no core change
is required** by anything the audit found. Everything below is a sharpening, not a repair —
and every item is cheap *only* because `Device` has **zero** implementors, `Vmm` has **one**
(`MockVmm`), and the first real `Vmm` is scheduled against the KVM-direct harness rather than
a hypervisor. All seven land against an empty adapter layer.

| # | Directive | Landed in | Ledger |
|---|---|---|---|
| P1 | A **VMM-vocabulary CI gate** over the 11 pure crates + `kayfabe-rt`, on API identifiers rather than the vendor's name | §6.0, §9.3, `ci.yml` | #38/#39 |
| P2 | The two QEMU-isms it catches, reworded ("QEMU's BQL" → "a whole-device lock"; "bottom-half equivalent" → "the adapter's serialized executor") | `kayfabe-vmm` rustdoc | — |
| P3 | **Generalise the foreign-lock rule from a mechanism to a CLASS**; name QEMU's and CH's instances; record CH's escape | §6.3, §8, §9.3 | #40 |
| P4 | Tag the "every vCPU stalls" finding **QEMU-conditional**; record CH's `Barrier` affordance; one §7.9 row, stated backend-independently, with **two** expected outcomes | §6.3, §7.9, §13 item 11, §14.2 item 3, §15 amendment 6 | #40 |
| P5 | `lock_region`/`unlock_region` **leave the trait** — a `kayfabe-linux-raw` capability wearing a `Vmm` method's clothes | §6.1, §6.8 | #41 |
| P6 | `Device` takes **`&self`** (+ `Send + Sync`), reconciling the port with `SharedDevice` | `kayfabe-vmm` rustdoc | #42 |
| P7 | `write_trap` granularity is the **host page size**; the typed page-size rule **stops at the adapter**; `IrqSpec::IntxLevel` is backend-conditional; `export_ram` presumes a **shareable launch backing** | §5.2, §4.4.1, §11 item 2, `kayfabe-vmm` rustdoc | — |

**★ The most valuable thing in the round, and it is a method rather than an item.** P3's
defect was not that §6.3 was *wrong* — it was careful, correctly L2-scoped, and every
sentence in it is true. It was that it stated a **mechanism** where an **invariant** belonged,
and the cost showed up immediately: on CH the rule is **vacuously satisfied** while CH's
actual foreign lock goes unmentioned. A rule that is vacuously true on a backend is worse than
no rule, because it reports compliance. That is worth generalising *as a habit*: when a rule
names a vendor's noun, ask what class the noun belongs to and whether the class is what we
meant. §6.0's gate is the same move applied to vocabulary.

**★ Where the round disagreed with the audit, recorded because a negative also counts.**

- **The "agnostic appears zero times" tell survives a challenge, and it is worth knowing
  why.** A naive `grep -ci agnostic` over the two L1 docs returns **4**, not 0 — because
  "di**agnostic**" contains it. Word-boundary matching (`grep -ciwE`) returns **0** in both,
  confirming the audit. Recorded because the *next* person to check will run the naive grep
  and conclude the finding was wrong. (`tests/Cargo.toml`'s package description does say
  "VMM-agnostic conformance suite" — one mention, in metadata, which is not where a
  normative requirement can live.)
- **P5 was offered as "move it, or give it a page-range parameter".** The second option is
  rejected outright rather than merely not-chosen: a page-ranged `lock_region` would be a
  correctly-typed `kayfabe-linux-raw` call spelled through a hypervisor port — the same
  category error with better arguments. §6.8 states it that way.
- **P6's `Send + Sync` supertrait is this round's addition, not the audit's.** The audit asked
  only for `&self`. `&self` alone leaves a `dyn Device` unregisterable in a `Sync` device bus
  without the *use site* restating the bound, which is exactly the kind of re-derivation an
  adapter should not have to do; and "entered concurrently from several vCPU threads" *is*
  `Sync`. It is compile-time asserted (`assert_send_sync!(dyn Device)`) so it cannot be
  dropped silently.

**★ One finding from actually landing the gate, which is §14's own currency.** The first run
failed on **the gate's own documentation**: `kayfabe-vmm`'s crate docs enumerated the pattern
list while explaining it, and the list is (necessarily) made of the very identifiers it
forbids. This is where the sibling analogy breaks: the unsafe-surface gate is
**writable-by-construction about itself** (`grep -w` makes the lint name `unsafe_code` and the
suffix `_unsafe.rs` unmatchable), and **no such trick exists here** — nothing distinguishes
citing `BQL_LOCK_GUARD` from using it. The fix taken is the one that keeps the no-allowlist
property: **the pattern has exactly two homes, `ci.yml` and §6.0, and both are outside the
gate's scope.** Recorded because the alternative — a first allowlist entry, for the docs, on
day one — is exactly how a gate stops being mechanical, and because it means "reword it" is
occasionally "move it", which the error text should not be trusted to convey on its own.

**What this round owes the build** — recorded in §14's spirit, i.e. so the log can refute it:

1. **The VMM gate will need one more pattern, and the addition will be a finding.** The
   current pattern is derived from *QEMU's* API. When the CH adapter lands, the equivalent
   leak (a `vm-device` type name, a `MutexGuard<Device>` shape) will not match it, and the
   gate will read green while the same decay happens from the other side. The honest answer
   then is a second pattern group, not a broader regex — a regex broad enough to catch both
   would start matching ordinary English.
2. **The `&self` `Device` will meet a core path that genuinely wants `&mut`,** most likely
   in the register/GSP model that has not ported yet, and the temptation will be to put a
   `RefCell` behind it. That is `&mut self` with extra steps and a runtime panic; the answer
   is `SharedDevice`'s ranked locks, which is why the implementor is the shell and not `Gpu`.
3. **§7.9's new row will be the first row anyone tries to "simplify" to one outcome,**
   because two expected outcomes in one cell reads like indecision. It is the opposite: the
   moment it collapses to the QEMU answer, the QEMU answer becomes the general one again,
   which is the precise failure P3 was raised to fix.

### 14.5 ★★ M2-b — `kayfabe-linux-raw` exists, and three things in §4 were wrong

**Provenance:** build findings, so §14's rule applies without the §14.2/§14.4 exception.
Shipped: `crates/kayfabe-linux-raw` — `HostPageSize` + pure `geometry`, `HostOffset` +
`checked_span`, `MappedRegion` / `RegionView` / `VolatileRegion` / `Reservation`, the
`mmap`/`munmap` primitives, **7** relaxations in **2** `*_unsafe.rs` files, 32 unit tests
and 8 `trybuild` rows (288 → 321, fast path 23.7 s → 26.3 s). CI gained gates **A**
(forbid-inheritance), **B** (one crate) and the **block ratchet**, all three negative-tested.
Deliberately NOT built: `memfd`+sealing, the isolate double-mmap, KVM/`epoll`, `Send`/`Sync`
for the region types, and the `KAYFABE_FORCE_HOST_PAGE_SIZE` env knob.

**F1. ★★ §4.4's belt-and-braces is unbuildable: `MAP_FIXED_NOREPLACE` and reserving first
are mutually exclusive.** §4.4 asks for both — *"`map_fixed_in` is a method on
`&mut Reservation` taking an offset… Belt and braces: the underlying call uses
`MAP_FIXED_NOREPLACE`"*. `MAP_FIXED_NOREPLACE` fails `EEXIST` if **anything** is mapped in
the target range, and in the reserve-first design the reservation itself always is, so every
placement would fail. Punching a hole with `munmap` first and racing to fill it is strictly
worse — it opens a window in which another thread's allocator can take the range, in a
process we share with the VMM.

The belt is therefore **relocated, not abandoned**, and it comes out stronger:
the *reservation* is acquired at a **kernel-chosen** address (no address is ever requested
by this crate), which cannot displace an existing mapping at all — a guarantee
`MAP_FIXED_NOREPLACE` merely *detects*; and the *placement* uses plain `MAP_FIXED` inside a
range we demonstrably own, with the accounting `MAP_FIXED_NOREPLACE` was meant to backstop
made a loud refusal (`RawError::OverlappingPlacement`). **Consequence for §4.4's text:**
strike the `MAP_FIXED_NOREPLACE` sentence and replace it with "the reservation is placed by
the kernel, never requested". `MAP_FIXED_NOREPLACE` now appears nowhere in the codebase, and
its absence is a decision rather than an omission.

**F2. ★ §14.2 prediction 1 CONFIRMED — but the awkward seam was not `Reservation`, and the
fix was the opposite of the one predicted.** The prediction was that the split would strain
where a type's safe half and unsafe half share a private invariant. What actually strained
was §4.1.1's *own* division of labour: *"`region_unsafe.rs` holds the syscalls and the
pointer work; `region.rs` holds the newtypes, the arithmetic and the bounds checks."* Taken
literally that produces a **safe file that validates and a dangerous file that trusts it** —
i.e. every relaxation acquires a precondition established in another file, which is the
single hardest shape to audit and exactly the shape §4.2.1 warns about one level up (the
unsoundness happens where the check is skipped; the block reads clean).

The build inverted it into a house rule, now the crate's headline invariant: **the pure
arithmetic still lives in the safe files, and the dangerous file *calls it itself*,
immediately above the block.** No relaxation in this crate has a caller-supplied
precondition. The cost is that the region *types* live in the dangerous file rather than the
safe one (2 dangerous files, not 4), which is a worse `ls` and a much better audit — and it
is a real trade, so it is recorded as one. §4.1.1's split should be reworded from "who holds
the bounds checks" to "who holds the *pointer*"; the arithmetic being pure and reusable was
always the point, and being *called from* the safe side was never load-bearing.

**F3. ★ §4.2's "copies only" never said what defines the copy under concurrent
modification — and §4.3's own argument applies to it.** §4.3 correctly rules that a
concurrent write by an agent outside the abstract machine is a data race and that atomics,
not `volatile`, are the defined primitive. A bulk `read_into` out of guest-writable RAM is
that same race, and there is no atomic `memcpy`. Three options exist: `copy_nonoverlapping`
(a race in the letter of the model, what every Rust VMM does), per-byte `read_volatile`
(equally undefined under concurrency, merely un-optimisable), and word-wise `Relaxed` atomic
copies (**defined**, several times slower).

Built: `copy_nonoverlapping`, with the gap named on the type rather than claimed away. What
makes it tolerable *here specifically* is that the copy happens **once** and the source is
never re-read, so the miscompilation such a race actually produces — an optimiser-inserted
re-read — is the double-fetch this API exists to prevent; the discipline and the hazard
cancel. **This is a design question, not an implementation one**, and it is left open on
purpose: the answer is a measurement (how hot is guest-RAM bulk read?) and a decision, and
neither belongs in the stage that had no consumer for either.

**F4. §4.5's `assert_lock_free` at the `munmap` site must fire *after* the call.** `Drop` is
the one syscall site an ordinary scope exit reaches with a lock held — so it is the one
worth asserting — but asserting first panics out of a drop **before the resource is
released**, trading a rule violation for a real leak. The syscall has already happened
either way; what the assert is for is making it loud. Pinned by a test in both polarities
(create-side and drop-side).

**F5. §11 item 3 needed a refusal, not just a `PROT_READ`.** "Pages an isolate only reads
are mapped read-only in the isolate" is a mapping property; without a matching *API* refusal
the first `write_from` to such a region is a `SIGSEGV`, i.e. a guest-triggerable crash. The
region carries its protection and returns `RawError::NotWritable`, and a `RegionView` cannot
launder a write past it.

**What M2-b owes the build,** in §14's spirit so the log can refute it:

1. **`Send` will be wanted for the region types**, and the temptation will be one blanket
   `unsafe impl` in the dangerous file. The honest form is per-type and argued: a
   `MappedRegion` is genuinely sendable, and a `Reservation` handed to a second thread while
   a first still holds a `&MappedRegion` from it is not — the compiler currently says no to
   both, which is why nothing was granted at M2-b.
2. **The `trybuild` rows will go red on a rustc diagnostic reword** and someone will delete a
   row to make it green. The `.stderr` files are snapshots of *today's* wording, not of the
   property; the instruction (`TRYBUILD=overwrite`, after confirming the errors are the same
   errors) is in the suite's own docs for exactly that morning.
3. **Refusal 5 will be quoted as if the compiler held it.** Four of §4.2.1's five refusals
   are types; the fifth — a semantically unbounded bounded object — has no mechanism and is a
   §11 review obligation. The crate docs say so in as many words, and the first time someone
   cites "the compile-fail suite" as covering it, that is the failure mode.

---

## 15. Proposed amendments to `l1_concurrency.md` (NOT applied here)

That file is being appended to concurrently by the implementation work, so this doc does not
edit it. **Seven** amendments are implied; each is small, and each is a place where the
current normative text would send an implementer somewhere wrong.

1. **§6.1 — the reactor's wake.** The text models the source set as a userspace list the
   loop re-reads on a wake ("signal the notifiable source, the loop re-joins"). With an
   epoll-shaped set the set lives in the kernel and additions need no wake at all. Amend the
   *meaning* of the wake to: **a removal barrier, injected work, and timer re-arm.** The
   `WakeRequest` API itself is unchanged and correct. (§3.6 here.)

2. **§9.1 — `unsafe` policy, the access mechanism.** "Volatile access to concurrently-
   GPU-written pages" should read **"aligned atomic access (`Relaxed`) for memory
   concurrently written by another agent; volatile is reserved for MMIO registers."** The
   intent is unchanged; the mechanism named is the one that is actually defined under
   concurrent modification. (§4.3 here.)

3. **§3.3 R1 — the permitted in-lock side effect.** R1 names `raise_irq` as the one
   exception. Two clarifications: (a) it is an exception only if the implementation is
   **irqfd-shaped**, which must be stated as a binding requirement on the `Vmm` contract,
   because the obvious QEMU implementation takes the BQL and inverts against our device lock
   (§6.3 here); (b) the general rule should be stated as **"no syscall under any lock"**,
   with the raw module's per-entry `assert_lock_free` as its mechanism and a greppable
   `*_under_lock` set as its only exceptions (§4.5, §6.1 here).

4. **§7.3 / §5.4 — retire must cancel, and reclamation returns objects.** §7.3 describes
   `Isolate::retire()` as "interrupt EVERY in-flight worker", which the code does not do and
   for which no vocabulary exists (G4). And §7.3's described reap is unwritable against
   `reap_retired`'s signature (G3b). Amend to: retire **requests** cancellation for every
   checked-out worker (latched, discharged lock-free), and every reclamation path **returns
   its reclaimed objects for a lock-free drop**.

5. **§1 law 8 — retire-eager / reap-deferred gains a trigger, a predicate and a bound.**
   Amend to name the quiesce point as the **observed GSP re-handshake / fn-47 idle release**
   (`C: nvkvm_gpu_emul.c:3458`), with `is_quiesced()` as the predicate, a defer deadline as
   the backstop, and a bounded escalation so a wedged verb cannot pin a reap forever.
   (§7.6 T3 here.)

6. **★ §7.1 and §3.3 R1 — "the caller blocks" is not the whole truth under a foreign VMM.**
   ★★ *Amended by §14.4 before it was ever applied: the amendment must be written as a
   **backend-conditional qualifier**, not as a flat correction. The failure it describes is
   QEMU's; on cloud-hypervisor §7.1's sentence is true as originally written (synchronous
   `VmOps` on the vCPU thread, no VM-wide lock), and CH additionally offers
   `BusDevice::write -> Option<Arc<Barrier>>` for parking one vCPU with no lock held. So the
   text to land is "true here, false there, and here is the adapter obligation on the side
   where it is false" — a flat "this is false" would replace one over-general claim with
   another.*
   §7.1's backpressure argument (*"the caller blocks (lock-free), the guest's RPC stalls, the
   guest slows down"*) and R1's "no blocking call under ANY lock" are both sound **for the L1
   harness and for a BQL-free trap path**, and both are silently false in QEMU as it dispatches
   today: our entry paths arrive with the BQL held, so a round-trip with every lock of *ours*
   correctly dropped stalls **every vCPU in the VM**, not just its caller. Amend §7.1 to carry
   the qualifier and the adapter obligation: **the trap path must be dispatched without the
   VMM's global lock** (§6.3 here), otherwise the backpressure is device-wide rather than
   self-limiting. This is the single most consequential thing in this list, because the
   sentence as written reads as a *proof of containment* and is being relied on as one.

7. **★ §3.3 R1/R2 — name the invariant they already provide.** R2's *"'write lock' now means
   'no torn bookkeeping', deliberately not 'no outstanding host work'"* is exactly
   **I-NOAMP** (no cross-process amplification: A's activity must never make B's vCPU wait
   through anything we introduced) at the lock layer, and R1's lock-free verb is its other
   half. Name it, so that the mean test's progress-under-pending assertion can be described as
   *the I-NOAMP canary* rather than as a generic liveness check, and so that the two places we
   can still amplify — the BQL and the memslot — are recorded as the named exceptions rather
   than discovered. Include the honest baseline: the guest driver already serializes on its own
   GPU/API locks across every GSP RPC (`ogkm: .../gpu/gsp/kernel_gsp.c:2848,2954`), so I-NOAMP
   is about **amplification**, never about independence. (§6.6 here.)

Two further notes for whoever holds those files: `core_state_and_consolidation.md` §4's
"eager host-side reclaim is fine for correctness, wire later" should be deleted (false in
both clauses); and `kayfabe-gsp`'s "resettable in-process" claim is true of the GSP FSM and
false of the `Spine`, which should be said in place rather than left to imply device-wide
resettability that does not exist.
