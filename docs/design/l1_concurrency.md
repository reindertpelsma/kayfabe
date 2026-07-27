# L1 concurrency design — threads, locks, and completion flow (decision #34, revised by #37)

**Status:** design for review, 2026-07-25 · The FIRST L1 step, written **before any L1
code** (owner mandate: design-first on the highest-risk seam). Scope: how the Linux OS
layer drives the proven L0 core (`core_state_and_consolidation.md` at `1e8d55b`) under
real concurrency — N vCPUs, blocking host ioctls, interrupt delivery — without breaking
a single core invariant.

**Revision #37 (2026-07-25, owner-directed):** this revision folds the review's
pressure points (`l1_architecture_summary.md` §6.3, P1–P5) and the owner's concurrency
refinements back into the design of record. In brief: the lock discipline is promoted
to three **asserted** invariants — no blocking under any lock, ranked lock order,
re-validate after re-lock (§3.3); the **intra-process non-blocking invariant** is
stated first-class (§3.5 — the requirement this revision is named for); the isolate
model is **explicitly revised** from one single-in-flight worker to a bounded pool of
single-in-flight workers (§7.2); the **completion-source reactor** is added as a
core-owned pure port (§6); and the **mean integration test** is specified as the
architecture's arbiter (§8.4). Where a prior decision changed, the change is named in
place and in the ledger (§10) — the doc reads as one design, not a changelog.

**Companion docs:** `core_state_and_consolidation.md` (§2 the port surface, §3 the
invariants — the hand-off contract this doc implements), `execution_plane.md` (§2.4
completion patterns a–e), `l1_architecture_summary.md` (the independent review whose
§6.3 pressure points this revision answers), the crate docs of
`kayfabe-core`/`kayfabe-vmm`/`kayfabe-isolate` (the compile-time-asserted concurrency
contract, decision #17), and the C memory ledger cited per-failure below. C-repo cites
are prefixed `C:`.

**How to read this doc:** §1 is the inherited law. §2 is the recommended architecture in
one picture. §3–§9 are the seven decisions, each with recommendation → rationale →
alternatives → what's genuinely open. §10 is the decision ledger. §11 is the honesty
section: the bets, and the single biggest risk.

---

## 0. Why this doc exists — the C's threading-bug ledger

The C research artifact's worst bugs were not GPU-semantics bugs; they were
threading/completion bugs. Each row below is a *proven* failure mode this design must
make structurally impossible (or explicitly, honestly residual):

| # | C failure (cite) | Mechanism | The L1 rule it produces |
|---|---|---|---|
| F1 | **0x110094 poll storm** (`C: mode2_execfwd_layer2`) | guest spin-polls a GSP status reg; every read a nested-virt vmexit; ~40k exits per phase | **every poll must be provably BOUNDED** (§4.2 — the rule was long stated as "never busy-poll", which is neither what went wrong nor testable). Guest-side: read-native overlays (Vmm cap 7, an L2 fill). **Our side: every L1 wait is event-driven (epoll/condvar/deadline) or a spin with a stated, asserted bound — never an unbounded one** |
| F2 | **#14 round-8 completion starvation** (`C: mode2_14_concurrent_apps` round 8) | delivery invoked only from the *doorbell* handler, gated on `any_completed`, one cross-proc SWGEN0 batch → a proc that polls-but-doesn't-submit starves once the other proc goes quiet | completion delivery is per-proc and driven off the **owner's own poll** (the core's `DeliveryPlane::on_poll` already encodes this); L1 must pump it from the right threads and must never re-introduce a cross-proc gate on *observation* |
| F3 | **Mode-1 blocking-sync hole** (`C: mode1_blocking_sync_gap`) | the os-event *producer* was a no-op TODO (`ISOLATE_CMD_POLL` did nothing) → `CU_CTX_SCHED_BLOCKING_SYNC`/NCCL hang forever; 20 apps passed only because they spin-polled | the os-event producer is a first-class L1 component: an epoll relay from host os-event fds to `CoreEvent`s. It is on the critical path of correctness, not an optimization |
| F4 | **Signal interruptibility** (`C: signal_interrupt_delivery_done`, #73) | a guest task blocked in a forwarded ioctl couldn't die; and the naive fix (abandon the descriptor on signal) was a UAF | every blocking host op must be interruptible (the tgkill/`SA_RESTART`-less pattern), and the requester must **never abandon a reply buffer** — reclaim, then return |
| F5 | **De-facto global serialization** (`C: remote_test_serialization`, the #14 collision class) | one shared bump allocator / shared VAS keying / scalar exec-plane state → concurrent processes corrupted each other, so the whole bench had to run strictly serially | per-proc state isolation (the core has it structurally); L1's locking must not collapse it back into one queue — confined blocking, not global blocking |
| F6 | **Teardown reap hang** (`C:` P0, lesson L10) | reaping heavy tables *at* client-root free hung the dying context's residual polls | retire-eager / reap-deferred: the reap runs at the adapter-declared quiesce point, on the executor, never inline in a teardown-path trap |

The core already turned F2, F5-state, and F6 into structure. What is left — and what this
doc designs — is the part only L1 can get wrong: **which threads run what, who may
block where, and how completions travel from a host fd to a guest IRQ.**

---

## 1. Inherited law — the core invariants L1 must preserve

Restated from `core_state_and_consolidation.md` §3 as *L1 obligations* (violating any
of these from the outside is possible and forbidden):

1. **Order-independence / protocol-not-trace.** L1 may deliver events from any thread
   in any order, but must feed the SAME facts — no dedup, no reordering that drops or
   synthesizes facts. (This is also what makes deterministic single-threaded testing of
   multi-vCPU interleavings *sound* — §8.)
2. **MISS = FAULT.** A core fault surfaces to the guest as a fault/refusal. L1 never
   catches-and-guesses, never retries a `Miss` into a different VAS.
3. **Per-`(GpuId, ·)` isolation (I1).** Every op routed with its correct target; no
   isolate/arena shared across `(Proc, GpuId)` pairs.
4. **Completion integrity (I2).** Per-proc queues; re-delivery off the owner's own
   poll; forge types to the system proc only; fence-jump refusals are final.
5. **Refcount soundness (I3).** No caching of resolutions across frees; always
   re-resolve through the graph.
6. **DoS containment (I4).** `apply` refusals stay contained: log-and-refuse the guest
   op; never tear down the device; never let one guest's refusal path serialize
   another's progress.
7. **The one gated ring.** All doorbells via `handle_doorbell`; nothing else ever calls
   `RmBackend::ring_doorbell`.
8. **Retire-eager / reap-deferred (L10).** L1 declares the quiesce point and calls
   `reap_retired` there — not inside teardown, not never.
9. **The concurrency contract (#17).** Core is `Send + Sync`; mutation `&mut`-exclusive;
   reads lock-free shared. L1 picks the strategy; must never cross-proc-serialize
   per-proc work (esp. completion delivery), must complete isolate I/O via `CoreEvent`s
   on the serialized executor (never re-entrantly from an isolate thread), and must keep
   the deterministic single-thread test mode viable (virtual clock stays a value).
10. **Purity + `forbid(unsafe_code)`.** OS code in adapter crates only; `unsafe` only in
    one tiny audited raw module (mmap/volatile — §9.1); logic crates stay `forbid`.

One more inherited fact that shapes everything: **the trap-minimization architecture
(decision #6/#12) keeps the steady-state hot path at ~zero traps.** Userspace
pushbuffers and completion semaphores are passthrough; static faked regs are RAM-backed.
The only mandatory guest→us events are the doorbell write, RPC-queue processing, and the
rare control-plane RM ops. That is what makes the locking below *affordable*: the
contended path is inherently low-frequency by design, and the C's actual perf ceiling
was vmexits, not lock contention (`C: mode2_baremetal_32` — zero Mode-2 overhead
bare-metal).

---

## 2. The recommended architecture (one picture, in prose)

Four thread roles, two ranked locks, one queue, one reactor. Everything else is a
refinement.

```text
 vCPU threads (N, owned by the VMM)          reactor loop (1 thread, §6)   executor (1)
 ───────────────────────────────────         ───────────────────────────   ─────────────
 KVM exit → Device::mmio_read/write          epoll_wait on source fds:     drains CoreEvent
   │  decode trap (pure)                       - host os-event fds         inbox in order;
   │  ┌───────────────────────────┐            - isolate worker pipes      runs reactor
   ├─►│ device lock (RwLock, rk 0)│            - cross-isolate pipes       dispatch (§6) +
   │  │  read: route/resolve      │            - notify (eventfd-shaped)   Device::event
   │  │  write: apply/refresh/    │            - timerfd (defer deadlines)   - Deferred(…)
   │  │         pump/poll/drained │          fd → CompletionSource →         - IsolateComplete
   │  └───────────────────────────┘          CoreEvent, wake executor       - LockedRegionFault
   │  ┌───────────────────────────┐          (touches NO core state)      under the same two
   ├─►│ per-Proc lock (Mutex,rk 1)│                                       locks + R1–R5 as a
   │  │  µs BOOKKEEPING ONLY:     │                                       vCPU thread would
   │  │  publish_backing, the     │
   │  │  doorbell act-phase,      │          isolate (1 process per (Proc,GpuId))
   │  │  worker checkout/commit;  │          ─────────────────────────────────────
   │  │  NEVER a blocking call    │          sandboxed, unprivileged; a BOUNDED
   │  │  (invariant R1, §3.3)     │◄────────► POOL of workers, EACH single-in-
   │  └───────────────────────────┘ per-wkr   flight on its OWN channel (§7.2):
   │  blocking verb: ALL locks     channels   per-worker loop: recv → real RM
   │  DROPPED, round-trip on the   (sync,     ioctl → reply; + signal handler
   │  checked-out worker; re-lock, 1-deep     for interrupt (#73 pattern)
   │  RE-VALIDATE (R5), commit,    each)
   │  write GSP reply, resume vCPU
```

- **vCPU threads** enter at trap dispatch and do short pure work under the locks —
  **microsecond bookkeeping only, never a blocking call (invariant R1, §3.3)**. A
  guest op that needs a host RM verb runs in three phases: **route/checkout**
  (device-read + proc lock: decode, resolve, check out an idle isolate worker),
  **round-trip** (ALL locks dropped: the blocking socket round-trip on the
  checked-out worker — the guest is blocked awaiting the RPC/trap reply anyway, so
  synchronous-on-the-calling-thread is the honest shape, §4), **commit** (re-acquire
  the same locks, **re-validate — the world may have changed in the gap (R5)** —
  then apply the reply and resume the vCPU).
- **The device lock** (`RwLock`, rank 0) guards the device-global spine: `RmGraph`
  mutation (`Gpu::apply`), projection refresh, routing maps, per-target
  `DeliveryPlane` pump/poll/drained, target minting. Its write sections are **pure
  logic only — no blocking call is ever made under it, or under any other lock**
  (invariant R1, §3.3).
- **Per-proc locks** (rank 1) scope the per-proc *bookkeeping* — route/act state,
  publications, worker-pool checkout/return — to the proc that owns it: the #14
  blast-radius boundary reused as the concurrency boundary. Blocking itself is
  confined not by holding a lock but by **ownership**: a pending verb occupies one
  checked-out worker and one blocked guest thread, nothing else. Two procs — and two
  threads of the SAME proc (§3.5) — proceed in parallel; a wedged host ioctl stalls
  only its own guest thread (with one honest residual — §11 B3).
- **The reactor loop** is the L1 shell of the §6 completion-source reactor — and the
  F3 fix done right: one `epoll_wait` (blocking; one wake per signal, which is F1's
  bound — §4.2) over the registered
  completion-source fds (host os-event fds, isolate worker pipes, cross-isolate
  pipes) plus the eventfd-shaped notify that wakes it when the source set changes,
  plus the defer timer. It maps fd → opaque `CompletionSource` (a pure table
  lookup), pushes a `CoreEvent`, and touches no core state; the *dispatch* — which
  proc a signalled source belongs to and what to do — is core-owned pure logic (§6)
  run on the executor. It is the ONLY producer to the executor inbox besides
  `Vmm::defer`.
- **The executor** is the serialized executor `Vmm::defer` names (in QEMU: the main
  loop/BH context; in the harness: an explicit loop). It runs the reactor dispatch
  and `Device::event` for every `CoreEvent` — deferred reaps, isolate completions,
  lock faults — under the same locks (and the same R1–R5 discipline) as any vCPU
  thread. Asynchronous isolate I/O completes here, never by re-entry from an isolate
  or reactor thread (inherited law 9).
- **Isolates** are processes, not threads: one sandboxed, unprivileged process per
  `(Proc, GpuId)`, spawned by the factory with the Mode-1 stub posture (namespaces,
  pivot_root, seccomp, cleared env/fds, unprivileged uid). Inside it lives a
  **bounded pool of workers, each strictly single-in-flight on its own 1-deep
  request/reply channel** — the §7.2 revision of the original single-worker
  decision; each channel is interruptible (§5.4).

Lock order, total, one-way, and **ranked**: **device (rank 0) → proc (rank 1) → leaf
(rank 2: executor inbox, recorder)**. Acquiring against rank order is an always-on assert
panic (invariant R3, §3.3). Never acquire the device lock while holding a proc lock;
no lock held across a thread join, a barrier, or any blocking call (R1). (The same
discipline `tests/tests/concurrency_stress.rs` already documents and enforces for the
mock harness — the ranks make it mechanical.)

---

## 3. Decision 1 — the driving model: how N vCPUs invoke the core

**★ THE central decision — decided; see the ledger (§10.1). The governing rules were
revised in #37 (§3.3).**

### 3.1 Options

- **(a) Per-`Proc` sharding (hybrid): device `RwLock` for the global spine + one
  `Mutex<Proc>` per proc for the per-proc planes.** The parallelism the core was
  *designed* for (its own docs: "two vCPUs driving different guest processes mutate
  their `Proc`s simultaneously with no shared lock"). Cross-proc ops (apply/refresh,
  delivery pump) take the device write lock, which excludes proc ops by protocol (§3.3).
- **(b) One device-global lock for everything.** Simplest; provably correct (the
  stress suite's shape); but every blocking host verb serializes the whole device —
  the C's de-facto bench-serialization symptom (F5) re-created at the lock layer, and
  the exact thing that turns "proc A's slow alloc" into "proc B's doorbell latency."
  *(Critique as framed in #34, i.e. with verbs held under the lock; invariant R1 now
  forbids that under ANY model — see §3.2 item 1 for the recalibrated comparison.)*
- **(c) `RwLock` only (concurrent reads, one writer).** Strictly a sub-case of (a)
  without the shards: reads scale, but all *mutation* — including every per-proc
  doorbell and every blocking verb — funnels through one writer slot. Same failure as
  (b) under concurrent load, marginally better for read-heavy phases.
- **(d) Actor-per-proc (message passing, one owner thread per shard).** Also sound
  (the core presumes no strategy), and superficially attractive for determinism. But a
  trap is a *synchronous upcall* — the vCPU cannot resume until the reply is computed —
  so an actor model forces a cross-thread round-trip onto the only mandatory hot-path
  trap (the doorbell), buys no data-race safety we don't already have from `&mut`
  (safe Rust + the borrow checker are the race guarantee, not the actor), and adds a
  queue whose depth/ordering is new state to reason about. Rejected for the trap path;
  the executor (§2) is exactly this pattern where it *does* fit — deferred work.

### 3.2 Recommendation: **(a), the hybrid shard model — with a staged landing**

Rationale, in order of weight:

1. **The blocking-ioctl reality decides it — though revision #37 relocates the fix.**
   `RmBackend` verbs are real host RM ioctls over a socket to a sandboxed worker:
   microseconds typically, milliseconds for allocs (the C measured alloc ~5.4 ms),
   *unbounded* in the failure case (D-state — the C's wedge class). The original
   design confined those calls under per-proc locks; invariant R1 (§3.3) now moves
   them out from under ALL locks — resolving the review's P1 (the device read-lock
   held across a verb coupled every device *write*, including the completion pump,
   to the slowest in-flight verb of any proc). What sharding still buys: per-proc
   bookkeeping (checkout/commit, doorbell act, observation) stays contention-free
   across procs under load, and a commit-phase re-validation contends only its own
   proc. Honest corollary: R1 *weakens* the urgency of sharding — under (b)/(c) a
   blocking verb no longer holds the one lock either, so their residual cost is
   µs-bookkeeping serialization — which makes the staged landing below (degenerate
   one-lock first) even safer. Confinement — by worker/thread ownership, not by
   lock-holding — of blocking is the design (see §4 for why not full-async).
2. **It is the shape the core already advertises.** Per-proc planes are disjoint by
   construction (#14's isolation, cashed in as concurrency); `publish_backing` already
   takes `&mut Proc`. The lock layout maps 1:1 onto the ownership layout — no
   impedance mismatch to maintain.
3. **The cross-proc ops are rare and coarse by design.** `Gpu::apply` (control plane,
   guest RM allocs — a handful per process lifetime), projection refresh (same),
   delivery pump/poll/drained (pure, microseconds). Serializing *those* under a device
   write lock costs nothing measurable and keeps them dead simple.
4. **I4 containment becomes a lock property too:** a hostile guest spamming refused
   `apply`s contends only the device lock's write slot (bounded, pure sections); it
   cannot occupy another proc's shard.

### 3.3 The rules that make (a) sound — three ASSERTED invariants + two structural rules

**(Revised in #37.)** The original R1–R4 were prose rules, and the review found its
sharpest pressure points exactly where they were protocol-not-mechanism
(`l1_architecture_summary.md` §6.3 — P1: the device read-lock held across a blocking
verb coupled every device write to the slowest verb in flight; P2: the observe→pump
lock transition was a convention one refactor could deadlock; P3: the route→act gap
was a staleness window with ad-hoc re-checks). Owner direction promotes the discipline
to three **asserted** invariants — enforced by panics/asserts in every tier that runs
tests, shaped into the API wherever the type system can carry them — plus two
structural rules. Together they are also the entire mechanism behind the §3.5
intra-process guarantee: one coherent spec, not two separate fixes.

- **R1 (ASSERTED) — no blocking call under ANY lock. Ever.** Generalizes the original
  "no host I/O under the device lock": no blocking isolate round-trip — no
  potentially-blocking syscall at all — is ever made while holding the device lock OR
  a per-proc lock. Locks bracket microsecond bookkeeping; the blocking verb is
  dispatched lock-free on a checked-out worker (§7.3). The one permitted in-lock side
  effect remains `Vmm::raise_irq` (an eventfd/irqfd write — non-blocking, bounded).
  - *Definition:* a condvar wait that atomically releases its own mutex is lock-free
    with respect to THAT mutex, but the waiter must hold no *other* lock — so e.g. a
    pool-full wait releases the device read lock too and re-enters from the top (R5
    then applies).
  - *Enforcement:* the blocking-verb API is ownership-shaped — it takes the
    checked-out `Worker` by `&mut` from a value the checkout phase moved OUT of the
    locked state, so the natural call site has no guard alive — plus a runtime assert:
    a thread-local lock-depth counter, maintained by the L1 guard wrappers, asserted
    zero at every blocking-verb entry. (Full compile-time enforcement of "no guard
    alive on this thread" is not expressible in safe Rust; the assert is the real
    teeth, the ownership shape is what makes violations contortions instead of
    accidents. Thread-local counters are not shared state — the §4.2 no-atomics rule
    is untouched.)
  - *Consequence for the core shape (owning B1's fallback):* a core act-phase runs
    under the proc lock, so it can no longer call a blocking `RmBackend` verb
    in-line. Verb-issuing paths take the **plan/execute/commit** shape: the locked
    core phase *emits* the verb (+ a typed continuation cookie), L1 executes it
    lock-free — still synchronously on the calling thread (§4 is unchanged: threads,
    no async) — and re-enters the core's commit/resume with the reply under
    re-acquired locks, with R5's re-validation built into the re-entry. This is the
    `IsolateComplete{session, cookie}` seam the original design held open as an
    emergency exit; #37 wires it as the *standard* shape for blocking verbs (B1
    revised — §11).
  - *How it would bite without it:* review P1, verbatim — proc A's 5.4 ms alloc (or,
    at RM's timeout, a multi-second one — §12.26) holds the device read lock; the completion
    pump is a writer; every proc's completion delivery and all control-plane progress
    queue behind A's slowest verb. That is F5's de-facto serialization, rebuilt at
    the lock layer — and intra-proc, it is sibling thread B stalled behind A's verb,
    the exact #37 violation.
- **R2 (revised) — locks bracket bookkeeping only; a proc lock is only ever held
  together with the device READ lock.** A proc op's *locked phases* (route/checkout,
  commit) hold device-read plus its proc `Mutex`; the blocking verb between them
  holds neither. A device *writer* (apply/refresh/pump) therefore still excludes all
  bookkeeping sections — it sees a world with no proc mid-mutation — but it does NOT
  wait for in-flight verbs: outstanding round-trips are tolerated, because their
  commit phases re-validate (R5). This is the P1 fix as a contract: "write lock" now
  means "no torn bookkeeping", deliberately *not* "no outstanding host work".
- **R3 (ASSERTED, sharpened) — single lock by default; multiple locks only in the
  declared, globally-consistent rank order.** Lock hierarchy is the
  deadlock-prevention discipline, made mechanical: every L1 lock has a declared
  RANK — **device = 0, proc = 1, leaf (executor inbox, recorder) = 2** — and a
  thread may acquire only in strictly increasing rank, at most one lock per rank.
  Acquiring against the order is an **always-on assert panic** (§12.2: `debug_assert`
  was the #37 wording; the build made it unconditional — one thread-local read is far
  cheaper than the lock it guards, and a silent production deadlock is the exact
  failure this exists to prevent) (per-thread held-rank
  watermark in the guard wrappers) — caught in T1/T2/the mean test, never a silent
  production deadlock. Cross-proc ops acquire write-device and need no proc locks
  (R2 guarantees bookkeeping exclusivity). This is the P2 fix: the observe→pump
  transition (release proc + read, *then* take write) stops being a convention — the
  panic fires the first time someone "conveniently" pumps under a held proc lock.
  - *How it would bite without it:* two threads, opposite acquisition orders, one
    wedged device under load — the classic, invisible until the unlucky
    interleaving, and undiagnosable in production precisely because it is silent.
- **R4 — the doorbell (and every mixed) path is route/act split.** Unchanged: route
  (token decode, `by_vchid` lookup) under the device read lock → act
  (materialize-on-first-touch, working-set gate, ring) under the proc lock. See §3.4
  — landed as decision #35.
- **R5 (ASSERTED, new) — re-validate after re-lock.** Dropping a lock means the world
  may have changed by re-acquire: a proc may have retired, a resource freed, a
  channel torn down, an apply/refresh may have rewritten routing. Any drop-lock
  pattern — the verb round-trip gap, a pool-full wait, the observe→pump transition —
  MUST re-resolve its references and re-check its decision on re-acquire; it never
  carries a stale reference or a pre-gap decision across the gap. MISS=FAULT applies
  to staleness too: if re-validation finds the target gone, the op surfaces a refusal
  — it does not "finish what it started" against a world that no longer contains its
  target.
  - *Enforcement:* route-phase products are typed as pre-validation *hints* — IDs,
    never held references (the core's ID-graph shape and inherited law I3 already
    force re-resolution through the graph) — plus the mean test's staleness canaries
    (§8.4): the mock mutates the world inside the gap and asserts the op re-resolves
    or refuses.
  - *How it would bite without it:* review P3, verbatim — route resolves
    `(ProcId, ChanId)` under the read lock; the guest tears the channel down while
    the verb is in flight; commit writes the reply into freed channel state. A
    use-after-retire no lock can prevent — only re-validation can. The C's F4 UAF
    (the abandoned reply buffer) is the same species.

The three ASSERTED invariants are **R1, R3, R5**; R2 and R4 are the structural rules
that make them cheap to honor. Every one of the three is exercised as a runtime assert
in the test tiers (§8.4) — the discipline is enforced, not merely documented.

### 3.4 The honest cost: a core-shape change, requested by design discussion

> **Status (decision #35, owner-confirmed refactor-NOW): LANDED, behavior-preserving,
> 143/143 green.** `Gpu` = `{ spine: Spine, system: Proc, procs }` — the device-global
> `Spine` (arch/rmgraph/`by_pdb`/`by_vchid`/targets/factory/retired) is separately
> borrowable from the proc set. Spine ops (`Spine::apply`, pump/poll/drained,
> `reap_retired`) take `&mut Spine + &mut Proc(system) + &mut impl ProcSet` — the
> `ProcSet` trait is the "L1 wrapper owns the `Proc`s" visitor seam (item 2 below), so
> L1 can store `Mutex<Proc>` cells and still drive the write-lock sections. Per-proc
> ops are route/act split per R4: `route_doorbell(&Spine)`/`exec_doorbell(&mut Proc)`,
> `route_engine_object`/`exec_engine_object`, `classify_control`/`forward_control`,
> `read_pushbuffer`/`apply_pushbuffer`, `route_pdb` + `*_in` forms of
> resolve/gate/arm_fence/fence_observed/present_scanout. The old `&mut Gpu` entry
> points remain as split-borrow compositions (the degenerate one-lock shape). The one
> deliberate exception: `signal_golden_capture` stays `&mut Gpu` (a `&mut Proc` form
> would dissolve the L5 system-typed-forge guarantee; it is a rare bring-up event —
> run it under the write lock).

Today `kayfabe_fwd::handle_doorbell` takes `&mut Gpu`, and `Gpu` owns its `Proc`s as
plain values in `procs: BTreeMap<ProcId, Proc>` — correct for the pure core (no
interior mutability, decision #17), but it means L1 cannot hand two threads two procs
without a structural seam. Model (a) therefore requires, before L1 code is written:

1. **The route/act split of the mixed entry points.** `handle_doorbell` factors into
   `route_doorbell(&Gpu, token) -> (ProcId, ChanId, GpuId)` (pure read) +
   `exec_doorbell(&mut Proc, …)` (the act phase, where the RmBackend calls live).
   `publish_backing` already has the target shape; `forward_engine_object` and the
   Case-1/Case-2 paths get the same treatment. Mock-suite stays green throughout — this
   is a refactor of *signatures*, not semantics.
2. **The ownership split of `Gpu`.** The device-global spine (`rmgraph`, routing maps,
   `targets`, factory, `system`) separates from the per-proc containers so L1 can wrap
   each `Proc` in its own `Mutex` while the spine sits under the `RwLock`. The core
   stays lock-free and pure — the locks live in L1; the core's contribution is an
   ownership layout that *permits* them (e.g. `Gpu` exposing the spine and the proc set
   as separately borrowable, or the L1 wrapper owning `Proc`s and the core's
   cross-proc ops taking iterator/visitor arguments). The exact mechanics are an L1-M1
   design-review item — per the hand-off contract, the port grows by design discussion,
   and this is that discussion's first request.

**Staging recommendation:** do the split FIRST, as a core refactor with the existing
mock suite as the safety net, and write all L1 code against the final (a)-shaped API —
but ship L1-M1 (single-process bring-up) running it under a **degenerate configuration:
one global lock** (a `RwLock` where every op write-locks). That is bit-for-bit the
stress-proven Stage-A shape, trivially correct, and turning on real sharding later is a
lock-configuration change, not a rewrite. The 2×-concurrent acceptance test (the #14
gate) is the forcing function that flips it. This avoids the classic trap of building
bring-up code against a temporary API and then re-plumbing under schedule pressure.

**Alternative if the owner rejects the core-shape change now:** ship (b) for L1-M1
verbatim (zero core edits), accept whole-device serialization on blocking verbs, and
schedule the split as its own later step. Honest cost: L1-M1 code is written against
`&mut Gpu` signatures that will change; the #14-concurrency milestone then carries both
the refactor and the re-plumb. Workable, but it re-runs the C's history — concurrency
retrofitted instead of designed in — which is the precise failure decision #9 names.
Not recommended.

*(#37 note: the landed route/act split is the first half of the shape R1 completes —
route under read-lock, act-bookkeeping under proc lock, and the act phase's blocking
verbs split out via plan/execute/commit, per the R1 consequence above.)*

### 3.5 The #37 invariant — intra-process non-blocking, stated first-class

Per-`Proc` sharding does NOT cover the case that matters most in practice: **a
multi-threaded guest process is ONE `Proc`.** Its sibling threads share the proc lock
and the isolate; no amount of cross-proc parallelism helps them. The guarantee, stated
as the invariant this revision is named for:

> **A blocking GPU-work verb issued by guest thread A must not stall guest thread B of
> the same process — in particular B's poll / event-wait / completion paths.**

The mechanism is nothing new — the pieces above, composed:

1. **R1** means A's pending verb holds no lock. B's bookkeeping — its own doorbell
   act, its poll RPC, its own checkout — takes the proc lock for microseconds
   regardless of how long A's verb has been pending. Without R1, the proc lock is a
   convoy and the guarantee is unmeetable *by construction*; this is why the lock
   discipline and #37 are one spec, not two.
2. **N workers per isolate (§7.2)** mean B's own verb does not queue behind A's on
   the wire: two single-in-flight workers = two concurrent RM verbs from one
   process's threads. (At the pool bound, B waits for a worker — bounded
   backpressure, not a lock convoy; and the poll path needs no worker at all, so
   guarantee 3 holds even at pool size 1.)
3. **The poll path is STRUCTURALLY independent of the RM-verb path.** A completion
   reaches B via the reactor (§6): host os-event fd → reactor → dispatch → observe +
   pump → IRQ — or via B's own poll RPC (pure bookkeeping under the µs proc-lock
   hold). Neither touches the isolate RM channels at all, so even a *wedged* verb
   cannot sit between B and its completions.

This is what the mean test (§8.4) proves structurally: B's independent op completes
and a poll fires WHILE A's verb is held pending by the mock.

---

## 4. Decision 2 — execution substrate: OS threads, not async, not a new state machine

**Owner-flagged decision (explicitly called out earlier) — confirmed (§10.2).**

### 4.1 Options

- **(a) OS threads with a small fixed role set** (§2: vCPU threads + reactor loop +
  executor + isolate worker pool). Blocking is expressed as blocking; the thread
  inventory is enumerable and appears in the §9 contract table.
- **(b) tokio-style async.** Rejected:
  1. The core is synchronous pure logic and `RmBackend`/`Vmm` are sync traits — async
     infects every port signature (async-trait objects, lifetimes across await points)
     for zero core benefit.
  2. The genuinely blocking things (host RM ioctls) don't become async under tokio;
     they become `spawn_blocking` — a thread pool in disguise, now with a scheduler we
     don't control between us and determinism.
  3. The C's lesson set contains **zero** "not enough concurrency" bugs and six
     "concurrency wrongly shaped" bugs (§0). Async maximizes concurrency surface and
     minimizes legibility of who-runs-where — the exact wrong trade for this project.
  4. Debuggability: a hung Mode-2 guest is diagnosed from thread stacks (`gdb -p`,
     `/proc/…/stack` — how every C bug was root-caused). Thread-per-role makes the hang
     *legible*; an executor's task soup does not.
- **(c) One explicit event-loop/state machine (single-threaded L1).** Maximal
  determinism, and superficially the purist answer. Rejected because traps are
  synchronous multi-threaded upcalls from KVM: a single loop forces every trap through
  a cross-thread round-trip (queue → loop → reply → wake), adding latency to the
  doorbell (the one mandatory hot-path trap) and re-creating (b)-of-§3's global
  serializer as a queue. The state machine we need **already exists — it is the core**;
  L1 should stay a thin threaded shell around it, not a second state machine. (The
  single-threaded loop DOES exist in this design — as the deterministic test harness,
  §8, where it belongs.)

### 4.2 Recommendation: **(a)**, with two hard rules

- **★ Every poll must be provably BOUNDED (F1).** ★ **This supersedes the older wording,
  "no busy-polling anywhere in L1", which was both wrong and untestable.** It was wrong
  because a short spin on something that completes in microseconds is fine and routinely
  beats a syscall — `std::sync`'s own mutexes spin before parking, and forbidding that
  forbids the fast path along with the bug. It was untestable because "no polling" is an
  *absence*, and a test cannot observe an absence: the mutation gate's three
  spin-versus-park survivors (`l1_architecture_summary.md` §mutation) are exactly that
  blind spot — mutants that turn a park into a spin stay green, because nothing asserts
  anything a spin would violate.

  What actually went wrong in the C was **unboundedness**: a guest loop with no ceiling on
  iterations, each one a nested-virt vmexit, ~40k exits per phase. So the rule names the
  defect: a poll is legal iff its bound is *stated* and the bound is *asserted*.

  The testable consequence: **assert a bound, not an absence.** Count the events a wait
  can generate and assert the count — the reactor's wake count against the signal count
  (`l1_os_shell.md` §3.4), the pool gate's wait count against the saturation events that
  caused it (§7.2, [`SharedDevice::pool_waits`]) — never "and it did not spin", which no
  test can see. A poll with no such counter is not merely unmeasured; it is the rule's
  violation, because an unstated bound is an unbounded one.

  In practice, every L1 wait is still `epoll_wait`, a condvar, or a deadline: those are
  bounded by construction (one wake per signal), which is why they remain the default and
  a spin needs an argument. The completion pump is edge-driven (§5.2); there is no
  periodic "scan everything" thread. If a backstop timer proves necessary it is a
  `Vmm::defer` deadline, armed only while something is outstanding, cadence bounded.
- **No atomics, no lock-free structures, no hand-rolled synchronization in L1 logic.**
  `std::sync` primitives (`RwLock`, `Mutex`, `Condvar`, `mpsc`) only. This keeps TSan
  as a meaningful ceiling and makes `loom` unnecessary (the stress suite's standing
  note: add loom only if a lock-free path is ever introduced — the rule here is:
  don't introduce one).

### 4.3 Where each blocking thing lives (the inventory)

| Blocking operation | Thread | Bounded by |
|---|---|---|
| host RM verb (round-trip on a checked-out worker channel) | calling vCPU thread (or executor) — **NO lock held (R1)**, plan/execute/commit | the host ioctl itself + the #73 interrupt path (§5.4) |
| wait for a free worker (pool at bound, §7.2) | calling vCPU thread, condvar — all locks released (R1), re-enter + re-validate (R5) | pool sizing + guest backpressure |
| wait for host os-event / isolate death / defer deadline | reactor loop, in `epoll_wait` | event-driven; no bound needed |
| `CoreEvent` inbox wait | executor, condvar | wake on push |
| guest semaphore wait | **the guest's own vCPU**, on a passthrough page | not our thread at all — decision #6's payoff |
| anything else | — | forbidden; a new blocking site is a design change |

---

## 5. Decision 3 — completion-delivery threading (where the C bled)

The core's completion plane is done (per-proc `CompletionQueue`, drain-gated
`DeliveryPlane`, poll-driven re-post — the F2 fix as structure; `FenceArms` for pattern
e). L1's job is to drive it from the right threads. Patterns (a)–(e) from
`execution_plane.md` §1.2/§2.4, threaded:

### 5.1 The passthrough patterns cost L1 nothing (a, c, e-read)

Shared-page semaphore polls (a), CE-method releases landing in shared pages (c), and
the NVENC mapped-fence *read* (e) are guest-visible host-GPU writes into passthrough
memory. No L1 thread is involved in the guest observing them — that is decision #6
working as intended. L1's only duty is what the core already gates: the pages were
published per-`Vas` before the ring (inherited law 7).

### 5.2 The interrupt/os-event path (b, d) — the pump, precisely

The flow, end to end, with thread attribution:

1. **Observation.** A host completion becomes observable to us one of two ways:
   - the isolate armed a host os-event fd — a registered completion source (§6);
     readiness fires in the **reactor loop's** epoll → `CoreEvent` → executor →
     dispatch (source → owning proc → observe). (The F3 producer, finally real.
     Register-at-arming, so there is no per-wait arming round-trip — the source
     enters the reactor's list once.)
   - a parse/exec path observes a `SemRelease`/fence advance synchronously (vCPU
     thread, under the owning proc's lock) → `CompletionQueue::observe` /
     `FenceArms::observe` right there.
   Observation is always per-proc state under the proc lock — never gated on any other
   proc (F2).
2. **Posting.** `deliver_completions(gpu, vmm, target)` — the pump — runs under the
   **device write lock** (it composes across procs' queues and consults the per-target
   drain gate; it is pure + one `raise_irq`, microseconds — R1-compliant). With R2
   revised, acquiring that write lock no longer queues behind any in-flight blocking
   verb — the review's P1 coupling (slowest verb delays every proc's delivery) is
   gone by construction. It is invoked on **edges only**:
   - after an observation lands (same thread continues: proc lock released → device
     write lock → pump);
   - on the guest's own completion-poll RPC (`poll_completions` — the starvation fix's
     entry, driven off the **poller's own** vCPU thread; the core re-posts the poller's
     un-acked events regardless of anyone else's activity);
   - on IRQSCLR (`completions_drained`, vCPU thread) — draining opens the gate, so
     pump once more in case pending piled up behind it;
   - on `CoreEvent::Deferred(CompletionRedeliver)` (executor) — the *bounded backstop*,
     armed via `Vmm::defer` only while a proc has outstanding un-acked completions,
     never periodic-forever (F1).
3. **Injection.** `Vmm::raise_irq(SWGEN0)` from inside the pump. Per-target gate: one
   batch outstanding per GPU's GSP queue (the transport constraint), but a batch
   carries many procs' events and re-posting is owner-poll-driven — the gate is a
   *transport* serialization, never an *observation* one. **L1 must not "optimize" the
   gate away or widen it** — over-posting desyncs the seqNum ring (L10), and the core
   owns the policy.

**What L1 must never do (the F2 checklist, explicit):** no delivery driven solely from
another proc's doorbell; no `any_completed`-style global gate in front of `observe`; no
single "delivery thread" that round-robins procs behind one queue (per-proc delivery
state is already per-proc — keep the *driving* per-edge, as above).

### 5.3 Blocking-sync / os-event relay without a poll storm

The F3 mechanism, restated for Mode-2: a guest kernel blocked in an os-event wait
(`MC_SERVICE_INTERRUPTS`-shaped RPC, blocking `cuEventSynchronize`, NCCL) is woken by
the SWGEN0 the pump raises. The producer chain is: host RM signals the os-event fd →
reactor epoll (blocking wait, zero cost while idle) → `CoreEvent` → executor →
dispatch → `observe` + pump → IRQ. Every hop is edge-driven, and none of it touches
the isolate RM channels (the §3.5 structural-independence guarantee). The C's failure was a missing
producer; the second-order C failure to also not repeat is *polling* producers
(the ineffective `m2_poll_kick` doorbell-replay) — there is none here.

### 5.4 Signal-interruptibility of a forwarded op (guest dies mid-op → no wedge)

The #73 design, ported to the isolate wire protocol (F4):

- Every verb request carries a txn id. The worker publishes its in-flight txn and
  installs a no-`SA_RESTART` signal handler; an **interrupt message** (out-of-band
  byte on the socket — it is 1-deep request/reply, so OOB here means a second tiny
  control pipe, or `SIGUSR1` via the worker's pidfd) makes the blocked ioctl return
  `EINTR`; the worker then replies `Interrupted{txn}` on the normal reply path.
- **The requester never abandons the reply buffer** (the C's UAF lesson, verbatim):
  on interrupt it still blocks for the `Interrupted` reply, then surfaces the refusal.
  ★ **What bounds that wait, corrected (§12.26):** *not* an EINTR unwind. RM's waits
  are uninterruptible (`ogkm: .../gpu/gsp/kernel_gsp.c:2963-3060` — busy-poll, no signal
  check; `.../resserv/src/rs_server.c:3164-3168` — bare refcount spin), so what the C
  measured as a "~3.4-3.5 s bounded EINTR unwind" was almost certainly RM's own **4 s**
  RPC timeout (`.../os/os.c:2136-2139`) elapsing. The wait is bounded by RM's timeouts
  (6 s for a GSP RPC — 4 s x 1.5, `.../gpu/gsp/kernel_gsp.c:2927`), which is what
  `VERB_BUDGET` must be sized against, and it carries the corollary that **an
  interrupted alloc probably completed** (§12.16's G4 open question, now with a prior).
  Per R1 the requester holds NO lock while waiting (revised in #37 — the original text
  held the proc lock throughout; under R1 a multi-second hold would stall every sibling
  thread of the dying process, a #37 violation on the teardown path of all places).
  The commit phase then re-acquires and re-validates (R5) — which in the retire case
  means: surface the refusal, hand the worker to the retiring pool, touch nothing
  stale.
- Trigger points: proc retire (guest process death detected via the RM protocol),
  guest reset, isolate watchdog (§11). `Isolate::retire()` = interrupt EVERY in-flight
  worker + refuse new checkouts; reap (waitpid, namespace teardown) is deferred to the
  quiesce point per L10.

---

## 6. Decision 4 — the completion-source reactor: a core-owned PURE port

**(New in #37 — the load-bearing addition.)** §5 defined *when* completions are
observed and delivered; this section defines the machinery that *watches* for them —
and, critically, which side of the hexagonal boundary each piece lives on.

### 6.1 The pattern

A reactor: one main loop joins a dynamic SET of completion sources and dispatches each
signal to its owning proc/op. The concrete source classes today:

- **host os-event fds** — the nvidia fds the isolate arms for RM os-events (the F3
  producer, §5.3);
- **isolate worker pipes** — the per-worker channels (§7.2): death/HUP detection and
  any out-of-band worker signal;
- **cross-isolate pipes** — the seam for cross-process signaling (completion-sema
  passthrough across procs; future, but a source class from day one so adding it is
  a list entry, not a redesign);
- **the notifiable source** — the wakeup primitive itself, signalled to make the loop
  re-read the source set (a source was added or removed) or pick up injected work.

Sources are added and removed **dynamically, as a plain list** — registered when an
isolate spawns or arms an os-event, deregistered at retire — and the main loop joins
whatever the list currently holds. No fixed slots, no rebuild-the-world on change:
signal the notifiable source, the loop re-joins.

### 6.2 ★ The boundary — state it hard

**The CORE owns the model, and the model is PURE.** In the core there are exactly:

- **opaque `CompletionSource` handles** — minted at registration, meaningless except
  as identity;
- **the source registry and its dispatch logic**: *"source S signalled → which proc →
  what to do"* (observe a completion, retire a proc on worker death, arm a pump
  edge). This is where the routing knowledge lives; it is bookkeeping over core
  state and runs under the same locks — and the same R1–R5 discipline — as any core
  entry;
- **an abstract "notifiable source"** — a thing that can be signalled to wake the
  join. The core can *request* a wake; it does not know what a wake IS.

**No fds. No syscalls. The words "eventfd" and "epoll" do not appear in the core** —
the core says "notifiable source" and "completion source", full stop. If a core-crate
diff ever contains an fd type or a syscall name, the boundary is breached and review
rejects it on sight.

**L1 owns the adapter:** the table mapping `CompletionSource` → fd, the real
`epoll_wait`, the real `eventfd` behind the notifiable source, and the registration
plumbing that turns "isolate armed an os-event" into an fd in the epoll set. The
reactor's *loop thread* is §2's reactor loop, and it keeps the old watcher's cardinal
property: it maps fd → `CompletionSource` (a pure table lookup), pushes a `CoreEvent`,
and touches ZERO core state. The *dispatch* runs on the executor under the normal
locks — inherited law 9 intact: no core re-entry from the loop thread.

### 6.3 Why this earns its abstraction

1. **It is the hexagonal fit.** The core already owns the completion *plane*
   (`CompletionQueue`, `DeliveryPlane`, `FenceArms`); without this port, the
   knowledge "which readiness means what" would smear into L1 — exactly the layer
   that must stay a thin shell (§8.1). With the port, L1's reactor shell is
   fd-plumbing with no decisions in it.
2. **It is what makes the concurrency deterministically mock-testable.** The mock
   drives source-signals directly — `dispatch(source)` — with ZERO syscalls: every
   completion interleaving §5 describes becomes a T1 scripted order, and the mean
   test (§8.4) can hold sources pending and fire them in adversarial orders with no
   timing dependence. Without the port, testing the completion flow means real epoll
   and real pipes — timing-dependent, the thing §8 forbids.
3. **It kills the F2 shape at the model level:** dispatch is per-source → per-proc by
   construction; the model has no place to even *write* an `any_completed`-style
   cross-proc gate.

**Residual (honest):** dispatch still *executes* on the single executor — the
review's P4 funnel. Mitigations unchanged: §5.2's non-executor edges (poll and
IRQSCLR fire from vCPU threads) mean every completion pattern has a non-executor
path or a bounded executor hop, and the mean test's "a poll fires while a verb is
pending" assert covers the guarantee that matters (#37). Per-proc executor sharding
is a named future seam if a measured workload ever shows executor latency — not
before.

---

## 7. Decision 5 — isolate driving

### 7.1 One process per `(Proc, GpuId)`, driven synchronously by its owner's threads

Options considered: thread-per-isolate on the QEMU side (a dedicated relay thread per
isolate), a shared verb thread-pool (the C stub's shape), async socket I/O, or —
recommended — **no dedicated QEMU-side relay threads at all**: the calling thread
(vCPU or executor) does the round-trip itself — per R1 with no lock held, on a worker
it checked out (§7.3).

Rationale:
- The verb surface is control-plane (alloc/map/schedule/ring/control) — short, and
  issued while the guest is blocked on the corresponding RPC/trap anyway. A relay
  thread would add a hop to every verb and create *idle thread × isolate count*
  scaling for nothing.
- **Deliberately absent from the verb surface: blocking host waits.** Waiting is done
  by the guest (passthrough sema) or by the reactor (os-event fd, §6). A verb that
  could block indefinitely by *design* (as opposed to by host failure) would break
  the confinement story — refuse to add one; that's a design-discussion tripwire.
- Backpressure is inherent: the caller blocks (lock-free), the guest's RPC stalls,
  the guest slows down. No queue to size, no overflow policy to invent.

One placement note that keeps the rest of the design small: **the isolate pool is
where ALL the blocking concurrency lives.** Calls into the VMM and the core remain
mainly memory management plus the inevitable synchronous work — short and clean.
Concurrency questions stay local to this section and §3; they do not leak into the
core's shape beyond the plan/execute/commit seam R1 already names.

### 7.2 REVISED (#37): from one single-in-flight worker to a bounded pool of single-in-flight workers

**The original decision (#34)** was one single-threaded worker per isolate, strictly
one verb in flight, derived from `Isolate::rm(&mut self)`: a shared reference to the
backend is unrepresentable, so single-in-flight came free from the type system — and
it deleted the C stub's whole thread-pool/`worker_inflight_txn[]`/slot-mapping
apparatus, itself a proven bug source. That decision flagged its own cost honestly:
*"a proc cannot overlap two of its own host verbs … if a real workload ever proves
otherwise …"*.

**The revision, and why.** The §3.5 intra-process invariant is that proof — arrived
at by *requirement* rather than by workload measurement: a multi-threaded guest
process is one `Proc` with one isolate, so single-in-flight-per-isolate makes thread
B's verb queue behind thread A's — a soft intra-process serialization exactly where
the invariant says none may exist. Therefore: **each isolate holds a bounded pool of
workers, EACH worker strictly single-in-flight.** Everything that made
single-in-flight clean is preserved PER WORKER — one `&mut`-owned client handle, one
1-deep request/reply channel, no txn multiplexing (txn ids still exist only for the
per-worker interrupt handshake, §5.4). N workers = N independent 1-deep channels = up
to N concurrent RM verbs from one process's threads. What stays deleted is the C's
*actual* bug source: there is still no shared in-flight slot table and no txn demux
on a shared channel — **concurrency comes from channel COUNT, never from channel
multiplexing.**

**Worker shape.** The isolate remains ONE sandboxed process per `(Proc, GpuId)` — the
sandbox, the RM client, and the handle namespace are per-process identities and must
stay singular. Workers are threads inside that process, each servicing exactly its
own channel end to end (recv → real RM ioctl → reply, plus the #73 signal handler);
the only state they share is the RM fd, which is kernel-mediated.

**★ CORRECTED (§12.26): what the shared RM fd actually buys, and it is not
parallelism.** This paragraph used to end *"concurrent ioctls on one RM client are
ordinary host behavior (multithreaded libcuda does it all day)"*. **Legal and ordinary
— but not concurrent.** Source-verified in `ogkm`:

- **Every** resource-server entry point reachable from an ioctl takes the per-client
  lock in `LOCK_ACCESS_WRITE` (`src/nvidia/src/libraries/resserv/src/rs_server.c:778`,
  `:1143`, `:1503`, `:1923`, `:2009`, `:2131`, `:2218`, `:2546`), and alloc *asserts*
  it (`:786-788`). The **only** client-READ site in the driver is kernel-internal, not
  reachable from an ioctl at all: `nvGpuOpsGetExternalAllocPtes`
  (`.../rmapi/nv_gpu_ops.c:4674-4676`, UVM). NVIDIA special-cased exactly one hot path
  to get same-client concurrency, which is strong evidence about the general case.
- **Alloc and free additionally take the GLOBAL API lock in WRITE.** There is one
  `g_RmApiLock` (`.../rmapi/rmapi.c:53-58`, `:535`); the default `apiLockMask` is
  `NVBIT(RS_API_CTRL)` only (`.../core/system.c:423`), so
  `serverAllocResourceLookupLockFlags` / the free equivalent override read-only back to
  WRITE (`.../rmapi/alloc_free.c:1714-1718`, `:1746-1748`). Only
  `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` escapes. The API lock is held **across** the GSP
  RPC (`.../gpu/gsp/kernel_gsp.c:398`, `:2954`).
- gVisor corroborates from production, under real CUDA: a per-client exclusive mutex
  held across the host ioctl (`gvisor: pkg/sentry/devices/nvproxy/frontend_unsafe.go:367-381`).

**Therefore the pool buys ~nothing on the wire**, and the honest justification is the
other one — the one §3.5 actually asks for: **liveness and latency isolation.** A verb
that takes six seconds (a GSP RPC at its timeout, below) must not make a *sibling*
thread's independent verb appear to hang, trip a guest-side watchdog, or serialize an
unrelated `Vas`'s first touch behind it. That is a property about *observability of
progress*, not about throughput, and it is the property the §3.5 invariant states. The
pool is how a slow verb stays one thread's problem.

**Calibration (owner-directed, explicit; recalibrated by §12.26).** Design the
*interface* for N-in-flight from day one — checkout/return, per-worker channels,
per-worker interrupt, per-worker reactor sources — but implement a **BOUNDED,
statically-sized pool first**, of **2–4 workers, deliberately NOT scaled to the vCPU
count** (`DEFAULT_POOL_WORKERS = 4`). The old "on the order of the vCPU count" phrasing
implied a scaling relationship that the locking evidence above says does not exist:
past the point where one slow verb cannot hide a fast one, extra workers are extra
host threads queued in **D state** on the same uninterruptible `down_write`, which
costs teardown latency and cancellation complexity and buys no wire concurrency.

Make the pool *dynamically* scaling only when a measured workload shows the bound
hurts. Premature dynamic scaling is a complexity trap: a spawn/reap policy,
thundering-herd wakeups on growth, worker-lifetime races — all cost, no demonstrated
benefit. Pool exhaustion is meanwhile well-behaved backpressure: the guest thread waits
(lock-free, R1) for a worker, exactly as it would wait for the host ioctl itself — and
the poll/completion path needs no worker at all (§3.5), so sync progress never queues
behind the pool.

### 7.3 Checkout/commit — mapping the pool to the shard

- The pool lives *in* the `Proc` (`Proc::isolates[gpu].workers`), so the proc lock
  guards the pool **bookkeeping** — checkout and return — and only that. Checkout
  (under device-read + proc, R2) marks a worker busy and moves its handle OUT to the
  calling thread; the round-trip runs lock-free (R1); return + result-commit
  re-acquires and re-validates (R5). The checked-out worker is `&mut`-owned by
  exactly one thread for the duration — the single-in-flight-per-worker guarantee is
  still the borrow checker's, just N times over. One lock, plus ownership transfer;
  no second lock, no ordering question.
- Pool-full: release ALL locks (R1's definition note), wait on the pool's condvar,
  re-enter from the top — full re-validation (R5), because the proc may have retired
  while waiting.
- Spawn (`IsolateFactory::spawn`) happens inside `Gpu::apply`/refresh (lazy, at
  target materialization) — which runs under the **device write lock**, and
  `fork+exec` of a sandboxed worker is not pure logic. This is a real R1 tension,
  resolved by making spawn **two-phase**: the factory under the lock only *reserves*
  (allocates the session slot, records intent — cheap); the actual
  fork/exec/namespace setup and pool bring-up run lazily at the first checkout
  (lock-free, R1-compliant) or on the executor. The `IsolateFactory` doc already
  permits "spawn (or lazily reserve)" — L1 chooses reserve.
- Reap: `retire()` eager (interrupt every in-flight worker + refuse new checkouts,
  §5.4); `waitpid` + teardown at the quiesce point the adapter declares (GSP
  re-handshake / idle), on the executor, via `CoreEvent::Deferred(DeferredReap)` →
  `Gpu::reap_retired()` (which also recycles the GPA arenas — the #80 leak's fix).
  Worker death out-of-band (crash) is a reactor source firing HUP (§6.1) →
  dispatch → retire the proc loudly (its completions die with it — the guest tore
  down or the sandbox failed; either way MISS=FAULT posture, no resurrect).
  **"No resurrect" is not free** — removing the proc is not enough, because the
  guest's client root is still in the graph and the next `refresh` would re-derive
  it. The retire additionally **condemns the component** (its client set); §12.13
  is the finding, the mechanism and the fault surface.

★ **Why a condemned component is never resurrected.** Stated here because the
mechanism is only as durable as its reason, and the obvious reason is the wrong one.
It is *not* "the dead worker left host state the core cannot reason about": the
isolate is a **process**, so when it dies the host kernel reasons about it for us —
its fds close, RM tears down its client objects and everything they own, its mmaps
go. Clearing our now-stale bindings (`host_channel`, `host_engine_objects`,
`host_vas`, each `Binding.host`) and re-materialising through the lazy first-touch
paths that already exist would be *almost* clean.

It is wrong anyway, because **the guest's DATA died with the isolate.**
`publish_backing` allocates host memory (`RmBackend::alloc_sysmem`) owned by that
isolate's RM client, and the host kernel frees it with the process. Re-materialising
hands the guest a fresh, **zeroed** backing for a VA it believes still holds its data.
That is not recovery, it is **silent data corruption** — strictly worse than the
resurrection bug it would be "fixing", which at least failed visibly at the next real
operation. So: fail loudly, and fail with the semantic real hardware already has. A
GPU that faults a channel does not silently hand back a fresh context; it makes the
context **sticky-fatal** until the application tears it down and builds a new one.
Every CUDA application already handles that path, because Xids exist.

★ **Sticky-fatal, not bricked — recovery does not require the process to die.**
Condemnation is keyed on the **client set**, so an application that re-initialises (a
fresh CUDA context allocates a new RM client) forms a component with no dup edge to
the condemned set, is therefore a *different* component, is therefore not condemned,
and simply works. And if the process *does* die, the guest kernel frees its clients on
its behalf, so the entry clears with no cooperation from the application at all. In
Mode-2 the guest kernel is the garbage collector at one boundary and the host kernel
is the garbage collector at the other; condemnation only has to refuse to paper over
the gap between them. Both halves are executable, not assumed — §12.17.

**Carve-out, to be stated per backing class.** Where a backing is **guest RAM**
double-mmapped into the isolate rather than host-allocated, the data lives in *guest*
RAM and survives the isolate's death, and transparent re-materialisation would be
legitimate there. No such backing class exists today (§12.17 checked it). Any future
path claiming the exemption must say so explicitly and prove its backing class.

---

## 8. Decision 6 — deterministic testability (non-negotiable)

The core's whole value is mock-testable determinism; L1 must not be the layer where
that dies. The design principle: **the threads are a shell; everything they do is a
pure function the tests call directly.** And the epistemic stance, stated up front
(#37): **this design is not trusted.** It is a pure-logic argument plus the C's scar
tissue; §8.4's harsh simulated harness, with its mean asserts, is what the
architecture actually converges to. Where harness and design disagree, the design
moves.

### 8.1 The thin-waist rule

All L1 *logic* — trap decode → core call → reply composition; readiness → `CoreEvent`
mapping; event dispatch; the pump-edge selection of §5.2 — lives in plain synchronous
functions with no thread, clock, or fd types in their signatures (they take
`&mut dyn Vmm`, core types, and byte slices). The threaded production shell (the ~few
hundred lines that own epoll fds, locks, and thread spawns) only moves bytes between
the OS and these functions. **Tests exercise the same functions as production** — the
only untested-by-determinism residue is the shell itself, which is covered by the
threaded stress tier (T2) and kept too small to hide logic in.

### 8.2 The test tiers

- **T1 — deterministic single-threaded (the default, the vast majority).** The harness
  is the §4.1(c) event loop, built where it belongs: `MockVmm`'s virtual clock (time
  moves only on `advance()`, deferred events fire in deadline order — the pinned
  semantics real L1 `defer` must match), a plain `VecDeque` inbox the test can inspect
  and **permute**, and direct calls to the thin-waist functions. Multi-vCPU
  interleavings are driven as *scripted call orders* from one thread — sound because
  of inherited law 1: the core is order-independent, so any interleaving's facts can
  be presented in any serial order and must yield the same derived state. The
  determinism differential suite (whole-`Gpu` `CoreSnapshot` over permutations)
  already proves the core half; T1 extends the same technique over the L1 waist.
- **T2 — real-thread stress.** `concurrency_stress.rs` extended to the L1 wrapper:
  real `RwLock`/`Mutex`/inbox, mock ports underneath, 16 threads, watchdog, bounded
  iteration counts, TSan on nightly as the race ceiling. This is where the shell and
  the lock discipline (R1–R5, via the guard-wrapper asserts) are validated —
  including mean scripts: a scripted `MockRmBackend` that *stalls* a verb, a scripted
  interrupt mid-verb (the §5.4 handshake), and a kill-the-worker HUP (the §7.3 death
  path). T2 runs **both lock configurations** — degenerate one-lock AND sharded —
  from day one (the review's P5: a late granularity flip must never be the untested
  mode when the #14 gate forces it on).
- **T3 — loom: not applicable by rule.** §4.2 forbids atomics/lock-free in L1 logic;
  if that rule is ever revisited, a loom model of the new path is the mandatory toll.

### 8.3 Clock discipline

No wall-clock read anywhere in L1 logic (the workspace already has no wall clock in
any logic crate — extend the rule): time enters only as `Vmm::now()` and leaves only
as `Vmm::defer(after, ev)`. The production shell derives its epoll timeout from the
earliest pending defer deadline — the one place wall time exists, inside the shell,
untestable and deliberately trivial.

### 8.4 The MEAN integration test — the arbiter (new in #37)

Specified now, implemented as L1's exit gate. This is the test the architecture
converges to; a design section that cannot survive it is wrong by definition.

**Shape:** multi-process × multi-thread × multi-GPU × multi-workload, entirely
mock-driven — no real GPU, no real fds. `MockRmBackend` workers whose verbs the
script can hold pending; reactor source-signals driven directly through the §6
dispatch (zero syscalls). Several guest procs, each **multi-threaded** (so several
threads share ONE `Proc` — the §3.5 case, which per-proc sharding cannot cover),
across ≥2 mock GPUs, concurrently running mixed workloads: an alloc/map-heavy
control-plane thread, a doorbell-heavy submission thread, a poll/event-wait-heavy
sync thread, and a process that tears down mid-flight.

**★ The parallelism assertion is PROGRESS-UNDER-PENDING, not wall-clock.** No sleeps,
no timing thresholds, no "finished within X ms" (§8.3 forbids the clock anyway).
Instead, the mock isolate HOLDS thread A's verb *pending* — a scripted latch the test
releases explicitly — and while it is held the test asserts:

- thread B (**same proc**) submits an independent op and it COMPLETES end to end;
- a completion source fires (mock signal) and B's poll observes it — delivery ran;
- a second proc makes full progress on another GPU;
- only THEN does the latch release; A's verb commits and A's op completes with a
  correct, re-validated result.

That proves the #37 invariant and the confinement story *structurally* — zero timing
dependence, deterministic on any machine, immune to the "it passed because the box
was fast" failure mode of wall-clock parallelism tests.

**The invariant asserts run hot the whole time** (they are the point, not
instrumentation):

- **lock-rank order (R3):** the per-thread rank watermark panics on any out-of-order
  acquisition, in every tier, every run;
- **no-blocking-under-lock (R1):** every blocking-verb entry (mock included) asserts
  the thread-local lock-depth counter is zero;
- **re-validate-after-relock (R5):** staleness canaries — while A's verb is held
  pending, the script retires A's proc / tears down the routed channel / runs an
  `apply` that rewrites routing, then releases the latch and asserts A's commit
  REFUSES (or re-resolves) instead of touching dead state.

The standing T2 mean scripts (stalled verb, interrupt mid-verb, worker HUP) compose
into the same run rather than living as isolated cases, and the whole test runs under
both lock configurations (§8.2).

**Pass = the design survived contact. Fail = the doc changes, not the assert.**

---

## 9. Decision 7 — the explicit thread-safety contract per interface

Owner's rule: thread safety is assumed unless documented otherwise — so every L1
interface states its contract. This table is normative; each row becomes a rustdoc
header on the interface when L1 code lands.

| Interface | Send/Sync | Who calls, from where | Concurrency contract |
|---|---|---|---|
| `Device::{mmio_read, mmio_write, event}` | (trait on `Gpu`) | vCPU threads (traps), executor (events) | **serialized per device by the adapter's locks** (§3); entry implies device lock held per R2-R4. Never called from reactor-loop or isolate context |
| `Vmm` impl (the real one) | `Send`, not `Sync` (as declared) | only as `&mut dyn` from within a core entry, i.e. under the caller's locks | one caller at a time by `&mut`; impl may keep fds/buffers without internal locks. `raise_irq` must be non-blocking (R1); `defer` must be deadline-ordered + deterministic (MockVmm-pinned semantics) |
| `RmBackend` impl (per-worker channel client) | `Send`, not `Sync` (the documented exception) | exactly one thread at a time, via the checked-out worker handle (`&mut`, moved out at checkout §7.3) — **NEVER while any lock guard is alive (R1, asserted)** | MAY block (the one sanctioned blocking site, §4.3), lock-free by invariant; MUST be interruptible (§5.4); MUST NOT be callable after `retire()` (refuse loudly); one handle per pool worker, single-in-flight per handle by `&mut` |
| reactor dispatch model (core, §6) | pure core logic (`Send + Sync` values) | executor only, under the normal locks | opaque `CompletionSource` registry + dispatch; NO fd or syscall types anywhere in its signatures ("notifiable source", never "eventfd") — the mock drives it syscall-free |
| `Isolate` / `IsolateFactory` | `Send + Sync` (supertraits, core-stored) | proc-lock holders / device-write-lock holders (spawn=reserve only, §7.3) | mutation `&mut`-exclusive per the core contract; `is_retired()` is a pure read |
| executor inbox (`CoreEvent` queue) | the ONE concurrent L1 structure (rank 2, leaf) | producers: reactor loop, `Vmm::defer` impl; consumer: executor | `std::sync::mpsc` (or Mutex<VecDeque>+Condvar); no capacity-unbounded growth from guest input (events are per-fd/per-deadline, not per-guest-byte); FIFO per producer, total order defined by the executor's drain |
| reactor loop thread (§6, the shell) | owns its epoll fd set + source↔fd table exclusively | nobody calls into it; it only produces inbox events (woken via the notifiable source) | touches ZERO core state — enforced by giving it no reference to the device, only the inbox sender + the source↔fd registry |
| isolate worker (thread in the isolate process) | n/a (process boundary) | its own 1-deep channel only | one channel end-to-end per worker; single in-flight per worker; interrupt via signal; shares only the kernel-mediated RM fd with sibling workers; the process's handle namespace dies with it (§7.2) |
| the raw module (below) | per-item | adapter crates only | the only `unsafe` in the workspace |

### 9.1 `unsafe` policy (decision #16/#16b, applied to L1)

- Every L1 logic crate: `#![forbid(unsafe_code)]`, same as the core. The CI grep-gate
  extends to L1.
- **One audited raw module** (working name `kayfabe-linux-raw`) for the operations that
  cannot be safe Rust: `mmap`/`munmap` of guest-RAM exports and shared pages, volatile
  access to concurrently-GPU-written pages (VolatileSlice-style — semas/USERD are
  written by real hardware; non-volatile access is UB-adjacent tearing), and — for the
  non-QEMU harness backend — KVM ioctls. It exposes only the bounded-object API
  (#16: `read(off,len)->Result`, no raw pointer escapes; domain-typed `Gpa`/`HostOffset`
  newtypes; `trybuild` compile-fail tests assert the dangerous patterns don't compile).
- **Flagged for the audit plan:** the mmap plumbing (double-mmap of guest RAM into
  isolates, `export_ram` → worker `MAP_FIXED`) is the memory-safety breakout surface
  the threat model defers to post-L2 — it is *born* in L1, so the raw module's fuzz +
  review debt starts accruing now, and its API review is part of L1's exit gate, not a
  later cleanup.

---

## 10. OWNER DECISIONS — the ledger

Status after revision #37 (the owner-directed refinements, folded in above). Items
marked **decided (#37)** were open questions or different recommendations in #34 and
are now settled by owner direction; items marked **superseded** record where #37
explicitly changed a #34 recommendation — the change and its rationale live in the
cited section, not only here. Two items remain genuinely open.

1. **★ The sharding model and its core-shape cost (§3) — decided.** Hybrid (a):
   device `RwLock` + per-`Proc` `Mutex`; the core refactor landed as decision #35
   (route/act split + `Gpu` ownership split, 143/143 green). #37 revised the rules
   that govern it: the three asserted invariants R1/R3/R5 + structural R2/R4, with no
   lock ever held across a blocking verb (review P1 resolved). Staging unchanged —
   L1-M1 ships the degenerate one-lock configuration (now even safer, §3.2 item 1),
   the #14 gate flips it, and T2/§8.4 run both configurations throughout (P5).
2. **★ Threads, not async (§4) — confirmed.** Fixed-role OS threads, no tokio, no
   atomics/lock-free in L1 logic (loom stays out by rule). Unchanged by #37 — and
   sharpened: R1's plan/execute/commit split applies to core *paths*, not to the
   thread model; verbs still run synchronously on the calling thread.
3. **Blocking verbs on vCPU threads (§4.3/§7.1) — decided (#37), amended.** Yes:
   the calling vCPU thread does the round-trip itself (no per-verb executor bounce,
   no relay threads) — but **lock-FREE on a checked-out worker via
   plan/execute/commit, never under the proc lock**. The #34 recommendation ("under
   the proc lock — the guest is blocked anyway") is superseded by invariant R1; see
   §3.3 for why the old shape was the P1 hole and an intra-proc #37 violation.
4. **Isolate wire protocol — SUPERSEDED (#37): single-in-flight per isolate → a
   bounded pool of N single-in-flight workers (§7.2).** Per-worker channels stay
   1-deep request/reply with no txn multiplexing — the C-stub bug apparatus stays
   deleted; concurrency comes from channel count. Interface designed for N from day
   one; pool statically bounded first; dynamic scaling only on measured need
   (premature dynamic scaling = complexity trap). The #34 escape hatch ("per-channel
   sub-verbs batched into one request") is retired — the pool is the answer to that
   traffic, and the §3.5 invariant is the workload-proof #34 said it would wait for.
5. **The pump's backstop timer (§5.2) — confirmed.** Edge-driven pump only, plus a
   defer-armed backstop while completions are outstanding; a *periodic* redeliver
   sweep stays forbidden (F1) — the "harmless safety poll" that historically creeps
   in during a debugging session.
6. **The §11 B3 residual (system-proc stall) — still OPEN.** Materially improved by
   #37: a stalled system verb no longer holds the device read lock (R1), so it no
   longer blocks apply/pump behind the write lock, and other system verbs proceed on
   other pool workers; the watchdog still converts unbounded → bounded-and-loud.
   Owner call remains: accept as residual, or demand a stronger story before L1-M1.
7. **The reactor boundary (§6) — decided (#37).** Core owns the pure
   source/dispatch/notify model; L1 owns fds/epoll/eventfd. The vocabulary rule —
   "notifiable source" and "completion source" in core, never "eventfd"/"epoll" —
   is review-enforced.
8. **The mean test as arbiter (§8.4) — decided (#37).** It is L1's exit gate, its
   invariant asserts run in every tier, and design-vs-harness disagreements resolve
   in the harness's favor.

---

## 11. Honesty — the bets, and the biggest risk

**Bets this design makes, named:**

- **B1 (revised by #37): synchronous-confined beats asynchronous-everywhere — with
  the plan/execute/commit seam now WIRED, not held in reserve.** The original bet was
  that blocking verbs could stay in-line under per-proc locks and the
  `IsolateComplete{session, cookie}` seam would never be needed. Invariant R1 cashes
  that seam in as the *standard* shape for every blocking verb: the locked core phase
  emits the verb + a typed continuation cookie, the calling thread executes it
  lock-free, the commit phase re-enters with the reply (§3.3). What survives of the
  bet — and it is the load-bearing part — is the THREAD model: no async runtime, no
  task soup; the calling thread drives its own verb end to end, and a hung guest is
  still diagnosed from thread stacks. What was conceded, honestly: the verb-issuing
  core paths carry the split's re-entrancy (a cookie'd resume point per verb site) —
  bounded, typed, and applied *uniformly by design* rather than retrofitted per-verb
  under debugging pressure, which was exactly the failure mode the original text
  feared about its own fallback.
- **B2: the trap-minimization premise holds**, i.e. the contended lock paths stay
  off the steady-state hot path because the hot path has ~no traps. This was proven
  for the C bare-metal (`C: mode2_baremetal_32`) but the Rust L1 will first run under
  the same nested-virt bench where vmexit costs dominate everything — perf conclusions
  from the bench must be read through that filter (the C's rom-device lesson: a
  correct trap-elimination showed zero exit-count win under nested virt).
- **B3 (RESTATED by §12.26 — it is a PLATFORM LIMIT, not a weakness of this design,
  and not something we solve).** The old text claimed R1 had bought device-level
  independence: *"the stalled system verb holds NO lock … apply/pump proceed freely —
  what stalls is only the specific system-proc op whose worker is wedged."* The first
  clause is true of **our** locks and irrelevant to the outcome, because the host kernel
  does not participate in our lock discipline.

  **What actually happens.** A wedged host ioctl holds the host **RM global API lock**
  — an uninterruptible `down_write` (`ogkm: kernel-open/nvidia/nv-linux.h` /
  `.../os-interface.c:330-338`) — **across** the GSP RPC (`.../gpu/gsp/kernel_gsp.c:398`,
  `:2954`), and every alloc/free of every client takes that same lock in WRITE
  (`.../rmapi/rmapi.c:53-58`, `:535`; `.../rmapi/alloc_free.c:1714-1718`, `:1746-1748`).
  So a wedged verb blocks **every other isolate's** RM calls, in D state, including
  every *user* proc's. No amount of lock-freedom in our bookkeeping changes that.

  **And that is not ours to fix, which is the honest and the important half.** This is a
  property of the platform shared by every GPU consumer on the box — containers, Mode-1,
  bare-metal CUDA, a second VM with passthrough. It is *bounded* by RM's own timeouts
  rather than unbounded: the GSP RPC timeout is 4 s (`.../os/os.c:2136-2139`,
  `defaultus`) × 1.5 = **6 s** (`.../gpu/gsp/kernel_gsp.c:2927`), and RM's own answer at
  expiry is to soldier on (*"Today, we will soldier on if GSP times out"*, `:2999-3002`)
  and escalate to `gpuMarkDeviceForReset` + Xid (`:2772-2792`). We claim neither to
  prevent it nor to be worse than the platform at it; the C's own whole-device wedge was
  a different bug (an untimed blocking isolate ioctl issued **under the BQL** from the
  doorbell trap, `C: nvkvm_gpu_emul.c:3504→3713→8644`, `C: nvkvm_isolate.c:1838-1840`),
  and that one *is* ours to not repeat — R1 is what forbids it.

  **What we do own**, and it is real but modest: the specific op is made **bounded and
  loud** rather than silently pending, via the #73 interrupt plus a watchdog deadline
  (`Vmm::defer`) that surfaces a fault. Two constraints on that, both from §12.26:
  - **`VERB_BUDGET` must be sized against RM's timeouts, not against a measured
    unwind** — ≥ 6 s of GSP RPC plus API-lock queueing behind other clients, so a merely
    slow verb is never mistaken for a wedged one.
  - **The "~3.4–3.5 s bounded EINTR unwind" the C measured was almost certainly RM's
    4 s timeout, not an unwind.** RM's waits are not interruptible: the API lock is a
    `down_write`, the GSP RPC is a busy-poll with no signal check
    (`.../gpu/gsp/kernel_gsp.c:2963-3060`), and the client drain is a bare
    `while (refCount > 1)` spin (`.../resserv/src/rs_server.c:3164-3168`). The
    consequence is a correctness statement, not a performance one: **an interrupted
    `NV_ESC_RM_ALLOC` almost certainly COMPLETED**, because RM had no interruptible
    point at which to abandon it. That is the same open question §12.16/G4 named, now
    with a strong prior and still owed a bench measurement.

  Still flagged as owner decision §10.6 — but the decision is now "accept a documented
  platform limit", not "accept a residual weakness".
- **B4: scripted-order T1 testing is a faithful proxy for real interleavings.** This
  rests entirely on the core's order-independence (inherited law 1) plus the thin-waist
  rule (§8.1). It is a *good* bet — the determinism differential suite is exactly this
  argument, already green — but its blind spot is the shell (lock acquisition order,
  condvar wakeups), which only T2/TSan covers. Keeping the shell small is therefore a
  correctness strategy, not a style preference; shell growth in review is a smell.
  (#37 shrinks the blind spot from both sides: the reactor port moves dispatch logic
  out of the shell into the mock-drivable core (§6.3), and the R1/R3/R5 asserts give
  the shell's lock behavior runtime teeth even in T2.)
- **B5 (new, #37): the drop-lock discipline trades convoy/deadlock risk for
  STALENESS risk.** R1 removes lock-holding across blocking calls; the price is that
  every gap is a window where the world changes, and every commit is a site that must
  re-validate (R5). A forgotten re-validation is a use-after-retire — *quieter* than
  the deadlock it replaced, which is precisely why R5 is an asserted invariant with
  mean-test staleness canaries (§8.4) rather than a convention. The bet: ID-shaped
  route products + forced graph re-resolution (inherited law I3) make re-validation
  the path of least resistance, so the per-site burden stays mechanical instead of
  becoming this design's own bug ledger.
- **B6 (new, #37; premise CORRECTED by §12.26): N workers re-admit — deliberately and
  boundedly — the concurrency the original design deleted.** The C stub's thread-pool
  bugs came from a shared in-flight slot table and txn demultiplexing over shared
  channels; the pool keeps per-worker 1-deep channels and per-worker `&mut` ownership,
  so that bug class has no home to return to. The bet is that channel-COUNT concurrency
  is categorically safer than channel-MULTIPLEXED concurrency — believed strongly,
  argued from the type system, proven only by the mean test.
  ★ **What the bet is NOT.** It is not a bet on throughput. RM serializes every
  ioctl-reachable path on the per-client WRITE lock and takes the global API lock in
  WRITE for every alloc/free (§7.2, cited), so N workers on one isolate produce N
  *queued* host threads, not N concurrent RM operations. The pool's value is entirely
  **liveness/latency isolation** — a slow verb must not make a sibling's independent
  verb appear to hang — which is why the bound is small (2–4) and explicitly not tied
  to the vCPU count. Widening it is therefore not a performance lever, and any future
  argument that it is should be treated as a sign the real bottleneck was misdiagnosed.

**The single biggest risk, stated plainly:** the design now commits to a core
ownership refactor, a lock discipline, a re-entrant verb shape, AND an N-worker
isolate protocol on the strength of pure-logic reasoning and mock testing, before any
real host ioctl latency distribution has been measured under this architecture. If
real RM verb latencies are much worse or much *weirder* than the C's measurements
(5.4 ms allocs, µs controls), the confinement story stays *correct* but per-op latency
could disappoint, and pressure will mount to widen concurrency further (bigger pools,
dynamic scaling, per-proc executors) — each widening re-opening exactly the race
classes the C bled on. The discipline this doc asks the owner to hold us to: widen
only through the named seams (§7.2's pool bound and its dynamic-scaling gate, §6.3's
executor-sharding seam), each as its own reviewed design change — never in a
debugging session. And since #37, that discipline is no longer enforced by owner
vigilance alone: the R1/R3/R5 asserts and the §8.4 mean test fail loudly when a
widening breaks the rules. The design is not trusted; the harness is.

---

## 12. Contact log — what the L1-M1 build changed in this design

> The doc's own stance (§8.4): *"Pass = the design survived contact. Fail = the doc
> changes, not the assert."* This section is that promise being kept. Each entry is a
> place where writing the code found the design wrong, silent, or over-stated. Entries
> are appended as stages land; nothing here is a plan, all of it is a finding.
>
> **★ Where the NVIDIA facts live now.** Several entries below (§12.21, §12.26, §12.27,
> §12.33) established RM/UVM behaviour from `ogkm` and from hardware measurement. Those facts
> are consolidated, with their citations and their **driver-version caveat**, in
> `../reference/rm_semantics_measured.md` — cite that when a design needs the fact; read the
> entry here when you need the reasoning, the alternatives rejected, or the bite-check. If the
> two ever disagree, the reference file is the one to fix.

### 12.1 The `get_mut` mechanic — spine ops acquire ZERO proc locks (stage 2)

The design said a spine op runs "under the device write lock with exclusive access to
every proc, expressed as the `ProcSet` argument" (§3.4) and left the mechanics as "an
L1-M1 design-review item". The naive reading — the write-lock holder locks each proc
cell to build the `ProcSet` — would hold N rank-1 locks at once and violate R3 on the
first two-proc device.

It doesn't have to, and the resolution is prettier than the problem: under the device
**write** guard the caller holds `&mut DeviceState`, and `Mutex::get_mut(&mut self) ->
&mut T` yields `&mut Proc` **without acquiring anything** — sound precisely because
`&mut` already proves the exclusivity the lock would otherwise establish. So
`ExclusiveProcs<'_>` implements the core's `ProcSet` over `&mut BTreeMap<ProcId,
RankedMutex<Proc>>` via `get_mut`/`into_inner`, and a spine op touches every proc with
**zero lock operations and zero rank interactions**. The write lock *is* the
exclusivity; the per-proc cells are simply transparent to it.

Pinned by `spine_ops_acquire_no_proc_lock_via_get_mut`, which asserts the thread's
rank-1 acquisition counter stays flat across apply/pump/drain/poll/reap. Worth keeping:
injecting a single spurious `cell.lock()` into `apply` does **not** trip R3 (rank 0 →
rank 1 is legal, increasing order) — that test is the only thing between this design
and a silently reintroduced convoy.

### 12.2 R1/R3 are ALWAYS-ON asserts, not `debug_assert`

§3.3 twice said "debug-assert panic". The build made both unconditional. A
thread-local read costs far less than the lock acquisition it guards, and the whole
argument for these invariants is that their violation is *invisible until the unlucky
interleaving* — compiling the detector out of the build that actually runs in
production inverts the point. §2 and §3.3 have been corrected in place.

### 12.3 Sharded mode costs the core's lock-free `&Gpu` reads — a real property change

`kayfabe-core`'s concurrency contract advertises that "any number of threads may share
`&Gpu` and resolve/route/inspect in parallel, lock-free". That survives the *spine*
(routing maps, graph — device read lock, genuinely shared), but once each `Proc` lives
in a `Mutex` cell, a per-proc **read** (`resolve`, `gate_working_set`) must take that
proc's rank-1 lock. The reads are microseconds and per-proc uncontended in the common
case, so this is cheap — but it is a property the design never reconciled, and it
should not be discovered later as a surprise. Named seam if a measured read path ever
needs it back: per-proc `RwLock` instead of `Mutex` (the `ProcSet` `get_mut` mechanic
above is unaffected either way). Flagged in the `resolve` rustdoc.

### 12.4 The system proc's lock cell was unspecified

§2's picture shows "per-Proc lock (Mutex, rank 1)" over the proc *map*, but
`gpu.system` is not in that map, and `Dispatch::Observe` can legally route to it
(kernel/CeUtils os-events are a real source class). `DeviceState` therefore carries
`system: RankedMutex<Proc>` as its own rank-1 cell, and proc-cell lookup branches on
"is this the system proc". Small, but it is exactly the kind of omission that becomes
an `unwrap` in a hurry.

### 12.5 MG-6 gap: the deferred-redeliver payload carries no `GpuId`

`kayfabe-vmm`'s `CoreEventKind::CompletionRedeliver` (the §5.2 backstop) names no
target, but delivery is **per-target** since MG-6 — every `GpuTarget` has its own GSP
queue and its own drain gate. A backstop that cannot say *which* GPU to pump is
under-specified on any 2-GPU device. Stage 2 pumps `GpuId::ZERO` and surfaces the
batch; **this must become a target-carrying payload when the defer plumbing lands**
(stage 3 / L2). Recorded as a real ABI gap, not a nit.

### 12.6 ★ Honest R1 status after stage 2 — the assert does not yet guard the trait

The `BlockingSection` assert fires on *its own* construction. It does not — cannot —
guard a bare `RmBackend` call, and the core's act phases (`exec_doorbell` and friends)
still invoke the backend **under the proc lock**, exactly as the core shapes them
today. That is correct only because stage 2's backends are mocks that never block: with
a real host verb it would be a live R1 violation **with no assert firing**.

Closing it is stage 3's job and is the substance of the R1 "consequence for the core
shape" (§3.3): convert the verb-issuing act phases to plan/execute/commit, so the
locked phase *emits* a verb rather than calling one — and give R1 teeth at the trait
boundary itself, so the assert covers the thing it names instead of a wrapper someone
must remember to use. Until then, R1 is enforced for the paths that opt in, which is
not the same as enforced.

### 12.7 The observe→pump edge needs the `Vmm`, which stage 2 does not have

§5.2 describes the executor observing a completion and then pumping. Pumping opens a
drain-gated batch that only a real deliverer (GSP encode + `Vmm::raise_irq`) can close,
so running that edge without a `Vmm` would wedge the delivery plane *by design*. Stage
2's executor therefore **observes only**; the pump edge lands with the `Vmm` seam in
stage 3. §5.2 implicitly assumes the `Vmm` is reachable at every completion edge — it
is not, and the doc should not have assumed it.

### 12.8 ★ §12.6 CLOSED — R1 now guards the verb, and the counter had to move down

Stage 3's job was to make the R1 assert cover a host verb rather than a wrapper.
Two things had to be true at once, and they pull in opposite directions:

- the **assert** must fire at the `RmBackend` call itself, which lives behind the
  `kayfabe-isolate` port;
- the **counter** is maintained by the L1 guard wrappers, which live in `kayfabe-rt`,
  an adapter that `kayfabe-isolate` may not depend on.

Resolution: the per-thread held-rank mask moved to `kayfabe_util::lockwitness`, the
bottom of the dependency graph. `kayfabe-rt`'s ranked guards *maintain* it;
`kayfabe_isolate::Worker::execute` — the one door to a verb — *asserts* it. §3.3 says
"a thread-local lock-depth counter, maintained by the L1 guard wrappers, asserted
zero at every blocking-verb entry" without noticing those are two crates. They are.

The ownership half went further than the doc asked. `Isolate::rm()` is **gone**: a
backend is not reachable from an isolate by reference at all. It lives in a pool slot,
and `checkout` MOVES a `Worker` out. So a locked core phase has nothing to call — the
old shape is not merely asserted against, it no longer type-checks. Reverting the fix
therefore fails at compile time in the core, and at runtime (a named R1 panic) for
anything that reconstructs the shape by hand. Pinned by
`r1_is_asserted_at_the_host_verb_itself_not_at_a_wrapper`.

`BlockingSection` survives with a smaller job: the pool-full condvar wait, and any
future non-verb blocking thing. It is no longer the teeth for verbs.

### 12.9 ★ The R5 gap turns first-touch materialization into a compare-and-swap

**The doc's R5 is only half the rule, and the missing half is the dangerous one.**
§3.3 says a commit that finds its target gone "surfaces a refusal — it does not
finish what it started". Written that way, every staleness is a refusal. Building it
that way immediately broke the existing multi-thread smoke test, and the reason is
worth stating precisely:

Lazy materialization (host VAS, host channel, engine object) reads "is it there?"
under the plan lock and writes "here it is" under the commit lock, with a lock-free
verb in between. That is a **compare-and-swap**, and two sibling threads of ONE proc
racing it is the *ordinary* case for a multi-threaded guest process — precisely the
workload §3.5 exists to serve. Refusing the loser turns a legal concurrent
submission into a spurious guest-visible fault: a worse bug than the use-after-retire
R5 was written to prevent, and one that only appears under concurrency.

So `Refusal` carries a `retry` flag, and staleness has two shapes:

| shape | example | resolution |
|---|---|---|
| **converging** | a sibling materialized the same host VAS / channel / engine object first | release the duplicate, **re-plan from the top** against the winner's state |
| **divergent** | proc retired, channel torn down, route rewritten, target gone | **refuse loudly** — MISS = FAULT |

The retry is bounded (`MAX_COMMIT_RETRIES`), because each pass observes a strictly
more materialized world: one pass is the expected worst case, and the bound exists so
a bug cannot turn a race into a spin. §3.3's R5 text should read "re-resolve **or**
refuse" — the doc's own §8.4 canary wording already said "re-resolves or refuses"
while R5's normative text said only refuse. The canary wording was right.

Found by: `threads_smoke_hammers_both_lock_modes_bounded` hanging (a worker thread
panicked on a `Stale::Rebound` its `expect` did not tolerate, poisoning the device
lock and leaving the executor thread spinning). Note the *shape* of that failure —
the first symptom of getting this wrong was a hang, not an assertion.

### 12.10 R5 applies to the FAILURE path, not just the reply path

`Proc::retire` retires the isolate, so a verb held in flight across a retire returns
an **RM refusal** rather than reaching the commit at all. Loud and mutation-free — the
invariant holds — but the fault the guest sees says `Rm(Other)`, i.e. "the host
failed", when the truth is "your process was torn down". A canary that only asserted
"it refused" would pass for the wrong reason.

The driver therefore re-validates on the verb-error path too: if the proc is no longer
live, the surfaced fault is `Stale::Proc`. §3.3 frames re-validation entirely around
"applying the reply"; it applies to *not* applying one as well.

### 12.11 Two smaller contacts

- **The §7.2 pool forces `RmBackend: Sync`.** The crate carried a documented
  `Send`-only exception ("reachable exclusively through `&mut`"). That was sound only
  while no `Box<dyn RmBackend>` was ever *stored* in core state. A pool stores N of
  them inside a `Proc` inside the `Sync` `Gpu`, so the bound is now structural. Cost to
  real impls: no `Rc`/`Cell` in a backend's private state.
- **An out-of-band retire does not rebuild the routing maps.** *(RESOLVED by §12.13 —
  see the ANSWER at the end of that entry.)* `Spine::retire_proc`
  (the worker-death path) removes the proc from the live set but leaves `by_pdb` /
  `by_vchid` naming it, because the *guest* has not freed anything — only a graph
  `refresh` rebuilds those. So a post-retire op resolves its route and then misses on
  the live-set lookup: `RetiredProc`, not `UnknownVchid`. Loud and mutation-free
  either way, but the fault surfaces one step later than on the graph-driven path, and
  the two teardown routes are therefore not fault-identical. Recorded rather than
  papered over; unifying them means deciding whether a host-side failure should
  retroactively edit the guest's routing truth, which is a design question, not a fix.
- **Cost of the split, measured in locks.** A verb-issuing op now takes each rank
  **twice** (plan + checkout; commit + check-in), where the stage-2 in-lock verb took
  it once. That number is pinned by `spine_ops_acquire_no_proc_lock_via_get_mut` —
  collapsing it back to one is exactly the regression R1 exists to prevent, so the
  test asserts the count, not just the absence of a panic.

### 12.12 The doorbell hot path DOES need the split (checked, not assumed)

The staging plan for stage 3 flagged `exec_doorbell` as the site that "per §3.5 should
not need to block at all", and asked for the answer either way. The answer is **it
needs the split**, for a reason the doc's framing obscures: §7.1 puts the isolate in a
*separate sandboxed process*, so `RmBackend::ring_doorbell` is not an MMIO store from
the QEMU process — it is an IPC round trip to the worker that owns the mapped doorbell
page. A steady-state doorbell therefore still issues exactly one host verb, and
holding the proc lock across it would be a live R1 violation on the hottest path
there is.

What IS true, and worth keeping: the steady-state plan is a single-verb chain
(`schedule = false`, channel already materialized), its commit only records
`poll.last_token`, and — because the plan phase resolves everything from core state —
a re-send that needs no host work at all (the idempotent engine-object replay) emits
`verbs: None` and **never touches the pool**. So the cost of the split on the hot path
is two µs-scale locked phases around one round trip, not a second round trip.

Also checked, per site, and this is the claim the whole "typed verb chain instead of a
resumable continuation" simplification rests on: **no site needs to consult core state
between two verbs.** `ensure_host_vas → alloc_sysmem → map_gpu_va`,
`alloc_vaspace → alloc_channel → schedule → ring`, and
`alloc_vaspace → alloc_channel → alloc_engine_object` are each purely
verb-output-to-verb-input; every core-derived value (engine kind, class, params, len,
the already-materialized handles) is known at plan time. The chain executes with zero
core access, which is what makes the seam a short typed struct rather than a
continuation machine.

### 12.13 ★★ THE MEAN TEST'S FINDING — an out-of-band retire is undone by the next refresh (**FIXED**)

**Stage 4's mean test (§8.4) found the design's "no resurrect" promise was not
implemented.** §7.3 says worker death out of band ends in "retire the proc loudly …
either way MISS=FAULT posture, **no resurrect**", and the `SignalOutcome::WorkerDied`
contract says the slot is "permanently dead (**never a respawn**)". Both held for
exactly as long as nothing else happened. The finding stands as written below; the
**FIXED** subsection at the end records the mechanism, the identity key it turns on,
and the answer it gives §12.11.

*(Both promises were originally justified by "a worker that died mid-verb may have left
host state the core cannot reason about". That justification is wrong and has been
replaced — §7.3, and "Why never a resurrect" below. The **rule** is unchanged; only the
reason it rests on, which matters because a rule with a bad reason gets optimised away.)*

`Spine::retire_proc` removes the proc from the live set and pushes it to `retired`. But
the *guest* has not freed anything — its client root is still in the RmGraph — so the
**next `Gpu::apply` of any event, from any client**, runs `refresh`, which matches
boundaries to live procs by client intersection, finds no match for that component, and
takes the `None` arm: a fresh `ProcId` from the monotonic mint, a fresh `Proc`, a
**newly spawned isolate** (new sandbox, new handle namespace — the respawn §7.3
forbids), a fresh GPA arena, and `by_pdb`/`by_vchid` rebuilt onto it. Measured, not
argued: after a HUP the guest's very next publish and doorbell both succeed, on host
handles minted in a brand-new isolate lane. Only the dead *worker slot* stayed dead;
the isolate came back around it.

Two consequences worth stating separately:

- **Correctness, and only then security.** A guest that can crash its isolate worker
  gets a clean new isolate on its next RM event — and, worse, gets it **silently**. The
  respawned isolate's `alloc_sysmem` backings are fresh and **zeroed**, so the guest
  resumes reading a VA it believes still holds its data and finds nothing there, with no
  fault anywhere. Data corruption first; the "loud retire" the design counts on lasting
  microseconds is the second-order problem.
- **Why the existing suite missed it.** `l1_verb_seam.rs`'s
  `worker_death_retires_the_proc_loudly_and_never_resurrects` is green because it never
  issues an `apply` after the HUP. The composed run found it because §8.4's script puts
  a worker HUP and an alloc/map-heavy `apply` workload *in the same run* — which is the
  argument for composing the mean scripts instead of isolating them, in one example.
  (That hole is now closed on both ends: the verb-seam test issues an unrelated
  `apply` after the HUP and re-asserts, so it would have caught this on its own.)

#### ★★ FIXED — the condemned component

The mechanism is the one this entry named, built as described. `Spine` carries a set of
**condemned components**, and `Spine::retire_proc` — the ONE out-of-band retire path,
which is why condemnation lives there and not in the adapter — records one on every
call, in addition to its existing three obligations.

**The identity key is the component's CLIENT SET, and that choice is load-bearing.**
Three candidates were on the table and two of them are wrong:

- `ProcId` — minted per derivation and dead with the proc. The *whole defect* was that
  the next derivation minted a fresh one. It cannot be the key of the thing that must
  survive re-derivation.
- `ProcAnchor` — only the *smallest* client handle of the component. A guest that frees
  that one client while keeping the rest silently re-labels the component and slips the
  condemnation. It is a fine thing to *report*, and a bad thing to key on.
- **Client set** — exactly what `refresh` already matches boundaries on (intersection),
  so condemnation and proc-matching agree by construction and survive every
  re-derivation the guest can provoke: re-labels, growth, and **splits** (freeing the
  dup edge splits the component; both halves intersect, so both stay condemned — they
  shared the blast radius).

Entries are canonical (pairwise disjoint, sorted — determinism, decision #27) and
**monotone**: a boundary that intersects an entry grows it with its own clients, so a
component that absorbs new clients keeps the radius it earned; the escape hatch of
"dup a fresh client in, then free the old one" is closed. An entry is dropped when NO
boundary intersects it — i.e. exactly when the **guest itself** freed the client root.

`refresh` runs the condemnation pass *before* it can mint anything. A condemned boundary
gets **no `Proc`, no isolate spawn, no GPA arena, no live routing entry** — it simply
`continue`s. Nothing else in the derivation changed, which is the point: the
graph-driven retire (step 3) and the `LateMerge` absorb arm are untouched and condemn
nothing, so no host failure of one process can DoS another, and no guest teardown can
DoS itself. `the_graph_driven_retire_paths_never_condemn` pins both.

**What a growing component does — the `DUP_OBJECT` question, answered.** Dup-connected
clients are ONE proc by construction: one isolate, one arena, one blast radius. So a dup
that joins a live proc to a condemned component has no honest completion — absorbing the
condemned clients resurrects them around a working isolate, and condemning the live proc
lets a guest kill a healthy process by dupping into a corpse. The answer is to refuse the
**event**: `GpuError::CondemnedMerge`. `Spine::apply` is transactional, so the offending
dup rolls back atomically, the live proc keeps serving and the condemned component stays
condemned. (A dup between a condemned component and a *brand-new* client — no live proc
on either side — is not a merge of blast radii; the new client simply joins the dead
component and is condemned with it. That is the monotone-growth rule above.)

**(b) What an op against a condemned component returns: `FwdFault::Condemned { anchor }`
— a real named fault, with no reverse lookup.** The entry was right that
`RetiredProc(ProcId)` is unrepresentable here. It is also unavailable *by policy*: the
only way to produce it would be to leave routing pointing at a dead `ProcId`, and the
mean run's own conservation sweep forbids that ("routing names a proc that is not
live"). Routing therefore misses — MISS=FAULT working as designed — and the miss is
*named* by two extra maps, `condemned_by_pdb` / `condemned_by_vchid`, rebuilt in
`refresh` step 4 from the **same projection** that fills `by_pdb`/`by_vchid`, keyed the
same way (`(GpuId, Pdb)` / `(GpuId, VChid)` — MG-3), and filled from the boundary that
was condemned. The guest's own key is looked up **forward**; it just resolves to
"condemned" instead of to a proc. No backwards resolve was invented to make a prettier
error, and the `RmGraph::gpu_of` / address-table doctrine is untouched. `Spine` also
exposes `is_condemned(HClient)` and `condemned_len()` for diagnostics and for the tests
to assert the *state* rather than infer it from refusals.

**★ ANSWER to §12.11 ("should a host-side failure retroactively edit the guest's
routing truth?"). No — and it does not.** The guest's truth is the RM graph and its
projection, and neither is touched by a worker death: the boundary still exists, still
has its clients, still declares its PDB and vChids. What a host-side failure edits is
the host-side **materialization** — whether that boundary gets a proc, an isolate, an
arena and a live route. `retire_proc` moves its routing into the condemned maps *at the
instant of the failure* rather than at the next unrelated event, which is what makes the
two teardown routes fault-identical: the same op answers `Condemned` immediately after
the HUP and after any number of later refreshes. §12.11's asymmetry is gone, and the
answer needed no edit to the guest's truth to get there.

**Residual, stated plainly.** `FwdFault::Condemned` names the component's *current*
anchor, so if a condemned component grows a new client with a smaller handle the
reported label moves with the component (it is a derived label, by definition). And a
condemned component's arena is released exactly once at the reap and recycled by the
normal #80 free-list — a genuinely NEW guest process can and should be handed that
range; what must never happen (and is pinned) is a re-derivation of the *condemned*
component receiving one.

Pinned where it was found: `l1_mean.rs`'s
`out_of_band_retire_must_not_resurrect_the_isolate` is the design's own invariant, no
longer `#[ignore]`d (its two post-refresh expectations moved from `RetiredProc(victim)`
to `Condemned{anchor}` for the reason above — a strengthening, documented at the test);
the mean run's conservation sweep now pins the FIXED behavior and the absence of false
condemnation; and six focused tests pin the properties the fix introduces (survival
across intervening applies, clearing on client-root free, no dispatchable sources,
arena-released-once, the graph-driven paths never condemning, component-wide
condemnation across a proc's GPUs sparing identical numeric ids on other targets, and
the refused merge).

#### ★★ Why never a resurrect — the reason, corrected

The mechanism above is right. The reason originally given for it was not, and a rule
whose stated reason does not survive scrutiny gets optimised away by whoever reads it
next. The reason was *"a worker that died mid-verb may have left host state the core
cannot reason about."* That is weak: the isolate is a **process**, so the host kernel
reasons about it for us the moment it dies — fds close, RM tears down its client objects
and everything they own, its mmaps go. Clearing our stale bindings (`host_channel`,
`host_engine_objects`, `host_vas`, each `Binding.host`) and letting the existing lazy
first-touch paths rebuild would be *almost* clean, which is exactly why someone would
try it.

**The real reason: the guest's DATA died with the isolate.** `publish_backing` allocates
host memory through `RmBackend::alloc_sysmem`, owned by that isolate's RM client, and the
host kernel frees it with the process. Re-materialising therefore hands the guest a
fresh, **zeroed** backing for a VA it believes still holds its data — not recovery,
**silent data corruption**, and strictly worse than the resurrection bug it would be
"fixing", which at least failed visibly at the next real operation. The correct posture is
the one real hardware already has: a GPU that faults a channel does not silently hand back
a fresh context, it makes the context **sticky-fatal** until the application tears it down
and builds a new one. Every CUDA application already handles that, because Xids exist.

★ **What makes "sticky-fatal" not "bricked" is the identity key.** Condemnation is keyed
on the **client set**, so a re-initialising application — a fresh CUDA context allocates a
new RM client — derives a boundary with no dup edge to the condemned set, is a *different*
component, is not condemned, and is simply served. And a process that dies instead needs
no cooperation at all: the guest kernel frees its clients on its behalf and the entry
clears. So the client-set choice is not only what makes condemnation *survive*
re-derivation (the argument above); it is also, and by the same property, what makes
**recovery** possible. Both readings of the key are now executable — §12.17.

**Carve-out, stated per backing class.** Where a backing is **guest RAM** double-mmapped
into the isolate rather than host-allocated, the data lives in *guest* RAM, survives the
isolate's death, and transparent re-materialisation would be legitimate. No backing class
in the core is of that kind today; `Vmm::export_ram` is the port one would be built on and
nothing in the core calls it (§12.17 records the check). Any future path claiming the
exemption must say so explicitly and prove its backing class — never by analogy.

### 12.14 Two smaller contacts from stage 4

- **A held latch must be released on unwind, or a failed mean assert reads as a hang.**
  The first composed run panicked inside the window; the panic was real and correct, but
  the scoped threads parked in the mock backend were never released, so `thread::scope`'s
  join-all waited forever and the failure *presented* as a wedge with its message
  swallowed. The latches are now a drop guard (`Latches`), so an unwind releases every
  one of them first. This is the same species as §12.9's lesson — "note the *shape* of
  that failure: the first symptom of getting this wrong was a hang, not an assertion" —
  and it applies to the harness as much as to the design.
- **The progress-under-pending assertion has teeth, verified by falsification.**
  Shrinking the isolate pool to one worker (decision #34's original shape) makes the mean
  test SIGABRT on its watchdog instead of passing slowly, and removing any one of the
  three staleness mutations makes its canary fail. Both were run. The assertion is
  structural, so neither result depends on how fast the box is.

### 12.15 §8.2's "race ceiling" has now actually been RUN — and it is a standing gate

§8.2 named TSan the T2 race ceiling; until 2026-07-25 it had never been executed
against the built L1. First run: **28 tests across the 4 threaded targets
(`concurrency_stress`, `rt_shell`, `l1_verb_seam`, `l1_mean`), 0 races, exit 0**, via

```text
KAYFABE_STRESS_WATCHDOG_SECS=1800 RUSTFLAGS="-Zsanitizer=thread" \
  cargo +nightly test -p kayfabe-tests \
  --test concurrency_stress --test rt_shell --test l1_verb_seam --test l1_mean \
  -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1
```

Measured inflation ~20× (`concurrency_stress` 290 s, `rt_shell` 69 s, `l1_mean` 20 s)
— which is what the `KAYFABE_STRESS_WATCHDOG_SECS` override exists for: the suites'
wedge-watchdogs must measure wedging, not instrumentation tax. And per the standing
lesson (a gate a human remembers is not a gate), this run is now the nightly `tsan`
job in `.github/workflows/ci.yml`, which adds `KAYFABE_SLOW=1` so the gated
16-thread stress soak is always part of the ceiling.

### 12.16 ★★ THE TEARDOWN AUDIT — four gaps that made leak-free reclamation a retrofit (**FIXED**)

A read-only audit of the core's teardown/reclamation completeness, run before L1-M2
designs the OS shell, found ten gaps. Four of them sit in signatures that every
reclamation call site must route through, so deferring them would have made leak-free
teardown exactly the retrofit this project refuses. They are recorded together because
they are one finding wearing four hats: **the core could enter every teardown state and
leave almost none of them.**

The owner's bar for the fix, verbatim: *"clean cleanup on gpu getting idle, restart
driver, process killed, isolate can be gc collected, etc etc. no leaks, safe."* So each
gap landed with a test that asserts the reclamation **happens**, not that the API now
permits it — `tests/teardown_reclaim.rs`, plus an acquire/release ledger
(`kayfabe_mocks::HostLedger`) replayed from the mock's own verb log, which is the
invariant that generalises all four: *every host object acquired is released exactly
once, every mapping unmapped exactly once, nothing released that was never acquired.*

#### G1 — a published backing's host memory handle existed nowhere in core state

`commit_publish` received `VerbReply::Published { host_vas, memory, host_va }` and, on
the **success** path, stored only `host_va`. `memory` appeared solely inside the
*refusal* path's orphan closure. So after a successful publish — the ordinary case, and
the majority of allocated host bytes — the `HostHandle` of the sysmem object was
unrecoverable. A reclaim could `unmap_gpu_va(vas, host_va)` and could never `free(memory)`.

The fix is a type, not a field: `Binding.host: Option<HostBacking>` where
`HostBacking { memory, host_va }`. Two separate `Option`s would have left *"mapped
somewhere, owning nothing freeable"* representable, and that state **was the defect**.
One `Option` over the pair makes bound-but-unfreeable untypeable — the house
`GpaSpace::release(arena)`-by-value pattern applied to the address plane. Cost: an
`kayfabe-mmu` → `kayfabe-isolate` crate edge (the address plane now names the handle
type), and `Binding::host_va` became an accessor at ~20 call sites.

Order in the release chain is unmap-then-free, and that is RM's rule rather than our
preference: RM frees children and dependents ahead of parents (`ogkm:
src/nvidia/src/libraries/resserv/src/rs_server.c:963-981`, `.../rs_client.c:1086-1122`)
and auto-unmaps a resource's inter-mappings inside `clientFreeResource_IMPL` *before*
`objDelete` (`.../rs_client.c:830-849`). So the ordering does not protect RM — it keeps
**our** mirror of the mapping honest, which is exactly what `HostBacking` is.

#### G3b — `reap_retired`'s signature made the correct reap unwritable

`Spine::reap_retired(&mut self) -> usize` dropped every retired `Proc` **in place**, and
`SharedDevice::reap_retired` called it under the device write guard. A real `Isolate`'s
`Drop` is `waitpid` + namespace teardown + fd close: **a blocking syscall under a rank-0
lock**, with no assert anywhere, because `Worker::execute`'s `assert_lock_free` guards
verbs and cannot see a drop. That is §12.6's shape verbatim — *an assert guarding a
wrapper rather than the thing* — one layer over, and stage 3 had already paid for it once.

Two changes, and the second is the one that matters:

- `reap_retired` returns `Reclaimed`, an opaque `#[must_use]` carrier of the corpses.
  The shell binds it *outside* the guard and lets it fall there. There is no accessor
  handing out a `&Proc`: a reaped proc is not a thing to consult.
- ★ **The drop is now a door with an assert on it.** `IsolateBox` is the only way core
  state owns an `Isolate`, and its `Drop` calls `assert_lock_free` exactly as a verb
  does. This had to be a newtype: `Drop` cannot be implemented on `dyn Isolate`, and an
  adapter's own `Drop` cannot be relied on to exist — the mock has none, and the mock is
  what the core is tested against. Re-introducing the old shape now **panics naming
  R1** (measured; the bite-check output is the R1 message with `rank(s) [0]`), where
  before it blocked silently.

  Honest limit, stated rather than hidden: the assert is skipped while the thread is
  already panicking, because a panic in `Drop` during an unwind aborts and would replace
  a real failure's message with a bare abort. So an isolate dropped under a lock *on an
  unwinding path* is not caught. Every non-unwinding drop is.

#### G3 — "quiesce" was never defined and never checked

`reap_retired` dropped every retired proc unconditionally. Meanwhile `verb_op` checks a
worker out, releases every lock, and runs the chain with a `Box<dyn RmBackend>` live on a
foreign thread's stack; the executor may legally run the reap in that gap. So the isolate
could be torn down while a live connection into it was outstanding, and the op's own
orphan disposal would then run `release_plan` against a sandbox that was gone. **A
deferred reap that runs too early is a use-after-free; too late is a leak.** The
information already existed (`pool_size` / `idle_workers`) and the core never consulted it.

`Isolate::in_flight()` + `is_quiesced()` now define it precisely: *no worker of this
isolate is checked out.* `reap_retired` **returns a non-quiesced proc to the retired
list** instead of dropping it — the core checks rather than trusts.

Two things the build settled that the brief left open:

- **`in_flight` must be asked for, never derived.** `pool_size() - idle_workers()` looks
  like the same number and is not: a slot that died out of band is neither idle nor
  checked out and can never become either (§7.3, "no resurrect"), so the subtraction
  reports a lost worker as a live round trip **forever** — an isolate that never
  quiesces, a proc that never reaps, an arena that never recycles. The implementation
  knows which slots are `Dead`; the core does not and must not have to.
- **★ This is not "the device is quiescent", and the doc must not let the two blur.**
  The device-level quiesce point is a *protocol event the guest sends*: fn=47
  `UNLOADING_GUEST_DRIVER`, emitted on **both** a real driver unload and a GPU-idle
  release when the last context exits (`C: src/qemu/nvkvm_gpu_emul.c:2450-2462`), with
  the reap running at the **re-handshake** that follows — which the C names in so many
  words: *"the re-handshake = the quiesced point (GPU was idle-released; next context
  boots). Purge dead-client resolution/backing state now — never at the free."*
  (`C: src/qemu/nvkvm_gpu_emul.c:3458-3461`, the #14 P0 fix; reaping at the client-root
  free instead hung the dying context's residual polls — lesson L10). That trigger is
  the adapter's and L1-M2's. `is_quiesced()` answers the *other* question — is
  attempting it safe for **this** `(Proc, GpuId)` right now — because the adapter's edge
  is device-wide while the hazard is per-isolate: a guest process can have a verb in
  flight across another process's idle-release.

#### ★ The gap the audit missed: G3's check creates a permanent-leak hazard, and closing it was mandatory

The audit did not see this and it is the most interesting thing the build found.
`SharedDevice::return_worker` resolved the proc through the **live** map and dropped the
handle on a miss, with a comment saying the retire path owned that disposition. It owns
the host *objects*; it does not own the *pool slot*, which stayed marked checked-out with
nobody holding it. Harmless while the reap trusted the caller — **fatal the moment it
checks**: the isolate never reports itself quiesced, the proc is deferred at every
quiesce point for the life of the device, and its GPA arena never returns to the window
(#80, exactly the leak the reap exists to prevent). A leak is not an acceptable price for
closing a use-after-free.

Found by §8.4's mean test, which went from green to `reaped (1, 0), expected (2, 0)` on
the commit that added the check — the proc whose verb was in flight when a teardown
retired it. Fixed by `Spine::checkin_retired`: **a retired proc still accepts worker
returns.** It accepts nothing else — it refuses new checkouts (§5.4), it is in no routing
map, and no op can reach it.

Worth noting where this lands historically: the C had no interlock here at all. Its
session reaper *argued* rather than checked — `C: src/qemu/virtio_nvgpu.c:113-118`, "a
pooled IOCTL worker may still be unwinding after `nvkvm_isolate_kill` (which joins the
isolate's reader thread, **not** the pool workers) … so freeing the session struct here
cannot UAF it." That is an argument about what the worker touches, not a guarantee that
it is done. `in_flight()` plus this return path is that missing interlock.

#### G4 — no cancellation vocabulary, and a mid-chain failure's orphans were unrecoverable

**The vocabulary half.** `RmError` had no `Interrupted`, despite §5.4 specifying an
interrupted reply on the wire. So a cancelled verb arrived as `RmError::Other(n)`, and
`verb_op`'s failure-path re-validation resolved it to `FwdFault::Rm(e)` **whenever the
proc was still live** — which is the *normal* cancellation case: a guest thread dies, its
process runs on. That is §12.10's wrong-reason conflation one layer over, and it would
have made every cancellation canary pass while reporting "the host refused" about a host
that did exactly what it was asked. `RmError::Interrupted` + `FwdFault::Cancelled { proc }`,
with the `Interrupted` arm tested **first** on the failure path, before proc-liveness.

Shape taken from the C's #73, not invented: the stub installs a SIGUSR1 handler *without*
`SA_RESTART` so a blocked `ioctl()` returns `-EINTR` (`C: src/stub/nvkvm_stub.c:699-708`,
`:2669-2678`); the interrupt arrives out of band as a **command**
(`ISOLATE_CMD_INTERRUPT`, `C: src/common/nvkvm_isolate_proto.h:53,122-131`) and the
worker answers **on the ordinary reply path** with `retval = -EINTR`. There is no separate
"interrupted" reply message in the C's protocol — which is why this is an `RmError`
variant and not a new `VerbReply`. And the worker *survives* it (`C:
src/stub/nvkvm_stub.c:1276-1281`), which is what distinguishes cancellation from worker
death (§7.3) and what makes the unwind meaningful in the cancelled case.

Cancellation slots into §12.9's table as a **third shape**, not a redesign of `Refusal`:

| shape | example | resolution |
|---|---|---|
| **converging** | a sibling materialized the same host VAS / channel / engine object first | release the duplicate, re-plan from the top |
| **divergent** | proc retired, channel torn down, route rewritten, target gone | refuse loudly — MISS = FAULT |
| **★ cancelled** | the requester interrupted its own in-flight verb (§5.4) | **non-retryable, orphan-carrying** — surface `Cancelled`, dispose of the residue |

**The mid-chain half.** `Worker::execute` promised all-or-nothing by unwinding internally
and returned a bare `Err(RmError)`; the unwind's own `free`s were `let _ = …`, as was
`VerbPlan::Release`'s whole body. So a chain that failed *and could not clean up* left
host objects in no `Orphans`, in no core state, enumerable from nothing. `execute` now
returns `Result<VerbReply, VerbFailure>` with `VerbFailure { err, orphans }`, and both
best-effort paths **record** what they could not dispose of instead of swallowing it.
`Orphans` moved down to `kayfabe-isolate` (the worker cannot depend on `kayfabe-fwd`) and
is re-exported. `Refusal` and `Orphans` are now `#[must_use]` — dropping either silently
leaks host objects, and the compiler is the only thing that reliably notices; verified by
falsification (all three sites warn, and warnings are `-D`).

★ **A named unknown, deliberately not reasoned about.** `VerbFailure::orphans` enumerates
every object **whose handle this execution received**. It cannot enumerate an object the
host may have created for a verb whose reply never arrived — an interrupted alloc. **The C
never settled this and has no reconciliation code**: its bookkeeping is gated on
`ret == 0 && nvstatus == 0` (`C: src/qemu/nvkvm_isolate_handlers.c:1444-1445`, `:1497-1501`),
its guest discards the reply entirely on the interrupt path
(`C: src/guest/nvkvm_virtio.c:461-471`), and most RM waits are not interruptible in the
first place (`ogkm: kernel-open/nvidia/nv.c` carries only a handful of `*_interruptible`
waits) — so a cancelled alloc plausibly *completed*. **OPEN QUESTION requiring a bench
experiment: does an interrupted `NV_ESC_RM_ALLOC` leave the object created, partially
created, or absent?** Until that is measured, isolate-session death stays the backstop
disposition and no per-object completeness may be claimed.

#### Also corrected: three doc claims the audit found false

- **`Orphans`' own rustdoc** said the only case with no releasing caller is a vanished
  proc, "then the whole isolate is retired and its handle namespace dies with it … Both
  dispositions are decided, neither is a leak." Both halves were wrong. The namespace
  dies at the **reap**, not at `retire()` — and since G3, only once quiesced — so between
  those moments the objects are *held*, which is a deferred disposition and a different
  thing from what the sentence claimed. And there is an unnamed **third** disposition: a
  worker that dies mid-chain, where nothing unwinds and the reply never returns.
- **`core_state_and_consolidation.md` §4**, "Eager host-side reclaim … Today reclaim =
  isolate-session teardown; fine for correctness, wired for footprint later." Neither
  clause held. Reclaim was not session teardown, it was *nothing* — G1 meant no path
  could name the object. And it was therefore not fine for correctness. Rewritten in place.
- **`gpa.rs`'s "safe by construction"** conflated a real construction with a claim about
  an adapter `Drop` the core neither performs nor checks. Now split three ways in the
  rustdoc: by construction (the arena is unreachable from any live proc), **checked**
  since G3 (the owning isolate is quiesced before the range recycles), and an explicit
  **adapter obligation** (that `Drop` really does tear the session down).

#### What remains — explicitly L1-M2's, and now an addition rather than a retrofit

The **reclamation policy** and its ledger: when to run a reclaim, and where undisposed
residue is recorded across a proc's lifetime. `kayfabe_fwd::dispose_on` returns that
residue as a `#[must_use]` value at both call sites precisely so the sink can be added
without reshaping anything. G2 (`refresh`'s silent drop of live host state) and G5
(device reset) were left untouched by design; nothing here forecloses the C's measured
two-phase reset ordering (`C: src/qemu/nvkvm_gpu_emul.c:2450-2478`, `:3462-3487` — reset
boot-gating state at fn-47, write position at the re-handshake, **preserve the seqNums**,
or it is an Xid 119 / the #12 hang).

Pinned by `tests/teardown_reclaim.rs` (14 tests), every one of which was bite-checked by
reverting its fix and confirming the failure named the right thing — including the one
that matters most, where re-introducing the in-guard reap produces the R1 panic verbatim
rather than a silent block.

### 12.17 ★★ The "no resurrect" JUSTIFICATION was wrong, and the RECOVERY half had never been tested

Two corrections to §12.13, neither of which changes the mechanism: the condemned
component is **exactly as built** (commit `1719dd8`). What changed is the reason it rests
on, and what the suite actually proves.

**(a) The justification.** Every site that justified "never a resurrect" said *"a worker
that died mid-verb may have left host state the core cannot reason about."* That does not
survive scrutiny — the isolate is a **process**, so when it dies the host kernel reasons
about it for us (fds close, RM tears down its client objects and everything they own, its
mmaps go), and clearing our stale bindings (`host_channel`, `host_engine_objects`,
`host_vas`, each `Binding.host`) to re-materialise through the existing lazy first-touch
paths would be *almost* clean. A rule defended by an argument that weak gets optimised
away by the next reader. The real reason is much stronger and is now stated at §7.3,
§12.13 ("Why never a resurrect — the reason, corrected"), `SignalOutcome::WorkerDied`,
`FwdFault::Condemned`, `Spine::condemned` / `absorb_condemned` / `retire_proc`,
`Isolate::worker_died` and the two tests that quote the contract: **the guest's DATA died
with the isolate** (`publish_backing` → `RmBackend::alloc_sysmem`, owned by that isolate's
RM client), so re-materialising serves a **zeroed** backing for a VA the guest believes
still holds its data — silent corruption, strictly worse than the resurrect it would be
fixing. The refusal is instead the semantic real hardware has: **sticky-fatal**, like an
Xid. `l1_os_shell.md` T5's `SIGKILL`-the-isolate argument was re-derived on its own terms
(unenumerable mid-chain allocations, G4) rather than left citing the retired phrase.

**Backing classes, checked rather than assumed** (the carve-out must never be granted by
analogy). Exactly one path gives a `Binding` a `host`: `VerbPlan::Publish` →
`RmBackend::alloc_sysmem` + map, committed in `commit_publish`. That is **host** memory,
owned by the isolate's RM client, so it is on the corrupting side of the line — and note
that `Binding::phys` is a GPA carved from the proc's own arena (a synthetic window), not
guest RAM, so the GPA is no evidence of survivability either. Bindings with `host: None`
(declared by the RPC `MapMemoryDma` source or the CE-PT-write capture feed) hold no host
state at all, but they are not an exemption either: publishing one later goes through the
same `alloc_sysmem`. Host VASes, channels and engine objects are pure host-side
materialization and would be re-derivable — which is precisely why the sysmem argument,
not the "unreasonable state" one, is the load-bearing half. **No guest-RAM backing class
exists today**: `Vmm::export_ram` is the port one would be built on (Mode-1's double-mmap
share) and nothing in the core calls it. `l1_os_shell.md` T6 already schedules "tear down
every double-mmap of guest RAM into it", so the class is *planned*; when it lands it must
claim the exemption explicitly and name its backing class, per §7.3.

**(b) ★ The recovery half is now tested, and it WORKS.** The suite pinned that
condemnation *sticks*; that an application can *come back* — the half a user actually
experiences, and the load-bearing claim of the corrected justification — was an assumed
claim. Five tests in `l1_mean.rs` pin it, all green on their first run (no adjustment was
made to any of them):

- `a_fresh_client_recovers_from_its_condemned_predecessor` — ★ the headline. A fresh RM
  client (no dup edge to the condemned set) derives a different component, gets a live
  `Proc`, a real isolate and its own GPA arena, and publishes + rings end to end through
  the #14 ring-gate, with its host token in its **own** isolate lane — while the condemned
  key still answers the EXACT `FwdFault::Condemned { anchor }`.
- `no_amount_of_recovery_clears_the_condemned_entry` — eight successive recoveries; the
  condemned key answers the same exact fault after every one, `condemned_len()` stays 1,
  and no `Proc` ever holds the condemned client.
- `process_death_clears_the_condemnation_with_no_application_cooperation` — the guest
  kernel's client-root free (measured on real GA106, 2026-07-25: a killed guest process
  emits **178 `fn=10` RM FREE RPCs** then fn-47, host stub still alive) clears the entry,
  after which the **same** client handle and the **same** PDB are served again. Distinct
  from the first test: there the entry remains and a new identity is served alongside it.
- `a_recovered_component_shares_no_arena_or_host_handle_with_the_condemned_one` — a
  disjoint GPA arena while the corpse still holds its range, not one host handle in
  common, every recovered handle in its own `(proc, GPU)` lane, and none of it disturbed
  by the corpse's reap. (After the reap the range itself may legitimately recycle — that
  is the #80 free-list, and it goes to a genuinely new process, which
  `a_condemned_components_arena_is_released_once_and_never_recycled_to_an_impostor`
  already pins.)
- `recovery_after_a_multi_gpu_condemnation_serves_both_targets` — a component spanning
  GPU0+GPU1 killed through its **GPU1** worker; the replacement likewise spans both
  targets (two fresh clients joined by a dup) and is served on both, the corpse stays
  condemned on both of ITS planes, and the four bystanders on byte-identical
  `Pdb`/`VChid` values are untouched by the death *and* by the recovery.

Every one bite-checked, house standard, by reverting the mechanism and confirming the
failure named the right thing: condemning any brand-new boundary while a corpse exists
(the "bricked" over-correction) fires *"the fresh client was condemned by association —
the identity key is wrong and §12.13's justification does not hold"*; dropping the
carry-forward (`self.condemned = carried`) makes the condemned component answer
`Ok(Published { .. })` where `Err(Condemned { .. })` is required — §12.13's original
defect, verbatim; making condemnation permanent fires *"the condemnation outlived the
process that earned it — an application that cannot even recover by DYING is bricked, not
sticky-fatal"*; releasing the arena at `retire_proc` instead of at the reap puts the
recovery inside the corpse's range; and collapsing the mock's per-isolate handle
namespace fires the cross-namespace assertion by handle value.

**What this settles.** The corrected justification is not merely more defensible, it is
*checked*: "sticky-fatal, not bricked" is now a property of the client-set key with tests
behind it, so the argument and the mechanism stand or fall together. Nothing was found
that contradicts it.

### 12.18 ★★ `Spine::apply` was NOT atomic — its rollback restored one field of seven

`Spine::apply` snapshots `self.rmgraph` and, on any derivation fault, restores it and
re-derives. Its own doc said the consequence out loud: *"the offending event is refused
atomically and no other `Proc`'s state is disturbed. A hostile stream can only ever earn
its own loud refusal."* That was false. `refresh` — the thing being rolled back — also
retires and **removes** `Proc`s, deregisters their completion sources, pushes them onto
`retired`, advances `next_proc`, mints `targets`, moves `geom.next_base` and carves GPA
arenas. The snapshot covered **none** of it.

So a fault raised after an earlier victim had already been absorbed left that victim
dead, and the rollback's re-derivation minted its client afresh: **new `ProcId`, newly
spawned isolate, newly carved arena**. The guest kept its handles and its PDB while every
host identity behind them silently changed, and anything it had published was gone. One
process's malformed event destroying another process's state is exactly the boundary-1 /
#14 guarantee the per-`Proc` design exists to provide — so this was security-relevant,
not merely untidy.

**And it was worse than a lost proc.** Bite-checking the arena case found a
**guest-reachable panic**: when the fault is `Gpa(WindowExhausted)`, the failed refresh
has already consumed arenas that the re-derivation needs, so re-deriving the *last-good*
graph fails too and `refresh(procs).expect("last-good graph re-projects")` fires —
`panicked at gpu.rs:714: last-good graph re-projects: Gpa(WindowExhausted)`. A hostile
guest could take the whole device down with a legal-looking `DUP_OBJECT`. That is
boundary-1 item 3 (never panic the core), reached through the *rollback*, which is the
one path nobody thinks to fuzz.

**The fix is validate-then-mutate, not a bigger snapshot.** Snapshotting the proc set is
not available: a `Proc` owns its isolates, so it is neither cloneable nor cheap, and a
deep copy would be wrong as well as expensive. But `project()` is already pure and
already runs first, so every refusal is decidable up front. `Spine::plan_refresh` now
computes, from `bounds` + current state, before a single proc is touched:

- the per-boundary `matching` set, and with it **`CondemnedMerge`** and **`LateMerge`**;
- the target set each boundary's proc will span (derived from the projection, not read
  back off the previously-synced procs), and with it **`HeterogeneousArch`**;
- the new per-target windows, and with them the `geom` overflow;
- and finally it **carves** the arenas the mutation pass will hand out, because
  exhaustion is a property of the windows, not an arithmetic prediction about them. A
  failed carve releases everything it took; `GpaSpace::release` is exactly `carve`'s
  inverse, so capacity and the set of ranges are restored (only the free list's LIFO
  order differs, which no invariant names).

The mutation pass that follows has **no `?` in it**. Atomicity is now structural rather
than claimed — the type of the thing being asserted changed, which is the only kind of
fix worth making here.

**`sync_rpc_mappings` is a different, weaker story, and the doc now says so.** It runs
after `refresh`, mutates address tables, and can fault (`UnbackedMapping`; `Overlap` out
of the bind). It has no plan. What restores it is the rollback's **re-run**: the
re-sync's stale-unbind pass drops every `rpc_bound` VA the last-good graph no longer
desires and re-binds every one it does, so the table comes back equal in content. The
residue is confined to the proc whose event it was, because a single event changes the
mapping set of one `(GpuId, Pdb)`. A correct narrow claim replaced a false broad one, on
`Spine::apply` itself.

**Three tests, in `security_boundary.rs`** — each pins one of the passes a fault could
land in, and each asserts the exact fault variant, never `is_err()`:

- `a_refused_merge_leaves_the_victim_it_reached_first_bit_identical` — ★ the headline,
  and the multi-victim ordering case that is the actual bug. Three independent procs; one
  hostile `Alloc` resolves two dups parked on the same not-yet-allocated source, so a
  single boundary matches all three. The middle proc is untouched and legally absorbed
  *first*; only then does the third — which has published a backing — earn
  `LateMerge { kept, absorbed }`. The bystander's `ProcId`, per-GPU `IsolateId`s, GPA
  arena ranges, client set, vas keys and PDB route must all be identical, and
  `retired_len()` must not have moved. **Bite-check:** with the fix reverted,
  *"a refused event must retire NOBODY — the middle proc was absorbed before the fault
  and its retirement was never rolled back — left: 1, right: 0"*.
- `a_refused_arena_carve_returns_every_arena_it_took_and_loses_no_proc` — the fault one
  pass later. A legal merge retires the absorbed proc, and *then* the surviving proc turns
  out to need an arena on a full window. Also pins the undo: the merged boundary needs
  arenas on two targets, the first carve succeeds, the second fails, and the test proves
  the first went back by requiring the last free arena on that target to still be
  claimable afterwards. **Bite-check:** with the fix reverted this does not fail an
  assertion, it **panics inside the rollback** (`gpu.rs:714`), which is how the
  guest-reachable panic above was found.
- `a_refused_map_sync_restores_the_binding_it_had_already_installed` — the
  `sync_rpc_mappings` half, built on the one shape that genuinely leaves residue: two
  `MapMemoryDma`s park on a memory handle that does not exist yet at **overlapping** VAs,
  and the alloc that promotes both makes the sync bind the first and refuse the second
  with the exact `Address(Overlap { pdb, va })`. The offending proc's table must be back
  to its last-good contents and the bystander's untouched. **Bite-check:** deleting the
  re-sync's stale-unbind pass leaves the half-installed binding in place —
  *"the half-installed binding must be gone"*, `left` carrying two ranges where `right`
  has one.

**What this settles.** Boundary-1's headline promise — a hostile stream earns only its own
refusal — is now a property of the control flow rather than of a snapshot that covered
one field out of seven, and the rollback path itself is no longer a panic surface.

### 12.19 ★ G7 — the wrong-window arena release was a `debug_assert`, i.e. nothing

`GpaSpace::release` takes its arena **by value**, and that was the whole safety story:
double release unrepresentable, a live `Proc`'s arena unreachable. It said nothing about
*which* window. Releasing target A's arena into target B's window was expressible in safe
code and checked only by `debug_assert!(arena.range.start >= self.window.start && …)` —
**compiled out in release**, which is where the device runs.

**What it actually costs, stated exactly** (the vague version of this claim is how the
`debug_assert` survived two passes). Under the geometry the core mints — one disjoint
window per target, MG-6 — a misroute produces two things, and *not* a third:

- target A **permanently loses** that range: A's cursor has already passed it and B now
  holds it, so it is never issued by A again — the #80 leak class, per target;
- a proc on target B is handed GPAs **inside target A's aperture** — a window only ever
  handing out ranges it owns is the entire content of "per-target window", and it is
  broken;
- but **not** two live procs on one range. That needs the recipient's *un-issued* region
  to contain the donated range, which disjoint windows rule out. It is one call site's
  property, not the type's, and `GpaSpace` is public — the next geometry on the roadmap
  (a MIG instance's window carved inside its parent GPU's, `multi_gpu_and_mig.md`) is
  precisely the nested shape that makes it a straight **double issue**: B hands the
  donated range out of its free list and then hands the *same* range out again as a fresh
  cut. That is the #14 collision class arriving through the recycling path.

**The fix, and why not a type-level one.** A brand — an arena that cannot even be *passed*
to the wrong window — needs an invariant lifetime per window. The windows live in a
runtime-keyed `BTreeMap<GpuId, GpuTarget>`, so every entry shares one lifetime parameter
and brands cannot tell target A from target B; per-entry brands need existential
lifetimes (`generativity`-style), which is not available in safe stable Rust here. So the
structural half went where the mistake actually happens: **an arena is stamped with its
owning target at carve time** (`GpaSpace::owned_by` / `GpaArena::owner`), and
`Spine::reap_retired` routes it home by *its own* owner instead of by the map key it was
filed under — there is no longer a key for a caller to get wrong. The window check itself
is a loud `Result<(), ForeignArena>`, and the error **carries the arena back**, because a
refusal that consumed the range would have traded a collision for a leak.

Two smaller hardenings came with it: `GpaArena` is **no longer `Clone`** (a clone is two
releases of one range, which is exactly the double-issue the by-value signature exists to
forbid — the derive had quietly handed it back), and `reap_retired`'s
`if let Some(t) = self.targets.get_mut(&gpu)` no longer **silently drops** an arena whose
target is missing: the range is recorded on `Reclaimed::orphaned()`. That arm is
unreachable today, which is precisely the argument that lets a silent drop survive a
review.

**Four tests.** The two allocator-level ones are written so the assertion is on the
**consequence**, not on the refusal — a test that only asserted `Err` would pass for the
wrong reason the moment the refusal moved, and could never show the collision:

- `g7_a_window_only_ever_hands_out_ranges_it_owns` (`gpa.rs`) — ★ the owner's exact ask,
  with the core's real disjoint geometry: GPU 0's arena is released into GPU 1's window,
  tolerantly, and then GPU 1 serves procs. Every range it hands out must lie inside GPU
  1's own window. The refusal's exact `ForeignArena { refused_by, arena }` is asserted
  afterwards, including that the arena came back unchanged and still routes home.
- `g7_a_cross_recycled_arena_would_be_one_range_in_two_live_hands` (`gpa.rs`) — the
  collision, with the geometry it needs (two targets over one range), asserting that two
  procs served by GPU 1 get **disjoint** ranges.
- `g7_the_reap_routes_each_arena_home_and_orphans_nothing` (`teardown_reclaim.rs`) — a
  proc spanning GPU0+GPU1 is reaped; nothing is orphaned and each window recycles **its
  own** range (re-carved and compared, LIFO).
- `g7_an_arena_the_reap_cannot_route_home_is_reported_not_dropped` — the `else` arm.

**Bite-checks.** Deleting the guard *and* the `debug_assert`s — i.e. reproducing the
pre-fix code as it behaves in a **release build** — fails both allocator tests naming the
exact ranges: *"two LIVE procs were handed OVERLAPPING GPA ranges: P2 4294967296..8589934592
vs P3 4294967296..8589934592 (the cross-recycled arena was 4294967296..8589934592)"*, and
*"GPU1 handed a proc 4294967296..8589934592, which is NOT inside its own window
12884901888..21474836480"*. (Removing only the `Result` leaves the `debug_assert`s to fire
in a *debug* build, which is why the bite-check has to take them too — that asymmetry is
the bug.) Re-introducing the silent drop gives *"the range it could not return must be
NAMED, not silently dropped — left: [], right: [(GpuId(0), 137438953472..206158430208)]"*;
routing the reap's release to `GpuId::ZERO` instead of the arena's owner turns the GPU1
arena into a reported orphan — the guard converting a silent cross-recycle into a visible
one.

### 12.20 ★ G6 — the per-process arena had `alloc` and no `free`

`GpaArena` was a bump allocator with `alloc`, `is_untouched`, and no way back.
Reclamation therefore existed only at **whole-arena granularity, at proc reap**. A
long-lived process that maps and unmaps repeatedly — the exact process this project
exists for — walked its cursor to the end and took a permanent `FwdFault::Arena` with no
recovery. That is the C's #80 leak (`teardown_hardening_done`: *"Even a well-behaved
guest leaked the GPA window (no-free bump allocator) until all GPU mmaps failed"*),
reproduced at **intra-proc** granularity after being fixed at window granularity and then
at proc granularity. Measured: with a 512 KiB arena the process died at map/unmap **cycle
128**, and "clean cleanup when the GPU goes idle" was unreachable by construction.

**A free list, not a collector** (settled, and worth restating because the temptation is
real). The `RmGraph` already models DUP_OBJECT refcounting from declared protocol facts,
so liveness is *known exactly* rather than inferred; and cross-`Proc` references cannot
reach another proc's GPA, because a **user↔user** dup between two clients makes them the
same `Proc` (★ §12.27 corrects the over-broad form of this sentence: a dup into a *kernel*
client is a reference and does NOT merge — but it also never reaches a GPA, because the
system component has no data plane, and the referenced resource keeps its ALLOCATOR's
component alive, so nothing is reclaimed early). A tracing GC would re-derive what the graph states and would make
reclamation non-deterministic, breaking the `CoreSnapshot` differential property the core
is built on. So: a coalescing free list on `GpaArena`, and a move-only token.

**What landed.**

- `GpaArena::alloc` returns a **`GpaBlock`** — the allocation's `Gpa`, its length, and the
  `ArenaId` that cut it. Neither `Copy` nor `Clone`, and `#[must_use]`.
- `GpaArena::free(block)` takes it **by value**, mirroring `GpaSpace::release(arena)` one
  level up, so a double free is not a runtime check that can be forgotten — it is a value
  that no longer exists. A `compile_fail` doctest on `free` is the proof; deriving `Copy`
  on `GpaBlock` makes that doctest fail with *"Test compiled successfully, but it's marked
  `compile_fail`"*, which is the bite-check.
- **`ArenaId` carries a generation**, bumped per carve. Without it a block cut by a dead
  proc fits perfectly inside the arena the window later handed to a *live* proc at the
  same address (LIFO recycle — #80 working as designed), and a range-only check would take
  it. That is the ABA, and the generation closes it structurally rather than by timing.
- The free list **coalesces** with both neighbours and gives a range that runs up to the
  bump cursor back to the cursor. Not an optimization: a non-coalescing list shreds a
  mixed-size map/unmap stream into unusable slivers and exhausts the arena anyway — the
  same bug wearing a free list. A fully drained arena returns to genuinely pristine
  (`is_untouched()` again).
- Wiring: `commit_publish` keeps the token beside the binding in `Vas::blocks` (the same
  split G1 made between placement and allocation — `Binding` is `Copy` and a free token
  must not be), and returns the GPA immediately if the bind refuses. A new
  `kayfabe_fwd::unpublish_backing` is the intra-proc counterpart of `reap_retired`: it
  unbinds, returns the GPA to **this proc's own** arena, and hands back the host `Orphans`.
  The two halves are one call deliberately — a GPA recycled while its host memory is still
  mapped is the `ALREADY-MAPPED` class.

**Deferred, named rather than half-done.** When `Spine::refresh` drops a `Vas` whose
VASpace the guest freed, that Vas's blocks go with it and the GPAs are not returned to the
arena. Returning them there would recycle a GPA whose host memory is still allocated and
possibly still mapped, which is precisely what `unpublish_backing` pairs to avoid; the
host-side half of that teardown is L1-M2's reclamation policy (the same bucket as G1's
`reclaim_plan`), and the GPA half must land with it, not before it. Until then the
residue is bounded by the proc's own arena and released whole at reap.

**Six tests + the compile-fail doctest.** Both allocator-level ones assert the
**consequence**, not the refusal — a test that only asserted `Err` could never show the
collision, and would pass for the wrong reason if the refusal moved:

- `g6_an_arena_serves_far_more_than_its_size_across_alloc_free_cycles` (`gpa.rs`) — a
  64 KiB arena serves 16 MiB, then a mixed-size stream freed out of order, then must be
  **pristine** again. **Bite-check** (make `free` a no-op): *"cycle 16 exhausted a
  reclaimed arena: ArenaExhausted { len: 4096 }"*.
- `g6_a_stale_block_cannot_be_freed_into_the_arena_that_replaced_its_range` — ★ the ABA
  the owner asked for, scripted by **order** rather than timing. **Bite-check** (pin the
  generation to 0): *"the stale free re-armed a range: B handed Gpa(0) to two live
  allocations"* — the double issue, named.
- `g6_a_block_cannot_be_freed_into_another_procs_arena` — the #14 boundary applied to
  reclamation. **Bite-check** (drop the `ArenaId` check): the range escapes into B.
- `g6_a_long_lived_process_that_maps_and_unmaps_never_exhausts_its_arena`
  (`teardown_reclaim.rs`) — ★ the headline, through the real publish path: 4096 cycles on
  a 512 KiB arena (32× over), then `live_bytes() == 0` and a host ledger whose **only**
  outstanding object is the Vas's own host VAS. **Bite-check:** *"cycle 128 could not
  publish: Arena"* — the exact predicted death.
- `g6_reclaiming_a_backing_that_is_gone_is_loud_and_leaves_the_arena_intact` — ★ the
  owner's "call free on an object that's missing — see if nothing races". Three flavours
  of gone (never published; already reclaimed; **VASpace destroyed through the real graph
  path**), each asserting the exact fault (`Address(Miss { pdb, va })`, `UnknownPdb`), and
  nothing races because the refusal happens before the table is touched. The arena is then
  shown intact: the reclaimed range is reused deterministically and two live publications
  never share a GPA.
- `g6_no_live_binding_ever_points_outside_its_own_procs_arena` — ★ **the invariant Stage
  2's safety argument rests on, which had never been pinned.** It holds, and structurally:
  `commit_publish` allocates from `proc.arenas[gpu]` and binds only into a `Vas` of that
  same `Proc`, and a cross-process reference cannot arise because a `DUP_OBJECT` makes the
  two clients **one** `Proc`. The test states both halves — a dup-joined pair publishing
  from ONE arena, two unjoined procs on disjoint ones — then reaps one, lets a **new** proc
  recycle its range, and sweeps every binding in the device. Nothing points into the
  released range. No finding here; the argument survived being made executable.

### 12.21 ★ G9 — `deviceInstance` is a raw guest `u32`, and it minted GPU targets forever

`GpuId` comes from `node.facts.device_instance.unwrap_or(0)` (`rmgraph.rs`, `walk_gpu`),
and `ensure_target` minted a fresh `GpuTarget` — its own guest-physical window and its own
`DeliveryPlane` — on first touch, with no cap, no validation, and no pruning. Every
neighbouring guest-reachable surface has a named cap (`MAX_OUTSTANDING_COMPLETIONS`,
`MAX_ARMED_FENCES`, `MAX_PUSH_TOTAL_BYTES`, `MAX_LIVE_HANDLES`); this one had none.

**Bench + open-kmod findings that decided the shape of the fix** (measured 2026-07-25):

- RM already enforces `deviceInstance < NV_MAX_DEVICES (32)` in **three** places
  (`ogkm alloc_free.c:1372-1390`; `device.c:118-129` → **`NV_ERR_INVALID_CLASS`**;
  `device.c:357-368`). **So a `< 32` cap is not where the risk lives** — it would still
  let a guest mint 31 windows and 31 delivery planes on a single-GPU box. Security theatre.
- The real check in RM is `osIsGpuAccessible` → `nv_is_gpu_accessible`
  (`kernel-open/nvidia/nv.c:5904-5910`), which scans the **host process's fd table**.
  Device allocs go through `/dev/nvidiactl`, which carries **no GPU identity**, so
  `deviceInstance` is the *sole* selector.
- `gpumgrGetPrimaryForDevice` **fails open to GPU 0** for an in-range-but-unpopulated
  instance (`gpu_mgr.c:688-691`).
- Trivially attacker-controlled: ~20 lines of raw `NV_ESC_RM_ALLOC` on `/dev/nvidiactl`,
  **no patched guest kernel**. Stock userspace never emits one.

⇒ The cap is the **ENTITLEMENT**: the roster of GPUs this device was actually realized
with. `Gpu::realize(arch, isolates, gpa, gpus)` declares it (`Gpu::new` is its N=1 case),
`RmGraph::entitle` holds it, and the refusal happens at the **`Device` alloc**, where RM
refuses it, with `RmGraphError::InvalidDeviceInstance` standing in for
`NV_ERR_INVALID_CLASS` — so a guest cannot distinguish us from a real single-GPU box by
probing instances.

Two more, both load-bearing:

- **Our own `unwrap_or(0)` is gone.** A `Device` with no declared instance was silently
  becoming GPU 0 — a default-target guess inside the one resolver whose entire discipline
  is MISS=None-never-a-guess. It now leaves its subtree **unroutable** (no route, no arena,
  no isolate, a loud `UnknownPdb` at use). A real Device cannot be in that state —
  `deviceId` is a required field of `NV0080_ALLOC_PARAMETERS` — which is exactly why
  guessing for it was indefensible. One existing test (`map_before_backing_and_pdb_resolves`)
  was silently relying on the guess; the test harness now declares instance 0, as a real
  Device does.
- **The same `deviceInstance` twice under one client is LEGAL on bare metal**
  (`device.c:368-380` rejects it only under `IS_VIRTUAL`). Device-per-client is not 1:1, so
  the entitlement check is a *membership* test and must never drift into a uniqueness one.
  Pinned by a test.

**Four tests** (`security_boundary.rs`), each asserting exact variants:
`g9_an_unentitled_device_instance_is_refused_and_mints_no_target`,
`g9_a_device_instance_flood_grows_no_device_state` (4096 instances; asserts the
**resource** — `targets.len()` — before the return values, so it can show what it protects),
`g9_an_undeclared_device_instance_is_unroutable_not_gpu_zero`,
`g9_the_same_device_instance_twice_under_one_client_is_legal`.
**Bite-check** (restore `unwrap_or(0)` and drop the entitlement check): the flood is
accepted (*"left: Ok(()), right: Err(Graph(InvalidDeviceInstance { instance: 1 }))"*) and
the undeclared Device routes onto GPU 0 (*"an undeclared instance must NOT route onto
GPU 0 — left: Some(ProcId(1)), right: None"*).

### 12.22 ★ G10 — two unbounded device-global lists, and a carry-forward that was O(n² log n)

`Spine::condemned` and `Spine::retired` were unbounded, and the condemned list was
rescanned on **every** apply. Three separate problems, and the third turned out to be the
big one.

**(a) The caps.** `MAX_CONDEMNED_COMPONENTS` and `MAX_RETIRED_PROCS`, both 1024. The
interesting question is what to refuse. Refusing a **condemnation** would be worse than
useless — it un-condemns a component whose isolate is already dead, so the next refresh
re-derives it with a fresh isolate and serves the guest a **zeroed** backing for a VA it
believes still holds its data: §12.13's silent corruption, reintroduced by a memory cap.
Refusing a **retirement** leaves a proc live whose worker is gone. Dropping corpses leaks
exactly the isolates and GPA arenas the retired list exists to reclaim. So the refusal
lands on the only guest-reachable action that *consumes* the resource: deriving a **new**
`Proc` (`GpuError::SpineCapacity { what, cap }`, raised in `plan_refresh`, hence atomic per
§12.18). Everything already condemned stays condemned, every live proc keeps serving, and
recovery is the guest's own — free the dead client roots and the entries prune. Backpressure,
not a brick.

**(b) The scan.** "Does this boundary intersect a condemned component" was a nested scan,
O(|boundaries| × |condemned|), both factors guest-driven. The entries are pairwise
disjoint, so client → entry is a *function*: build the index once and the pass is
O(total clients · log n).

**(c) ★ The carry-forward, which was much worse and was not in the brief.** Every
intersecting boundary called `absorb_condemned`, which drains and re-sorts the whole
carried list — **O(n² log n) per apply**. Measured: the 1024-component test took **55 s**.
The fix is the same answer computed instead of searched: boundaries are pairwise disjoint
and so are the entries, so two boundaries' merged sets can overlap ONLY by hitting a common
entry. Union-find over entry indices, keyed by boundary, yields exactly the fixpoint the
repeated absorb was grinding out — near-linear, identical result. **55 s → 3.8 s**, and
every existing §12.13 condemnation test stayed green unchanged, which is the evidence that
the two computations agree.

**Two tests** (`security_boundary.rs`, gated `KAYFABE_SLOW` — they walk a bound to its cap,
which is exactly what that gate is for; measured 3.7 s of the fast path's 23.5 s):
`g10_condemnation_is_capped_and_refuses_new_procs_never_the_condemnation` — drives the
hostile pattern (spawn a worker, kill it, repeat), asserts the exact `SpineCapacity`
refusal, that **nothing was un-condemned to make room**, that a bystander still resolves,
and that freeing one dead client root restores service; and
`g10_the_retired_list_is_capped_and_a_reap_clears_it` — same shape, plus "the refusal drops
no corpse".

### 12.23 ★ The per-event graph CLONE: kept, and here is exactly what forces it

`Spine::apply` does `let snapshot = self.rmgraph.clone()` on **every** RM event. §12.18's
validate-then-mutate raises the obvious question: if `refresh` is now infallible, is the
clone still needed? It was evaluated properly rather than left unexamined.

**`RmGraph::apply` IS atomic on failure** — every error return precedes every mutation, on
all six arms (`Alloc`, `Dup`, `SetPageDir`, `MapMemoryDma`, `Unmap`, `Free`, plus
`apply_map`'s park/capacity paths). That was an argument; it is now a test,
`rmgraph_apply_is_atomic_on_failure`, which fingerprints nodes + dup edges + mappings and
requires byte-identity after each of five refusable events, each asserted by exact variant.
**Bite-check:** making the `ConflictingMap` arm mutate before returning fires *"a refused
event must leave the graph byte-identical"*. This is the precondition for ever deleting the
clone, so it is worth having on its own.

**But the clone cannot go**, and the reason is specific: **three faults are raised AFTER
`RmGraph::apply` has already mutated the graph, and none is pre-computable without the
post-event graph** —

1. `project()` → `ProjectionError::PdbCollision` / `VchidCollision`. A `SetPageDir` that
   duplicates a live PDB on one target is accepted by the graph and refused by the
   projection.
2. `plan_refresh` → `LateMerge` / `CondemnedMerge` (a dup edge the graph accepts).
3. `sync_rpc_mappings` → `UnbackedMapping` / `Address(Overlap)`.

Each is a function of the post-event graph, and a single `Alloc` can promote arbitrarily
many parked dups / page-dirs / maps, so the post-state is not a local function of the
event. Without the rollback the offending fact stays in the graph and **every subsequent
apply re-derives and re-faults** — a permanent control-plane wedge for every other process.
That is the exact global-DoS class `apply_map`'s parked-map wedge guard already names. The
clone is load-bearing.

**★ And the measurement says it is not the problem anyway.** Control-plane cost, debug
build, alloc+map pairs against a growing graph:

| events | with clone | without clone |
|---|---|---|
| 1000 | 0.85 s | 0.64 s |
| 2000 | 3.35 s | 2.50 s |
| 4000 | 13.5 s | 10.1 s |
| 8000 | 54.8 s | 41.6 s |

Deleting the clone saves **24%** — and leaves the curve **still quadratic** (4× events →
~4.1× time at every step). The clone is one of *three* O(graph) passes per event; `project()`
re-derives every boundary and `sync_rpc_mappings` rebuilds the whole desired-mapping set,
both from scratch, on every single control-plane event. `Spine::apply`'s own doc claim —
*"a clone here is off the performance-critical path"* — is wrong about the wrong thing: the
clone is a quarter of a control plane that is O(live objects) per event end to end.

**★ This is a finding, not just a performance note.** With `MAX_LIVE_HANDLES = 2^18`, a
guest can make each control-plane event cost O(live objects), so N events cost O(N²) —
a guest-reachable complexity DoS (boundary-2) of the same species as the parked-map linear
scan that was already hardened. PyTorch startup allocates thousands of RM objects, so it is
reachable benignly too. **Deferred deliberately, with the two candidate fixes named rather
than guessed at:**

- **incremental derivation** — `project`/`sync` recompute only what the event touched. This
  is the fix that actually removes the quadratic, and it is a redesign of the "derived state
  is a pure function of the graph, never accreted" rule (decision #27) that the whole
  determinism/differential property rests on. Needs an owner decision, not an afternoon.
- **an undo journal in `RmGraph::apply`**, which would let the clone go (O(changes) instead
  of O(graph)) — worth ~24%, and it is ~200 lines in the most safety-critical file in the
  repo, where getting it wrong reintroduces exactly the non-atomicity §12.18 just removed.

Doing either blind, in a security round, to buy a constant factor off a curve that stays
quadratic, would be the wrong trade. The clone stays, the reason is written down, and the
quadratic is now a named, measured item instead of an assumption.

### 12.24 ★★ OUT-OF-BRIEF FINDING — a dup alias at a freed origin's key wipes the namespace

Running the `KAYFABE_SLOW` gate for this round's green check made the `RmGraph` refcount
fuzz property (`a4_dup_object_is_reference_counted`) draw a case it had never drawn before,
and it fails. **It is pre-existing**: the same case fails with every one of this round's
core changes stashed, i.e. at `934829a`. The seed is persisted in
`tests/tests/fuzz_rmgraph_invariants.proptest-regressions`, so it now reproduces
deterministically. It is **not fixed here** — see below.

**Reduced to six events.** Client A allocates a `Client`-classed object at handle `H`;
client B dups it (so the resource outlives A's handle); A frees `H`; B dups it **back into
A at the same handle `H`**; A allocates an unrelated object `X`; A frees `H`.

Observed: freeing `H` — which is now a **dup alias**, not the origin allocation —
**destroys `X`**, and does not even drop the alias it was asked to drop.

```
BEFORE [NodeKey { client: A, handle: 0x100 }, NodeKey { client: A, handle: 0x300 }]
AFTER  [NodeKey { client: A, handle: 0x100 }]
```

**Root cause.** `free_subtree` asks `is_client_root(key)`, which is
`self.node(key).is_some_and(|n| n.key == key && matches!(n.kind, ObjectKind::Client))`. That
test *intends* "this handle is the origin allocation of a Client resource", and it is
correct for an ordinary alias (whose origin key differs). But once the origin handle has
been freed and a later `Dup` re-creates **exactly** the origin's `(client, handle)` key as
an alias, the resource's immutable `node.key` still equals it — so an alias is
indistinguishable from the allocation, and the free takes the whole-namespace branch. The
same conflation sits in the non-client branch's `res.node.key != hkey` parent-walk guard.
The graph therefore violates its own stated rule, written two lines above the bug: *"a dup
alias is a leaf reference with no children of its own."*

**Impact.** A wrong-**destroy**: objects the guest never asked to free are removed from the
graph, which is the authority every derived plane syncs to — so their `Vas`es, channels and
routes vanish with them. It is guest-self-inflicted as modelled (the free is issued in the
namespace being wiped), but two things make it worth more than that: dup-connected clients
are **one `Proc`**, so an alias-free in one client can take out objects the *other* client
in the same component owns; and the core has no client-existence check at all — it happily
allocs and dups into a namespace whose client root is already freed, which real RM refuses
with `NV_ERR_INVALID_CLIENT`. That missing check is plausibly the more faithful place for
the fix, and it is a second finding in its own right.

**Why it is reported and not fixed.** Both candidate fixes are changes to the refcount
model in the most safety-critical file in the repo, and this round's brief is four other
items:

- *(a)* distinguish "origin allocation" from "alias at the origin key" — a per-`Resource`
  `origin_live` flag, cleared when `node.key` is freed and never re-set by a `Dup`. ~6
  lines, but it changes what `is_client_root` and the parent walk mean.
- *(b)* refuse `Alloc`/`Dup` into a namespace whose client root is gone
  (`NV_ERR_INVALID_CLIENT`), which makes the state unreachable rather than handled.

They are not equivalent — (b) is the faithful one and also forbids other unmodelled states,
(a) is the local one — and picking between them is an owner decision, not a guess to make
inside a security pass. **Consequence to be explicit about: the `KAYFABE_SLOW` gate is RED
on this one test.** The fast gate (243 tests) is green, and every other slow test is green.

**Status: FIXED in §12.25** (variant (a), generalized: the *declaration* is recorded, not an
`origin_live` flag). Variant (b) — the client-existence check — is CONFIRMED as a separate,
still-open gap; §12.25 has the evidence and the scope.

### 12.25 ★★ §12.24 FIXED — the graph was DISCARDING a declared fact (`Alloc` vs `Dup`)

§12.24's bug, restated as the defect it actually is: `RmGraph` recorded *that* a handle
references a resource and threw away *how the guest said it got there*. `RM_ALLOC` and
`DUP_OBJECT` are two different declarations with two different lifecycle meanings — one
creates the object, the other takes a leaf reference to someone else's — and the graph
stored both as the same `NodeKey → ResId` edge. Every predicate that later needed "is this
handle the original allocation?" had no recorded fact to read, so it re-derived the answer
from incidental structure: `handle == resource.node.key`. That is decision #14 violated in
the file decision #14 is about. Handle values are **reusable by design**, so the derivation
is not merely fragile — it is false the moment a `Dup` lands on a freed origin's key, and a
guest can arrange that in four events.

**The fix.** The handle table now stores the declaration:

```rust
enum HandleRef { Origin(ResId), Alias(ResId) }   // Alloc said Origin; Dup said Alias
handles: BTreeMap<NodeKey, HandleRef>
```

`Origin` is minted only by the `Alloc` arm (which also mints the fresh `ResId`, so exactly
one `Origin` exists per resource by construction); every `DUP_OBJECT` — direct or promoted
out of `pending_dups` — inserts `Alias`, whatever handle **value** it lands on. Nothing
else about the refcount model changes: `refs` is still the liveness set, `ResId` is still
the identity, teardown is still last-reference-wins.

Honest about the shape, per the house preference for unrepresentable-over-checked: this is
**a recorded discriminator plus corrected predicates**, not a type that makes the bad state
impossible to write down. What it does buy is that the fact now exists exactly once, at the
only place that can know it (the event handler), and every consumer *reads* it. The
stronger form — splitting `Resource::refs` into `origin: Option<NodeKey>` + `aliases` —
duplicates state the handle table already holds and would need its own agreement invariant,
which is a worse trade.

**Three predicates asked the discarded question; all three were wrong, only one was known.**

| site | asked | now asks | symptom of the old form |
|---|---|---|---|
| `is_client_root` | `n.key == key && kind == Client` | `Origin` + kind | §12.24: freeing an alias wipes the namespace |
| `free_subtree` parent descent | `res.node.key != hkey → skip` | `!is_origin() → skip` | an unrelated parent's free drags the alias in through a parent edge the alias never declared |
| `apply`'s `Alloc` idempotency | payload equality alone | `Origin` + payload equality | `RM_ALLOC` over a live alias silently answered "already done", leaving an alias where the guest asked for an allocation (real RM: handle in use) |

The second and third were **not** covered by the refcount fuzz — it generates only
self-parented objects (no parent edges to descend) and never re-sends an alloc — so each
got a named regression test. All four new tests live in `tests/tests/object_model.rs`
beside the existing free-subtree/refcount ones.

**The symptom, measured, with one correction to §12.24.** Six events, `H = 0x100`,
`X = 0x300`, B's alias `0x900`:

```
BEFORE nodes = [A:0x100, A:0x300]      refs(A,H) = [A:0x100, B:0x900]
AFTER (buggy) nodes = [A:0x100]        refs(A,H) = [B:0x900]   (A,X) live? false
AFTER (fixed) nodes = [A:0x100, A:0x300] refs(A,H) = [B:0x900]  (A,X) live? true
```

The correction: §12.24 read the surviving `A:0x100` in the AFTER line as "the alias was not
dropped". It was dropped — that line is the `nodes()` list, i.e. **resources**, and the
resource survives on B's reference, which is correct in both versions. The bug is exactly
one thing, and it is the bad one: **`X` is destroyed.** A wrong-destroy of an object the
guest never named, in the authority every derived plane syncs to.

**Bite-check** (each predicate reverted individually, everything else fixed):

- `is_client_root` reverted → the persisted seed `07c3b0b3…` fails again with
  `left {(A,0x7000_0005)}` vs `right {(A,0x7000_0005), (A,0x7000_0006)}`, and
  `freeing_a_dup_alias_on_a_reused_client_handle_never_tears_down_the_namespace` fails with
  the same shape. **The fuzz does not catch the other two.**
- parent-descent guard reverted → fuzz GREEN,
  `a_dup_alias_on_a_reused_handle_is_not_dragged_into_its_origins_parent_free` fails: the
  alias is gone from `refs` after an unrelated device free.
- `Alloc` idempotency guard reverted → fuzz GREEN,
  `alloc_over_a_dup_alias_is_loud_even_when_the_payload_matches` fails `Ok(())` vs
  `Err(ConflictingAlloc)`.

**Mutation.** `rmgraph.rs` is the file the L0 99.2% was measured on, so every branch the fix
adds was hand-mutated (a full `cargo mutants` campaign was skipped for disk headroom, ~7 GB
free against a 4.9 GB `target/`): `is_origin → true` (4 tests fail), `is_origin → false` (a
dozen), `is_client_root`'s `&&` → `||` (3), the `Alloc` guard → `true` (3, incl.
`wo_retried_duplicate_events_are_idempotent`) and → `false` (3), the `Alloc` arm recording
`Alias` (2), the direct-`Dup` arm recording `Origin` (4), the promotion path recording
`Origin` (2). `HandleRef::res → Default::default()` is unviable (`ResId: !Default`). No new
branch survives.

**Deferred finding 1 — the client-existence check (§12.24 variant (b)) is CONFIRMED.**
Measured on the fixed tree:

```
alloc into a namespace whose client root was freed => Ok(()), node live
alloc into a client that NEVER had a root         => Ok(())
dup  into a namespace whose client root was freed => Ok(()), alias live
```

Real RM refuses all three with `NV_ERR_INVALID_CLIENT` (`ogkm`'s `serverAllocResource` →
`clientGetResource`; there is no path that allocates under a client handle that does not
resolve). The core has **no client-existence check anywhere** — `Alloc` validates only
handle collision and (for a `Device`) the GPU entitlement; the declared `parent` is never
required to exist. The fix would be: on `Alloc` (except the client root itself) and on the
`dst` of a `Dup`, require the namespace's client root to be live, and refuse with a new
`RmGraphError::InvalidClient`. It is **not entangled** with §12.25 — the identity fix is
sound on its own, because it makes the free do exactly what the declarations say regardless
of whether the namespace has a root. It is deferred deliberately: it changes the *accept*
surface (which streams the graph admits at all) rather than the *teardown* semantics, so it
wants its own round, its own order-tolerance argument (a `Dup` may legally precede its
source's `Alloc` — does a parked edge need a live root at park time or at promotion time?),
and its own look at every test that builds a namespace without a root.

**Deferred finding 2 — `RmNode.key` is not a unique identity, and `nodes()` can report two
live resources with the SAME origin key.** Same disease (a handle value is not an identity),
different organ. `Alloc (A,H) → Dup to B → Free (A,H) → Alloc (A,H)` leaves the ghost alive
on B's alias while the re-alloc mints a second resource whose origin key is *also* `(A,H)`.
Measured:

```
live nodes (key, mem_phys) = [(A:0x100, None), (A:0x100, Some(0xdead0000))]
pdb_of(A:0x100)            = Some(0x1111000)   ← the GHOST's PDB; the live VAS declared 0x2222000
references(A:0x100)        = [B:0x900]         ← the ghost's ref set, not the live resource's
```

Every by-origin-key lookup (`pdb_of`, `references`, `map_ref_count`, `gpu_of`'s post-free
fallback, `backing_of`'s fallback) resolves to whichever resource sorts first by `ResId` —
the ghost — and `project()` keys `vases`/`channels` on `node.key`, so the two collapse into
one entry carrying the wrong PDB. The honest fix is to stop passing origin keys across the
API boundary and expose the opaque `ResId` as the resource identity (`nodes()` yields
`(ResourceId, &RmNode)`, the by-origin lookups take a `ResourceId`), which touches
`project.rs`, `gpu.rs` and much of the suite — a refactor, not a patch, and out of scope
here for the same reason (b) was. Pre-existing, unchanged by §12.25 (the state is reached by
free-then-realloc, which this fix does not touch); recorded so it can be scheduled.

### 12.26 ★★ THE CROSS-`Proc` LIFETIME QUESTION — answered (c), and the answer needed the SYSTEM plane pinned down first

**The question, as posed.** RM's kernel-internal clients take **refcounted references into
a user client's memory** — `memCopyConstruct_IMPL` shares the source's memdesc and HW
resource and then refcounts both (`ogkm: src/nvidia/src/kernel/mem_mgr/mem.c:986`, `:993`,
`:1027-1031` — `pHwResource->refCount++`, `memdescAddRef`, `DupCount++`, plus the circular
`dupListItem` list at `:1036-1039`). The C **measured** the corresponding failure on the
bench, 2026-06-18: releasing a user client's overlays at its free *"yanks the backing out
from under the still-polling scrub"* owned by a different client
(`C: src/qemu/nvkvm_gpu_emul.c:2055-2065`). Its fix was not per-proc reclamation but a
**global** quiesce point (`C: :2074-2128`, consumed at the GSP re-handshake, `C: :3458`).
Our model has neither a refcount nor a global quiesce: `publish_backing` allocates host
memory owned by the *user proc's* isolate RM client, and the isolate **process** boundary
is the garbage collector (`l1_os_shell.md` §7.0). So: does a system-proc verb ever
reference user-proc-owned host memory?

#### ★ Step 1 first, because it nearly dissolves the problem: FORGE, and the scope

The C forges kernel-CeUtils completions deliberately and scopes it precisely — *"The scrub
is a no-op on our sparse/pre-zeroed backing … SCOPE: kernel CeUtils only — user-CE and GR
channels are excluded (the host executes + releases those for real)"*
(`C: nvkvm_gpu_emul.c:4032-4058`). **That reasoning holds here, and its weakest premise is
our strongest one.** The C's argument rested on a claim about the *content* of one flat
emulated-FB overlay space ("pre-zeroed"). Ours rests on the isolation boundary instead:

1. **Cross-process, residue is unreachable by construction.** Host memory is allocated by
   `RmBackend::alloc_sysmem` inside the owning proc's **own** isolate process, GPAs come
   from that proc's **own** disjoint arena, and the memory dies with that process. Another
   guest process's isolate is a different host process; the host kernel does not hand one
   process's freed pages to another un-zeroed. What the guest kernel's scrub exists to
   guarantee is therefore already guaranteed, by the same boundary #14 was fixed with.
2. **Intra-process, the residue is the guest process's own data** — which is precisely
   what a scrub-before-reuse inside one process is entitled to see.
3. **Scope, stated as a rule and not as a habit:** forge **only** completions whose work
   is provably a no-op against our backing model — the kernel CeUtils scrub, the GR golden
   capture. User-CE and GR channels are excluded exactly as in the C: the host executes
   and releases those for real. `Traffic::System` is the type that says so, and
   `signal_golden_capture` is typed to `Gpu::system` by name so a user-proc forge is
   unrepresentable rather than merely forbidden.
4. **The one scope condition worth writing down**, because it is a premise and not a
   theorem: if a future backing class is ever host **vidmem** rather than sysmem, RM's own
   scrubber owns that memory's hygiene and this argument must be re-derived, not assumed.

**But forging is not what makes the hazard absent, and it would have been wrong to stop
there.** It removes one class of forwarded system work. The load-bearing fact is stronger
and more general, and it is stated as a rule below.

#### ★★ The answer: (c) — and the rule that makes it a theorem rather than an accident

> **THE SYSTEM PLANE RULE. The system proc has no data plane.** It publishes no backing,
> owns no host memory, and forwards no verb that names guest memory. Every real byte the
> guest kernel moves on a user process's behalf is forwarded through **that user proc's
> own** isolate — which is also the isolate whose death reclaims it. Kernel-internal
> completions are forged (Step 1).

With that rule, a cross-`Proc` host reference has no way to exist in either direction: the
system proc owns nothing for a user proc to reference, and it mints nothing, so it never
holds a handle a user isolate owns. Enforced at the one site that mints host memory —
`plan_publish` refuses `Gpu::system` with `FwdFault::SystemDataPlane`, **before** any host
verb exists, so there is nothing to orphan. It is a loud refusal rather than a silent
impossibility on purpose: the day someone needs the system proc to publish, this lifetime
question must be re-opened deliberately, with a refcount or a global quiesce point, rather
than discovered afterwards.

**Why (a) and (b) were rejected, on the merits and not on effort.**

- **(a) a refcount/hold from the referencing proc onto the referenced backing** — RM's own
  answer, and the wrong one *here*. A refcount is the right mechanism when the two parties
  share one allocator and one namespace; ours deliberately do not. The hold would have to
  outlive the owner's isolate **process**, and the thing it is holding is host memory the
  host kernel frees when that process exits — so the refcount could keep our *bookkeeping*
  alive over memory that is already gone, which is a UAF wearing a safety belt. To make it
  real we would have to stop the isolate process boundary from being the collector, i.e.
  give up §7.0, i.e. give up the property that makes "reclaim everything on every path"
  achievable rather than aspirational. And per the house rule: *a hold that can be
  forgotten is not a hold* — this one could not even be *kept*.
- **(b) quiesce-gated deferred reclamation** — the C's answer, and it is genuinely the
  right answer *for the C's model*, which had one isolate, one flat GPGA overlay space, and
  no per-owner identity on a backing at all; a global barrier was the only tool available.
  We already have its per-isolate half where it belongs (`reap_retired` + `is_quiesced`,
  §12.16/G3). Extending it to a **device-global** barrier would (i) couple every proc's
  reclamation to every other proc's activity, re-creating exactly the F5-shaped
  serialization the design deletes, (ii) leave the hazardous interval merely *short*
  instead of *empty*, and (iii) still say nothing about a reference taken across the
  barrier. It converts a correctness question into a timing question, and this project's
  whole posture is to refuse that trade (MISS = FAULT, not MISS = usually fine).
- **(c) a structural proof** — chosen, and *made* structural rather than assumed. It was
  the core's unstated assumption before this round; now it rests on three facts, two of
  which already existed and one of which had to be built.

#### ★ The fact that had to be built: a handle now carries its namespace

`HostHandle` was `HostHandle(pub u64)` with the ownership rule in **prose** — its own
rustdoc said "scoped to ONE isolate's handle namespace", and `HostBacking::memory`'s said
"a handle from another isolate is a different object, boundary 2". Nothing read either
sentence. It is now `{ isolate: IsolateId, raw: u64 }`, minted by the backend that knows
the answer, and `Worker::execute` refuses — **before running the first verb** — any
`VerbPlan` naming a handle from another namespace (`RmError::ForeignHandle`, empty
`Orphans` by construction). One central enumeration, `VerbPlan::handles()`, so a new plan
variant that carries a handle is a mistake in exactly one file.

**Why this is not ceremony, which is the part worth being precise about.** The mock
namespaces its fake handle *values* (`(id+1) << 32 | n`), so a cross-namespace use there is
provably invalid and comes back `BadHandle`. **A real host does not.** RM mints
client-scoped handles from one shared base — `RS_CLIENT_HANDLE_BASE`
(`ogkm: src/nvidia/generated/g_resserv_nvoc.h:173`), one `serverSetClientHandleBase` for the
whole driver (`.../rmapi/rmapi.c:105`) — so the same raw value is **live and different** in
every other client. A foreign `free` on real hardware does not fault; it destroys a
bystander's object. A foreign unmap tears down a bystander's mapping. The mock's answer is
survivable and the host's answer is the C's bug, and **only the stamp distinguishes them** —
which is why the fact belongs in the type and not in the backend's luck. This is the
recorded-fact shape of §12.25 (not a branded-lifetime scheme), and the object-plane twin of
`GpaBlock`'s `ArenaId` (§12.20): *a block names the arena that cut it.*

The three facts together:

1. **A handle names its isolate**, and using one elsewhere is refused centrally (new).
2. **GPA arenas are per-`(Proc, GpuId)` and disjoint** (#14/MG-5), so two components cannot
   alias physically even if the guest declares it. A guest-declared RPC mapping into a
   system `Vas` naming a user proc's GPA binds with `host: None`, and the #14 ring gate
   refuses to execute against an unpublished binding — the guest can *declare* the alias
   and can never *use* it.
3. **The system proc has no data plane** (new), so it is neither end of a shared pair.

#### ★ The second finding: the system component is UNCONDEMNABLE, and that was a silent no-op

Falls out of the same analysis. `SignalOutcome::WorkerDied`'s consequence is retire +
condemn, whose recovery story is *"a fresh RM client is a different component"* (§7.3). The
system component's clients are the **guest kernel's**, held for the lifetime of the loaded
module — so that recovery requires the guest kernel to mint clients, which is exactly what
would have been condemned. **Condemning the system component is device-fatal by
definition.**

What was actually happening: `SharedDevice::signal_source` called
`Spine::retire_proc(SYSTEM_PROC)`, which reached for the system proc in a `ProcSet` that
does not contain it (`Gpu::system` is a field, not a map entry — §12.4), missed, and
returned `false` into a discarded result. The device carried on with a permanently dead
system worker slot and **no fault anywhere**: the loudest rule in the design, silently
absent for the one proc that cannot survive it.

Fixed on both sides, because a fix on either alone is undone by the other:

- `Spine::retire_proc` refuses `SYSTEM_PROC` **by rule**. Note honestly that this has no
  observable effect *today* — the map miss produces the same `false` — so the regression
  test constructs the future mistake (a `Proc` at the `SYSTEM_PROC` key of a real
  `ProcSet`) and asserts the refusal holds anyway. The asymmetry that makes this a real
  hazard rather than a hypothetical: `SharedDevice::proc_cell` **does** resolve the system
  proc while `ExclusiveProcs` does not, and "let's make these consistent" is an ordinary
  thing for a later change to do.
- `SharedDevice::signal_source` re-types the outcome to `SignalOutcome::DeviceFatal`. The
  slot still dies (never a respawn); what does not happen is the retire and the
  condemnation. RM's own analogue is the same shape at the same scope:
  `gpuMarkDeviceForReset` + `NV2080_NOTIFIERS_GPU_UNAVAILABLE`
  (`ogkm: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2779-2789`) — **device** level, never
  client level. Escalating it into a guest-visible device-unavailable notification is
  L1-M2's (T4/T7); the core's obligation is to make it distinguishable and loud.

#### Tests — `tests/tests/cross_proc_lifetime.rs` (10), all bite-checked

The C's disproof is ported as the thing it is: a system-proc verb naming a user proc's
backing, asserted refused **before it runs**, with the owner then dying **both ways** —
cleanly (per-object reclaim → client-root free → retire → reap) and by condemnation
(out-of-band worker death, isolate gone) — and the reference re-attempted afterwards. The
second attempt is the point: a rule that refuses before the owner dies and permits after is
not a rule. It refuses identically, because a handle's namespace is a property of the
*value*, so there is nothing to have gone stale. Conservation
(`kayfabe_mocks::HostLedger`) is asserted across four teardown orderings — owner first,
referencer first, both-before-reap, and a reach taken *between* retire and reap — with each
proc's isolate attempting the C's cross-namespace disposal in **both** directions first, so
the ledger proves that a script actively trying to reach across the boundary changes
nothing about the accounting. Where per-object reclamation is possible the ledger must
`is_balanced()`; for a condemned owner it must instead show **no** `free_of_unknown` /
`unmap_of_unknown` / `double_free` and outstanding objects **only** on the dead isolate —
namespace death is a real disposition, and saying otherwise would be a lie about what was
proven.

Bite-check (each fix reverted individually, everything else in place):

| reverted | what fails, and how |
|---|---|
| the `Worker::execute` foreign-handle gate | **7 of 10** fail. The refusal degrades to `BadHandle` **with a foreign handle in `orphans`** — i.e. the disposal was really attempted on the system connection and the residue is a handle the caller will now try to dispose of again: `left: Err(VerbFailure { err: BadHandle(HostHandle(iso1:0x200000002)), orphans: Orphans { free: [HostHandle(iso1:0x200000002)] } })` vs `right: … ForeignHandle { handle: …, worker_isolate: IsolateId(0) }, orphans: {}`. On real hardware the `BadHandle` does not happen at all — the free succeeds, on a bystander. |
| `VerbPlan::handles()`'s `unmap` half only | exactly 1 fails — `a_foreign_unmap_is_refused_as_loudly_as_a_foreign_free`, with the foreign VAS reaching the backend. A gate that scanned only `free` passes every other test in the file. |
| `plan_publish`'s `SystemDataPlane` refusal | `the_system_proc_has_no_data_plane` fails `left: Err(UnknownPdb { … })` vs `right: Err(SystemDataPlane)` — the right outcome for entirely the wrong reason, which would start succeeding the moment anything gave the system proc a `Vas`. |
| `signal_source`'s `DeviceFatal` branch | `a_system_worker_death_is_device_fatal_not_a_silent_no_op` fails `left: WorkerDied { proc: ProcId(0), … }` vs `right: DeviceFatal { … }` — i.e. reverts precisely to the silent no-op. |
| `Spine::retire_proc`'s `SYSTEM_PROC` guard | `the_spine_refuses_to_retire_the_system_proc_even_when_it_is_in_the_proc_set` fails on *"the system proc is unconditionable — by rule, not by lookup failure"*, and the component is condemned. |

Suite: **247 → 257**, 0 skipped, 0 ignored.

#### ★★ WHAT THIS ROUND DID **NOT** SETTLE — an owner decision, and it is bigger than this brief

The research done to answer Step 1 turned up a foundational problem that this round
deliberately does not touch, because it changes what a `Proc` **is** (decision #14's
grouping rule).

`project()` groups clients into `Proc`s by **dup-connected component**, and the suite
encodes the assumption that UVM's gpu-ops client is *per guest process*
(`rmgraph_order_independence.rs::dup_edge_groups_uvm_and_compute_into_one_proc` — "A+its
UVM, B+its UVM"; the C's own comment says "typically UVM's *per-process* gpu-ops client",
`C: nvkvm_gpu_emul.c:2571`). **The source says otherwise.** UVM creates exactly **one** RM
client for the whole module: `nvUvmInterfaceSessionCreate` is called once from
`uvm_global_init` (`ogkm: kernel-open/nvidia-uvm/uvm_global.c:117`, reached from
`module_init`, `uvm.c:1159-1165`), stored in the singleton `g_uvm_global`
(`uvm_global.h:52`, `:253-255`). Every dup lands in *that* client:
`nvGpuOpsDupAddressSpace` (`.../rmapi/nv_gpu_ops.c:2753-2760`) and `nvGpuOpsDupMemory`
(`:8444-8450`) both pass `session->handle` as the destination, with the **user's** client as
the source; `nvGpuOpsDeviceCreate` allocates device/subdevice under that same client
(`:2265`, `:2283-2286`) and its only callers pass `uvm_global_session_handle()`
(`uvm_gpu.c:1455`, `:1566`). There is no per-`uvm_va_space` client anywhere. The C's own
bench corroborates: *"UVM's RM client (0xc1d00001)"*, singular
(`C: memory/cuctxcreate_800_pinned.md:46,49`).

**Consequence, if the grouping rule is left as-is:** every guest CUDA process is
dup-connected to the one UVM client, so union-find collapses **all of them plus the guest
kernel into a single `Proc`** — one isolate, one arena, one host VAS — which is #14
un-fixed. And the second process would not even get that far: its UVM dup merges a
component that has already touched its data plane, which is `GpuError::LateMerge`, a hard
refusal. Neither is a hypothetical; both are direct consequences of the two cited facts.

**The candidate fix, and why it is not made here.** Type clients as User vs Kernel from a
**declared protocol fact** and let a dup *into* a kernel client be a reference rather than a
merge. The fact exists and is observable on the wire we already parse: RM creates the client
on GSP lazily at first-device alloc via `NV_RM_RPC_ALLOC_SHARE_DEVICE_FWCLIENT`, stamping
`root_alloc_params.processID = KERNEL_PID` (`0xFFFFFFFF`) when
`privLevel >= RS_PRIV_LEVEL_KERNEL` and the real `pClient->ProcID` otherwise
(`ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:68-88`, driven from
`.../gpu/device.c:179-186`), and channel allocs carry `ProcessID` + privilege too
(`.../gpu/fifo/kernel_channel.c:2701-2712`). So the discriminator is declared, not inferred
— which is the only kind this project accepts.

**But making that change flips this round's answer**, and that is exactly why it is an owner
call rather than a guess: with kernel clients no longer merging, the UVM client's dup of a
user proc's memory becomes a genuine **cross-`Proc` reference**, and the question posed at
the top of this entry becomes live in a way it is not today. The good news, and the reason
the work above is not wasted under either rule: the system-plane rule and the handle stamp
hold in **both** worlds — the reference would be a reference to a *guest* memory object,
and as long as the system proc mints no host memory and no handle can be used off its own
isolate, no *host* allocation is ever shared. What a kernel-client split would newly raise
is a **coherence** question (two procs materializing two host backings for one guest
buffer), not a use-after-free — and forging the kernel's data movement instead of forwarding
it is precisely what keeps that question closed too.

**Recommended next round, for the owner to accept or redirect:** (i) confirm the
one-global-UVM-client fact on the bench by logging distinct `DUP_OBJECT` dst clients across
two concurrent CUDA processes; (ii) decide the grouping rule; (iii) only then design the
kernel-client boundary. Until (ii) is decided, `dup_edge_groups_uvm_and_compute_into_one_proc`
and the `LateMerge` guard encode an assumption the source contradicts, and that is recorded
here rather than quietly patched.

### 12.27 ★★★ WHAT A `Proc` IS — the grouping rule, corrected against hardware

§12.26 closed with an owner decision it deliberately did not take: *the suite encodes an
assumption the source contradicts.* The measurement came back, and the source was right.
This entry is the correction, and it is the most invasive change this core has had —
decision #14's definition of what a *process* is.

#### The measurement (RTX 3060, driver 580.159.04, 2026-07-25 — not a hypothesis)

kprobes on RM's dup funnel `rmapiDupObjectWithSecInfo` and on `rpcRmApiDupObject_GSP` /
`rpcRmApiAlloc_GSP`. ★ **The method is reusable and is written up in
`../reference/rm_semantics_measured.md` §3** — `strace`/`LD_PRELOAD` cannot see these (they
are in-kernel RM calls, never a userspace ioctl) and **ftrace refuses the RM core**, which is
compiled `notrace`, so it took a throwaway `register_kprobe` module. Also recorded there:
only **25 of the 82** dups reach GSP, so a rule keyed on the GSP wire must be correct on that
subset alone.

- `nvUvmInterfaceSessionCreate` fires **exactly once per `nvidia_uvm` module load** — one
  RM client for the whole module.
- Two concurrent CUDA processes issued **82 dups each, every one with the same
  destination**: `dst=0xc1d00069`, sources `0xc1d00067` (A) and `0xc1d00068` (B).
- A **third** process run later joined that same destination. The destination changes only
  across a module reload.
- Over the whole trace: dups with the UVM session as *source* = **0**; dups with a user
  client as *destination* = **0**; userspace `NV_ESC_RM_DUP_OBJECT` = **0** (CUDA never
  issues one). A strict one-directional star into the session client.

**Consequence under the old rule** ("a `Proc` is a dup-connected component of clients"):
every guest CUDA process dup-connects to the one UVM client, so union-find collapses **all
of them plus the guest kernel into a single `Proc`** — one isolate, one arena, one host
VAS. #14 un-fixed. And process #2 would not even get that far: its UVM dup absorbs a
component that has already touched its data plane, which is `GpuError::LateMerge`, a hard
refusal. The stale C comment at `C: src/qemu/nvkvm_gpu_emul.c:392` ("UVM's *per-process*
gpu-ops client") is where the wrong reading came from; the C's own bench note about a
singular *"UVM's RM client (0xc1d00001)"* was the accurate one. **That comment is in the
other repo and was not touched.**

#### ★ The discriminator, and the three things it is NOT

`rpcRmApiAlloc_GSP` with `hClass == NV01_ROOT` carries `NV0000_ALLOC_PARAMETERS`:

```
GSPALLOC hClient=0xc1d00067 parm.processID=0x0000dd13   <- process A's pid
GSPALLOC hClient=0xc1d00068 parm.processID=0x0000dd14   <- process B's pid
GSPALLOC hClient=0xc1d00069 parm.processID=0xffffffff   <- UVM session = KERNEL_PID
GSPALLOC hClient=0xc1e0006a..76 parm.processID=0xffffffff  (other RM-internal clients)
```

Stamped at `ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` — `privLevel >=
RS_PRIV_LEVEL_KERNEL → processID = KERNEL_PID (0xFFFFFFFF)`, else the client's `ProcID`.
This is a **declared protocol fact**, so it is *recorded*, never inferred — the house rule
whose violation §12.25 was.

1. **The handle VALUE is not a discriminator.** The UVM session `0xc1d00069` sits
   numerically *between* the two user clients, sharing `RS_CLIENT_HANDLE_BASE`.
   `RS_CLIENT_INTERNAL_HANDLE_BASE (0xC1E00000)` exists and other kernel clients use it —
   **UVM's session does not.** Keying on the range mis-files the single most important
   kernel client in the system. The test file encodes the measured handles verbatim so
   this stays load-bearing.
2. **`processName` is empty in every record.** Unusable.
3. **It cannot be inferred from the dup graph**, which is what it decides about. It arrives
   at client-creation time, on the `NV01_ROOT`, *before any dup exists* — which is exactly
   what keeps grouping order-independent under the new rule as well as the old.

#### ★★ THE RULE

> A `DUP_OBJECT` edge is a **grouping** edge iff **both** endpoints are declared
> `ClientKind::User` clients. Every declared `ClientKind::Kernel` client belongs to the ONE
> reserved **system** component (`project::SYSTEM_ANCHOR`), by rule and never by dup. A
> client that has not (yet) declared merges with nobody.

A dup into a kernel client is therefore a **reference**, not a merge — which is what it is
on the wire. User↔user dups still merge, because that is genuine sharing and genuine
sharing is one blast radius, which is what #14 is about.

Requiring *positive* evidence about both ends (rather than excluding the known-bad shapes)
is deliberate: a future third `ClientKind` is unmergeable until someone decides what it
means, instead of silently defaulting into user grouping.

**Attribution is by ORIGIN.** `RmGraph::nodes()` reports each resource at the handle that
*allocated* it, so a user's VASpace dup'd into the session stays in the **user's** `Proc`;
the kernel component owns only what the kernel itself allocated. This is why the new
cross-`Proc` reference materializes no second `Vas`, and it is the hinge of the coherence
answer below.

#### What a kernel client belongs to, reconciled with §12.26

The system component **is** `Gpu::system`. It is synced by the same
`sync_proc_to_boundary` as any user proc (a guest-kernel channel must materialize by
exactly the same rules), and it is deliberately outside every lifecycle verb of the
refresh loop: not minted (it exists from realize), not matched or merged (its membership is
the declared kind, not a client intersection), not retired by the vanish pass, and — §12.26
— **not condemnable**, because condemning the guest driver's own component is device-fatal
by definition. `SYSTEM_ANCHOR` resolves to `Gpu::SYSTEM_PROC` by name in the routing
rebuild, so a guest-kernel PDB routes.

That last point turns §12.26's `plan_publish` refusal from a vacuous guard into a
load-bearing one: before this change `Gpu::system` owned nothing, so
`the_system_proc_has_no_data_plane` passed for the wrong reason (`UnknownPdb`). Now the
system proc has clients, a `Vas`, and a routable PDB — and `FwdFault::SystemDataPlane` is
what actually refuses. §12.26 anticipated exactly this ("would start succeeding the moment
anything gave the system proc a `Vas`"); the replacement test asserts the refusal *with*
the `Vas` present.

**`HClient(0)` is now RESERVED.** `SYSTEM_ANCHOR` is `ProcAnchor(HClient(0))`, and a
`ProcAnchor` is a client handle, so a guest that declared client 0 could anchor a *user*
component at the reserved label and have its PDBs and vChids resolve onto the system proc.
`RmGraph::apply` therefore refuses **every** event naming client 0
(`RmGraphError::ReservedClient`), enumerated centrally in `clients_named` so a new
`RmEvent` variant carrying a client is a mistake in one place. This is
protocol-faithful: `0` is `NV01_NULL_OBJECT`, and RM mints client handles from
`RS_CLIENT_HANDLE_BASE`. The reservation is a *fact*, not a hope.

#### Missing `processID` — MISS = FAULT at the declaration

A `Client`-class alloc with no declared `ClientKind` is a loud
`RmGraphError::UndeclaredClientKind`, refused at the declaring event. Both available
guesses are catastrophic and the doctrine forbids either: "user" folds the guest kernel's
session into a process's blast radius (and through it every other process — #14 un-fixed);
"kernel" folds a guest process into the guest kernel's isolate. RM stamps `processID`
unconditionally, so an absent one means the ABI seam failed to decode it — a defect that
must be loud where it happens, not papered over with a default (the deleted `unwrap_or(0)`
disease, one level up).

Two corollaries, both refusals rather than tie-breaks:

- **One root per namespace.** A second `Client`-class origin in one namespace is
  `RmGraphError::DuplicateClientRoot`. In RM the `hClient` **is** its root object's handle,
  so this cannot happen on a real driver; allowing it would let a guest declare two roots
  with *different* kinds and make the classification order-dependent.
- **The index tracks the root HANDLE, not the resource** — §12.25's lesson one level up. A
  client root that a dup alias keeps alive in another namespace no longer occupies its own
  namespace, which is free to declare a fresh root with a fresh kind. Pruning on the
  resource's death instead leaves the namespace permanently un-declarable, and then prunes
  the *new* entry when the old alias finally dies. The client-root index (`client_roots`)
  is an index and not a second source of truth; it exists for **complexity**, not
  convenience (G10, §12.22: the one-root check and the per-client kind lookup would
  otherwise be O(clients × resources) with both factors guest-driven).

**An undeclared, resource-less dup endpoint conjures nothing.** It is not admitted to the
client universe until it declares — otherwise it mints a phantom, resource-less `Proc` that
is matched and then RETIRED the instant the declaration lands, which is exactly the
"intermediate state differs from the fully-applied one" the parked-dup rule already
refuses. A client that owns live resources but has not declared still gets its own
component (see the lifetime property below); it merges with nobody.

#### ★★ The coherence re-verification — it holds, and it is now non-vacuous

§12.26 argued the new cross-`Proc` reference raises a **coherence** question, not a
use-after-free, *because* the system proc mints no host memory and forges kernel data
movement. **That argument survives the rule change, and it got stronger.** Re-derived under
the new grouping:

1. **The system proc still mints nothing.** `plan_publish` refuses `Gpu::SYSTEM_PROC`
   before any host verb exists. Now non-vacuous (above).
2. **A dup'd object materializes no second `Vas`.** Attribution is by origin, so the
   session client's alias of A's VASpace does not appear in the system component at all.
   There is one `Vas` per `(GpuId, Pdb)`, owned by the allocator's proc, and `by_pdb` maps
   a PDB to exactly one proc — a second proc *cannot* publish into it (`UnknownPdb`). The
   "two host backings for one guest buffer" shape is unrepresentable, not merely avoided.
3. **A host handle still names its isolate** (§12.26) and is refused off it.
4. **GPA arenas are still per-`(Proc, GpuId)` and disjoint**; a guest-declared mapping into
   a system `Vas` naming a user proc's GPA binds `host: None`, and `gate_vas` refuses to
   execute against an unpublished binding — the guest can *declare* the alias and can never
   *use* it.

★ **And the property that answers it structurally, which was undocumented:** *any genuine
cross-process sharing must traverse a **user↔user** dup edge to be usable at all, and that
edge is exactly the merge.* For process B to execute against A's address space, B's channel
must name a VASpace **in B's namespace** — i.e. an alias of A's VASpace dup'd into B's
*user* client — and that dup merges B into A's `Proc`: one isolate, one arena, one backing.
If no such user↔user dup exists, B cannot reach A's address plane at all and any attempt is
a loud MISS. So user↔user sharing is correct *by construction*, by the same property that
made condemnation-by-client-set work.

#### ★ The lifetime question, sharpened — and the answer the model already had

The owner's question was whether reclaiming host RM objects when an isolate dies is
*faithful* (RM invalidating shared objects when the creator dies) or *divergent*. Checked
in `ogkm`, the answer is **divergent in principle**: RM does not invalidate on creator
death, and UVM's `uvm_va_space` is bound to the `/dev/nvidia-uvm` **file**, not to the
process — `kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81` says so explicitly ("*it's legal
for the associated process to die then for another process with a reference on the file to
perform the unregisters*"), `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` promises resources
"*freed when the last reference to the file is dropped rather than when this process
exits*" (`uvm.h:160-167`), and "zombie" ranges exist precisely for that case
(`uvm_va_range.h:265-268`, reaped by `UVM_CLEAN_UP_ZOMBIE_RESOURCES`). External allocations
(`uvm_map_external.c`) and device-P2P dups (`uvm_va_range_device_p2p.c:352-371`) are the
same shape. So **a kernel client's reference to a user process's object CAN outlive that
process.**

That would be a use-after-free if a `Proc`'s host lifetime were keyed on its client root.
**It is not, and this falls out of the origin-attribution rule for free:** a component is
derived from the resources that are *live*, reported at their allocator's key. So as long
as any surviving reference — including the session's dup — names a resource client A
allocated, A's boundary still exists, the same `Proc` survives the match, and its isolate,
arena and published host backing are untouched. The isolate outlives the guest client for
exactly as long as RM's own refcount says the object does; the LAST reference going is what
retires the proc. Asserted end-to-end by
`a_kernel_dup_keeps_the_owning_procs_isolate_and_backing_alive_past_the_clients_free`.

Note the creation-side bound that keeps this small: `nvGpuOpsDupAddressSpace`/`DupMemory`
pass `NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` (`nv_gpu_ops.c:2759`, `:8490`),
forcing the `RS_SHARE_TYPE_PID` check (`client_resource.c:219-231`), so the memory duped
into the session always belonged to a client owned by the *calling* process at dup time.
Nothing forces that same process to drop the last fd reference — which is the residual, and
it is a *functional* gap (a loud refusal on a shape we do not serve), never corruption.

#### ★ Where NVIDIA declines to define behaviour (recorded, not designed against)

Mode-1 could ignore these because it was a syscall proxy trying to be correct everywhere.
Mode-2 **reconstructs intent**, so these mark where we should be conservative rather than
inventive — MISS = FAULT, declare the refusal, never invent a semantic NVIDIA never
promised. Collected for a later round:

| Where | What it declines |
|---|---|
| `ogkm src/nvidia/src/kernel/rmapi/client_resource.c:219-231` + `sharing.c:344-353` | `RS_SHARE_TYPE_PID` is the default `DUP_OBJECT` policy: cross-PID dup denied unless opted out. |
| `libraries/resserv/src/rs_client.c:537-547`, `sdk/nvidia/inc/nvos.h:2277` | A kernel client dups *unconditionally* unless `REJECT_KERNEL_DUP_PRIVILEGE` is set. |
| `arch/nvalloc/unix/src/osapi.c:2540-2542`, `kernel-open/nvidia/nv-mmap.c:542-549` | An mmap context may only be armed by the process owning the RM client; an inherited fd carries none. |
| `arch/nvalloc/unix/src/osapi.c:552-557` | RM client lifetime is keyed to the **file**, not the process. |
| `arch/nvalloc/unix/src/escape.c:962-994` | `NV_ESC_REGISTER_FD` links are permanent and unshareable. |
| `kernel-open/nvidia-uvm/uvm.c:93-101`, `:782-788` | Once the UVM fd is shared, the kernel declines to track mm ownership; a second process cannot mmap through it (`-EOPNOTSUPP`). |
| `kernel-open/nvidia-uvm/uvm.h:2688-2692` | **"undefined behaviour"**, verbatim, for a non-duplicated UVM handle across processes. |
| `kernel-open/nvidia-uvm/uvm.h:541-551`, `:1145-1148` | `NV_ERR_NOT_SUPPORTED` / `NV_ERR_PAGE_TABLE_NOT_AVAIL` when a GPU VA space was registered by a different process, or that process exited. |
| `kernel-open/nvidia-uvm/uvm_va_space.c:1557-1559`, `uvm_user_channel.c:132-140`, `uvm_map_external.c:989-993` | `rmCtrlFd` is **explicitly discarded** (`TODO: Bug 1624521`); UVM relies entirely on RM's PID check. |

#### Where the canonical record of a host object should live — SCOPED, not started

The owner's central-registry proposal, evaluated against Mode-1's actual code rather than
its shape. **The mechanism does not transfer whole, and the part that does is bookkeeping.**

- In Mode-1, the decoupling is **literally true for fd-backed objects**: the stub opens
  `/dev/nvidia*` and hands QEMU an `SCM_RIGHTS` copy, so QEMU owns an independent kernel
  reference to the same `struct file` (`C: src/qemu/nvkvm_handle.h:4-7`, `:26`;
  `nvkvm_isolate_handlers.c:228-256`). QEMU `mmap`s and `ioctl`s it directly (`:1818`,
  `:1068`), an isolate kill never touches the global table (`ARCHITECTURE.md:93-97`;
  `nvkvm_isolate_kill` at `nvkvm_isolate.c:1009-1112`), and the RM objects are released
  only by QEMU's own `close` (`virtio_nvgpu.c:128-129`). This is exactly why the #80
  **reaper** had to be written: isolate death did *not* free the GPU memory.
- It is **bookkeeping only for RM handles**. Mode-1's isolate path keeps nothing but a
  grow-only `uint32_t client_allow[]` reach-gate (`C: virtio_nvgpu.h:297-300`); the rich
  `nvkvm_client`/`nvkvm_object` graph is reached only from the legacy non-isolate frontend,
  and the four-table `nvkvm_tables.c` with its `isolate × handle → fd` map is **unwired**
  (`C: docs/REFACTOR_PLAN.md:203-223`, gaps G3/G5). Cross-isolate work is *re-issued in the
  owning isolate*, never centrally resolved (`nvkvm_isolate_handlers.c:951-965`,
  `:1576-1584`). gVisor's `nvproxy` tables are likewise pure bookkeeping — and it has no
  separate isolate process at all, so the question does not arise there.
- **Security is tied to the CREATOR.** Moving allocation to the main process would collapse
  the privilege boundary the isolate exists for. So a central record can never mean central
  *allocation*, and therefore — in Mode-2, where the host RM object physically lives inside
  the isolate's RM client — a central record gives **enumeration, ordering and an
  independent cleanup path, but not lifetime**. The only way to buy lifetime is the Mode-1
  trick of the main process holding a real dup'd device fd, which puts the raw GPU fd back
  in the main process; that is a posture change, not a refactor.

**Does this round need it? No — and that is a finding, not a dodge.** Under the rule above,
the surviving cross-`Proc` reference is to a *guest* RM resource, and the origin-attribution
rule already keeps the allocator's isolate and backing alive for exactly as long as that
reference lives. Nothing needs to outlive an isolate, so the tension with the system-plane
rule does not become real.

**Scoped as its own round (do not start it here):** today the record of a host object is
distributed across `Binding.host`, `Channel.host_channel`/`host_token`/`host_engine_objects`
and `Vas.host_vas`, per-proc. Audit gap G1 — a successful publish's host memory handle
recorded nowhere, so nothing could ever free it — was this in miniature, fixed one binding
at a time. A central registry **at the layer where OS calls are introduced** (the L1 shell,
not the pure core), keyed by `IsolateId` and enumerable per proc, would make enumeration,
cleanup ordering and orphan detection structural rather than per-case. The grouping rule
lands first; this is recorded so that when it is designed, it is designed as bookkeeping
with an honest name — not as a lifetime mechanism it cannot be.

> ★ **DECIDED (2026-07-26): Option A, record-only** — `l1_os_shell.md` §7.8.1, decision #47.
> The measured basis for refusing the lifetime variant (a dup'd control fd is a **capability**;
> RM gates on the file and the uid, never the pid) now has a citable home at
> `../reference/rm_semantics_measured.md` §8 rather than living in this entry's argument.

#### Tests — `rmgraph_order_independence.rs`, rewritten to reality (15), all bite-checked

The file's `scenario()` used to give A and B **two distinct UVM clients**
(`HClient(0xA1)`, `HClient(0xB1)`) and `dup_edge_groups_uvm_and_compute_into_one_proc`
asserted on it. That scenario cannot occur; it is replaced by the measured one — A
(`0xc1d00067`), B (`0xc1d00068`), ONE kernel session (`0xc1d00069`) both dup into, plus a
second *user* client of A joined by a genuine sharing dup, which keeps the union-find path
and its minimum-anchor tie-break under test now that the UVM edge no longer merges.

New/rewritten coverage: two user `Proc`s from one shared kernel client; full runtime
isolation (own isolate, disjoint arenas, no shared host handle, both publish the identical
VA and ring their own channel); order-independence of the kernel *declaration*
(before/between/after — all identical); a third process joining later disturbing neither of
the first two; **no `LateMerge` for this shape** (and a user↔user peer dup onto a touched
proc still earning one); `HClient(0)` refused on every event shape; undeclared/duplicate
client roots refused with exact variants; the dup-kept-alive root freeing its namespace;
the system proc holding clients + a `Vas` and still refusing its data plane; and the
lifetime property above. `fuzz_rmgraph_invariants` gained INV6 (the system component is
**exactly** the declared kernel clients; no user boundary holds one, none takes the system
anchor) and its generator now emits all three declarations, including a hostile process
claiming kernel privilege — whose worst outcome is self-denial, since the system component
has no data plane.

Bite-check (each guard reverted individually, everything else in place):

| reverted | what fails |
|---|---|
| the grouping predicate (kernel dups merge again) | **9** fail, incl. `one_kernel_client_two_processes_stay_two_procs`, `two_processes_sharing_one_kernel_client_stay_fully_isolated`, `a_second_process_joining_the_shared_kernel_client_is_never_a_late_merge` — i.e. #14 collapse *and* the round-2 refusal, both reproduced. |
| the kernel→system assignment (`anchor_of`) | **10** fail across `rmgraph_order_independence` + `determinism`. |
| `UndeclaredClientKind` | `an_undeclared_or_doubly_declared_client_root_is_a_loud_refusal`, on the exact variant. |
| `ReservedClient` | `client_handle_zero_is_refused_so_the_system_anchor_cannot_be_squatted`. |
| `DuplicateClientRoot` | that same test **and** fuzz `a4_dup_object_is_reference_counted` (the independent refcount tracker mirrors the refusal). |
| client-root index pruned on resource death instead of handle drop | `a_root_kept_alive_by_a_dup_no_longer_occupies_its_own_namespace` + fuzz `a4`. |
| step 2s (the system proc's boundary sync) | **3**, incl. `the_system_proc_has_clients_and_a_vas_and_still_no_data_plane` and the whole-core determinism snapshot. |
| `SYSTEM_ANCHOR` → `SYSTEM_PROC` routing | `the_system_proc_has_clients_and_a_vas_and_still_no_data_plane`. |

Suite: **257 → 269**, 0 skipped, 0 ignored.

---

### 12.28 ★★ THE RETRY LEDGER — §12.9's re-plan was safe by ASSUMPTION, now by measurement

§12.9 settled the *policy*: converging staleness re-plans (bounded), divergent staleness
refuses. The owner's warning on it was exactly right — *"the retries are genuinely good,
but watch out if the action that was performed already had side effects"* — and the
mechanism does address it: a converging `Refusal` carries `Orphans`, and
`SharedDevice::verb_op` disposes of them on the same worker, still lock-free, **before**
the retry.

**What had never been tested is the ledger across N attempts.** `Stale::Rebound` — the one
variant the retry path exists for — had **zero test coverage anywhere in the workspace**
before this pass (a grep for it hit only `kayfabe-fwd`'s own source). "Retry is safe" was
an assumed claim, and this campaign has been a sequence of assumed claims turning out to be
half-true.

`tests/retry_ledger.rs` (3 tests) makes it measured. Every attempt in it has **already
allocated real host objects** — a host VAS, a host memory object, a host GPU mapping —
before it loses its race, and every ordering edge is a `VerbHold` latch (no sleeps, no
timing).

| test | what it forces | what it asserts |
|---|---|---|
| `converging_retries_release_every_host_object_they_allocated` | 4 publishers, every one held inside `map_gpu_va`, so **all four plan against `host_vas == None`** before any commits; 1 wins, 3 converge and re-plan | **Outstanding(ledger) == Reachable(core)**, objects *and* mappings, in both lock modes. Not a verb count — a set equality: an object that exists and that no `Vas`/binding/channel can name *is* the definition of a leak |
| `the_commit_retry_bound_still_releases_the_attempt_that_hits_it` | the `(GPU, PDB)` `Vas` is torn down and re-won under the parked publisher on **every** round, `MAX_COMMIT_RETRIES` times | the op ends in `Stale::Rebound` (never a spin, never a success), and the outstanding set equals exactly what the *script* abandoned — i.e. the final, refused attempt released its own allocation. The bound changes what the caller is told, not what the host is left holding |
| `a_retry_whose_replan_diverges_refuses_without_leaking_the_attempt` | attempt 1 converges and re-plans; the proc retires while attempt 2 is lock-free in flight | the fault is `Stale::Proc` **on the exact proc** (not the previous attempt's `Rebound`, not an anonymous `Rm(..)`), and the residue is stated: a retired isolate refuses the disposal too, so the objects are the §12.16 G4 residue whose disposition of record is the isolate session's death |

`MAX_COMMIT_RETRIES` became `pub` for this — the bound-hitting test must drive exactly as
many rounds as the bound permits (one too few never reaches it; one too many waits on an
attempt that is never made). It is documented as *not* a knob.

**Bite-check** — the orphan release on the converging path deleted (`dispose_on` → drop),
everything else in place:

| test | observed |
|---|---|
| `converging_retries_release_every_host_object_they_allocated` | FAILED. Outstanding **11**, reachable **5** — exactly **2 extra per losing attempt** (3 losers × {duplicate host VAS, memory object}), each named by handle in the diff |
| `the_commit_retry_bound_still_releases_the_attempt_that_hits_it` | FAILED. Outstanding **25** vs expected **16** — **9 extra**: the first attempt's duplicate host VAS plus **one memory object per retry**, all 8 of them |
| `a_retry_whose_replan_diverges_refuses_without_leaking_the_attempt` | still passed, correctly: its disposal is *already* refused by the retired isolate, so removing the disposal changes nothing. The three tests are independent claims, and this one says so |

**No leak was found in the shipped code.** The converging path's ledger balances exactly,
including at the bound. That is the result the test was written to be *able* to falsify.

### 12.29 ★ POOL SATURATION was correct and INVISIBLE — a saturated pool looked exactly like a hang

The bounded worker pool is right and stays right (§7.2's calibration: RM serialises every
ioctl-reachable path on the per-client write lock, so the pool buys **liveness isolation**,
not throughput — which saturates at a handful of workers; the default is 4, re-anchored on
that and not on the vCPU count). What was missing is *observability*: from outside the
process, "every worker is in flight and three guest threads are queued" and "the device is
wedged" present identically — guest threads stop finishing, and nothing anywhere says why.

`PoolGate` now carries per-target `PoolWaits`, surfaced by `SharedDevice::pool_waits()`:

- `saturated` — checkouts that found the pool full (counted even when the wait turns out
  free: the pool *was* full when the plan phase asked);
- `parked` — of those, the ones that actually blocked (`saturated - parked` = near misses);
- `peak_waiters` — **how long the queue got**, the number that says whether the pool is
  merely touched or genuinely the constraint;
- `waiting` — depth right now.

Three deliberate choices:

1. **Counted, never timed.** There is no clock in the core (§8.3), and a duration would
   measure the verb behind the queue rather than the queue.
2. **Keyed by `GpuId`, not `(ProcId, GpuId)`.** A `ProcId` is monotonic and never reused, so
   per-proc keying is an unbounded map a guest can grow for free — a boundary-1 hazard
   inside a *diagnostic*. The target set is bounded by the device's entitlement.
3. **`pool_waits()` takes only the gate's own mutex**, never the device lock — it has to be
   callable while every guest thread is blocked, which is the entire situation it exists
   for.

Tests: `pool_saturation_is_counted_so_it_is_distinguishable_from_a_hang` (one worker, one
holder, **two** waiters — pins the depth, not just the fact; and pins `waiting` returning to
0, because a diagnostic that leaks a waiter would report permanent congestion on a healthy
device) and `an_unsaturated_pool_reports_nothing_at_all` (two publishers against four
workers cannot saturate even through a converging re-plan, so the map stays empty).

**Bite-check** — the counter updates removed from `wait_for_return`:
`pool_saturation_is_counted_so_it_is_distinguishable_from_a_hang` FAILED with
`bounded wait exhausted after 20000 polls: both requesters parked on the pool gate`. The
other 20 tests in the file stayed green, which is the point: nothing else in the suite can
see saturation at all.

### 12.30 ★★ THE MISS AUDIT — MISS=FAULT had an undeclared second answer, and it was right

MISS=FAULT is founding and stays. But the codebase has always had a second answer, and
having it is correct: a `MapMemoryDma` that arrives before its VASpace's
`SET_PAGE_DIRECTORY` is **deferred**, not faulted, because the guest legitimately maps
before it binds a page directory. So the real rule is a two-way split that was being decided
site-by-site by whoever wrote the site — and undocumented judgement is what drifts.

The split is now declared, in `kayfabe-core`'s crate docs and at each site:

- **not yet knowable ⇒ DEFER** — the fact may still arrive, the guest is not wrong;
- **never knowable ⇒ FAULT** — MISS=FAULT proper.

Three properties are load-bearing and are stated there: (1) **the category belongs to the
SITE, not to the absence** — the same missing fact defers in derivation and faults at use,
and the deferral is what makes the fault *exact*; (2) **a DEFER must be recoverable by a
fact arriving** (every deferring site is re-evaluated on the next `apply`; a deferral with
no re-evaluation path is a hang); (3) **getting it wrong is asymmetric in opposite
directions** — a FAULT that should defer is a hung or spuriously-refused guest (§12.9's own
lesson: the first symptom was a *hang*), a DEFER that should fault is a security question.

**The inventory** (category → why), audited across `rmgraph.rs`, `project.rs`, `gpu.rs`,
`kayfabe-fwd` and `kayfabe-mmu`:

| site | category | why |
|---|---|---|
| `RmGraph::client_root_of` / `client_kinds` | DEFER | an object may legally precede its client root (order tolerance, #4) |
| `RmGraph::resource_of` / `origin_of` | DEFER | `Dup`-before-`Alloc` is legal; `resolve_pending_dups` promotes later |
| `RmGraph::origin_of_kind` | **fused** — DEFER + a never-knowable FAULT | see finding **B** |
| `RmGraph::pdb_of` | DEFER | `SET_PAGE_DIRECTORY` may arrive (or be parked) after the alloc |
| `RmGraph::walk_gpu` / `gpu_of` | DEFER | no `Device` ancestor *yet*; the instance-less-Device arm is unreachable (`deviceId` is required by `NV0080`) — see finding **A** |
| `RmGraph::backing_of` | **split by the caller** | "unobserved" defers; "not a Memory" / "no declared backing" are never knowable and surface as `UnbackedMapping` |
| `project::resolve_vaspace_handle` / `resolve_channel_vas` | DEFER here, FAULT at use | inherits `origin_of_kind`'s fusion |
| `project`: parked dup edge skipped | DEFER | turning a transient parked dup into a refusal would make the same facts in a different order produce a different end state |
| `project`: `is_user(dst) && is_user(src)` | DEFER on undeclared ends | grouping needs positive evidence about BOTH sides; absence is never read as "user" |
| `project`: `VasFacts.gpu` / `.pdb`, `ChannelFacts.gpu` / `.vas_origin` | DEFER | not routable *yet*; never a default-GPU0 guess |
| `project`: `by_pdb` / `by_vchid` inserted only for a resolved target | DEFER | an unroutable object enters no routing map; its use faults by name |
| `project`: `PdbCollision` / `VchidCollision` | FAULT | not a miss — an ambiguity, and hostile ambiguity is the F1 guard's whole job |
| `Gpu::sync_rpc_mappings`: `m.pdb == None` | **DEFER** | ★ THE canonical exception the taxonomy is written around |
| `Gpu::sync_rpc_mappings`: `gpu_of(vaspace) == None` | DEFER | same, on the multi-GPU axis — deferring is what keeps GPU0 from being guessed |
| `Gpu::sync_rpc_mappings`: `m.mem_phys == None` | **FAULT** (`UnbackedMapping`) | a backing is an alloc-time fact; an unbacked memory stays unbacked |
| `Gpu::sync_proc_to_boundary`: vas/channel with unresolved target | DEFER | materializes nothing, re-evaluated next apply, `ChanId` slot kept stable |
| `Spine::plan_refresh`: `LateMerge` / `CondemnedMerge` / arena exhaustion | FAULT | decided before any proc is touched (§12.18) |
| `fwd::route_pdb` / `route_doorbell` / `route_engine_object` | **FAULT** | *use* sites: `UnknownPdb` / `UnknownVchid` / `MalformedToken` / `NotAnEngine`, with §12.13's `Condemned` split where it applies |
| `fwd::resolve_in` | FAULT ×2 | unknown `(target, pdb)`; unbound VA |
| `fwd::gate_working_set_in` | FAULT ×4 | incl. `chan.vas_pdb == None` — the same absence `sync_proc_to_boundary` **defers** on. At ring time there is no "later" |
| `fwd::plan_publish` / `plan_doorbell` / `plan_engine_object` / `plan_control` | FAULT | `RetiredProc`, `SystemDataPlane`, `UnknownPdb`, `NoVas`, `NoTarget` |
| `fwd::checkout` → `Ok(None)` | **DEFER — with the CALLER choosing** | "no worker" *will* change; a caller that can wait parks (and is now counted, §12.29), one that cannot gets `PoolSaturated` |
| `fwd::commit_*` → `Refusal { retry: true }` | **DEFER at the commit seam** | §12.9's converging staleness — bounded, because a defer must terminate |
| `fwd::commit_*` → `retry: false` | FAULT | divergent: nothing that can arrive brings the target back |
| `AddressTable::resolve` | **FAULT, no deferring case exists at this layer** | the table IS the guest's TLB; a TLB has no "later" |
| `AddressTable::unbind` → `None` | FAULT at the caller | `unpublish_backing` refuses: the arena must never accept a range it does not owe |
| `SourceRegistry::dispatch` | FAULT (`SourceFault`) | handles are never reused, so a miss is never a stale-alias guess |

**Findings.** No site required a *behaviour* change — every category in the shipped code is
defensible, which is itself the audit's most useful result. Three **documentation** defects,
two of them stating the opposite of their own code:

- **A — `RmGraph::gpu_of` documented the pre-G9 behaviour.** Its comment read *"A Device
  that declared no `deviceInstance` is the single-GPU default selection … routing to
  `GpuId::ZERO`"*. False since §12.21: `walk_gpu` returns
  `node.facts.device_instance.map(GpuId)`, i.e. `None`. A public resolver's doc asserting a
  default-GPU0 guess, inside the one doctrine ("never guess GPU 0") the change existed to
  enforce. **Fixed** (the comment now states the correction and why it matters).
- **B — `RmGraph::origin_of_kind` fuses two categories under one `None`.** "The handle does
  not resolve" is DEFER; "it resolves, to an object of the **wrong kind**" is never knowable
  — no future event turns a TSG into a VASpace — and is a *hostile-input fact* (a guest
  naming a TSG as its `hVASpace`). The end state is safe either way (both make the object
  unroutable and its use faults loudly), so nothing downstream is wrong; what is lost is the
  **distinction**, and only one of the two is security-relevant. This is the same shape
  §12.13 decided was worth splitting for condemned-vs-unknown routing misses. Splitting it
  means a `Result`-shaped resolver threaded through `project` — a design change, not a
  documentation pass. **Recorded and documented at the site; deliberately not changed.**
- **C — `Gpu::sync_rpc_mappings`'s doc contradicted its own body.** It claimed *"MISS=FAULT
  is preserved — a mapping with no resolvable PDB or backing is a loud fault, never a silent
  skip"*, while the body `continue`s on both an unresolved PDB and an unresolved target
  (correctly). The body was right and the sentence was wrong — and it was wrong about the
  single most-cited exception in the codebase. **Fixed**, with the three-way split spelled
  out.

(Minor, out of scope, noted in passing: `RmRecorder::hold`'s doc says holds are "matched
newest-spec-first"; the code uses `Vec::position`, i.e. **oldest** first. Mock-only, and the
new tests rely on the FIFO order the code actually has.)

### 12.31 ★ "NEVER BUSY-POLL" was the wrong rule — the testable one is "every poll must be provably BOUNDED"

F1 was stated as *never busy-poll anywhere*. Two things are wrong with that.

It is **wrong on the merits**: a short spin on something that completes in microseconds is
fine and routinely beats a syscall — `std::sync`'s own mutexes spin before parking — so the
rule as written forbids the fast path along with the bug. What actually went wrong in the C
was **unboundedness**: a guest loop with no ceiling, each iteration a nested-virt vmexit,
~40k exits per phase.

It is also **untestable**, and the mutation gate already showed exactly where that bites.
Three survivors in the L1 campaign degrade the pool's backpressure from *parking* to
*spinning*; every one stays green, because "no polling" is an **absence** and a test cannot
observe an absence. That blind spot was recorded as "uncomfortable"; it is better read as
the rule being mis-stated.

Restated, in §4.2 (normative) and cross-referenced from §1's F1 row, `l1_os_shell.md` O4 +
the gate table, and `l1_architecture_summary.md`'s mutation section and D8:

> **Every poll must be provably BOUNDED.** A poll is legal iff its bound is *stated* and the
> bound is *asserted*. The testable consequence: **assert a bound, not an absence** — count
> the events a wait can generate and assert the count (the reactor's wake count against the
> signal count; the pool gate's wait count against the saturation events that caused it,
> §12.29). A poll with no such counter is not merely unmeasured: it is the rule's violation,
> because an unstated bound is an unbounded one.

`epoll_wait`, condvars and deadlines remain the default *because* they are bounded by
construction (one wake per signal) — a spin now needs an argument rather than being
forbidden by fiat. D8's forbidden periodic sweep is forbidden for a sharper reason than
before: its iteration count is a function of uptime, not of outstanding work, so it has no
bound at all, while a backstop armed only while completions are outstanding does.

No existing test asserted "no polling", so nothing had to be restated; the first assertion
in the new form is §12.29's saturation counter, and the second is the `await_bounded` helper
in `l1_verb_seam.rs`, whose ceiling is stated in one place and whose exhaustion is a named
failure rather than a wedge.

Suite: **269 → 274**, 0 skipped, 0 ignored.

### 12.32 ★★ L1-M2 STAGE M2-a — THE LEDGER COMPOSED, T0/G2 MEASURED, AND A BUG THE FIX ITSELF INTRODUCED

`l1_os_shell.md` §10 puts M2-a first on purpose: **build the measuring instrument, take an
honest baseline, and only then build what fixes it.** This is what that produced.

**The ledger is now part of THE mean test, not beside it.** `kayfabe_mocks::HostLedger`
existed and was used by two focused suites; it was not part of the composed
multi-proc × multi-thread × multi-GPU × multi-workload run. It is now: `mean_run` takes a
census at quiesce and `l1_mean.rs` asserts it in **both** lock modes. The form is the
strongest one §12.28 proved out — **`Outstanding(ledger) == Reachable(core state)` as a set
equality, not a count** — because an object that exists and that no `Vas`, binding or
channel can name *is* a leak even when the totals agree. `reachable_objects` /
`reachable_maps` moved into `kayfabe-tests` so the two suites share one definition.

The census splits three ways, and the split is the point:

| class | meaning | asserted |
|---|---|---|
| **LEAKED** | outstanding on a **live** proc, unreachable from core state | **== 0** — §7.0's backstop does not apply, so nothing will ever free it |
| **DANGLING** | core state names it, the ledger says it was released | **== 0** — a use-after-free shape, strictly worse than a leak |
| **namespace-death residue** | outstanding on an isolate whose `Proc` was reaped | reported and pinned at its exact value (6 objects, 2 mappings); disposed of in bulk by the session's death (§7.0) |

Plus R6's intra-arena half, which no ledger of *handles* can see: **GPA bytes a live proc's
arena still has handed out that no `Vas::blocks` token can name.** That is the class §7.8
flagged as "unfixable without a core change", and it is measured here rather than argued.

**★ The first baseline read ZERO — and that was a true negative, not a pass.** The mean
script's two existing subset-frees (`P_CHANFREE`, `P_REROUTE`) target channels phase 0
deliberately leaves **virgin**, and whose held `AllocChannel` commit refuses and orphans. So
`retain` was dropping `Channel`s holding `host_channel: None`: nothing to leak. The script
had never reached T0 at all. Adding `t0_churn` — a live proc declaring a VASpace, publishing
into it, declaring a channel, ringing it, forwarding an engine object onto it, then freeing
**both** while it keeps running, three rounds each on two procs across two GPUs, composed
into the same window as the parked verbs and the workload threads — produced the honest
baseline:

| baseline (per run, IDENTICAL in both lock modes) | value |
|---|---|
| leaked host **objects** | **24** — 6 host VAS + 6 sysmem + 6 channel + 6 engine object |
| leaked host **mappings** | **6** |
| leaked **GPA** | **24 576 bytes** (12 288 per proc) |
| dangling | 0 |
| double-free / free-of-unknown / unmap-of-unknown | 0 |
| namespace-death residue | 6 objects, 2 mappings |

Exactly **4 objects + 1 mapping + one 4 KiB block per subset-free** — linear in the number
of frees, which is what "a training job's steady state" means. `host_token` correctly
contributes nothing: it is not a handle.

**The fix, in the shape §7.6 T0 names.** A `pending_release: BTreeMap<GpuId, Orphans>` on
`Proc`, **filled before the `retain`** by `stage_dropped_vases` / `stage_dropped_channels`
(unmaps first, then memory objects, then the host VAS; engine objects before their channel —
RM's children-before-parents order, `ogkm: rs_client.c:830-849`, `rs_server.c:963-981`), and
**drained lock-free** on a checked-out worker via `Orphans::release_plan`. Keyed by `GpuId`
because a handle only exists inside its own isolate's namespace. Two drain sites, one
mechanism: `kayfabe_fwd::checkout_and_drain` (opportunistic — the worker is checked out
anyway) and `SharedDevice::drain_pending_releases` (the backstop sweep for a proc that goes
quiet). The **GPA half runs under the device write lock and that is correct** — returning a
`GpaBlock` to a `GpaArena` issues no verb, so R1 does not apply to it; only the host disposal
has to wait for a worker.

**★★ And the fix introduced a real bug, which a COMPOSED script caught immediately.** The
first version drained on *every* checkout. `retry_ledger.rs` wedged on its next run, and the
cause was ours: a publisher parked **inside its mapping verb**, holding a host VAS, when the
guest freed that VASpace — the drain then freed the VAS underneath the parked verb, which
came back `RmError::BadHandle`. **Our own reclamation had become a use-after-free**, and it
surfaced to the guest as an anonymous host error rather than as staleness (§12.10's polarity,
one layer over).

The guard is the predicate the reap already uses for the identical reason (§12.16 G3):
`Isolate::is_quiesced`. `Proc::checkout_with_pending_release` reads it **before** its own
checkout and takes the queue only if the isolate was otherwise idle — one indivisible act,
because splitting it is how that ordering gets got wrong later. It is *sufficient*, and that
is a property of the plan/execute/commit shape rather than a hope: a plan checks its worker
out as the last thing it does under the lock (§7.3), every path that returns a worker
disposes of its own orphans **before** the check-in, and an op that plans *after* the fill
cannot name the dropped objects at all because `retain` removed them from core state.

The general rule, worth stating once: **per-object reclamation must never race an in-flight
verb, and no lock can exclude one — only the isolate's own quiesce predicate can.** Getting
it wrong is asymmetric exactly as `is_quiesced`'s docs already say: too early is a
use-after-free, too late leaves the queue for the next op.

**Bite-checks** (revert, observe, restore):

| reverted | observed |
|---|---|
| the two `stage_dropped_*` calls (the fill) | `l1_mean` FAILED with `(24, 6, 24576)` vs `(0, 0, 0)`, every leaked handle named; **5 of 6** `t0_subset_free` tests FAILED with exact symptoms — queue length `0` vs `5`, arena `live_bytes` `4096` vs `0`, and the release chain simply absent from the verb log |
| the `is_quiesced` idle test only (fill intact) | `retry_ledger::the_commit_retry_bound_…` **wedged** (watchdog SIGABRT) and `t0_subset_free::the_drain_never_races_a_verb_in_flight_on_the_same_isolate` FAILED, printing the offending `UnmapGpuVa`+2×`Free` that ran while the verb was parked. The other five T0 tests stayed green — the idle test is an independent claim and the suite says so |

**Post-fix: 0 / 0 / 0**, in both lock modes, with the same non-zero namespace-death residue
(6 objects, 2 mappings) as before — that number is the §7.0 backstop working, not a leak, and
it is now pinned rather than assumed. `sweep_conservation` additionally requires **no live
proc still owes a release** at the quiesce point, so a drain that silently never fired could
not masquerade as a fixed leak.

**Collateral, and it was predicted in writing.** `retry_ledger.rs`'s re-stale helper carried
the sentence *"they are never freed — dropping a runtime `Vas` abandons its host backing
(per-object reclamation is L1-M2's)"*, and its strict unexpected-verb arm tripped on the
first post-fix run with an `UnmapGpuVa`. Its expectation moved from a script-built union to
the same set equality the headline test uses — a strictly stronger statement of the bound's
own question.

`tests/t0_subset_free.rs` is the focused suite (6 tests): the two planes' release order, the
GPA return *and* its reuse, the quiet proc's backstop sweep, the in-flight regression above,
and the one disposition T0 deliberately does **not** own (a retired isolate refuses the
disposal, so its residue is the session's death).

Suite: **274 → 280**, 0 skipped, 0 ignored; fast path ~22.6 s.

---

### 12.33 ★★ THE CROSS-`Proc` REFERENCE, END TO END — alive and usable: YES. Freed at refcount 0: **NO**

§12.26 answered the *refusal* half of the cross-`Proc` lifetime question (may one component
reach another's host objects? no, and it is a typed refusal with no timing component).
§12.27 then established that after the grouping rule, **kernel↔user is the only cross-`Proc`
reference that exists**: two *user* clients that share are one `Proc` by construction, so the
UVM session client dup'ing a guest process's VASpace is the whole category.

What was never tested is the *survival* half, and it is two separate claims:

> 1. the object outlives its owning process and stays **usable** through the surviving
>    reference;
> 2. the last reference dropping ⇒ refcount 0 ⇒ the object is **actually freed**.

`rmgraph_order_independence.rs::a_kernel_dup_keeps_the_owning_procs_isolate_and_backing_alive_past_the_clients_free`
proved a weaker thing than it read as: the `Proc` is still *present*, and the proc retires
when the dup goes. Present is not usable, and retire is not free.

**Grounding.** RM keeps a dup'd object alive by refcount — `memCopyConstruct_IMPL`'s
`pHwResource->refCount++` / `memdescAddRef` / `DupCount++`
(`ogkm: src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`) — and a kernel reference genuinely
can outlive the owning process, because `uvm_va_space` hangs off the **file**, not the
process (`ogkm: kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`), with
`UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` stating it outright: resources are freed "when
the last reference to the file is dropped rather than when this process exits"
(`ogkm: kernel-open/nvidia-uvm/uvm.h:160-167`).

#### Claim 1 — alive and usable: **YES**, and the split is RM's refcount, executable

`cross_proc_lifetime.rs` section 5 builds the measured shape (one user compute proc; the one
kernel/UVM session client, which belongs to the **system** component, dup'ing its VASpace),
runs a full workload on both planes, then kills the owner by freeing its client root. What
happens is exactly right, at *object* granularity:

| the owner's… | dup'd by the session? | disposition at the owner's death |
|---|---|---|
| VASpace (+ its host VAS, its published backings) | **yes** | survives — `Proc`, isolate, arena and `by_pdb` route all intact |
| GR/CE channels (+ host channel, engine objects) | no | **freed per object**, engine object before channel, on the owner's own isolate |

And the survivor is *exercised*, not inspected: after the owner is dead the test publishes a
brand-new range through the surviving VASpace and asserts the exact verb pair
(`AllocSysmem` + `MapGpuVa`) ran **on the owner's still-live isolate, into the owner's
original host VAS**. The exec plane, meanwhile, faults `FwdFault::UnknownVchid` — the
channels really are gone. That table *is* `memCopyConstruct_IMPL`'s refcount, and nothing in
the core was written to produce it: it falls out of attribution-by-origin plus
components-derived-from-live-resources.

The condemnation path answers the same question the opposite way, correctly: a condemned
owner is **not** kept usable by its kernel reference (`FwdFault::Condemned`, the §12.17
no-resurrect rule — a dup outlives the *process*, never the *isolate* that held the objects),
the UVM session is **not** dragged into the condemnation (it is the system component's, and
every other guest process shares it), and the entry clears only when the guest frees the
**owner's** client root — releasing the *reference* does not clear it.

#### Claim 2 — freed at refcount 0: **NO**. Not per object, and there is no window in which it could be

The last reference dropping retires the owner and reaps it. **Not one `Free` verb is ever
issued for its host VAS or its backings.** Measured: 2 objects + 1 mapping outstanding on the
owner's isolate, `double_free` / `free_of_unknown` / `unmap_of_unknown` all empty, ledger
`is_balanced() == false`.

It is structural, not an oversight anyone can patch locally:

- `Spine::refresh` step 3 retires a vanished component with `procs.remove(id)` +
  `Proc::retire()` and **no `sync_proc_to_boundary`** — so `stage_dropped_vases` never runs
  and the objects are not even *queued*. From that instant the core cannot name them.
- Even queued, a retired isolate refuses every verb including the disposal
  (`t0_subset_free.rs::a_retired_procs_queue_is_left_to_the_session_death_backstop`), and the
  reap runs under the device write lock where R1 forbids one anyway.
- **The asymmetry that makes this case special.** On the ordinary teardown the adapter
  reclaims off core state and *then* applies the guest's own client-root free — that is
  precisely what `the_ledger_balances_across_every_teardown_ordering` scripts, and why it can
  assert `is_balanced()`. Here the retiring free arrives through a **foreign client**
  (UVM's), inside a single `Gpu::apply`. There is no pre-reclaim window at any layer, because
  the owner's teardown path never watches the client that ends its life.

**Is it a leak? No — it is a different disposition, and the distinction is the finding.** The
isolate is a host process; the reap drops it and its death closes its descriptors, so RM
frees the whole client tree (§7.0, the C's #80 backstop). GPA is conserved independently and
is asserted: the arena routes home and the very next process gets the range back. So nothing
is lost. What is lost is the *property*: on this one path "refcount 0" and "per-object
`Free`" are not the same event, and any future code that assumes they are — a reclamation
ledger, a quota, an accounting hook — will be wrong here first.
`the_last_reference_dropping_retires_the_owner_but_frees_nothing_per_object` asserts the
truth rather than the claim we wanted, so that the day someone closes it, a test changes.

If it is ever closed, the shape is visible from here: §12.18 already settles every
retirement in `plan_refresh` **before** a `Proc` is touched, so a "these procs are about to
retire" edge exists to hang a pre-retire drain on. Not done — it is a reclamation-design
decision, not a test's to make.

#### Bite-checks (revert, observe, restore)

| reverted | observed |
|---|---|
| `RmGraph::drop_handle`'s refcount (`if res.refs.is_empty() && res.map_refs == 0` → unconditional remove) — i.e. the origin free destroys the resource despite the dup | `a_kernel_reference_keeps_its_owners_object_alive_and_usable_after_the_owner_is_killed` FAILED at *"★ the owning `Proc` must NOT retire while a kernel dup still references a resource it allocated"* — the **premature free**; and `the_last_reference_dropping…` FAILED at `drain_pending`'s `"a live proc"` because the proc was already gone |
| `Spine::refresh` step 3's `dead` list (never retire a vanished component) | `the_last_reference_dropping_retires_the_owner_but_frees_nothing_per_object` FAILED at *"the LAST reference going is what retires the proc"* — the **leak**: the proc never retires, so its arena never returns and its isolate never dies |
| §12.27's grouping rule, both halves (the `is_user && is_user` dup predicate **and** the kernel-client → `SYSTEM_ANCHOR` assignment) — i.e. pre-`062ea67` dup-connected components | `a_condemned_owner_is_not_kept_usable_by_its_kernel_reference` FAILED at *"★★ the UVM session client is NOT dragged into the condemnation"* — one dead guest process takes down the session every other guest process shares |

Suite: **280 → 283**, 0 skipped, 0 ignored; fast path ~23 s.

---

### 12.34 ★ The `*_unsafe.rs` naming rule — a CI gate landed while it is free

The workspace is `unsafe_code = "forbid"` (root `Cargo.toml`, `[workspace.lints.rust]`) and
stays that way; the lint is what *bans* the escape hatch. The L1 OS adapter
(`kayfabe-linux-raw`) will eventually need one audited relaxation, and the rule that keeps it
auditable is a naming one:

> **An auditor must be able to enumerate the entire escape-hatch surface with `ls`.** Every
> `.rs` file that uses the keyword is named `*_unsafe.rs`.

Landed **now**, with zero such files in the tree, because a convention introduced after the
first exception exists is one nobody can trust retroactively — and because a gate is cheapest
to get right when it has nothing to find.

CI step: *Unsafe-surface gate*, same house style as the §6.2 hexagonal boundary grep. Three
decisions it turns on:

1. **Exit-code polarity.** `grep -l` exits 0 on a HIT, and a hit here is a *violation* — the
   inverse of the usual reading. The verdict is an `if [ -n "$offenders" ]` over captured
   output, and the pipeline ends in `|| true`. Verified that the `|| true` is load-bearing:
   without it, `bash -e` (what Actions runs) aborts the step with exit 1 on the **success**
   case, because `find -exec grep +` returns 1 for "no matches". A gate that fails when it
   passes gets deleted within a week.
2. **Prose counts.** A mention in a comment, doc or string is a violation too — not because a
   comment is dangerous, but because a gate whose verdict depends on reading intent is one
   that eventually gets mis-read, and the cost is one-sided: prose can always be reworded, a
   block cannot.
3. **The one exception is lexical, not editorial.** `grep -w` counts `_` as a word character,
   so the lint name `unsafe_code`, the lint `unsafe_op_in_unsafe_fn` and the filename suffix
   `_unsafe.rs` never match, while the keyword forms and prose forms all do. The rule
   therefore stays *writable by construction* — say `unsafe_code`, or put it in a
   `*_unsafe.rs` file — which is why there is no allowlist to negotiate and nothing to argue
   about in review.

`find` walks the whole repo minus build output rather than a list of known directories: a
gate that enumerates today's crates stops covering the code the moment someone adds one.

**Verified in both polarities**, and in the exception:

| planted | observed |
|---|---|
| nothing (tree as-is) | `GATE PASSED`, exit 0 |
| `crates/kayfabe-util/src/oops.rs` with a real block | exit **1**, printing the offending path and the two legitimate fixes |
| the same file renamed `oops_unsafe.rs` | `GATE PASSED`, exit 0 — the exception works |
| `crates/kayfabe-util/src/prose.rs` containing only a comment mentioning it | exit **1** — prose really does count |

Two pre-existing doc comments were reworded to land it (`kayfabe-core`'s "thread-unsafe
exceptions" → "thread-hostile exceptions"; `security_boundary.rs`'s "NO `unsafe`" → "no
`unsafe_code` at all"). Both were *describing the absence* of the thing, which is exactly the
prose the lexical exception is designed to keep writable — and the reword cost two words.
The convention is documented in `kayfabe-rt`'s crate docs (the adapter neighbourhood the
relaxation will land in), cross-referenced from `kayfabe-core`'s thread-safety section where
`forbid(unsafe_code)` is already argued.

---

### 12.35 ★★★ THE TEARDOWN POST-CONDITION — a leak now fails the test that caused it, and §12.33 is CLOSED

Two changes, landed in this order on purpose (**the instrument before the fix it measures**,
this project's standing discipline): a universal drop-guard post-condition on every test
device, and then the reclamation fix whose acceptance test it is.

#### (a) The instrument — `kayfabe_tests::Guarded`, a drop guard on the test device

§7.8's conservation ledger was **opt-in**. Two focused suites went looking for leaks and
§12.32 composed the census into the mean run; every other test was green because nobody
asked. The owner's framing:

> *"in the tests hold assertions like the data structure got cleaned but host cleanup didn't
> happen, even if the test was a success — these after-checks let it fail instead, because
> it's a leak/violation."*

So: `Guarded<D>` wraps the test's device (`Gpu` or `Arc<SharedDevice>` behind one
`TeardownView` trait, so the two shapes cannot disagree about what "reachable" means) and
asserts on `Drop`. **A test that leaks now fails even though its own assertions passed.**

The invariant, per isolate, in §12.28's strong set-equality form:

```text
  Outstanding(ledger)  ==  Reachable(core state)  ∪  Staged(pending_release)
```

| difference | class | meaning |
|---|---|---|
| `Outstanding − (Reachable ∪ Staged)` | **UNACCOUNTED** | ★ the owner's case exactly: the structure is gone and **nothing was ever queued** to free what it held. Nothing will ever free it, because nothing can address it |
| `(Reachable ∪ Staged) − Outstanding` | **DANGLING** | core state names something the ledger has no live record of — a use-after-free shape, strictly worse than a leak |
| `double_free` / `free_of_unknown` / `unmap_of_unknown` | **corruption** | never a disposition, and **not declarable** at all |

**★ Putting `Staged` on the accounted side is the sharp edge, not a softening.** A proc that
freed a VASpace and then ended the test has left a *queue*, and the T0 drain will take it —
failing there would make the guard something everybody turns off. The distinction that
survives is **"queued" versus "never queued"**, which is precisely the distinction §12.33
proved the core could not make. It needed a new read window to be expressible at all:
`Proc::staged_releases` (and `Spine::retired_procs`, because a vacated corpse still holds its
queue until the reap — a walk that missed it would report every correctly-staged proc as a
leak).

**The opt-out, and the design decision that decides whether this survives.** There is
deliberately **no `allow_leaks` flag**: a bare skip gets sprinkled around and the guard is
dead inside a month. The only way out is `ResidueClaim`, which names the *exact* expected
residue — per isolate, per host class (keyed on the `VerbKind` that minted the handle),
exact counts — with a **mandatory** `why` in the constructor. Three properties fall out and
they were the requirement: the declaration **is** the documentation; a residue that grows
past it still fails; and `grep -rn 'ResidueClaim::on'` enumerates every place we knowingly
leave state. Comparison is **equality, not a bound**, in both directions — a residue that
*shrank* fails too, so a fix cannot leave behind a claim that has quietly become a lie.

**On §7.8's "never in a `Drop`".** §7.8 says the mean run's census must never run in a
`Drop`, citing §12.14 (an unreleased latch turns a failed assert into a hang). That rule is
about `l1_mean.rs`'s parked verbs and it **still holds there** — the mean run keeps its
explicit census. It does not generalise: this guard runs after every spawned thread has been
joined (the device outlives them), and it **skips itself entirely while the thread is already
panicking** — the same discipline `IsolateBox`'s own `Drop` uses for its R1 assert, and for
the same reason. The residual cost is exact and small: a test that is already failing is not
additionally audited.

#### ★★ What the retrofit caught — 26 tests, and three classes nobody was looking for

Retrofitted across the whole suite (25 device-construction sites). **26 tests failed on the
first run.** Most were §12.33's class at other trigger points, which was expected. Three
were not, and they are reported here rather than quietly declared:

| ★ finding | where | what it was |
|---|---|---|
| **19 992 host memory objects + 19 992 GPU mappings leaked per proc** | `soak_llm_like` (20k-token, 3 procs) — also 992 each in the 1k-token runs | the KV-cache ring rotated slots with a raw `vas.table.unbind(va)`, which drops the `HostBacking` on the floor: the GPA block never returns, the host object is never freed, the mapping is never undone — and the proc **never dies**, so §7.0's process-boundary backstop never fires either. This is the #80 leak shape in the exact workload the project cares most about, and the suite's own assertions (no hang, no GPA exhaustion, routing correct) could not see it. Fixed by routing through `kayfabe_fwd::unpublish_backing` (new shared helper `kayfabe_tests::unpublish_and_release`) |
| **4 host objects + 4 mappings leaked per run** | `weird_order_regressions::wo_13_multiiter_realloc_same_va_new_backing_each_iter` | the same raw-unbind bypass, in #13's own alloc→use→free→realloc churn loop. Fixed the same way — **and the fix falsified one of the test's assertions**: `assert_ne!(published.gpa, prev)` ("the bump allocator never reuses") was only true because the block was never returned. With proper reclamation the *same* GPA is the correct answer, so the assertion was replaced by the property #13 actually needs — a **fresh host object** each iteration, in this proc's own arena |
| **the conservation ledger was silently destroyed by a memory bound** | `concurrency_stress::stress_multi_vcpu_interleaved_ops` (`KAYFABE_SLOW=1`) | the 16-thread soak drains the global verb log every 4096 ops to bound memory, with `std::mem::take(&mut rec.log)`. Correct-looking, unexamined since it was written, and it **deletes every acquisition in the drained prefix** — so the whole run's live host state read back as ~4000 DANGLING handles per isolate. Fixed in the mock, not the test: `RmRecorder::compact()` folds the drained prefix into a carried `HostLedger` first, so `ledger()` is the same value whether or not the log was ever drained. The memory bound is unchanged |

Two smaller harness findings, both fixed rather than declared:
`concurrency_stress::same_proc_interleaving_is_exact` sharded a `Proc` out of `Gpu::procs`
into a `Mutex` and never put it back (100 001 objects reported unaccounted — the proc was
simply invisible to the audit); and the `Gpu::retire_proc` convenience was added so the
five `spine.retire_proc(&mut gpu.procs, …)` split-borrow call sites stopped being written
out by hand.

#### (b) The fix — **removal itself** is the central, final step

§12.33's finding: `Spine::refresh` step 3 removed a vanished component with
`procs.remove(id)` + `Proc::retire()` and **no** `sync_proc_to_boundary`, so
`stage_dropped_vases` / `stage_dropped_channels` never ran and the host objects were not even
*queued*. The plan had been to stage inside `plan_refresh`. The owner's framing is better and
is what was built:

> *"if you do something out of the locks, you should assume when re-acquiring the lock the
> data may have changed underneath, including removal. This is best preventable if you do the
> removal in a CENTRAL place, usually after a real cleanup, so this out-of-order isn't really
> possible."*

So the sequence is **decide → stage → drain → remove**, in one place, in that order:

| phase | where | why there |
|---|---|---|
| **decide** | `RefreshPlan::vanishing`, computed in `plan_refresh` from `matches` — the complement of the survivors | §12.18 already settles every retirement before a `Proc` is touched, so this adds no refusal and no failure path; it only *names*, up front, the set the mutation used to discover halfway through |
| **stage** | `Spine::vacate` — the **only** `procs.remove` in `refresh` — runs the ordinary `stage_dropped_*` with empty live sets | staging is pure bookkeeping (handles into `pending_release`, `GpaBlock`s back to the proc's own arena), issues no verb, so R1 permits it under the device write lock |
| **drain** | `Proc::drop`, lock-free, gated on `is_quiesced` | M2-a's rule: per-object reclamation must never race an in-flight verb, and **no lock can exclude one — only the quiesce predicate can**. A `Proc` drop is *already* required to be lock-free (`IsolateBox::drop` asserts it, §12.16 G3b), so this relies on an obligation the design already enforces rather than adding one |
| **remove** | the value falls, last | "removed before cleaned" is no longer *expressible*, the same move `release(arena)`-by-value made for double-release |

Two structural consequences worth naming:

- **`Proc::vacate` vs `Proc::retire` — a clean death is not a condemnation.** Both remove the
  proc from the live set and both refuse every new op. Only `retire` stops the isolates. That
  split is what makes the drain possible at all: a component that vanished cleanly has a
  *healthy* sandbox whose handles can and should be freed per object, whereas a worker HUP or
  a condemnation has an untrustworthy one and §12.17's no-resurrect rule outranks reclaim —
  its residue stays §7.0 namespace death, now **staged and therefore nameable** instead of
  unrecoverable.
- **`Spine::retire_proc` goes through the same `vacate`.** It was the second
  `procs.remove` + `retire()` site with no staging. Centralising it costs nothing (its queue
  is refused by its own stopped isolate) and buys the property that there is exactly one
  place where a `Proc` leaves the live set.

#### The before/after transition on §12.33, stated explicitly

`the_last_reference_dropping_retires_the_owner_but_frees_nothing_per_object` was §12.33's
honest record of the truth we did not want. It is now
`…_and_frees_its_objects_per_object`, and the transition is:

| | before (b) | after (b) |
|---|---|---|
| guard on `cross_proc_lifetime::uvm_referenced_gpu` | **UNACCOUNTED on IsolateId(1)** — 2 objects (`AllocVaSpace`, `AllocSysmem`) + 1 mapping | clean |
| `frees_on_owner` after the reap | `[chan]` only | `[chan, backing, host_vas]` |
| `HostLedger::is_balanced()` | `false` | `true` |

The guard **did** fail beforehand on that exact path, which is the check the brief asked for
— had it not, the instrument would have been too weak and would have needed fixing first.

#### The declared residue, in full (`grep -rn 'ResidueClaim::on'` — 12 sites)

Every one is the same *class* — **a violently-killed proc's isolate is stopped, so its staged
release cannot drain and §7.0 namespace death is the disposition** — except the three marked
as harness bypasses:

| site | isolate | declared |
|---|---|---|
| `cross_proc_lifetime::a_condemned_owner_cannot_dangle_a_system_reference` | owner | VaSpace 1, Sysmem 1, maps 1 |
| `cross_proc_lifetime::a_condemned_owner_is_not_kept_usable_by_its_kernel_reference` | owner | VaSpace 1, Sysmem 1, maps 1 |
| `l1_mean::mean_run` ×2 (`P_TEARDOWN`, `P_HUP`) | 3, 6 | VaSpace 1, Sysmem 1, Channel 1, maps 1 **each** — i.e. exactly §12.32's pinned "6 objects, 2 mappings" namespace-death residue |
| `l1_mean::a_condemned_components_arena_is_released_once_…` | victim | VaSpace 1, Sysmem 1, maps 1 |
| `l1_mean::a_recovered_component_shares_no_arena_or_host_handle_…` | victim | VaSpace 1, Sysmem 1, Channel 1, maps 1 |
| `l1_verb_seam::r5_canary_proc_retired_in_the_gap_refuses_loudly` | 1 | VaSpace 1 |
| `l1_verb_seam::worker_death_retires_the_proc_loudly_and_never_resurrects` | victim | VaSpace 1, Sysmem 1, maps 1 |
| `retry_ledger::a_retry_whose_replan_diverges_refuses_without_leaking_the_attempt` | 1 | Sysmem 1 (the G4 residue the test already asserted by count) |
| `t0_subset_free::a_retired_procs_queue_is_left_to_the_session_death_backstop` | 1 | VaSpace 1, Sysmem 1, maps 1 — this test's entire subject, now stated in the guard's vocabulary |
| `teardown_reclaim::g3_a_worker_whose_proc_retired_in_the_gap_still_reaches_its_slot` | 1 | VaSpace 1 |
| **harness bypass** `teardown_reclaim::g1_a_published_backing_can_actually_be_freed_at_teardown` | 1 | `dangling(3, 2)` — a hand-rolled release chain run straight on a worker, core state deliberately left standing |
| **harness bypass** `teardown_reclaim::g1_a_full_process_lifecycle_leaves_the_host_ledger_balanced` | 1 | `dangling(5, 2)` — same |
| **harness bypass** `c_bug_regressions::cb14_host_{vas,channel}_touch_alone_blocks_a_late_merge` ×2 | B | `dangling(1, 0)` — a **fabricated** `HostHandle` written straight into core state, because the "one clause touched and no other" state is not reachable through the protocol |
| **seam gap** `present_seam::render_target_exports_to_surface_presents_and_vblanks` | 1 | `Alloc 1` — the graphics producer's render target is allocated directly on the isolate because the present seam has **no core-side owner for host scanout memory yet**. The object is real and core state genuinely cannot name it; the claim is the honest statement of where the seam stops |

#### Bite-checks (revert, observe, restore)

| reverted | observed |
|---|---|
| `Spine::vacate`'s two `stage_dropped_*` calls | `c_bug_regressions::cb_lifecycle_process_churn_never_exhausts_the_window` FAILED with **24 UNACCOUNTED isolates**, each `{AllocVaSpace: 1, AllocSysmem: 1, AllocChannel: 1}` + 1 mapping; `cb_lifecycle_full_teardown_reap_rebuild_identical` and the `rt_shell` lock-mode differential with it — i.e. §12.33's class, at every clean-death trigger |
| `Proc::drop`'s drain (early return) | `the_last_reference_dropping_…_and_frees_its_objects_per_object` FAILED with `left: [chan]` vs `right: [chan, backing, host_vas]`. **`t0_subset_free` stayed fully green** — the violent-death path never depended on the drain, which is the split being asserted rather than assumed |
| a `ResidueClaim` over-declared by one object (`AllocSysmem: 1` → `2`) | `t0_subset_free::a_retired_procs_queue_…` FAILED with **`★ DECLARED RESIDUE MISMATCH`**, printing claim vs actual and *"a residue that GREW is a regression; one that SHRANK means the claim outlived its cause"* — the declaration cannot rot in either direction |
| `RmRecorder::compact`'s carry-forward (back to a bare `mem::take`) | `stress_multi_vcpu_interleaved_ops` FAILED with **~4000 DANGLING handles on every one of the 8 isolates** — the finding reproduced exactly |
| `soak_llm_like`'s rotation back to the raw `table.unbind` | `soak_1000_tokens_single_proc_baseline` FAILED with **992 UNACCOUNTED `AllocSysmem` + 992 mappings** — the leak reproduced exactly |

Suite: **287 → 288**, 0 skipped, 0 ignored, both lock modes; fast path ~23.7 s (unchanged),
`KAYFABE_SLOW=1` green. Exactly **one** test was added — `RmRecorder`'s
`recorder_compact_preserves_the_ledger_exactly`, pinning the drain/ledger invariance the
third finding turned on. Nothing else needed one: the guard is a post-condition on the tests
that already exist, which is the whole point of it.

#### What is deliberately NOT closed

The drain runs at the corpse's `Proc::drop`, i.e. at the reap. A *live* proc's
`pending_release` is still drained opportunistically or by the executor's backstop sweep
(§7.6 T0) and the guard counts it as accounted, not as owed — the reading of *"no live proc
still owes an **unqueued** release"* that the brief's wording admits and that the T0 design
requires. A stricter "the queue must be empty at quiesce" belongs to the mean run, where
`sweep_conservation` already asserts it at a point it controls.

---

### 12.36 ★ QUEUE DISCIPLINE — written down normatively, and the count audited

The owner's constraint — *"the isolate/main code should have no more queues than threads, and
a shared queue system that's maybe abstract for scheduled tasks"* — is now normative in
`l1_os_shell.md` §6.6 (next to the thread-placement rule it generalises), as the
**SCHEDULER / ACCUMULATOR** distinction:

> A **SCHEDULER** owns work and needs a thread, because something must decide *when* its
> contents run. There is exactly ONE: the executor inbox. An **ACCUMULATOR** holds work for
> whoever next passes through and needs NO thread. **Queues are drained by existing threads,
> never by new ones; there is one scheduled-task abstraction, not several.** Anything wanting
> its own thread argues against §7.1's relay-thread rejection first, and says which of its
> three free properties it is giving up.

**Audited inventory (2026-07-26): 1 scheduler, 1 thread of our own — the rule holds.** The
full table is in §6.6; the shape of it is: the inbox is the scheduler; `pending_release`,
`Spine::retired` and `Spine::condemned` are accumulators drained by threads that were already
there; `CompletionQueue` is the *guest's* mailbox on the far side of the seam; `PoolGate` is
a condvar, not a queue; an `Orphans` inside a `Refusal` is a `#[must_use]` value in flight.

**The one to watch, named rather than left implicit:** the `DeferQueue` (§6.4) is the only
structure that orders work *by time*, which is a scheduler's defining property. It stays
legal precisely because it owns no thread and its output is an inbox event. The day something
proposes to give it a drain thread, the count goes to two — and §6.6's table is where that
argument has to be had.

### 12.37 ★★★ BOUNDARY-1 BROKEN — a planted dup alias silently condemned a **bystander**, and the entry poisoned handle VALUES it had given back

The standing rule (`Spine::apply`'s own doc, decision #9): *"a hostile stream can only ever
earn its own loud refusal."* Two composing defects broke it — the first time in this core
that a hostile guest could earn **another process's** refusal.

#### C1 — the planted alias

Four facts composed, each individually reasonable:

1. `RmEvent::Dup` checks that `dst` is a free handle and that the caps hold. It does **not**
   check that `dst`'s client namespace exists — an alias planted into a never-allocated
   namespace is accepted and parked.
2. It is inert while `dst` is undeclared: `project` filters undeclared dup endpoints out of
   the client universe, and the grouping predicate needs positive evidence about *both* ends.
3. It fires the instant the victim declares. `Alloc(Client cV, User)` makes `is_user(dst)`
   true, so the union runs — on the **same apply that first creates the victim's boundary**,
   so the victim has no live `Proc` yet.
4. With no live proc to protect, the condemnation was **silent**: `plan_refresh` returned
   `CondemnedMerge` only `if let Some(&live) = matching.first()`; otherwise it pushed `None`
   and `apply` returned `Ok(())`.

```text
A: Alloc(Client cA, User) … A's worker is killed   ⇒ cA condemned
A: Dup { src: (cA, obj), dst: (cV, 0x7777) }        ⇒ accepted, inert, Ok
V: Alloc(Client cV, User)                            ⇒ V's OWN first event
   ⇒ Ok(()). V condemned, permanently, anchor = cA. Nobody is told.
```

`hClient` is predictable (in RM the `hClient` **is** its root object's handle), so the attack
costs one dup per candidate namespace, up to the handle budget.

**★ The asymmetry was the defect.** Dragging a *live* proc into a condemned component was a
loud `CondemnedMerge`; dragging a *not-yet-live* client in was silent. Whether a victim got a
refusal or a silent death depended only on the arrival order of its own client-root alloc.

**Fix — the CONDEMNATION LINE in the grouping predicate, not a louder refusal.**

> A `DUP_OBJECT` edge is a grouping edge iff both endpoints are declared `User` clients
> **and both are on the same side of the condemnation line**.

Making the silent arm loud was considered and rejected: refusing the victim's *own* `Alloc`
is still the victim paying for the attacker's action, so it does not fix boundary-1 at all —
it only relabels the damage. Removing the merge removes the transfer of fatality itself.
Condemnation is a completed fact about a set of clients; a live client that aliases a
condemned client's resource acquires exactly one dead resource, which is attributed to its
**origin** and already answers `FwdFault::Condemned` wherever it is reached. It does not
acquire the corpse's history, and nothing the attacker does can make it.

Two consequences, both good:

- **Components are homogeneous.** An allowed edge never has exactly one condemned end, so a
  component is wholly condemned or wholly alive, and "is this boundary condemned?" has one
  answer for all its clients. Two always-on `debug_assert`s state it where it is used.
- **`GpuError::CondemnedMerge` is retired.** The situation it named is now unrepresentable
  rather than refusable — the merge does not happen, so there is nothing to refuse. This is
  also the *fourth* answer the old test's doc said did not exist ("absorb, condemn, or refuse
  the event"): **do not make it a merge**. It is strictly better than the refusal it
  replaces, because the refusal was only ever reachable in one of the two arrival orders.
  `a_dup_into_a_condemned_component_is_refused_atomically` became
  `a_dup_across_the_condemnation_line_merges_nothing`, asserting the same isolation with an
  additional claim the old one could not make: the corpse's exec plane is still condemned
  *through the freshly minted alias*.

`project` therefore takes the flattened condemned client set as a third argument. It stays a
deterministic pure function — of `(graph, condemned)` rather than of the graph alone — and
the set is itself order-independent. Every caller that is not `Spine::refresh` passes the
named `NO_CONDEMNED`, so "this projection considers no client dead" is a statement the call
site makes rather than one it omits.

#### C2 — the entry retained handles the guest had freed

`Spine::refresh`'s carry-forward re-added the **whole** old entry on every refresh, including
client handles the guest had since freed and which existed nowhere in the graph. An entry
vanished only wholesale, when *no* boundary intersected it. But handle reuse is explicit
design (`RmGraph`'s resource/handle split exists for it, §12.25) and `drop_handle` prunes the
client-root index on free *precisely so* the namespace can be re-declared.

```text
allocate c1..cN, dup-join them, get one worker killed   ⇒ entry = {c1..cN}
free c2..cN, keep c1 alive forever                       ⇒ entry STILL {c1..cN}
… any later, unrelated process whose hClient lands on a freed value
  is condemned the moment it declares.
```

That contradicts the invariant written one screen above it (`absorb_condemned`): *"Absorb,
never widen … it must never reach a client that did not [share the blast radius]."* **A
recycled handle value never shared the blast radius.**

**Fix:** intersect the carried entry with the clients the projection still sees (every user
boundary's clients plus the system component's). Growth over *live* clients is preserved; dead
handle values fall out. The carried set is seeded from the intersecting boundaries' clients —
all of which are known by construction — so no entry can come out empty.

#### The evasions monotone growth exists to stop: re-verified, all still fail

Three tests, one per shape, added rather than argued:

- `evasion_dup_a_fresh_client_then_free_the_old_root_still_fails` — a fresh live client
  aliases the corpse's VASpace and then the corpse's root is freed. The resource stays alive
  on the attacker's alias, so its origin client stays *known*, so the entry does not shrink
  and the dead-backed VASpace keeps answering `Condemned`. The test goes one step further and
  gives the fresh client a **channel bound to the aliased VASpace** — the one genuinely new
  state the condemnation line creates, a live `Proc` naming an address plane it does not own —
  and pins that ringing it is a named `FwdFault::UnknownPdb`, never a served ring.
- `evasion_splitting_the_condemned_component_still_fails` — freeing the joining dup yields two
  boundaries; both stay condemned (each under its own anchor) and they stay **one** entry.
- `evasion_relabelling_the_condemned_component_still_fails` — freeing the anchor client
  re-labels the survivor; it stays condemned, now under its own anchor, and the freed handle
  value stops being named.

#### Bite-checks (house standard: revert, confirm the exact symptom, restore)

- **C1 reverted** (`let _ = condemned;` in place of the line, `debug_assert`s neutered so the
  original symptom rather than the new invariant reports):
  `a_planted_dup_alias_cannot_condemn_a_client_that_has_not_declared` fires
  *"★★ the victim was condemned by an edge IT never created — a hostile stream earned another
  process's refusal, which boundary-1 forbids"*;
  `evasion_dup_a_fresh_client_then_free_the_old_root_still_fails` fires *"the fresh client
  lives"* (the same bystander death, self-inflicted arm); and
  `a_dup_across_the_condemnation_line_merges_nothing` fires *"the dup minted or dropped a
  proc"* (4 vs 5 — the merge absorbing a live proc). With the `debug_assert`s **live**, all
  three instead fire *"a boundary mixed condemned and live clients — the condemnation line
  leaked out of the grouping predicate"*, which is the structural statement of the same fact.
- **C2 reverted** (carry-forward re-adding the whole old entry):
  `a_condemned_entry_must_not_poison_client_handles_the_guest_has_freed` fires *"★★ a client
  handle the guest FREED is still condemned — the entry is poisoning a VALUE, and a recycled
  value never shared the blast radius"*, and nothing else in the suite moves.

#### Docs corrected, because both asserted a protection the code lacked

`Spine::condemned` ("monotonically grown … dropped only when NO boundary intersects it"),
`absorb_condemned` ("Absorb, never widen"), `Spine::apply` ("a hostile stream can only ever
earn its own loud refusal" — which rollback alone never guaranteed, since it says nothing
about *whose* event is refused, nor about an event that is accepted and still kills a
bystander), and `plan_refresh`'s "all four fault conditions".

#### ★★★ What this leaves open — the SAME primitive, doing something WORSE (reported, not fixed)

C1's planted alias has a second effect, and it is worse than the condemnation. **Drop the
condemnation entirely and the same one event puts an unrelated later process into the
ATTACKER's live `Proc`** — one isolate, one GPA arena, one host VAS. Reproduced on this head
(throwaway probe, deleted; attacker `0xE0`, victim `0xE8`):

```text
A: (a normal compute process, publishes, live)                ⇒ ProcId(1)
A: Dup { src: (cA, vaspace), dst: (cV, 0x7b000001) }          ⇒ accepted, inert
V: a normal compute process at hClient = cV                   ⇒ ProcId(1)   ← the ATTACKER's
   proc.clients = {0xE0, 0xE8}; one isolate; one arena.
```

That is #14 un-fixed for the chosen pair, reachable with one event and a guessable `hClient`.
It is **not** a condemnation defect — nothing here is condemned — so the condemnation line
does not touch it, and it is deliberately not fixed in this change, because every available
fix trades against a *stated* property:

- **Refuse a `Dup` whose `dst` client namespace has not declared a root** (which is what RM
  itself does — `hClientDst` is resolved in the client DB, and a non-existent one is
  `NV_ERR_INVALID_CLIENT`). This is the RM-faithful answer and it makes the planted alias
  unrepresentable. It also **breaks decision #4's order-independence as currently stated**:
  `rmgraph_order_independence.rs`'s own reference scenario dups into the UVM session before
  `uvm_session()` declares it, the permutation and fuzz properties shuffle roots after dups,
  and `an_undeclared_client_merges_with_nobody_until_it_declares` asserts the accept-then-
  merge behaviour on purpose.
- **Park the edge until `dst` declares** — preserves order-independence and fixes nothing:
  the merge still fires on the victim's own `Alloc`.
- **Record whether `dst` was declared when the `Dup` arrived, and refuse the merge if not** —
  fixes it, and is order-dependent by construction. Note the two orders *already* disagreed
  observably in the condemned case (one was a loud `CondemnedMerge`, the other a silent
  death), so the property was not as intact as it looked; but making that disagreement the
  mechanism is a decision about decision #4, not a bug fix.

Two further shapes fall out of the same root cause — **a client-handle VALUE is used as a
component identity while the value is recyclable** — and both were reproduced:

- **A freed namespace whose resources are still alive stays condemned.** Attribution is by
  origin (`RmGraph::nodes` reports a resource at its origin key), so an attacker that dups
  `cX`'s VASpace into a namespace it keeps and then frees `cX`'s root leaves `cX` in the
  projection's client universe. C2's shrink therefore (correctly, on its own terms) keeps
  `cX` in the entry — and a later process handed `hClient = cX` is condemned on arrival, no
  proc, no route. C1's silent death by a different route, at one live handle per namespace.
- **…and without any condemnation, that same squat merges the later process into the
  squatter's proc** — the first bullet's breach, reached through a freed-and-recycled
  namespace instead of a never-declared one.

The honest fixes for these live one level down (attribute components by a never-reused
resource identity, or refuse to re-declare a namespace that still holds live resources) —
a change to the graph's **identity** model, not to the condemnation model.

**Provenance, stated honestly:** in Mode-2 these events come from the guest's stock NVIDIA
kernel driver, whose RM validates `hClientDst` before emitting the dup RPC, so a hostile
*user* process in the guest cannot produce them — only a compromised guest kernel can. That is
equally true of C1, and the core's own threat model (decision #9, and the
`security_boundary.rs` / `security_invariants.rs` fuzz suites) treats the RM event stream as
hostile input regardless. So it is in scope by the same rule that put C1 in scope.

Two smaller notes from the same read:

- `FwdFault::UnknownPdb` is what a live channel bound across the condemnation line answers,
  where `FwdFault::Condemned` would be the more precise miss. `gate_working_set_in` takes only
  `&Proc` by design (R1's plan/act split), and the condemned maps are on the `Spine`, so
  naming it exactly is a signature change, not a one-liner. Safe either way — both are loud —
  but the distinction §12.13 drew between "condemned" and "unknown" is lost at this one site.
- `Spine::sync_proc_to_boundary` sets `p.anchor` and `p.clients` twice in a row (a duplicated
  pair of lines, harmless). Left alone deliberately: unrelated to this change.

### 12.38 ★★★ THE CRITERION WAS WRONG — "could this fact still arrive?" should have been "…in a LEGAL PROTOCOL TRACE?"

§12.30 inventoried ~28 MISS sites as DEFER (*"not yet knowable"*) or FAULT (*"never
knowable"*) and concluded **no site needed a behaviour change**. That conclusion was wrong
for at least one site, and the *criterion* is why. It asked **"could this fact still
arrive?"** when it should have asked **"could this fact still arrive in a legal protocol
trace?"**

The owner's ruling, now binding, and the amendment to decision #4:

> **Order-independence holds where the NVIDIA protocol ALLOWS it, NOT wherever an ordering
> is merely expressible. If RM would error on "use before exist", so must we.**
>
> Decision #4 therefore reads: order-independence over **any order of *legal protocol
> facts***. The clause is the whole change. Nothing is lost — a dup into a nonexistent
> namespace is not a legal trace, so no legal trace stops being order-independent — and
> what is removed is the pretence that we owe order-independence to streams the guest's own
> RM can never emit. **Modelling an ordering the hardware forbids was the vulnerability.**

#### ★ The three categories — the first pass collapsed two of them

| # | name | rule | why it is not the other two |
|---|---|---|---|
| **1** | **DEFER (protocol)** | the guest may *legally* send this before that | RM tolerates the order, so refusing it refuses a real guest |
| **2** | ★ **DEFER (observation)** | the protocol *does* order it, but **we may not have observed the earlier fact** | **measured**: only **25 of 82** dups reach GSP (`../reference/rm_semantics_measured.md` §3). A dup's *source object* can be one RM saw and we did not. Faulting here **hangs a legal guest** |
| **3** | **FAULT** | the protocol forbids the ordering **and** we would have observed the earlier fact | RM refuses it, so no legal trace is lost; deferring it **is** the vulnerability |

Confusing 2 and 3 is a bug in **both** directions: faulting an observation gap hangs a legal
guest; deferring a protocol violation is the security hole. §12.30 had no name for
category 2, which is exactly why it mis-filed the one site that mattered — it saw "the fact
might still arrive", which is true of both, and stopped there.

**The practical dividing line is *which fact* is missing, not which site is asking.**

- A **client root** (`NV01_ROOT`) is *always* on the GSP wire — it is literally the RPC
  `AllocFacts::client_kind` is decoded from (`GSPALLOC hClient=… processID=…`,
  `../reference/rm_semantics_measured.md` §4). Its absence is **category 3**.
- An **object-level** fact (an object's parent, a VASpace alloc, a dup's source object) may
  never have reached the wire. Its absence stays **category 2** *even where RM would have
  ordered it* — which is the honest answer, and the one §12.30 could not express.

#### The proof it matters: `RmEvent::Dup` accepted an alias into a never-declared namespace

Reported open at the end of §12.37 and reproduced on this head. Nothing is condemned; the
attacker is an ordinary live compute process.

```text
A: (a normal compute process, publishes, live)                ⇒ ProcId(1)
A: Dup { src: (cA, vaspace), dst: (cV, 0x7b000001) }          ⇒ accepted, inert
V: a normal compute process at hClient = cV                   ⇒ ProcId(1)   ← the ATTACKER's
   proc.clients = {cA, cV}; one isolate; one GPA arena; one host VAS.
```

That is **#14 un-fixed for a pair the attacker chooses**, at the cost of one event per
guessable `hClient` — and `hClient` is guessable because in RM the `hClient` **is** its root
object's handle. Root cause, stated once: **a client-handle VALUE is used as a component
identity while the value is recyclable.**

Real RM refuses it. `serverCopyResource` resolves **both** client handles in the client
database before it looks at a single object handle
(`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:1674` →
`_serverLockDualClientWithLockInfo`, `NV_ERR_INVALID_OBJECT_HANDLE` at `:3486-3487` /
`:3547-3550`), then `clientValidate(pClientDst)` at `:1696`, whose own refusal is
`NV_ERR_INVALID_CLIENT` (`ogkm src/nvidia/src/kernel/rmapi/client.c:782`).

#### The fix — ONE rule, at event acceptance, refusing what RM refuses

> **`RmGraph::undeclared_namespace` — no event may name a client namespace that does not
> exist.** The offender gets `RmGraphError::UndeclaredClient(HClient)` (RM's
> `NV_ERR_INVALID_CLIENT`), the graph is not mutated, and the state is *unrepresentable*
> downstream rather than detected there.

It is central, not per-arm, because that is what makes it structural: by the time any arm of
`apply` runs, every namespace the event names exists. RM checks `hClient` first at every
ioctl-reachable entry point, so the placement is faithful as well as convenient:

| our event | RM entry point | `ogkm .../resserv/src/rs_server.c` |
|---|---|---|
| `Alloc` (non-root) | `serverAllocResource` | `:778`, then `clientValidate` `:824` |
| `Dup` (**both** ends) | `serverCopyResource` | `:1674`, then `clientValidate(dst)` `:1696` |
| `SetPageDir` (an RM control) | `serverControl` | `:1503` / `:1519`, then `clientValidate` `:1547` |
| `MapMemoryDma` | `serverInterMap` | `:2218`, then `clientValidate` `:2232` |

**Two exemptions, each argued rather than convenient:**

- **The client-root `Alloc`** — it is the event that *creates* the namespace, and RM
  bypasses the client lock for exactly it (`serverAllocClient`, `:764`).
- **`Free` and `Unmap` — the TEARDOWN verbs.** A namespace with no root is
  indistinguishable from one whose root was *just* freed (freeing a root drops every handle
  in it, leaving nothing to tell them apart), and a teardown verb arriving after its
  namespace died is a benign race a real guest produces. `Unmap`'s unknown-VAS arm is
  already silently `Ok` for that reason and `Free`'s is the more precise `FreeUnknown`.
  Faulting here would be the FAULT-that-should-DEFER direction. **The rule loses nothing:**
  once no *creating* event can name an undeclared namespace, an undeclared namespace holds
  no handles, so both verbs are inert by construction.

#### ★ What the rule closed, beyond the dup — a second, independent finding

`RmGraph`'s `Alloc` arm had the *same* hole one level down, and §12.30's table filed it as
DEFER on its first row (*"an object may legally precede its client root (order tolerance,
#4)"* — false about RM, `rs_server.c:778`).

An object allocated into a namespace that had not declared itself used to **mint a whole
user `ProcBoundary`** — isolate, GPA arena, routable `Vas` — anchored at a client of
*unknown* `ClientKind`. That is precisely the guess §12.27 refuses to make, reached by
omission instead of by a default. And it is exploitable in the direction §12.26 cares about:
declare the objects first, declare the client root **last** as `Kernel`, and for the window
in between the guest kernel's own VASpaces have a real user data plane — the thing
`FwdFault::SystemDataPlane` exists to forbid. When the `Kernel` root finally lands the
objects migrate to the system component and the user proc is retired, so nothing was even
loud about it.

#### The full re-labelled inventory (§12.30's table, re-derived, not trusted)

★ = category changed. Citations are `ogkm` unless stated.

| site | §12.30 | **now** | why |
|---|---|---|---|
| ★ `RmGraph::client_root_of` / `client_kinds` | DEFER | **3 · FAULT** (`UndeclaredClient`) | RM resolves `hClient` first at every entry point (`rs_server.c:778`, `:1503`, `:1674`, `:2218`) and a client root is always on the GSP wire (`rm_semantics_measured.md` §4). The read-side "no live root ⇒ groups with nobody" arm stays — reachable only via a *freed* root an alias outlives |
| ★ `RmEvent::Dup` — dst / src **namespace** | (unlisted) | **3 · FAULT** (`UndeclaredClient`) | `serverCopyResource` locks BOTH clients (`rs_server.c:1674`) → `NV_ERR_INVALID_CLIENT`. THE squat vector |
| `RmGraph::resource_of` / `origin_of` — dup **source object** | DEFER | **2 · DEFER (observation)** | RM *does* order it (`clientGetResourceRef(pClientSrc, hResourceSrc)`, `rs_server.c:1700`), but only 25/82 dups reach GSP — the source may be an object RM saw and we did not. **Must stay a deferral** |
| `RmGraph::origin_of_kind` | fused DEFER + FAULT | **2 + 3, still fused** | unchanged; §12.30 finding B stands (splitting it is a `Result`-shaped resolver through `project`) |
| `RmGraph::pdb_of` / parked `SetPageDir` | DEFER | **2 · DEFER (observation)** | the *control* requires its VASpace to exist (`serverControl` → `clientGetResourceRef`, `rs_server.c:1560`), so this is not category 1 — but the VASpace alloc is an object-level fact, so the gap is real. Re-labelled, not re-behaved |
| `RmGraph::walk_gpu` / `gpu_of` — no `Device` ancestor yet | DEFER | **2 · DEFER (observation)** | same shape: `serverAllocResource` requires `hParent` to resolve, so parent-before-child *is* ordered by RM — and the parent alloc is object-level. **A site I could have wrongly faulted; I did not** |
| `RmGraph::gpu_of` — Device declared no instance | DEFER | **1 · DEFER**, unreachable | `deviceId` is required by `NV0080_ALLOC_PARAMETERS`; the alloc-time membership check (`InvalidDeviceInstance`) already refuses a bad one |
| `RmGraph::backing_of` | split by caller | **2 (unobserved) / 3 (wrong kind, no backing)** | unchanged behaviour; the "unobserved" half is now named as observation, not protocol |
| `project::resolve_vaspace_handle` / `resolve_channel_vas` | DEFER here, FAULT at use | **2 here, 3 at use** | inherits `origin_of_kind`; unchanged |
| `project`: parked dup edge skipped | DEFER | **2 · DEFER (observation)** | now covers *only* the source object — both namespaces are guaranteed declared at acceptance |
| `project`: `is_user(dst) && is_user(src)` | DEFER on undeclared ends | **1 · DEFER** | reachable only through a freed root; absence is still never read as "user" |
| `project`: `VasFacts.gpu`/`.pdb`, `ChannelFacts.gpu`/`.vas_origin` | DEFER | **1 / 2 · DEFER** | `.pdb` absent is category 1 (`SET_PAGE_DIRECTORY` genuinely follows the alloc); `.gpu` absent is category 2 (the Device is an object) |
| `project`: `by_pdb` / `by_vchid` inserted only for a resolved target | DEFER | **unchanged** | an unroutable object enters no routing map; its use faults by name |
| `project`: `PdbCollision` / `VchidCollision` | FAULT | **3 · FAULT** | not a miss — hostile ambiguity |
| `Gpu::sync_rpc_mappings`: `m.pdb == None` | DEFER | **1 · DEFER** | ★ THE canonical exception: a guest legitimately maps before it binds a page directory |
| `Gpu::sync_rpc_mappings`: `gpu_of(vaspace) == None` | DEFER | **2 · DEFER (observation)** | deferring is what keeps GPU 0 from being guessed |
| `Gpu::sync_rpc_mappings`: `m.mem_phys == None` | FAULT | **3 · FAULT** (`UnbackedMapping`) | a backing is an alloc-time fact; an unbacked memory stays unbacked |
| `Gpu::sync_proc_to_boundary`: unresolved vas/channel | DEFER | **1 / 2 · DEFER** | materializes nothing, re-evaluated next apply, `ChanId` slot kept stable |
| `Spine::plan_refresh`: `LateMerge` / arena exhaustion | FAULT | **3 · FAULT** | decided before any proc is touched (§12.18). `CondemnedMerge` retired in §12.37 |
| `fwd::route_pdb` / `route_doorbell` / `route_engine_object` | FAULT | **3 · FAULT** | *use* sites: the operation is now, so there is no "later" |
| `fwd::resolve_in` | FAULT ×2 | **3 · FAULT** | unknown `(target, pdb)`; unbound VA |
| `fwd::gate_working_set_in` | FAULT ×4 | **3 · FAULT** | incl. `chan.vas_pdb == None` — the same absence `sync_proc_to_boundary` DEFERS on. At ring time there is no "later" |
| `fwd::plan_publish` / `plan_doorbell` / `plan_engine_object` / `plan_control` | FAULT | **3 · FAULT** | `RetiredProc`, `SystemDataPlane`, `UnknownPdb`, `NoVas`, `NoTarget` |
| `fwd::checkout` → `Ok(None)` | DEFER, caller chooses | **1 · DEFER** | "no worker" *will* change; a caller that can wait parks (counted, §12.29), one that cannot gets `PoolSaturated` |
| `fwd::commit_*` → `Refusal { retry: true }` | DEFER at the commit seam | **1 · DEFER** | §12.9's converging staleness — bounded, because a defer must terminate |
| `fwd::commit_*` → `retry: false` | FAULT | **3 · FAULT** | divergent: nothing that can arrive brings the target back |
| `AddressTable::resolve` | FAULT | **3 · FAULT** | the table IS the guest's TLB; a TLB has no "later" |
| `AddressTable::unbind` → `None` | FAULT at caller | **3 · FAULT** | the arena must never accept a range it does not owe |
| `SourceRegistry::dispatch` | FAULT | **3 · FAULT** | handles are never reused, so a miss is never a stale-alias guess |

**Two sites re-labelled 1 → 2 are the ones worth re-reading** (`pdb_of`, `walk_gpu`): RM
genuinely orders both — a control needs its object, an alloc needs its parent — so under the
corrected criterion they *look* like category 3. They are not, because the earlier fact is
object-level and may never have reached the wire. Faulting them would have been the
hangs-a-legal-guest error, and stating them as category 2 is the whole reason the third
category exists.

#### Unrepresentable-by-construction vs runtime-checked

- **Runtime-checked**, one place: `RmGraph::undeclared_namespace` → `UndeclaredClient`. It
  cannot be made a type without giving `RmEvent` a "namespace exists" witness, which would
  move the check to whoever mints the witness — the same check, further from RM's own.
- **Unrepresentable downstream, as a consequence:** a parked dup edge into a nonexistent
  namespace, an object owned by an undeclared client, a `ProcBoundary` anchored at a client
  of unknown `ClientKind`, and (already, from §12.37) a component with one condemned end.
- **Newly unrepresentable, and noted at its test:** a *parked* dup whose source is a
  `Client`-classed object. A namespace holds exactly one `Client` origin — its root — and
  the namespace exists iff that root does, so such a dup either resolves immediately or is
  refused. §12.25's worst mis-fire (an alias promoted to "origin" of a client root) has lost
  its input.

#### Tests

- `l1_mean::a_planted_dup_alias_cannot_squat_a_later_process_into_the_attackers_proc` — the
  squat, end to end: attacker live and publishing, one planted dup, victim arrives at the
  squatted `hClient` and must get **its own** `ProcId`, isolate, arena and host handles.
  The isolation claim is asserted *before* the refusal claim on purpose, so reverting the
  fix reports the breach rather than "the dup was accepted".
- `rmgraph_order_independence::a_dup_into_an_undeclared_namespace_is_refused_in_every_order`
  — the other half of the corrected property: an illegal order is refused **wherever** it
  appears in the stream, mutation-free.
- `rmgraph_order_independence::an_undeclared_client_merges_with_nobody_until_it_declares` —
  **changed on purpose**; it asserted the accept-then-merge behaviour deliberately, and that
  behaviour was the hole. Its doc now says what it used to claim and why it changed.
- `security_boundary::b5_dangling_dup_is_inert_and_unknown_free_is_loud` — now states BOTH
  ends of the taxonomy in one test: unobserved **source object** parks (category 2),
  undeclared **destination namespace** faults (category 3).
- `miss_taxonomy.rs` — the new file Part 3 asks for: every FAULT site refused with its
  **exact** variant, and every DEFER site proved to **resolve when the fact arrives** (a
  deferral that never resolves is a hang, and nothing previously proved it did not). Each
  deferral test asserts the ABSENCE first (nothing materialized, nothing routed, the use
  faults by name) and then the ARRIVAL, so a site that silently *dropped* the fact fails
  even though it "deferred" correctly.
- `security_boundary::b2_pending_pdb_flood_is_capped_loud` — the audit's other find:
  `Capacity::PendingPdbs` was the ONE capacity variant with no test at all. The other four
  parked/live tables each had one, so the gap was invisible by symmetry. It is genuinely
  guest-reachable: a parked `SET_PAGE_DIRECTORY` is a legitimate ordering, so the fact must
  be retained, so the table must be bounded.
- **Seven pre-existing FAULT assertions tightened from `matches!{ .. }` to `assert_eq!`**,
  because "assert the exact variant, never `is_err()`" is only half a standard if the
  fields that carry the meaning are wildcarded: `UnbackedMapping` (both fields, ×2 sites),
  `PdbCollision`/`VchidCollision` (the colliding id and BOTH claimants — i.e. everything
  that says *what* collided), `MalformedToken` (the token), `NotAnEngine` (the class — the
  whole content of that fault), `RetiredProc` (WHICH proc, where `_` would have passed had
  the core named the survivor), `AddressFault::Malformed` (which was hidden behind
  `FwdFault::Address(_)`, so a `Miss` would have passed for a `Malformed`), and `NoVas` at
  the `chan.vas_pdb == None` site — the one absence §12.30 singles out as deferring in
  derivation and faulting at use, which makes it exactly the variant worth pinning.

#### The permutation/fuzz properties, restated rather than weakened

`kayfabe_tests::legal_order` is a *stable topological pass* over one partial order (a
namespace declares before it is named). Every rotation, reversal, interleave and random-key
shuffle in `rmgraph_order_independence`, `determinism`, `multi_gpu`, `security_invariants`
and `fuzz_rmgraph_invariants` still produces a genuinely different order — just never an
impossible one — and the orders it *would* have produced get their own named assertion. It
deliberately reorders **nothing** else: an object may still arrive before its parent, a
`SET_PAGE_DIRECTORY` before its VASpace, a `MAP_MEMORY_DMA` before either end, and a
`DUP_OBJECT` before its **source object**.

#### Bite-checks (house standard: revert, confirm the exact symptom, restore)

- **The `Dup` arm reverted** (`undeclared_namespace` returning `None` for `Dup`):
  `a_planted_dup_alias_cannot_squat_a_later_process_into_the_attackers_proc` fires
  *"★★ the victim was merged into the ATTACKER's `Proc` by an edge it never created"* with
  `left: ProcId(1), right: ProcId(1)` — the breach itself, not a missing refusal.
  Also fires, on the same revert: `a_planted_dup_alias_cannot_condemn_a_client_that_has_not
  _declared` (*"the hostile stream must earn its OWN loud refusal, on its OWN event"*),
  `miss_taxonomy::every_event_naming_an_undeclared_namespace_is_refused_by_name` (the
  squat-vector row), `rmgraph_order_independence::a_dup_into_an_undeclared_namespace_is_
  refused_in_every_order`, `…::an_undeclared_client_merges_with_nobody_until_it_declares`
  and `security_boundary::b5_dangling_dup_is_inert_and_unknown_free_is_loud`.
- **The `Alloc` arm reverted:** `an_object_allocated_into_an_undeclared_namespace_mints_no_
  boundary` fires on its FIRST assertion — *"★★ a boundary was minted for a client whose
  `ClientKind` is UNKNOWN"* — because that test, like the squat test, asserts the end state
  before the mechanism. `every_event_naming_an_undeclared_namespace_is_refused_by_name`
  fires on its `Alloc` row.
- **Each deferral bitten at its own mechanism**, and two of the bites corrected the tests:
  parked `SetPageDir` **dropped** ⇒ *"the parked PDB drained onto the resource"* (neutering
  the *drain* alone does NOT bite, because `pdb_of` also reads the parked table — worth
  knowing); parked map dropped ⇒ *"the parked map replayed"*; the dup promotion neutered ⇒
  the refcount-set assertion; the Device target's back-fill **and** live re-walk both dead
  ⇒ three tests including `defer_an_rpc_mapping_with_no_gpu_target_populates_when_the_
  device_lands` (neutering `cache_targets` alone does not bite, because `gpu_of` re-walks —
  the cache is durability, not the resolution path); a fresh `ChanId` per apply ⇒ *"the
  ChanId slot is the SAME one"* (`Some(ChanId(1))` vs `Some(ChanId(0))`); the
  `UnbackedMapping` fault downgraded to a skip ⇒ both exact-variant sites; the `NoVas` ring
  gate downgraded ⇒ `cb14_ring_gate_on_vas_freed_channel_refuses_nonempty_allows_empty`;
  the `PendingPdbs` cap removed ⇒ `faulted_at: None` vs `Some(262144)`.

#### What this does NOT change

Not the condemnation model (§12.37 stands), not attribution-by-origin, and not the two
shapes §12.37 left open that live one level down in the graph's **identity** model — a freed
namespace whose resources are still alive stays condemned, and re-declaring such a namespace
is still possible. Those need a never-reused resource identity for components, or a refusal
to re-declare a namespace that still holds live resources. §12.38 removes the
*never-declared* vector completely; the *recycled* one is unchanged and still open.

### 12.39 ★★★ THE RECYCLED NAMESPACE — §12.38's surviving sibling, and it is the CHEAPER of the two

**Status: DESIGN, not landed.** No `crates/**` or `tests/**` change accompanies this entry.
Every claim below is marked `[measured]`, `[src]` (with file:line) or `[inferred]`; the
`[inferred]` ones are reachability arguments read off our own source, and each names the
test that would settle it. `ogkm` = `C: research_clones/ogkm`, `C:` =
`/workspace/nvidia-gpu-passthrough`.

§12.38 closed the **never-declared** squat: a `DUP_OBJECT` planted into a namespace nobody
owned yet, firing on an unrelated later process's own client-root `Alloc`. Its closing note
filed the sibling as *"a freed namespace whose resources are still alive stays condemned,
and re-declaring such a namespace is still possible"* and guessed it needed *"a never-reused
resource identity for components"*. Both halves of that note are right about the *organ* and
wrong about the *severity*: the surviving vector is **not** the weak leftover of the strong
one. In its cheapest shape it costs the attacker **four events, no host resources, and
nothing observable until it fires** — strictly cheaper in footprint than the vector §12.38
closed, which at least minted a phantom client universe entry.

---

#### 1. THE VECTOR, AS AN ATTACK

**Attacker:** A1 (`core_security_threat_model.md` §2) — a hostile guest *userspace* process
issuing arbitrary `RmEvent`s with arbitrary handle values. It does **not** need A3 (a
compromised guest kernel), and it does not need a race: both shapes below are deterministic.

**Victim:** any later guest process that is handed an `hClient` value the attacker
previously owned and freed.

**What they end up sharing:** one `Proc` ⇒ one isolate (one host RM client), one GPA arena,
one host VAS, one `pending_release` queue. That is **#14 un-fixed for a pair the attacker
chose** — the identical payoff as §12.38's vector, reached by a different door.

**What the attacker must guess or control:** only the victim's `hClient` **value**.
`[measured]` The guest RM's client-handle index is sequential and guessable — the C's PoC
had *"attacker client landed 12 after victim"* (`C: docs/HARDENING_PLAN.md:183`) — and
`[src]` the generator is an incrementing index into a wrapping 2^20 space with no
quarantine (§2 below). The attacker's own side needs no guessing at all: `[src]` the shipped
Linux driver honours a **caller-supplied** `hRoot` verbatim, so the attacker declares exactly
the namespace values it wants to squat (§2(a)).

##### Shape A — the PARKED FACT that outlives its namespace (the cheap one)

`[src]` `RmGraph::free_subtree` computes `doomed` as *the live handles in the namespace*
(`crates/kayfabe-core/src/rmgraph.rs:1078-1082`) and then prunes the parked tables by
membership in `doomed` (`:1144-1155`). A **parked** dup's `dst` is by definition *not* in
`handles` — that is what "parked" means — so it is not in `doomed`, so **it survives the
free of its own namespace's client root.** The same holds for `pending_pdbs` and
`pending_maps`, whose retains are keyed the same way (`:1147-1155`).

```text
A: Alloc(Client cV, User)                     ⇒ legal; hRoot is caller-supplied [src §2(a)]
A: Dup { src: (cA, H_LATER), dst: (cV, H_X) } ⇒ src unobserved ⇒ PARKED (rmgraph.rs:976)
A: Free (cV, cV)                              ⇒ the root dies; the parked edge does NOT
                                                 (rmgraph.rs:1144 — dst is not a handle)
   … graph footprint from here: one parked edge. No resource, no client in the
     projection's universe (`origin_of(dst) == None` filters it, project.rs:376),
     therefore NO phantom Proc, NO isolate, NO arena. Invisible.

V: (an ordinary compute process handed hClient = cV) Alloc(Client cV, User)
A: Alloc (cA, H_LATER) = a VASpace                ⇒ resolve_pending_dups promotes the edge
                                                     (rmgraph.rs:1161-1176) — minting a live
                                                     ALIAS **inside the victim's namespace**
   ⇒ project: is_user(cV) && is_user(cA), neither condemned ⇒ uf.union(cA, cV)
     (project.rs:441, :458, :461)
   ⇒ ONE boundary {cA, cV} ⇒ ONE Proc.
```

**Cost:** 4 attacker events per candidate `hClient`, bounded only by `MAX_PARKED` (2^18,
`rmgraph.rs:360`). Nothing is minted on the host until the victim arrives. `[inferred]`

**Note what §12.38 does *not* do here.** Every event in that sequence names a namespace that
existed **at the moment it was issued**. `undeclared_namespace` (`rmgraph.rs:707-724`) is
satisfied honestly and completely. This is not a bypass of the rule; it is a hole the rule
never covered.

##### Shape B — the ORPHANED RESOURCE whose origin key names a dead namespace

`[src]` A resource survives its origin handle's free while any foreign alias references it
(`drop_handle`, `rmgraph.rs:1370-1394` — faithful RM refcounting), but its identity
`RmNode.key` still carries the **`HClient` value** of the namespace that allocated it, and
`dups()` reports that stale key as the edge's `src` (`rmgraph.rs:1572-1584`).

```text
A: Alloc(Client cA, User) ; Alloc(Client cV, User)
A: Alloc (cV, H_VAS) = VASpace(+PDB, under a Device)
A: Dup { src: (cV, H_VAS), dst: (cA, H_ALIAS) }   ⇒ resource refs = {(cV,H_VAS),(cA,H_ALIAS)}
A: Free (cV, cV)                                   ⇒ every handle in cV dies; the RESOURCE
                                                      lives on A's alias, origin key still
                                                      (cV, H_VAS); client_roots loses cV
                                                      (rmgraph.rs:1378-1380)
V: Alloc(Client cV, User)                          ⇒ is_user(cV) flips true
   ⇒ the surviving edge ((cA,H_ALIAS) ← (cV,H_VAS)) becomes a grouping edge ⇒ union
   ⇒ the victim's first RM event puts it in the ATTACKER's Proc.
```

**Cost:** 5 events, and — unlike Shape A — a **visible** footprint between the free and the
victim's arrival, because the orphan's origin client re-enters the client universe through
`g.nodes()` (`project.rs:372-374`), which carries **no `kinds` filter** (the filter at
`:395` guards only the dup-chained branch). `anchor_of` then treats an unknown-kind client as
**not kernel**, i.e. as a user boundary (`project.rs:469-473`). `[inferred]`

##### ★ Shape B's second face — a resource-amplification DoS, and an attacker CHOICE

`[inferred]` Each Shape-B plant leaves a **live user `Proc` for a namespace that no longer
exists**: a boundary is minted (`gpu.rs:1890-1894`), and because the orphan VASpace's GPU
target and PDB both survive the free (the sticky per-resource target cache,
`rmgraph.rs:740-743`, `:915-918`), `span_of` is non-empty (`gpu.rs:1412-1428`) so an isolate is spawned and
a GPA arena is carved (`gpu.rs:1952-1962`). A spray therefore consumes arenas until
`GpaError` — at which point *a legitimate new process* cannot get one either. That is a
device-wide DoS earned by one process, i.e. the boundary-1 shape, in the resource dimension.

And it hands the attacker a **choice**. The victim's arrival merges its boundary with the
phantom's, and `plan_refresh` refuses a merge whose absorbed proc is not `is_untouched()`
(`gpu.rs:1504-1513`). The orphan `Vas` is routable, so `by_pdb` routes a `publish_backing`
to it (`kayfabe-fwd/src/lib.rs:859`) and the attacker can mark it touched at will. So:

- leave it untouched ⇒ **the victim is merged into the attacker's `Proc`** (isolation break);
- touch it ⇒ **`GpuError::LateMerge` on the victim's own client-root `Alloc`** (a bystander
  refusal — the *other* boundary-1 violation, the one §12.37 was written to remove).

The attacker picks which of the two guarantees to break. That is worth stating plainly: the
vector is not "an isolation bug OR a DoS", it is both, selectable.

##### Honest severity verdict

- **Shape A is not weaker than the vector §12.38 closed.** Same payoff, 4 events instead of
  1, and a *smaller* pre-fire footprint (zero host state vs. a client-universe entry). The
  only extra requirement is that the attacker once owned the namespace — which `[src]`
  costs it nothing, because `hRoot` is caller-supplied.
- **Shape B is slightly weaker as an isolation break** (it leaves an arena+isolate per
  candidate, so a blind spray converts into a DoS long before it covers a wide `hClient`
  range) and **stronger as a DoS**. Targeted at one predicted value it is equally cheap.
- **Neither requires a race, a condemnation, or A3.** `[inferred]`
- **★ And neither requires an attacker at all, eventually.** `[src]` The guest RM's client
  index wraps at 2^20 per driver load (§2(b)) and never rewinds on free, so on a
  long-running guest the recycled-namespace state arrives *by itself*. This is a
  **correctness** defect that happens to be exploitable, not a hardening nicety.

---

#### 2. RM GROUND TRUTH — asked before designing, because the wrong answer here HANGS A LEGAL GUEST

The question that had to be settled first: **does real RM reuse `hClient` values?** If it
does, any design that *forbids* re-declaring a namespace rejects a stream the guest's own RM
emits — the FAULT-that-should-DEFER direction, and the failure mode this project cares most
about avoiding. The answer is unambiguous.

**(a) Guest-chosen or RM-chosen? — BOTH, and the shipped build lets the caller choose.**
`[src]` `serverAllocClient` takes the caller's value verbatim: `hClient = pParams->hClient;`
(`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:612`). The guard that would reject a
caller-supplied id is `#if !(RS_COMPATABILITY_MODE)` (`:613-616`), and
`RS_COMPATABILITY_MODE=1` is set for the shipped `nv-kernel.o`
(`ogkm src/nvidia/Makefile:129`, consumed by `ogkm Makefile:13,34`) — so the reject branch,
and the handle re-encode at `rs_server.c:3346-3350`, are compiled **out**. `hRoot == 0`
means "RM, generate one" (`rs_server.c:3319`; the same `0 to generate` convention as
`hObjectNew`, `ogkm src/common/sdk/nvidia/inc/nvos.h:483`). `[measured]` The C artifact
depends on this: it remaps every guest client into a `0xdeadNNNN` handle *"the host RM
accepts"* (`C: src/qemu/nvkvm_gpu_emul.c:438-441`) — a value outside
`RS_CLIENT_HANDLE_BASE` entirely, which only works because caller-supplied roots are
honoured. The sole gate is the live-duplicate check (`rs_server.c:3352-3357`,
`NV_ERR_INSERT_DUPLICATE_NAME`), which the C hit in the wild.

**(b) The generator — monotonic index, WRAPS, no free list, no quarantine.** `[src]`
`_serverCreateEntryAndLockForNewClient`, `rs_server.c:3319-3341`:

```c
NvU32 clientHandleIndex = pServer->clientCurrentHandleIndex;      // :3320
do {
    hClient = CLIENT_ENCODEHANDLE(handleBase, clientHandleIndex); // :3324
    clientHandleIndex++;                                          // :3325
    if (clientHandleIndex > RS_CLIENT_HANDLE_DECODE_MASK)
        clientHandleIndex = 0;                                    // :3329   <-- WRAP
} while (_serverFindNextAvailableClientHandleInBucket(...) != NV_OK);
pServer->clientCurrentHandleIndex = clientHandleIndex;            // :3340
```

`CLIENT_ENCODEHANDLE(base, index) = base | index` over bits 19:0 (`rs_server.c:303-313`);
`RS_CLIENT_HANDLE_BASE = 0xC1D00000`, `RS_CLIENT_HANDLE_MAX = 0x100000`
(`ogkm src/nvidia/generated/g_resserv_nvoc.h:173`, `:190`). So: **monotonic, and it wraps at
2^20 client allocations per driver load** (`g_resServ` is constructed once per module load —
`ogkm src/nvidia/src/kernel/rmapi/rmapi.c:97`, torn down at `:144`). Availability is defined
purely as *absence from the live sorted list* (`_serverFindNextAvailableClientHandleInBucket`,
`rs_server.c:3220-3255`): **no free list, no reservation, no quarantine.** The same design
one level down for object handles — `clientGenResourceHandle_IMPL` wraps explicitly with `%`
at 2^19 per client (`ogkm .../resserv/src/rs_client.c:962-974`,
`RS_UNIQUE_HANDLE_RANGE 0x00080000` at `g_rs_client_nvoc.h:54-55`).

**(c) Does RM carry a generation/epoch we could mirror? — NO.** `[src]` Searched every
struct that could hold one: `RsClient` (`ogkm src/nvidia/generated/g_rs_client_nvoc.h:99-130`),
`RmClient` (`g_client_nvoc.h:176-212`), `CLIENT_ENTRY` (`g_rs_server_nvoc.h:75-89`),
`RsResourceRef` (`g_rs_resource_nvoc.h:690+`). None has an id, serial, generation or epoch.
The nearest candidates are `RmClient::ProcID` (`g_client_nvoc.h:195`) — **PIDs recycle too**,
so it is not a durable identity either — and the server's `activeClientCount` /
`activeResourceCount` (`rs_server.c:334-335`), which are *populations*, not stamps: they go
down on free and are never attached to a client. **`hClient` alone cannot distinguish a dead
client from a later live client that got the same value, and RM offers nothing that can.**

**(d) On free, immediately re-allocatable?** `[src]` Yes. `_serverFreeClient_underlock`
`objDelete`s the client (`rs_server.c:521`), removes the entry from the sorted list under the
list lock (`:524-528`) and zeroes+frees the entry (`_serverPutClientEntry`, `:3201-3212`).
`clientCurrentHandleIndex` is **not** rewound, so the *generator* re-issues only after the
wrap — but a **caller-supplied** `hRoot` can retake a freed value on the very next alloc.

**(e) What the reference implementations assume.** `[src]` gVisor nvproxy keys a
process-global `clients map[nvgpu.Handle]*rootClient`
(`C: gvisor/pkg/sentry/devices/nvproxy/nvproxy.go:211`), inserts on root alloc and **deletes
on free** (`.../object.go:395-398`) — i.e. it assumes only *live* uniqueness and is therefore
safe under recycling. A live duplicate is merely `Warningf("nvproxy: client handle %v already
in use")` and then **overwrites** (`.../frontend.go:1209-1218`). There is no comment about
handle reuse anywhere in nvproxy; it sidesteps cross-lifetime identity entirely by refusing
to checkpoint with live clients (`.../save_restore_impl.go:24-30`).

**(f) Our own artifact measured it.** `[measured]` `C: src/qemu/nvkvm_gpu_emul.c:1983-1988`:
*"on a client-ROOT free, purge the freed client's entries from the never-reaped client-keyed
tables so process churn cannot leak slots or alias a later process that reuses the same RM
handle VALUE (**RM reuses client handle values across process lifetimes**)"*. Its own fix was
a monotonic host-handle mint (`:445-448`, `:2035-2038`) — the same answer this entry reaches.

> **★ THE RULING.** RM recycles `hClient` values by design, has no epoch, and the shipped
> driver lets a caller name any value it likes. **Therefore: no design may refuse to
> re-declare a recycled namespace, and no design may treat an `hClient` value as durable.
> The identity must be minted by US, and it must be minted at DECLARATION.**

---

#### 3. ROOT CAUSE, STATED ONCE

§12.38 already named it — *"a client-handle VALUE is used as a component identity while the
value is recyclable"* — and left it. Stated at the precision the fix needs:

> **Every live *handle* in the graph implies a live client root in its namespace** — §12.38
> guarantees no handle is created in an undeclared namespace, and `free_subtree` drops every
> handle in a namespace when its root dies (`rmgraph.rs:1078-1082`). So handles are
> **safe**. The graph has exactly **two** references to a client namespace that are *not*
> handles, and both of them dangle:
>
> 1. **`Resource::node.key.client`** — a resource kept alive by a foreign alias outlives the
>    namespace that allocated it (Shape B).
> 2. **The parked tables** (`pending_dups`, `pending_pdbs`, `pending_maps`) — keyed on
>    handles that do not exist yet, therefore never in `doomed` (Shape A).
>
> `[src]` `rm_semantics_measured.md` §9 records that for RM *"'a namespace with no root' and
> 'a namespace that never existed' are the same state"*. **In our graph they are not** — a
> rootless namespace can still be the origin of live resources and the target of live parked
> facts. That divergence is the whole bug.

Both shapes need fixing, and — this is the non-obvious part — **neither fix subsumes the
other.** An identity change alone leaves Shape A open, because a promoted parked dup mints a
*live alias in the victim's live namespace*, whose identity resolves correctly to the victim
and merges anyway. A parked-table purge alone leaves Shape B open, because the orphan
resource is not a parked fact.

---

#### 4. THE DESIGN — two parts, and the small one is not optional

##### Part A — the TEARDOWN COMPLETION rule (small, self-contained, needs no identity change)

> **Freeing a client root destroys the namespace's parked facts as well as its handles.**
> `free_subtree`'s root arm additionally drops every `pending_dups` entry whose `dst` **or**
> `src` names that client, every `pending_pdbs` entry whose target does, and every
> `pending_maps` entry whose `client` does.

**Faithful, not convenient.** `[src]` RM destroys the whole namespace children-before-parents
on a root free (`ogkm .../resserv/src/rs_client.c:830-849`) and every subsequent op naming it
is `NV_ERR_INVALID_CLIENT` (`rs_server.c:1674` → `client.c:782`). A parked fact is a fact we
accepted but could not yet resolve; promoting it after its namespace died would create an
object in a namespace RM says does not exist. **Nothing legal is lost**: the promoted alias
could never be legally *used*.

Note this restores the invariant in §3 to something total and `debug_assert`able: *after a
root free, the graph holds no reference of any kind to that namespace except the origin keys
of resources a foreign alias keeps alive* — which is exactly what Part B is for.

##### Part B — the IDENTITY MODEL: a namespace IS its client-root resource

Three candidates were considered. All three make the same state unrepresentable; they differ
in what mints the identity.

| | **D1 — epoch on `HClient`** | **D2 — fresh `ClientUid` at declaration** | **★ D3 — `ClientId := the root's `ResId`** |
|---|---|---|---|
| identity | `(HClient, ClientEpoch)` pair; `HClient` stays primary | opaque `ClientUid(u64)` from a new counter; `HClient` demoted to wire form | the `ResId` the client-root `Alloc` already minted |
| new state | `next_client_epoch: u64`; `client_roots: BTreeMap<HClient, (ResId, Epoch)>`; `origin_epoch` per resource | `next_client_uid: u64`; `BTreeMap<HClient, ClientUid>`; `owner: ClientUid` per resource | **none** — `owner: ResId` per resource; `client_roots` already maps `HClient → ResId` (`rmgraph.rs:515`) |
| unrepresentable | a stale epoch grouping with a live one | a stale uid naming anything | a dead namespace owning a live boundary |
| still runtime-checked | "is this epoch current?" at every read | uid lookup misses | `resources.contains_key(owner)` |
| memory | +16 B/resource, +2 counters | +8 B/resource, +1 counter, +1 live index | **+8 B/resource, 0 counters** |
| hostile churn | epochs are u64; nothing retained per dead namespace | ditto | ditto; and `ResId` already has this exact doc (`rmgraph.rs:82-88`, *"never reused"*) |
| honest cost | **two ways to name a namespace** — every site must remember to carry the epoch, and the one that forgets is the next §12.38 | a second never-reused counter beside `next_res_id` doing the same job | **couples namespace identity to the root OBJECT's identity** |

**D3 is recommended**, and the tie-break is faithfulness rather than economy: `[src]` **in RM
the `hClient` IS its root object's handle** — `serverAllocClient` writes the client handle
back as the allocated object's handle (`rs_server.c:625`;
`ogkm src/nvidia/src/kernel/rmapi/client.c:226-227` stamps it into
`NV0000_ALLOC_PARAMETERS.hClient`), which is why `RmGraphError::DuplicateClientRoot` exists
at all (`rmgraph.rs:423-430`). So "the namespace is its root resource" is not a modelling
convenience we impose; it is what RM already means. D3's stated cost — coupling to the root
object — is therefore not a coupling we introduce.

Two properties fall out that D1/D2 would have to be argued into:

- **`ResId` is already documented as never reused** and already exists precisely because
  *"a handle value can be freed and re-allocated while a `DUP_OBJECT` alias keeps the
  ORIGINAL resource alive"* (`rmgraph.rs:82-88`). D3 is the same sentence applied one level
  up. Adding a *second* never-reused counter to say the same thing is the drift §12.35
  centralised removal to avoid.
- **A live alias implies a live root, so the `dst` side needs no snapshot.** Freeing a root
  drops every handle in the namespace including aliases, so a live alias's namespace root is
  necessarily the *current* one. `dst` resolves live, `src`/attribution use the stored
  `owner`. The asymmetry is not a wart — it is the difference between "a handle in a
  namespace" (dies with it) and "a resource allocated by a namespace" (outlives it).

**What changes, concretely.**

- `Resource` gains `owner: ResId`, resolved at the origin `Alloc` from
  `client_roots[client]`. §12.38 guarantees it always resolves (every creating event names a
  declared namespace), so it is a plain `ResId`, **not** an `Option` with a default — the
  deleted-`unwrap_or(0)` discipline (§12.27).
- The client root's own `owner` is itself.
- `project`'s client universe, `is_user`/`is_kernel`, the grouping union, `anchor_of` and
  the attribution loop key on `ClientId` instead of `HClient`
  (`project.rs:368-374`, `:441`, `:461`, `:469-473`, `:524`). A resource whose `owner` is no
  longer in `resources` is **owned by nobody**: it enters no boundary, mints no `Proc`, no
  isolate, no arena. That kills Shape B *and* its amplification DoS in one line.
- `ProcAnchor` **stays `HClient`** (`lib.rs:238`). It is a deterministic *label* of a live
  component, and under D3 every component is a set of live declared namespaces, so anchors
  remain distinct live values. Not changing it keeps the blast radius of D3 off the spine's
  public routing types. ★ Residual, stated: anything that *persists* a `ProcAnchor` past its
  component's death — `Spine::condemned_by_pdb` / `condemned_by_vchid` (`gpu.rs:834-837`) —
  is still storing a recyclable value, and must be keyed by `ClientId` too or re-derived.

##### How Part B lands on the condemnation machinery — it SIMPLIFIES it

`Spine::condemned: Vec<BTreeSet<HClient>>` (`gpu.rs:828`) becomes
`Vec<BTreeSet<ClientId>>`. Everything that reads it — `absorb_condemned`, the `owner_of`
index, the boundary-intersection pass, `plan_refresh`'s `matching`
(`gpu.rs:1470-1474`) — is already a set-intersection on client identity and changes only its
element type.

The interesting consequence is at §12.37's **C2 shrink** (`gpu.rs:1818-1830`, the
`known` intersection). C2 exists because *"a recycled handle value never shared the blast
radius"* — retaining a freed `HClient` in a condemned entry poisoned a value the guest would
hand out again. **Under D3 that poisoning is structurally impossible**: a `ClientId` is never
reused, so a retained dead `ClientId` can never name a future namespace. C2's correctness
role therefore disappears.

**The shrink must nevertheless stay, for a different and now-stated reason: capacity.**
`MAX_CONDEMNED_COMPONENTS` (`gpu.rs:33-48`, the const at `:41`) is enforced at the mint site
(`gpu.rs:1524-1536`), and an entry that never drops means a guest that churns condemned
components fills the list and earns `GpuError::SpineCapacity` on every *subsequent* proc
mint — a device-wide DoS. So the rule becomes cleaner than it was: **an entry is dropped when
no live resource has an `owner` in it** — a precise liveness statement rather than an
approximation, and one that no longer has to be argued for correctness.

★ And it makes §12.37's evasion story *better*, not worse. An orphaned resource of a
condemned component keeps answering `FwdFault::Condemned` even after its origin namespace is
gone, because the condemned entry holds its `owner` `ClientId` and that id is never reused —
whereas today the same answer depends on a dead `HClient` value still being "known", which is
exactly the fragility C2 patched.

##### Rejected alternatives, each for a stated reason

- **D0 — refuse to re-declare a namespace that still holds live resources** (the option
  §12.38's closing note offered). **Rejected: this is the hangs-a-legal-guest error.** §2
  (b)/(d) show RM recycles by design, and `[measured]` §12.27's own hardware measurement has
  the UVM session dup-aliasing 82 objects per CUDA process
  (`docs/reference/rm_semantics_measured.md` §3) — so a dead process's resources aliased into
  a kernel client are the *ordinary* case, not an attack. Refusing the successor's own
  `Alloc(Client)` would refuse a victim for a predecessor's state: precisely the
  bystander-refusal shape §12.37 removed.
- **D4 — make a root free destroy aliased resources too.** Rejected: it contradicts RM
  refcounting (`memCopyConstruct_IMPL` shares and refcounts the memdesc,
  `ogkm src/nvidia/src/kernel/mem_mgr/mem.c:986-1039`, §12.26) and would silently destroy
  memory a live client is still using — the corruption-over-refusal direction.
- **D5 — filter the client universe by `kinds` on the `nodes()` branch too** (a one-line
  patch to `project.rs:372`). Rejected as *the fix*, kept as a consequence: it would stop the
  phantom `Proc` (Shape B's DoS) but not the merge, because the merge fires only once the
  victim has re-declared — at which point `kinds` contains the value again. It treats the
  symptom whose disappearance is the tell.

---

#### 5. THE TESTS THAT WOULD HAVE TO BITE

House standard: the primary test asserts **the isolation breach itself**, so that a revert
reports the breach rather than a missing refusal (§12.38's bite-check discipline).

1. **★ `a_recycled_namespace_cannot_squat_a_later_process_into_the_attackers_proc`**
   (`tests/tests/l1_mean.rs`, beside the §12.38 squat test). Shape A end to end. The
   load-bearing assertion is the same shape as its sibling's —
   `assert_ne!(victim, attacker, "★★ the victim was merged into the ATTACKER's `Proc` by an
   edge planted in a namespace the attacker had already freed")` — failing as
   `left: ProcId(1), right: ProcId(1)`. Then, exactly as §12.38's does: distinct isolate,
   disjoint arena, a ring served in the victim's own lane, `host_identities` disjoint. The
   mechanism assertion (the parked edge is gone after the root free) comes **last**.
2. **`a_recycled_namespace_cannot_inherit_the_previous_tenants_address_plane`** — Shape B.
   Same breach-first shape, plus: the orphaned VASpace's PDB must **not** appear in
   `spine.by_pdb` under the victim's `ProcId`.
3. **`a_resource_whose_namespace_died_belongs_to_no_boundary`**
   (`tests/tests/object_model.rs`) — the amplification. After `Free(root)` with a surviving
   foreign alias: no new `Proc`, no isolate, no arena carved. Asserted on counts, so a
   regression that re-mints the phantom is loud.
4. **★ `a_recycled_namespace_is_a_DIFFERENT_component_and_is_not_condemned`** — **the
   hangs-a-legal-guest gate.** Condemn a component, free its roots, re-declare the same
   `hClient` values as an ordinary new process: it must get its **own live `Proc`**, its own
   isolate and arena, and must be servable end to end. This is the test that fails if anyone
   later "fixes" this by refusing re-declaration (D0).
5. **`an_orphaned_resource_of_a_condemned_component_still_answers_condemned`** — the
   regression gate on §12.37. Proves D3 does not weaken condemnation stickiness when the
   corpse's namespace itself is gone (today this holds only because the dead `HClient` is
   still "known").
6. **`miss_taxonomy::a_parked_fact_does_not_survive_its_namespaces_root_free`** — one row per
   parked table (`pending_dups` dst, `pending_dups` src, `pending_pdbs`, `pending_maps`), so
   Part A is asserted per-table rather than per-scenario.
7. **`rmgraph_order_independence::a_freed_and_redeclared_namespace_projects_identically_in_
   every_order`** — re-declaration inside the shuffle domain. `legal_order`
   (`tests/src/lib.rs:238-263`) needs no change: its partial order is *"a namespace declares
   before it is named"*, and re-declaration satisfies it.
8. **`security_invariants`, one new property:** for all legal event streams, two clients whose
   roots were minted by *different* `Alloc`s share a `Proc` **only if** a resolved user↔user
   dup edge exists between their *current* declarations. This is the D3 invariant stated
   directly, and it is the one a fuzzer can search.

**Bite-checks to run before landing** (revert, confirm the exact symptom, restore): Part A
reverted (`free_subtree`'s new retains removed) ⇒ test 1 fires with the breach message and
test 6's `pending_dups` rows fire; Part B reverted (`owner` re-derived as
`client_roots[node.key.client]` at read time) ⇒ tests 2 and 3 fire; the C2 shrink removed
⇒ test 4 must **still pass** (proving the shrink's role is now capacity, not correctness)
while a new capacity test fires.

---

#### 6. WHAT THIS DESIGN DOES **NOT** CLOSE

- **§12.25 Deferred finding 2 — object-handle recycling *inside* one namespace.**
  `Alloc (A,H) → Dup to B → Free (A,H) → Alloc (A,H)` still leaves two live resources with
  the same origin `NodeKey`, and `project` still keys `vases`/`channels` on `node.key`
  (`project.rs:534`, `:583`). D3 fixes the **client** axis only. It makes the object axis
  strictly easier — `owner` proves `ResId` can be threaded through the graph's payloads — but
  the projection still has to move to `ResId` keys, and that remains the named refactor.
- **The live-duplicate surface is unchanged.** `ConflictingAlloc` / `ConflictingDup` /
  `DuplicateClientRoot` are about two live claimants on one value; this entry is only about
  one *dead* and one *live* claimant, separated in time.
- **`ProcAnchor` is still an `HClient`**, deliberately (§4). Every site that persists an
  anchor beyond its component's life — `condemned_by_pdb` / `condemned_by_vchid` — is a
  residual named above, not something this design closes.
- **It does not restrict which `hClient` values a guest may use.** `[src]` §2(a) says any
  value is legal; D3 makes the value *irrelevant to identity*, which is the only defensible
  posture.
- **It says nothing about the host side.** `HostHandle`, `IsolateId`, `ProcId`,
  `CompletionSource` are already monotonic never-reused mints (`l1_os_shell.md` §7.7); this
  entry brings the *guest-facing* identity model up to that standard, no further.
- **It does not fix the live component SPLIT** (§7, finding 3 below), which is an independent
  defect that this design neither causes nor cures.

---

#### 7. OPEN QUESTIONS, AND THINGS FOUND ALONG THE WAY THAT CONTRADICT EXISTING CLAIMS

**Open, each with the experiment that settles it — written as questions rather than guessed:**

- **O1 — does the ordinary CUDA lifecycle produce the orphaned-resource state?** The UVM
  session holds 82 dup aliases per process (`rm_semantics_measured.md` §3). If `nvidia_uvm`
  drops them *after* the dying process's client root is freed — even briefly — Shape B's
  precondition occurs with no attacker at all. **Experiment:** kprobe
  `rmapiFreeClient`/`serverFreeClient` and the dup-free path on the bench, run one CUDA
  process to exit, and record the ordering. Serialized bench run, one boot.
- **O2 — how fast does a real guest wrap the 2^20 client index?** `[src]` bounds it; nothing
  measures the rate. **Experiment** (the subagent's, cheap and definitive): loop
  `NV_ESC_RM_ALLOC{NV01_ROOT_CLIENT, hRoot=0}` + `NV_ESC_RM_FREE` ~1.05M times against a
  driver that has not been reloaded and confirm the returned handle rolls from
  `0xc1d000000|0xFFFFF` to `|0x00000`. Settles whether recycling is a "long-running guest"
  event or a "busy guest" event.
- **O3 — is the attacker's LateMerge choice (§1) actually reachable?** Does a
  `publish_backing` routed to the orphan `Vas` mark the phantom proc `!is_untouched()`?
  **Experiment:** a core test, no bench needed — it is a direct consequence of
  `gpu.rs:1504-1513` and `kayfabe-fwd/src/lib.rs:859`, and asserting it costs one test.
- **O4 — can a guest reach `serverAllocClient` with a caller-supplied `hRoot` unprivileged?**
  `[src]` `escape.c:465-489` shows no `hRoot` sanitisation on `/dev/nvidiactl` and the C's
  `0xdeadNNNN` remap works, but the C's host RM may run at a different privilege.
  **Experiment:** as a non-root user, `NV_ESC_RM_ALLOC` with `hClass=NV01_ROOT`,
  `hRoot=0xdead0001`; check the value returned in `NV0000_ALLOC_PARAMETERS.hClient`. Only
  affects how cheaply the attacker *chooses* its squat values — the vector stands either way,
  since the generator is sequential and guessable `[measured]`.

**Three existing claims this pass found to be wrong or narrower than written:**

1. **`project.rs`'s undeclared-endpoint note is right about the dup branch and silent about
   the other one.** The comment at `project.rs:389-397` reasons that after §12.38 *"the only
   way to reach here undeclared is a root that has since been FREED"* — true, and it duly
   filters. But the **`g.nodes()` branch** of the same expression (`project.rs:372-374`) has
   no such filter, so a client with no declared root **does** enter the universe and
   `anchor_of` files it as a **user** boundary (`:469-473`). §12.38's own summary line — *"an
   object allocated into an undeclared namespace mints no boundary"* — is therefore true at
   *alloc* time and **false after a root free**. `[inferred]`
2. **`Spine::condemned`'s doc claims component splits are handled; the argument only covers
   the CONDEMNED path.** `gpu.rs:795-796` says the client-set key makes condemnation survive
   *"component splits (both halves intersect, both stay condemned)"* — correct, because a
   condemned boundary gets `None` and touches no proc. On the **live** path both halves of a
   split match the *same* `ProcId` (`plan_refresh` reads the pre-refresh `p.clients`,
   `gpu.rs:1470-1474`), both are pushed into `boundary_pid`, and `sync_proc_to_boundary` runs
   twice on that one proc (`gpu.rs:1899-1902`) — the second call overwriting the first, so one
   half of the split silently loses its clients, vases and channels while `plan.vanishing` is
   empty. Reachable by a legal guest (dup-join two user clients, then free the alias).
   `[inferred]` — independent of this entry, and it wants its own round and its own test
   (`a_live_component_that_splits_yields_two_procs`).
3. **`RmGraphError::ReservedClient`'s citation is weaker than its conclusion.** Its doc
   (`rmgraph.rs:407-418`) justifies reserving `HClient(0)` with *"RM mints client handles from
   `RS_CLIENT_HANDLE_BASE` and can never produce it"*. `[src]` §2(a): with
   `RS_COMPATABILITY_MODE=1` the base is **not** binding on a caller-supplied `hRoot`, so
   that sentence is not the operative reason. The refusal itself is still correct and should
   stand — but on the stronger ground that `0` is `NV01_NULL_OBJECT` and means *"RM, generate
   one"* on the alloc path (`rs_server.c:3319`), so no live client can ever *hold* it. Same
   verdict, sound citation. `[src]`

**One claim CONFIRMED rather than corrected, worth recording because the design turns on it:**
`rm_semantics_measured.md:179-183`'s *"handle values are per-client and reusable … a handle
is never an identity on its own"* is now backed all the way to the generator
(`rs_server.c:3319-3341`) and to the absence of any epoch in RM's own structs. Nothing in
either repo claims NVIDIA handles are never reused; every "never reused" statement in the
Rust tree is scoped to **our own** mints (`ResId`, `ProcId`, `IsolateId`, `HostHandle`,
`CompletionSource`) and says so. D3 is the extension of that existing, consistent posture —
not a new one.

### 12.40 ★★★ §12.39 LANDED — and the design was wrong about its own severest claim

**Status: LANDED.** Three defects, in the order they were fixed, each with the full gate
between them so a regression is attributable. 349 → **359 tests**; clippy
`--all-targets -D warnings` clean, `fmt --check` clean, `check --target
aarch64-unknown-linux-gnu` clean. `ogkm` = `C: research_clones/ogkm`.

The headline is not any of the three fixes. It is that **§12.39's central severity claim —
that an orphaned resource's surviving `Proc` is a *phantom* and must be filtered out — is
false, and a landed, cited, end-to-end test already said so.** Implementation found it on
the first run; the fix that follows from the truth is a different (and smaller) one.

---

#### 1. WHAT §12.39 GOT WRONG — the "phantom `Proc`" is not a phantom

§12.39 §1 (*"Shape B's second face — a resource-amplification DoS"*) and §4 (*"a resource
whose `owner` is no longer in `resources` is **owned by nobody**: it enters no boundary,
mints no `Proc`, no isolate, no arena. That kills Shape B *and* its amplification DoS in
one line"*) both rest on filtering the orphan out of the projection. **That filter was
written, and it broke two tests immediately** — `cross_proc_lifetime::a_kernel_reference_
keeps_its_owners_object_alive_and_usable_after_the_owner_is_killed` and
`…::the_last_reference_dropping_retires_the_owner_and_frees_its_objects_per_object`, whose
own assertion message is the refutation:

> *"the owning `Proc` must NOT retire while a kernel dup still references a resource it
> allocated — that retire reclaims host memory RM says is live (`ogkm:
> src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`)"*

That is §12.33's landed rule, and it is right. `memCopyConstruct_IMPL` refcounts the
memdesc, so RM keeps the resource alive; `uvm_va_space` is bound to the **file**, not the
process (`ogkm kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`), so a kernel reference
genuinely keeps *using* it after the owning process is killed. Our `Proc` is what owns the
isolate whose host memory backs it. Retiring it frees host memory a live client is still
reading — the **corruption-over-refusal** direction, which is exactly what §12.39's own
rejected alternative D4 refuses for the same citation. `[src]`

**So the surviving `Proc` is the design working, not a leak.** The consequences:

- **§12.39's "arena-exhaustion DoS" is withdrawn.** A spray of orphans costs the attacker
  a live alias per orphan (bounded by `MAX_LIVE_HANDLES`) and is *indistinguishable* from
  the measured ordinary case — the UVM session holds 82 dup aliases per CUDA process
  (`docs/reference/rm_semantics_measured.md` §3), so "a dead process's resources aliased
  into a kernel client" is the steady state, not an attack. Arena exhaustion is already a
  loud `GpaError`. `[inferred]`
- **§12.39's D5** (*"filter the client universe by `kinds` on the `nodes()` branch"*),
  offered as *"rejected as the fix, kept as a consequence"*, is **rejected outright**: it
  is not a weaker version of the fix, it is the corruption bug.
- **The exclusion still exists — but its trigger is a RE-DECLARATION, not a free.** That
  is the rule that landed (§4 below), and it is strictly narrower than what §12.39 wrote.

---

#### 2. BUG 1 — `anchor_of` read an ABSENCE as "user", and the guest kernel got a data plane

**Reported as:** *"the `g.nodes()` branch has no `ClientKind` filter, so a client whose
root has been freed is filed as a user boundary — Shape B's enabler and an arena DoS."*
**Found to be:** the *membership* is correct and load-bearing (§1 above); the
**classification** was the defect, and it is a different and sharper one.

`project`'s `is_kernel` read `RmGraph::client_kinds`, which only knows namespaces with a
**live root**. Once the root is freed there is nothing to read, `is_kernel` was false *by
absence*, and `anchor_of` (`project.rs:469-473`) filed the namespace as a **user**
component. So an orphaned resource of the guest **KERNEL's** own namespace minted a user
`ProcBoundary` — isolate, GPA arena, routable `Vas`, and `publish_backing` would mint host
memory into it. That is the guest-kernel-obtains-a-user-data-plane shape
`FwdFault::SystemDataPlane` exists to forbid, reached by omission.

★ And it directly contradicted a comment three screens up in the same function: the
grouping predicate's *"either end is undeclared — grouping requires positive evidence
about BOTH sides; **absence is never read as user**"*. The predicate honoured that rule.
`anchor_of` did not.

**Fix — a recorded fact, not a filter.** `Resource` gains `owner_kind: ClientKind`,
recorded at the origin `RM_ALLOC` from the namespace's live root (§12.38's gate guarantees
one exists). `RmGraph::client_declarations()` answers *"which declaration does this
namespace project under?"* — the live root if there is one, else the newest declaration
that still owns a live resource. `is_user` deliberately stays on the **live-root** map:
grouping still requires positive evidence about a live declaration at both ends (§12.27,
unchanged).

- **Test:** `cross_proc_lifetime::an_orphaned_kernel_resource_never_becomes_a_user_data_plane`
  — the mirror of §5's user→kernel case. Breach-first: proc count, plane count, then
  `by_pdb[(GPU, UVM_PDB)] == SYSTEM_PROC`, then the exact `FwdFault::SystemDataPlane`.
- **Bite-check:** `is_kernel` reverted to `kinds` ⇒ fires on its FIRST assertion,
  *"★★ a USER `Proc` was minted for the guest KERNEL's namespace"*, `left: 2, right: 1`.
- **Doc corrected:** `RmGraphError::UndeclaredClient`'s summary (*"an object allocated
  into an undeclared namespace mints no boundary"*) now says what it is true of — **alloc
  time** — and what it was false of: after a root free, which is the only way to reach an
  undeclared namespace at all now.

---

#### 3. BUG 2 — the component SPLIT, the dual of the merge

Confirmed exactly as reported. `Gpu::plan_refresh` matched boundaries to procs on
`p.clients ∩ b.clients` with nothing to stop one proc matching several boundaries, so a
split `P{A,B} → b1={A}, b2={B}` gave `matching(b1) == matching(b2) == [P]`; `survivors`
took `.first()` of each, so `vanishing` was **empty**; both boundaries took `pid = P`, and
`sync_proc_to_boundary` ran twice on it, the second overwriting the first. One half lost
its clients, vases and channels **silently**, its `Pdb` left `by_pdb` entirely (its anchor
named no live proc), and its isolate and arena stayed live under the other half. The merge
guard (`matching.len() > 1`) structurally cannot see it: each boundary matches exactly one
proc.

**Semantics, decided deliberately.** A split is triggered by freeing the dup edge that
joined two client sets — ordinary, legal guest behaviour, so **refusing it would hang a
legal guest** (the test asserts the `Free` is accepted). The rule that landed:

> **A live `Proc` is claimed by the FIRST boundary in ascending anchor order that
> intersects it; every other boundary that would have matched it mints a NEW `Proc`.**

`bounds.procs` is anchor-ordered and an anchor is its component's smallest client, so the
claimant is the half that still holds the proc's own anchor whenever that client survives —
the proc keeps its identity rather than having it reassigned by iteration order. Absorbed
procs are claimed too, so a later boundary cannot adopt a corpse.

**Conservation.** The departing half leaves through the **existing** staged-death path and
nothing else: `sync_proc_to_boundary(keeper, b_first)` stages every vas/channel that is not
the keeper's into `pending_release`, exactly as a subset-free does, so each host object is
reclaimed once. Verified against `HostLedger` (`double_free` / `free_of_unknown` /
`unmap_of_unknown` all empty) and against `Guarded`'s teardown post-condition.

★ **The honest cost, stated rather than hidden.** Host objects name the RM client namespace
they live in and **cannot move between isolates**, so the departing half necessarily
re-materialises: its address table starts EMPTY. That is a real loss, and it is the *safe*
loss — an unbound VA is a loud `AddressFault::Miss` / `FwdFault::UnknownPdb` at use, never
a silently zeroed backing. There is no third option: the two clients shared one host RM
client, and un-sharing that is not expressible. What changed is that the loss is now
staged, routed and attributable instead of silent.

- **Test:** `l1_mean::a_live_component_that_splits_yields_two_procs` — two user processes
  join, both publish and ring out of the one isolate, the guest frees the alias. Breach
  first (`route_pdb` on each half, then `assert_ne!(pid_a, pid_b)` naming *one isolate, one
  GPA arena, one host VAS*), then: exactly-its-own-clients, the keeper is the anchor half,
  the new proc's `host_identities` is **empty** (inherits nothing), `pending_release` grew
  (staged, not dropped), ledger clean, both halves served end to end in their own token
  lanes.
- **Bite-check:** `claimed` removed ⇒ fires on the FIRST routing lookup —
  *"★★ the split DROPPED one half's address plane: Pdb(…) no longer routes (UnknownPdb…)"*.
  `teardown_reclaim::g6_…` also fires (*"the two halves of the split must be two `Proc`s"*).
- **Doc corrected:** `Spine::condemned`'s claim that the client-set key handles *"component
  splits (both halves intersect, both stay condemned)"* is now marked as sound **for the
  condemned path only** — a condemned boundary gets `None` and touches no proc — and as the
  live-path defect it was.
- **`g6_no_live_binding_ever_points_outside_its_own_procs_arena` updated**: it read
  `reaped.len() == 1` for the *collapse*. Freeing that scenario's client root while a peer
  holds an alias IS a split; it now asserts two procs, disjoint arenas per half, and that
  the keeper's routing survives.

---

#### 4. BUG 3 — the recycled namespace, Parts A and B (and neither subsumes the other)

**Part A — teardown completion.** `free_subtree` prunes the parked tables by membership in
`doomed`, the set of live HANDLES the free removed; a parked fact's key is by definition
not a live handle, so **every parked fact survived the free of its own namespace's client
root**. The root arm now additionally drops every `pending_dups` entry whose `dst` **or**
`src` names that client, every `pending_pdbs` entry whose target does, and every
`pending_maps` entry whose client does. Faithful: RM destroys the whole namespace
children-before-parents on a root free (`ogkm .../resserv/src/rs_client.c:830-849`) and
every subsequent op naming it is `NV_ERR_INVALID_CLIENT` (`rs_server.c:1674` →
`rmapi/client.c:782`). Nothing legal is lost — the promoted alias could never be legally
used.

★ **A second reason Part A is load-bearing, not noticed in §12.39:** a parked `dst` also
**blocks an alloc** at that handle value (`ConflictingAlloc`). Leaving the stale entry
would refuse the namespace's *next tenant* its own allocation — the
hangs-a-legal-guest direction. The purge is what makes a recycled namespace **usable**, not
merely safe. (This is what the A4 refcount fuzz property caught: its independent
`RefTracker` still modelled the old prune, and the divergence it reported was the graph
*accepting* an alloc the model refused. The model now mirrors the rule and says why.)

**Part B — identity.** `ClientId` = the client root's `ResId`, never reused; `Resource`
gains `owner: ClientId` (a plain field, no `Option`, no default — §12.38's gate guarantees
it resolves). Three consumers, and each one had to change or the vector survives:

| site | rule |
|---|---|
| `project`'s client universe + attribution loop | a resource projects under its origin `HClient` **only while its `owner` is the declaration that namespace projects under** |
| `project`'s grouping predicate | the dup's source identity is read through the **destination** handle (`owner_of(dst)`, live by construction) and compared against `src.client`'s current declaration — the edge's reported `src` key carries the recyclable value |
| `Spine::plan_refresh` matching + `Spine::condemned` | **`ClientId`, not `HClient`** |

★ **The rule is narrower than §12.39's, and the narrowing is §1's finding:** while a
namespace has **no** live root its orphan's own declaration is still the one it projects
under, so its component — isolate, arena, host VAS — survives, per §12.33. The exclusion
bites at exactly one moment: a **re-declaration**, which mints a new `ClientId` and leaves
the old declaration's resources owned by a namespace that no longer projects. They belong
to no boundary, the old component is retired through the ordinary staged-death path, and
the new one inherits nothing.

★★ **Two things §12.39 did not anticipate, both mandatory:**

1. **`ProcBoundary` must NOT carry `ClientId`.** A `ClientId` is minted from a counter, so
   its *value* depends on arrival order, and `Boundaries: PartialEq` is the
   order-independence property. So the identity is compared, never reported: `Proc` gains a
   private `client_ids` set (runtime state), `ProcBoundary` stays pure `HClient`, and
   `project`'s signature is unchanged. Stated once: **`clients` LABELS a component,
   `client_ids` MATCHES one.**
2. **`Spine::condemned` had to move to `ClientId` after all**, for a hole §12.39 did not
   name and §12.37's C2 shrink cannot reach. C2 drops a freed client from a condemned entry
   because it leaves `known` — but an **orphan resource of the dead component keeps its
   namespace in the projection**, so the freed value never falls out, and the next process
   handed that `hClient` is condemned on arrival. A bystander death, and the guest cannot
   recover from it because it did nothing. With `ClientId` the shrink's correctness role
   does disappear exactly as §12.39 predicted, and it stays for **capacity**
   (`MAX_CONDEMNED_COMPONENTS`).

**Not done, deliberately:** `ProcAnchor` stays an `HClient` (§12.39 §4's own choice), so
`condemned_by_pdb` / `condemned_by_vchid` still persist a recyclable label past their
component's death. Still a named residual.

**Tests + bite-checks.**

| test | bite | symptom |
|---|---|---|
| `l1_mean::a_recycled_namespace_cannot_squat_a_later_process_into_the_attackers_proc` (Shape A) | Part A reverted | *"★★ the victim was merged into the ATTACKER's `Proc` by an edge planted in a namespace the attacker had already FREED"*, `left: ProcId(1), right: ProcId(1)` |
| `l1_mean::a_recycled_namespace_cannot_inherit_the_previous_tenants_address_plane` (Shape B) | `projects` neutered | *"★★ the SUPERSEDED declaration's address plane must belong to nobody"* — `Ok(ProcId(10))` vs `Err(UnknownPdb{…})` |
| " | grouping owner-conjunct removed | *"★★ the victim was merged into the ATTACKER's `Proc` by a dup edge whose `src` names a namespace the attacker had already freed"*, `left: ProcId(7), right: ProcId(7)` |
| " | `plan_refresh` matched on `p.clients` | *"★★ the victim INHERITED the previous tenant's `Proc` — its isolate…"*, `left: ProcId(9), right: ProcId(9)` |
| `l1_mean::a_recycled_namespace_is_a_different_component_and_is_not_condemned` (**the hangs-a-legal-guest gate**) | runtime `ids` keyed on the `HClient` value | *"★★ a process was condemned on arrival because its `hClient` landed on a value a dead component still held"* |
| `miss_taxonomy::a_parked_{dup_destination,dup_source,page_directory,map}_does_not_survive_its_namespaces_root_free` (one row per parked table) | Part A reverted | all four fire, each naming its own table |
| `rmgraph_order_independence::a_freed_and_redeclared_namespace_projects_identically_in_every_order` | `projects` neutered | *"★★ the SUPERSEDED declaration's address plane must belong to nobody"* |

★ **Shape A's bite leaves Shape B green and vice versa** — §12.39's claim that neither fix
subsumes the other, measured rather than argued.

**Not landed from §12.39 §5:** test 3 (`a_resource_whose_namespace_died_belongs_to_no_
boundary`) asserts the claim §1 refutes and is **not** written; its corrected form (a
resource of a *superseded* declaration belongs to no boundary) is asserted inside Shape B
and the order-independence test. Test 5 is folded into the gate test as its first claim,
with the exact `FwdFault::Condemned`. Test 8's `security_invariants` property is **not**
written — still open.

---

#### 5. ★ FINDING 4 — an origin key is not unique among live resources, and it was reachable

Found by the gate test, which failed *before* it could reach its own claim. `Alloc (A,H) →
Dup to B → Free (A,H) → Alloc (A,H)` leaves **two live resources reporting at the same
`(client, handle)`**, and `pdb_of` / `gpu_of` / `backing_of` / `references` /
`map_ref_count` all answered with a forward `find`, i.e. the **oldest** — while
`crate::project` keys `vases`/`channels` on `node.key` and fills them in ascending `ResId`,
i.e. **newest wins**. The two disagreed.

That is not cosmetic. The current tenant's `Vas` took the **orphan's** declared `Pdb`, so
`by_pdb` filed a page-directory base the current tenant never declared onto the current
tenant's `Proc` — and whoever holds an alias to the orphan routes straight to it. Handle
values are deterministic in practice (every CUDA process presents the same ones — the whole
#14 shape), so this is the ordinary state of a churned namespace, not an exotic one.

Fixed by making all five origin-key scans **newest-wins** (`rev()`), which is the
declaration that projects. This removes the ambiguity; it does **not** move the projection
off `NodeKey` keys — §12.25's deferred finding 2 / §12.39 §6 is still the named refactor,
and it is now the last place a recyclable value is an identity.

**Bite:** `pdb_of`'s `rev()` removed ⇒ the gate test fires on
*"★★ the successor's own address plane was never materialized"*.

---

#### 6. Everything else this pass touched or contradicts

- **`RmGraph::client_kinds` vs `client_declarations` are two different questions** and are
  now documented as such: *"is there a LIVE declared root here?"* (grouping — positive
  evidence, §12.27) and *"which side of the user/kernel line does this component sit
  on?"* (assignment — answerable after the root is gone). Conflating them was Bug 1.
- **`fuzz_rmgraph_invariants::RefTracker`** gained the Part-A prune. Its A4 failure is
  recorded in the `.proptest-regressions` file and now replays green.
- **`sync_proc_to_boundary` had `p.anchor` / `p.clients` assigned twice** (a stray
  duplicate); removed while adding `client_ids`.
- **Open, unchanged:** `ProcAnchor` as a persisted `HClient` in the condemned routing maps;
  the projection's `NodeKey` keys (§12.25 finding 2); §12.39 §7's O1–O4 bench questions;
  §12.39 test 8's fuzz property.

### 12.41 ★★★ THE FAMILY SWEEP — every value NVIDIA recycles, audited; the last identity-from-a-recyclable-value closed, and one NEW defect found in the same organ

**Status: LANDED (one fix) + a verified, unlanded finding.** 359 → **363 tests**; gate clean
(`KAYFABE_SLOW=1 cargo test --workspace`, `clippy --all-targets -D warnings`,
`fmt --check`, `check --target aarch64-unknown-linux-gnu`). `ogkm` =
`C: research_clones/ogkm`, `C:` = `/workspace/nvidia-gpu-passthrough`.

The brief: §12.38/§12.39/§12.40's four defects are **one mistake repeated** — *we assumed
an NVIDIA handle or key means more than it does* — so sweep the family rather than the
bug. What follows is the inventory, the one member closed, and what the sweep found that
the brief did not predict.

---

#### 1. ★ THREE THINGS IN THE BRIEF THAT TURNED OUT TO BE WRONG

1. **"The `NodeKey`-keyed projection … establish whether this is exploitable."** It is
   **not** a cross-process isolation break, and cannot be: a `NodeKey` carries its
   `HClient`, so two colliding resources are always in ONE namespace, hence (attribution
   is by origin client) one component; and every routing consumer looks the object up in
   its **own** `Proc` (`proc.vases.get(&(gpu, pdb))`, `kayfabe-fwd/src/lib.rs:1075`,
   `:1092`, …), which is the isolation backstop. It is a **correctness** defect of the
   hangs-a-legal-guest kind, plus a disarmed F1 guard. Measured, not argued (§2).
2. **`rs_client.c:962-974` — the citation is EXACT** (`clientGenResourceHandle_IMPL`,
   monotonic `handleGenIdx++ % RS_UNIQUE_HANDLE_RANGE`, `0x0008_0000` = 2^19,
   `ogkm src/nvidia/generated/g_rs_client_nvoc.h:54-55`, no free list — it re-probes the
   live map and wraps). But it is **not the cheap door**. The cheap door is that a caller
   supplies `hObjectNew` **verbatim** (`0` = "generate", `ogkm .../inc/nvos.h:483`;
   `clientAssignResourceHandle` branches on exactly that, `rs_client.c:998-1005`), the
   only validation is *not currently live* (`clientValidateNewResourceHandle_IMPL`,
   `rs_client.c:1446-1470`, `NV_ERR_INSERT_DUPLICATE_NAME`), and the free just
   `mapRemove`s (`rs_client.c:1137`) with **no quarantine**. So a guest re-takes a freed
   object handle on the very next ioctl — no wrap needed. Object handles are the exact
   mirror of §12.39's `hClient` finding, one level down.
3. **§12.40's residual — "`condemned_by_pdb`/`condemned_by_vchid` still persist a
   recyclable label past their component's death" — is FALSE as written.** Both maps are
   `clear()`ed and rebuilt whole from the projection on every `Spine::refresh`
   (`gpu.rs:2116-2117`, `:2130`, `:2149`); `retire_proc`'s direct inserts (`:2251`,
   `:2261`) survive only until the next refresh, and nothing can recycle a value without
   an event, i.e. without a refresh. `route_pdb` reads live `by_pdb` first
   (`kayfabe-fwd/src/lib.rs:859`), so a live entry always shadows. The residual that IS
   real is `ProcAnchor` being an `HClient` in `Boundaries` — see §5, finding N2.

---

#### 2. THE DEFECT, MEASURED — `Alloc → Dup → Free → Alloc` at ONE object handle

Five ordinary events, each legal on real RM (§1.2's citations):

```text
A: Alloc (A,H) = VASpace, SET_PAGE_DIRECTORY pdb1
K: Dup { src: (A,H), dst: (K,alias) }        ⇒ the guest KERNEL (UVM) aliases it
A: Free (A,H)                                 ⇒ the resource LIVES on K's alias
                                                 (`ogkm .../mem_mgr/mem.c:1027-1031`)
A: Alloc (A,H) = a DIFFERENT VASpace, pdb2    ⇒ TWO live resources at one (client,handle)
```

Measured on the pre-fix tree (probe, `project` + `Gpu`):

```text
by_pdb            = {(GPU0, pdb2) → A}          ← pdb1 is GONE
proc.vases        = {(GPU0, pdb2)}              ← the ghost's Vas staged for release
pdb_of((A,H))     = pdb2                        ← the GHOST answers with the successor's PDB
references((A,H)) = [(A,H)]                     ← K's alias is invisible to the refcount API
```

**Four consequences, one cause** (`ProcBoundary::vases`/`channels` and `Proc::chan_ids`
keyed on `RmNode::key`, a value RM recycles):

| # | consequence | class |
|---|---|---|
| 1 | the dup-kept ghost leaves `by_pdb`/`by_vchid` entirely; the alias holder's every op takes `UnknownPdb`/`UnknownVchid` | **correctness** — §12.33's landed rule (*a kernel reference keeps its owner's object alive **and usable***) is false for the one shape that reaches it |
| 2 | `pdb_of`/`gpu_of`/`references`/`map_ref_count`/`backing_of` answer about the SUCCESSOR when asked about the ghost | **correctness** — a fact reported for one resource is another's |
| 3 | `chan_ids.entry(key).or_insert_with(mint)` hands both incarnations ONE `ChanId` ⇒ one runtime `Channel`, one `host_channel`, one `host_token`, and step 4 files BOTH vChids onto it | **correctness**, and the sharpest: a doorbell on the ghost's vChid rings the successor's host channel |
| 4 | the F1 guards skip on `prev == node.key`, so two genuinely different live claimants of one `(GpuId, Pdb)`/`(GpuId, VChid)` read as *"the same claimant re-declaring"* and pass **silently** | the ambiguity guard **disarmed** on the one input shape that defeats it |

★ **Reachability is ordinary, not exotic.** Guest handle values are deterministic (every
CUDA process presents the same ones — the whole #14 shape), the C artifact records the
same fact in its own words — *"handle values are reused across contexts and process
lifetimes"* (`C: src/qemu/nvkvm_gpu_emul.c:670-674`, `:1926-1928`, `:1940-1942`) — and the
measured UVM session holds **82 dup aliases per CUDA process**, so "a freed object still
aliased into a kernel client" is the steady state.

---

#### 3. THE FIX — `ResourceKey`, and why NOT the obvious `ResId`

> **A resource's identity is its origin handle plus the *incarnation* of that handle
> value:** `ResourceKey { origin: NodeKey, incarnation: u32 }`. Minted at the origin
> `RM_ALLOC` as **the smallest ordinal no LIVE resource at that key holds**, stable for
> the resource's lifetime, and `0` unless a dup is keeping a ghost alive at that value.

**`ResId` was the obvious key and is unusable.** It is already the never-reused identity —
but it is minted from a counter, so its *value* depends on arrival order, and
`Boundaries: PartialEq` compared across shuffled orders **is** decision #4. That is
verbatim §12.40's reason for keeping `ClientId` out of `ProcBoundary`, one level down. The
ordinal has no such dependence: allocations at ONE handle value are **totally ordered by
the protocol** (an `Alloc` onto a live handle is `ConflictingAlloc`, mirroring RM's
`NV_ERR_INSERT_DUPLICATE_NAME`), so it is a pure function of the declared facts — which is
what `rmgraph_order_independence::a_recycled_object_handle_projects_identically_in_every_order`
asserts over every permutation of both phases.

**It is a disambiguator among the LIVE, not a ghost log.** Reusing a dead incarnation's
ordinal is deliberate: identity only has to separate things that are simultaneously live,
and bounding the ordinal by the live set is what stops the table growing with guest churn.

**The counter-discipline held.** RM recycles by design, so no fix may *refuse* the
recycle — `a_recycled_object_handle_never_steals_the_ghosts_address_plane` asserts the
second `Alloc` is **accepted** and that BOTH planes publish and ring end to end, out of the
one proc's own isolate lane. (Bite C below is exactly the shape that gets this wrong: it
turns a legal guest's alloc into a `PdbCollision` refusal.)

**What changed, exactly:**

| site | before | after |
|---|---|---|
| `RmNode` | `key: NodeKey` (documented as *"Identity"*) | `key` is a **label**; `+ incarnation`, `+ id() -> ResourceKey` |
| `ProcBoundary::vases` / `::channels` | `BTreeMap<NodeKey, _>` | `BTreeMap<ResourceKey, _>` |
| `Boundaries::by_pdb` / `::by_vchid` | value `(ProcAnchor, NodeKey)` | `(ProcAnchor, ResourceKey)` |
| `ProjectionError::{Pdb,Vchid}Collision` `a`/`b`, `pdb_claims`, `vchid_claims`, `engine_refine` | `NodeKey` | `ResourceKey` |
| `Proc::chan_ids`, `Channel::key`, `Vas::origin` | `NodeKey` | `ResourceKey` |
| `Mapping::vaspace` / `::memory`, `RmGraphError::ConflictingMap::vaspace` | `NodeKey` | `ResourceKey` |
| the five `rev().find(node.key == …)` scans (§12.40 finding 4) | O(resources) linear, newest-by-`ResId` | `current_at` — the resource whose **origin handle is live** there, else newest, via a `by_origin: BTreeMap<ResourceKey, ResId>` index in O(log n) |
| `project`'s per-node resolution | `pdb_of(node.key)` / `gpu_of(node.key)` | `pdb_of_resource(node.id())` / `gpu_of_resource(node.id())` |

★ **A complexity fix rides along, and it was not optional.** `by_origin` exists because
`next_incarnation` must not be an O(n) probe per `Alloc` (O(n²) on a guest-driven alloc
flood — the G10 rule, §12.22). The same index then turned §12.40 finding 4's five linear
scans into lookups; `project` called two of them **per node**, so the old shape was
O(resources²) per event.

★ **`current_at` is also strictly more correct than the `rev()` it replaces.** "Newest by
`ResId`" and "the one whose origin handle is live" diverge as soon as an *older*
incarnation dies and its ordinal is retaken. The recorded fact — there is at most one live
origin handle per key — is read first; the newest-ghost rule survives only as the
genuine-tie fallback, which is what §12.40 finding 4 actually meant.

**Tests + bite-checks** (each fix neutered individually, everything else intact):

| test | bite | symptom |
|---|---|---|
| `l1_mean::a_recycled_object_handle_never_steals_the_ghosts_address_plane` | **A** — `vases` keyed on `ResourceKey::first(node.key)` | fires on its FIRST assertion, *"★★ the dup-kept GHOST VASpace lost its ADDRESS PLANE"*, `left: Err(UnknownPdb { gpu: GpuId(0), pdb: Pdb(0x6100_0000) })` vs `right: Ok(ProcId(7))` |
| " | **C** — `pdb_of(node.key)` instead of `pdb_of_resource(node.id())` | the legal `Alloc` is **REFUSED**: `Projection(PdbCollision{ a: …incarnation 0, b: …incarnation 1 })` — the hangs-a-legal-guest direction, caught by the expect message that says so |
| " | **E** — `next_incarnation` ≡ 0 | same first assertion, same shape |
| `l1_mean::a_recycled_channel_handle_never_shares_the_ghosts_host_channel` | **B** — `channels` keyed on the origin handle | *"★★ a live channel lost its EXEC PLANE to a recycled handle value"*, `left: (Some(ProcId(7)), None)` vs `right: (Some(ProcId(7)), Some(ProcId(7)))` |
| " | **B2** — `chan_ids` keyed on the origin handle | same first assertion (the route never materializes) |
| " | **B3** — `chan_ids` **and** the `by_vchid` build both on the origin handle (both route, one id) | *"★★ the ghost and its successor were handed ONE `ChanId` …"*, `left: ChanId(0)`, `right: ChanId(0)` |
| " | **E** | fires at B3's assertion |
| `object_model::two_live_vaspaces_at_one_recycled_handle_still_collide_on_a_shared_pdb` | **D** — F1 guard compares `prev.origin != node.id().origin` | the ambiguous claim is **accepted**: `Ok(Boundaries{…})` where `by_pdb` files only incarnation 1, vs `Err(PdbCollision{…})` |
| " | **E** | same |
| `rmgraph_order_independence::a_recycled_object_handle_projects_identically_in_every_order` | **A** | *"★★ the component must own BOTH incarnations"*, `left: {pdb2, keeper}` vs `right: {pdb1, pdb2, keeper}` |
| " | **E** | same |

★ **Nothing in the pre-existing 359 caught any of it** — the suite is green under every
bite above except through the four new tests. That is the honest measure of why this
survived §12.25, §12.39 and §12.40.

★ **Scope note on the F1 guard, because it is a POLICY claim and not an RM one.** CPU-RM
performs **no** cross-VASpace uniqueness check on the PDB physical address:
`deviceCtrlCmdDmaSetPageDirectory_IMPL` (`ogkm .../gpu/mem_mgr/dma.c:426-460`) passes
`physAddress` through to `gvaspaceExternalRootDirCommit`, whose only assertion is that
*this* VAS has no PDB yet (`.../gpu_vaspace.c:2853-2857`). Refusing two live VASpaces on
one PDB is **our** F1 decision (#18C), unchanged here — the test pins that the decision is
enforced on the input shape that used to evade it, not that RM enforces it. **OPEN
QUESTION O5:** whether GSP-RM enforces PDB-physaddr uniqueness is not visible in the open
sources (CPU-RM RPCs the params through, `dma.c:508-516`). Experiment: two live
externally-owned VASpaces, identical `physAddress`, observe the second's status.

---

#### 4. ★★ NEW DEFECT FOUND BY THE SWEEP, VERIFIED, **NOT LANDED** — two orphan GENERATIONS in one namespace release host memory RM says is live

Same organ as §12.40 Part B, one level up, and it is the **corruption** direction.

`RmGraph::client_declarations` writes `out.insert(r.node.key.client, (r.owner, r.owner_kind))`
while iterating ascending `ResId` (`rmgraph.rs`), so a rootless namespace keeps only its
**newest** orphan declaration. `project`'s `projects()` then answers `false` for every
resource of any **older** declaration. Measured (probe, on the fixed tree):

```text
declare A(gen1), VAS pdb1, kernel dups it, free A's root   ⇒ by_pdb: {pdb1, kernel}   ✓ (§12.33)
re-declare A(gen2), VAS pdb2, kernel dups it               ⇒ by_pdb: {pdb2, kernel}   ✓ (§12.40)
free A's root again                                        ⇒ by_pdb: {pdb2, kernel}
   …while gen1's VASpace is STILL LIVE in the graph (the kernel's alias refcounts it)
```

gen1's resource belongs to **no boundary, permanently**. Its component leaves through
`vanishing`, is vacated, and `stage_dropped_vases` releases its host VAS and its published
backing memory — **while a live kernel client still references the resource RM refcounts**
(`ogkm .../mem_mgr/mem.c:1027-1031`). That is precisely what §12.40 §1 refused as D4, and
what `cross_proc_lifetime`'s landed assertion forbids; the two-generation shape simply
walks around it.

**Reachability:** two process lifetimes at one recycled `hClient`, each taking a UVM alias
— i.e. the ordinary state of a long-running guest, given a client index that wraps at 2^20
with no free list (§12.39 §2(b)) and 82 dup aliases per CUDA process.

**Why it is not landed here:** the honest fix is the residual §12.40 already named —
**`ProcAnchor` and `ProcBoundary::clients` are `HClient`s**, so two generations at one
`hClient` are *not expressible* as two boundaries no matter what
`client_declarations` returns. Making `client_declarations` return a set only moves the
collapse into the union-find. It is the same shape as this entry's fix (a label promoted
to an identity) applied to the component plane, it changes `Boundaries`' public shape, and
it wants its own order-independence argument — a round, not a patch. Filed as **N1**.

---

#### 5. ★ THE INVENTORY — every key the core keys, indexes, caches or compares on

`[src]` = cited to `ogkm`/`gvisor`/the C artifact; `[meas]` = measured on this tree;
`[code]` = read off our own source.

| value | what the code assumed | what RM actually guarantees | verdict |
|---|---|---|---|
| `HClient` | a live namespace, durable | caller-supplied verbatim (`rs_server.c:612`, guard compiled out `:613-616`); generator wraps at 2^20, no free list, **no epoch anywhere** (`:3319-3341`, `:3220-3255`) `[src]` | CLOSED §12.38/§12.39/§12.40 (`ClientId`) |
| `HObject` / `NodeKey` | a resource identity | caller-supplied verbatim (`nvos.h:483`, `rs_client.c:998-1005`); validated only against the LIVE map (`:1446-1470`); freed with **no quarantine** (`:1137`); generator wraps at 2^19 (`:962-974`) `[src]` | **CLOSED HERE** (`ResourceKey`) |
| `HClient` as a *component* label (`ProcAnchor`, `ProcBoundary::clients`) | one namespace ⇒ one component | as `HClient` above — two lifetimes are indistinguishable by value | **OPEN — N1/N2** (§4) |
| `Pdb` | a unique live address plane per target | RM checks **nothing** across VASpaces (`dma.c:426-460` → `gpu_vaspace.c:2853-2857` asserts only *this* VAS is unbound) `[src]` | our F1 policy; guard **repaired** here (was disarmed by the `NodeKey` collapse). O5 open on GSP |
| `VChid` | one live channel per target | chid is an EHEAP alloc returned on free (`kernel_fifo.c:988`, `:997`); the only quarantine is the opt-in `kfifoChidMgrRetainChid` refcount (`:899-903`, `:940`, `:1051`) `[src]` | `by_vchid` is rebuilt whole per refresh `[code]`; the ghost/successor ambiguity is closed here |
| `deviceInstance` | a physical GPU | a bounds-checked index into `pGpuGrpTable` (`device.c:97`, `:119-128`; `gpu_mgr.c:636-653`) `[src]` | CLOSED §12.21 (G9 entitlement); attacker-controlled and treated as such |
| `GpuVa` | — | guest-chosen | keys `Vas::blocks`/`rpc_bound`/`pt_pages`, all per-`Vas`; overlap is a loud `AddressFault::Overlap` `[code]` |
| `ClassId` (`Channel::host_engine_objects`) | one engine object per class per channel | — | an **idempotency table** by design; two logically distinct objects of one class on one channel are indistinguishable (the second is `Stale::Rebound`). Latent, stated `[code]` |
| `ResId` / `ClientId` / `ProcId` / `CompletionSource` / `ArenaId::generation` / `SlotId` | never reused | minted from monotonic counters `[code]` | sound |
| `ChanId`, `WorkerId` | dense, per-owner | minted per `Proc` / per isolate `[code]` | sound **provided** the owner travels with them — `SourceKind::Worker` complies |
| **`IsolateId`** | *"the isolate whose RM client namespace this handle lives in"* (`HostHandle`'s own doc) | — | ★ **OPEN — N3**, below |
| `BatchId` | a batch of one proc's completions | minted per `DeliveryPlane`, i.e. **per `GpuTarget`** | ★ **OPEN — N4**, below |
| `OsEventRef` | a completion identity | minted by the GUEST: `OsEventRef(addr.0 ^ payload)` (`kayfabe-fwd/src/lib.rs:1916`) `[code]` | ★ **OPEN — N5**, below |
| `HostHandle::raw` | — | client-scoped from one shared base, so A's `0x…07` and B's `0x…07` are both live and unrelated (`g_resserv_nvoc.h:173`) `[src]` | sound — the isolate travels with the handle (§12.26) |

**Named open findings, each with the experiment or the change that settles it:**

- **N1 — two orphan generations release live host memory.** §4. Verified `[meas]`.
- **N2 — `ProcAnchor` is an `HClient`.** §12.40's own residual, restated with N1's
  evidence: it is not merely a stale *label*, it is what makes N1 unfixable in place.
- **N3 — `IsolateId` does not carry the `GpuId`, but there is one isolate per
  `(Proc, GpuId)`.** `Gpu` spawns every isolate as `IsolateId(pid.0)` (`gpu.rs:1060`,
  `:2076`, `:2096`) with the `GpuId` only as a separate argument, and
  `HostHandle::belongs_to` compares **only** `IsolateId`
  (`kayfabe-isolate/src/lib.rs:133-135`). So `Worker::execute`'s foreign-handle gate
  (`:705`) — documented as *"the ONE place the `(Proc, GpuId)`-scoped-handle rule is
  enforced"* — **structurally cannot** distinguish proc P's GPU0 handles from its GPU1
  handles `[code]`. `plan_control` takes an adapter-supplied `obj: HostHandle` and a
  caller-chosen `target_gpu` and pairs them unchecked
  (`kayfabe-fwd/src/lib.rs:1655-1684`), under a comment that *asserts* the pairing
  (*"MG-5: `obj` is a handle in THAT isolate's namespace"*). Blast radius is one proc's
  two host RM clients, so it is a **correctness/host-object** hazard, not a cross-process
  break. ★ **The mock cannot see it**: `MockRmBackend` builds a fresh namespace per
  `spawn(id, gpu)` and folds the GPU into the raw value
  (`kayfabe-mocks/src/lib.rs:1031`, `:1065`) — the exact *"the mock namespaces its fake
  handle values, a real host does not"* case `HostHandle`'s own doc warns about — and
  `HostLedger::leaked: BTreeMap<IsolateId, _>` merges the two targets' leak accounting, so
  §12.35's teardown post-condition is weaker than it reads. **Fix shape:** `IsolateId`
  becomes the `(proc, gpu)` pair it already denotes. Not landed: ~142 references, ~70 of
  them `IsolateId(pid.0)` in tests that pin `IsolateId == ProcId` as a documented
  property, and it changes what the leak ledger measures.
- **N4 — `BatchId` is minted per target and consumed per proc.** Each `GpuTarget` has its
  own `DeliveryPlane` with `next_batch` from 0 (`gpu.rs:1040`, `:1701`, `:2579`), while a
  `Proc` has ONE `CompletionQueue` whose `in_flight` spans every target (`gpu.rs:299`).
  For a proc on two GPUs both post `BatchId(0)`, and `completions_drained(GPU0)` sweeps
  GPU1's still-outstanding events into `awaiting_ack`
  (`kayfabe-completion/src/lib.rs:158-168`) `[code]`. MG-6's *gate* still holds; the
  accounting behind it does not. **Fix shape:** key `in_flight` by `(GpuId, BatchId)`, or
  mint device-globally. Not verified by a test — settle it with a two-GPU proc, one batch
  outstanding per target, drain one.
- **N5 — the guest mints its own completion identity.** `OsEventRef(addr.0 ^ payload)`
  (`kayfabe-fwd/src/lib.rs:1916`), and `CompletionQueue::ack` removes **every** entry
  equal to it across three queues (`kayfabe-completion/src/lib.rs:171-175`) `[code]`. A
  guest choosing a colliding `addr ^ payload` can cancel an unrelated pending completion
  **of its own proc** — contained by the per-proc container, so latent. Settle: does any
  path let one proc's ack reach another's queue? (Read says no.)
- **N6 — `pending_pdbs.insert` has no idempotency/conflict arm** (`rmgraph.rs`, the parked
  `SetPageDir` arm), so a second parked declaration on one unresolved handle silently
  replaces the first, while a *direct* redeclaration is documented "last wins". Probably
  consistent, but the silence is undeclared. `[code]`, unverified.
- **O5** — GSP-side PDB uniqueness (§3).

---

#### 6. Everything else this pass touched

- `RmNode::key`'s doc now says what it is — *"a **label, not an identity**"* — and points
  every keyed/indexed/deduped use at `RmNode::id()`.
- `RmGraph::references_of` / `pdb_of_resource` / `gpu_of_resource` / `map_ref_count_of`
  are the identity-shaped siblings of the handle-shaped resolvers; the handle-shaped ones
  are now documented as answering *"what is at this handle value **now**"*, which is a
  different and legitimate question, not a degraded one.
- `ResourceKey` is re-exported from `kayfabe_core`'s prelude beside `NodeKey`.
- **Open, unchanged:** N1–N6 and O5 above; §12.39 §7's O1–O4 bench questions; §12.39 test
  8's `security_invariants` fuzz property.

### 12.42 ★★★ N2 LANDED, AND N1 WITH IT — a component is labelled by a DECLARATION, not by an `hClient` value

**Status: LANDED.** 363 → **365 tests** — two new `#[test]`s; the third piece of coverage
is a new PHASE inside the existing mean run, which is where it belongs. Gate clean
(`KAYFABE_SLOW=1 cargo test --workspace`, `clippy --all-targets -D warnings`,
`fmt --check`, `check --target aarch64-unknown-linux-gnu`).
`ogkm` = `C: research_clones/ogkm`.

§12.41 §4 measured a corruption and deliberately did not land it: after
`declare A → alias → free root → re-declare A`, generation 1's still-live resources
belonged to **no** boundary, so their component vanished and `stage_dropped_vases`
released host memory a live kernel client still references
(`ogkm .../mem_mgr/mem.c:1027-1031`). §12.41 also named why it could not be patched:
`ProcAnchor` and `ProcBoundary::clients` were `HClient`s, so **two generations of one
`hClient` were not expressible as two components**. This entry closes both.

---

#### 1. THE CHANGE — `ClientKey`, and it is `ResourceKey` one level up

`ClientKey { client: HClient, incarnation: u32 }` is the identity of one client-namespace
**declaration**. It is minted at the root `Alloc` as *the smallest ordinal no LIVE
declaration at that value already holds* — verbatim `ResourceKey`'s rule, and legal to
**report** for verbatim `ResourceKey`'s reason: the ordinal is order-independent because
declarations at one `hClient` value are **totally ordered by the protocol** (a second root
`Alloc` while the value has a live root is `DuplicateClientRoot`, mirroring RM's
`NV_ERR_INSERT_DUPLICATE_NAME` at `rs_server.c:3352-3357`; and the `Free` that releases a
declaration must follow the `Alloc` that made it, because an event naming an undeclared
namespace is refused). A `ClientId` — minted from a counter — still cannot be reported, and
still is not: `Boundaries: PartialEq` **is** decision #4.

| plane | before | after |
|---|---|---|
| `ProcAnchor` | `HClient` | `ClientKey` |
| `ProcBoundary::clients` / `Proc::clients` | `BTreeSet<HClient>` | `BTreeSet<ClientKey>` (+ `client_values()`, the lossy by-value view for diagnostics) |
| `RmGraph::client_declarations` | `BTreeMap<HClient, _>` — **one slot per value**, live root wins else newest orphan | `BTreeMap<ClientKey, _>` — **every live declaration** |
| `RmGraph::client_kinds` | `(HClient, ClientKind)` | `(ClientKey, ClientKind)` — the live-ROOTED declaration |
| `RmGraph::nodes_with_owner` / `owner_of` | `ClientId` | `ClientKey` (`owner_key_of`) |
| `Spine::condemned`, `Proc::client_ids` | `ClientId` | **unchanged** — `ClientKey` ordinals are reused after death, `ClientId`s never are. `clients` LABELS, `client_ids` MATCHES (§12.40's rule, still exact) |

`RmGraph` gains one index, `decls: BTreeMap<ClientKey, Declaration>`, refcounted by the
resources each declaration owns. A declaration is live iff it has a live root **or** owns a
resource a foreign alias keeps alive — the same statement, because a live root *is* a live
resource of its own namespace. It is bounded by the live resource count (G10, §12.22), it
made `client_declarations` O(declarations) instead of O(all resources), and its refcount
reaching zero is exactly when an ordinal becomes reusable.

★ **`project`'s membership predicate is GONE, and that is the fix.** The client universe
*is* the set of live declarations, and every live resource projects under the declaration
that allocated it. §12.39/§12.40 held isolation with an **exclusion**; §12.42 holds it with
the **identity**, so nothing RM says is live is dropped on the floor. The dup-endpoint chain
§12.27 added went with it, now provably redundant rather than merely usually so: a resolved
dup's `dst` handle is live ⇒ its namespace has a live root (§12.38) ⇒ that root is a live
resource; and the `src` side resolves to the declaration that owns the resource.

★ **The grouping predicate reads the edge's `src` VALUE nowhere at all now.** Both ends are
declarations: `dst` is the live root of the destination handle's namespace, `src` is
`owner_key_of(dst)` — the declaration that allocated the resource, read through the
destination handle, which is live by construction.

---

#### 2. WHAT WAS WRONG IN THE BRIEF, AND IN §12.40

- **§12.40's "the exclusion bites at exactly one moment: a re-declaration … the old
  component is retired through the ordinary staged-death path and the new one inherits
  nothing"** is the defect, written down as the rule. Retiring that component *is*
  releasing host memory RM refcounts. The corrected sentence: a re-declaration mints a new
  declaration and changes **nothing** about the old one, which keeps its component, its
  isolate, its arena and its host VAS until its last resource dies.
- **Two landed assertions had to be inverted**, and they were the ones stating the defect:
  `l1_mean::a_recycled_namespace_cannot_inherit_the_previous_tenants_address_plane` and
  `rmgraph_order_independence::a_freed_and_redeclared_namespace_projects_identically_in_
  every_order` both asserted *"the SUPERSEDED declaration's address plane must belong to
  nobody"*. Belonging to nobody is the corruption. They now assert that it belongs to a
  **different component** — which is the isolation property they were actually written for,
  stated without the collateral damage.
- **The brief's framing that the N1 test could be written on the conservation ledger alone
  is not quite right, and the first draft of the test was wrong because of it.** `vacate`
  **stages** before it removes (§12.35), so at the instant the successor declares, the
  ledger still shows the victim's objects outstanding — the `Free` verbs have not run yet.
  A ledger-only assertion **passed under the bite**. The test now asserts the pair the
  teardown post-condition itself uses — `Reachable(core state)` first, then the ledger after
  a `reap_retired()` that drives the scheduled reclamation — so the report is *"we released
  something RM says is live"* and not *"a count differed"*.
- **`is_condemned(client)` needed a two-armed answer**, not the one-armed one the naive port
  gave it. A value with a live root is answered about that declaration only (a fresh tenant
  of a condemned predecessor's value is **not** condemned); a value with none is answered
  about its orphans (§12.37's evasion gate: freeing the root must not launder the
  condemnation away). The one-armed version broke
  `evasion_dup_a_fresh_client_then_free_the_old_root_still_fails` immediately.
- **`a_root_kept_alive_by_a_dup_no_longer_occupies_its_own_namespace` changed answer**, and
  the new answer is the honest one: a fresh root at a value whose ORIGINAL client object a
  foreign alias still keeps alive is `incarnation: 1`, because both declarations are
  simultaneously live. The ordinal returns to 0 once the alias dies, which the test now also
  pins.

---

#### 3. THE TESTS, AND EVERY BITE — INCLUDING THE TWO THAT DID NOT BITE

New: `l1_mean::a_superseded_declarations_kernel_held_memory_is_never_freed_by_its_
successor` (integration: the whole device, the six-proc two-GPU world, three generations of
one `hClient`, the UVM session holding references, the conservation ledger);
`generation_recycle` wired into **`mean_run` phase 5 (g)** so the same script runs inside
the composed window (five parked host verbs, six workload threads, two out-of-band retires,
T0 churn) under **both** lock modes; and
`security_boundary::b2_a_parked_page_directory_rebind_is_accepted_at_the_cap` (N6).

| bite | what fired |
|---|---|
| **A** — restore §12.40's one-slot-per-`HClient` exclusion (the N1 fix reverted) | 4 tests. The N1 test fires on its FIRST assertion — *"★★ HOST MEMORY RM SAYS IS LIVE WAS TAKEN AWAY FROM ITS OWNER"*, `left: {}` vs `right: {iso7:0x800000001, iso7:0x800000002}`. Also the mean run, `a_recycled_namespace_cannot_inherit_…`, and `a_freed_and_redeclared_namespace_projects_identically_in_every_order` |
| **B** — collapse the incarnation in the component label (`ProcAnchor`/`clients` are `HClient`s again — the N2 fix reverted) | 5 tests. Same first assertion, opposite shape: `left` is a strict SUPERSET containing generation 2's host handles (`iso7:0x801…`) — the two lifetimes collapsed into one `Proc`, one isolate, one arena. The hangs-a-legal-guest gate `a_recycled_namespace_is_a_different_component_and_is_not_condemned` fires too |
| **C** — `next_client_incarnation ≡ 0` | 8 tests, across 5 files, including the hostile-stream fuzz property `a1b_gpu_spine_never_panics_on_hostile_stream` and `c_bug_regressions::freeing_a_dup_alias_on_a_reused_client_handle_never_tears_down_the_namespace`. The N1 test fires on the internal `debug_assert` in `retain_declaration` — *"two identities at one live ClientKey"*, `ClientId(42)` vs `ClientId(51)` |
| **D** — `release_declaration` no-op (declarations never die) | 20+ tests across 8 files. The new order-independence assertion fires by name: *"the orphaned declaration outlived the last resource it owned"*, `[{B,0}, {B,1}]` vs `[{B,1}]` |
| **E** — N6 reverted (the parked-PDB capacity gate ignores the key again) | `b2_a_parked_page_directory_rebind_is_accepted_at_the_cap`, `left: Err(CapacityExceeded(PendingPdbs))` vs `right: Ok(())` |
| **F** — `client_kinds` reports `ClientKey::first` instead of the root's real ordinal | 3 tests (`a_recycled_namespace_cannot_inherit_…`, the hangs-a-legal-guest gate, `a_root_kept_alive_by_a_dup_…`). ★ **Did NOT reach the new N1 test or the mean run** — neither depends on the live-root map's ordinal, because neither needs a dup to group |
| **G** — remove the grouping predicate's live-rooted-source conjunct | ★★ **DID NOT BITE. Whole suite green.** |
| **H** — key `is_user`'s source check by the `HClient` VALUE (§12.40's shape) | ★★ **DID NOT BITE. Whole suite green.** |
| **G + H together** | `a_recycled_namespace_cannot_inherit_the_previous_tenants_address_plane`: the orphan merges into the attacker's proc, `left: Ok(ProcId(7))` vs `right: Ok(ProcId(9))` |

★ **G and H are the finding.** The two guards are **mutually redundant** under §12.42:
`kinds` holds only live-rooted declarations and is keyed by the *declaration*, so
`is_user(src_decl)` already refuses an orphan; and conversely the conjunct already refuses
what a value-keyed `is_user` would let through. Neither is individually falsifiable by the
suite. Both are kept — this is the predicate an isolation break walks through — and the
redundancy is now stated in the code rather than being an accident waiting to be
"simplified".

---

#### 4. N6 — a real, small, hangs-a-legal-guest defect, not the doc gap §12.41 guessed

§12.41 filed N6 as *"`pending_pdbs.insert` has no idempotency/conflict arm … probably
consistent, but the silence is undeclared"*. The last-wins semantics **are** consistent with
the resolved arm. The actual defect is one line earlier: the `MAX_PARKED` gate fired
*before* looking at whether the key was already parked, so at the cap a guest re-binding a
VASpace it had already parked a PDB for was refused `CapacityExceeded` for an event that
**adds nothing to the table** — the hangs-a-legal-guest direction. Its two sibling parked
tables never had that shape (a re-parked `Dup` returns early as
idempotent-or-`ConflictingDup`; `pending_maps` gates behind `!contains`), which is why the
asymmetry survived §12.38's audit: that audit checked the cap's *presence*, not its arm.
Fixed and tested.

---

#### 5. DEFERRED, WITH REASONS

- **N3 — `IsolateId` does not carry the `GpuId`.** Deferred. It is a different axis
  (host-object namespacing, not component identity), ~142 references with ~70 test sites
  pinning `IsolateId == ProcId` as a documented property, and it changes what the leak
  ledger measures — including the helper this round's N1 test reads
  (`HostLedger::leaked_on`). ★ Observation for whoever takes it: the N1 test is *slightly*
  weaker than it looks because of N3 — `outstanding_on(gen1)` merges a proc's per-GPU
  isolates. It does not affect the result here (generation 1 spans only GPU0), but a
  two-GPU generation would need the split first.
- **N4 — `BatchId` minted per target, consumed per proc.** Deferred. It is the same
  *family* (a value used as an identity outside the scope it is unique in) but a different
  *organ*: the completion plane, not the component plane. It needs its own decision (key
  `in_flight` by `(GpuId, BatchId)` vs mint device-globally), its own two-GPU
  progress-under-pending test, and it shares nothing with the RM-graph identity model.
- **N5 — `OsEventRef` is guest-minted.** Deferred, unchanged: contained by the per-proc
  container, and the settling question §12.41 poses ("does any path let one proc's ack
  reach another's queue?") is a read of the completion crate, not of this one.
- **O5** — GSP-side PDB uniqueness. Unchanged, still an experiment.
- **Open, unchanged:** §12.39 §7's O1–O4 bench questions; §12.39 test 8's
  `security_invariants` fuzz property (still not written).

---

### 12.43 ★★★ `gpa_read`/`gpa_write` WERE A GUEST-STEERABLE LOCK INVERSION — and the harness could not express it

**Status: LANDED.** 365 → **372 tests** (six focused port-contract tests + one composed
mean test; the mean one is in `l1_mean.rs`, where the doctrine says it belongs). Gate
clean (`KAYFABE_SLOW=1 cargo test --workspace`, `clippy --all-targets -D warnings`,
`fmt --check`, `check --target aarch64-unknown-linux-gnu`). Uncommitted, for review.

**The brief** (`l1_os_shell.md` §10.1 item 6, from the `qemu_102_facilities.md` inventory):
`Vmm::gpa_read`/`gpa_write` are classified **in-lock legal** on the grounds that they are
*"a memcpy into an already-installed mapping"*. On a QEMU backend the obvious
implementation is `address_space_rw`, which **takes the VMM's global lock when the target
GPA lands on MMIO**. Verified against the tag, first thing, because the whole §6.3.1
lesson is that a named API is a claim about a version:

- **[src] `v10.2.0 system/physmem.c:3196-3209`** — `prepare_mmio_access`:
  `if (!bql_locked() && !mr->lockless_io) bql_lock();`
- **[src] `:3250`** (`flatview_write_continue_step`) and **`:3347`**
  (`flatview_read_continue_step`) call it whenever `memory_access_is_direct` is false.
- **[src] `:3448`** — `address_space_write` itself takes only `RCU_READ_LOCK_GUARD()`.

**All three citations are exact.** A guest that points a GPFIFO entry at a device page
therefore turns a rank-0-held memcpy into a global-lock acquisition beneath one of our
ranked locks — §6.3's ABBA, built to order, and invisible to all four of §6.3's
enforcement layers.

---

#### 1. ★★ The finding that mattered most: the harness could not express the hazard

`MockVmm`'s guest-physical space was a sparse `BTreeMap<u64, u8>` in which **every**
address is RAM and `gpa_read` had **no failing arm at all** — it returned `Ok(())`
unconditionally, reading absent bytes as zero. Not "the test would have been weak": the
test was **unwritable**. Any refusal assertion would have been a green instrument on a
path the harness could not construct (`testing_doctrine.md` §1).

Worse, the mock's own comment asserted that this *was* real-adapter behaviour — *"an
un-formable address is simply absent → reads 0, exactly a real adapter's unbacked-page
behavior"* — while `vmm_portability.rs`'s second backend in the same suite returned
`VmmError::BadGpa` for exactly that shape and called it *"the contract"*. **Two mock
backends disagreed about whether a wrapping range is serviceable**, and the sparse one was
wrong. `c_bug_regressions.rs::cbfuzz_gpfifo_range_gpa_near_umax_never_panics` had encoded
the wrong one as an expectation; it is strengthened in place, with the reasoning in its
doc comment (the property it *names* — bounded, never panics — is untouched).

#### 2. The fix — a POSITIVE proof in the port, one classification site in the core

`kayfabe_vmm::GuestRamMap` (new). Declared guest-physical regions of
`RegionKind::{Ram, Device}`; `resolve(gpa, len)` proves a range lies wholly inside one
**RAM** region or refuses:

| refusal | means | why it must stay distinct |
|---|---|---|
| `VmmError::NonRamGpa { gpa }` | a **device** is there | the guest-steerable inversion — the only signal it happened |
| `VmmError::BadGpa { gpa }` | **nothing** is there, or the range is un-formable, or it leaves its region | the ordinary miss |

`kayfabe_fwd::guest_read` is the **one** core-side site that touches guest memory and the
one place the refusal is classified (`NonRamGpa → FwdFault::NonRamGpa`, everything else →
`FwdFault::GpaRead`). A new CI gate holds it there: zero `.gpa_(read|write)(` in the pure
crates, exactly one in `kayfabe-fwd`. The code it replaced was `map_err(|_| GpaRead)` —
i.e. the discarded variant *was* the finding, which is §12.10's wrong-reason conflation on
the security-relevant arm.

**Where the refusal belongs: the PORT, not the core.** The core cannot know which GPAs are
RAM — only the VMM does. Putting the check in the port crate rather than in each adapter's
prose is the `RmGraph::undeclared_namespace` pattern: one central check, two argued
exemptions (a zero-length access is still checked; a range must lie in one region), the
`file:line` citations inline.

#### 3. ★ R5: the obligation does NOT arise, and that is a design choice, not luck

The brief asked whether the cached RAM resolution needs re-validating after a re-lock.
**No — provided nothing resolved crosses the port**, which is why there is deliberately no
`RamGpa` proof-token type. A token minted by `resolve` and consumed by a later copy would
be a resolved backing held across a lock gap, and `Vmm::unmap_guest` is a "NO" row that
runs lock-free on another thread: the token would *create* an R5 obligation and a
use-after-free surface that the indivisible resolve-then-copy shape simply does not have.
The core holds only a `u64`, and a `u64` cannot dangle. **The tempting
"make-it-unrepresentable" move was the wrong one**, and the reason is written into
`GuestRamMap`'s rustdoc so it is not re-attempted.

**What the fix DOES introduce is a new lock, and it must be ranked.** A real adapter's
region map is mutated by the coarse memory plane (§6.7 window install/remove — lock-free)
and read from an in-lock-legal accessor with rank 0 or 1 held. That needs synchronisation,
and it must be a **leaf**: ours, ranked below rank 1, acquiring nothing beneath it, with a
bounded memcpy as its critical section. Closing a foreign-lock hazard by introducing an
*unranked* lock of our own would trade one invisible inversion for another. The test
harness models exactly this shape (`kayfabe_tests::SharedVmm`).

#### 4. Interactions, checked rather than assumed

- **`guest_memory_lock.md`:** yes, and it is already closed — **by GL4**. The memcpy under
  the leaf lock could block on a uffd-WP fault if a DMA target were ever armed, which
  would be an R1 violation created by this fix. GL4 makes DMA-target and isolate-shared
  ranges **unlockable by construction, refused at registration**, and §3.1 classifies
  userspace pushbuffers as copy-once/not-lockable. So the hazard is unconstructible.
  Nothing in either doc said so; it is written down now.
- **§6.7 memslot/arena:** aligned, and it is what makes the fix cheap. The region map's
  granularity is the **window**, not the object: a `MAP_FIXED` placement inside an
  installed window changes no region, so `GpaArena::alloc/free` never touches the map and
  the leaf lock is contended only at proc create/destroy frequency.
- **§6.3.1 lockless IO:** unaffected. The flag suppresses *taking* the global lock on
  **our** regions; every other device's regions are untouched by it, and those are the
  reachable set.

#### 5. ★★ Three things in the brief / the inventory that were wrong or incomplete

1. **"Refuse a non-RAM GPA" is not quite the right rule — the right rule is "prove RAM".**
   **[src] `v10.2.0 system/physmem.c:3010-3017`**: `io_mem_init` calls
   `memory_region_enable_lockless_io(&io_mem_unassigned)`, commented *"Trivially
   thread-safe since memory accesses are rejected"*. So at ≥ 10.2 an **unassigned** GPA
   does *not* take the global lock — meaning a deny-list built from "MMIO is dangerous"
   would be both over- and under-inclusive. A positive allow-list is the only stable rule,
   and it is what landed.
2. **Upstream has a facility that looks like the fix and is not sufficient.**
   `MemTxAttrs.memory` (**[src]** `include/exec/memattrs.h:46`) makes `flatview_access_allowed`
   (**[src]** `system/physmem.c:3222-3238`) reject non-RAM with `MEMTX_ACCESS_ERROR`
   **before** `prepare_mmio_access` runs (`:3243` precedes `:3250`, and `:3339` precedes `:3347`). Tempting. But its
   RAM test is `memory_region_is_ram(mr)`, and **`memory_region_supports_direct_access`
   excludes `ram_device` regions** (`include/system/memory.h:3136-3151`) — which is exactly
   what `memory_region_init_ram_device_ptr` produces, i.e. **a VFIO-mapped device BAR
   passes the `attrs.memory` check and then takes the lock**. ROMD regions have the same
   shape on the write side. There is no `MEMTXATTRS_MEMORY` constant at v10.2.0 either.
   **Take the cached-pointer fix, not the attrs flag.**
3. **The straddling case was not in the brief and is the one a naive fix misses.**
   QEMU walks region by region (**[src]** `system/physmem.c:3289-3315`, `flatview_write_continue`), so a range that *starts* in RAM
   and runs into MMIO takes the lock on the **continuation step** — after the first bytes
   were a legal memcpy. A start-address-only check is therefore not a fix at all. The
   resolver names the boundary byte, and a dedicated test pins it
   (`a_read_straddling_ram_into_a_device_window_is_refused_at_the_boundary`, which bites
   under exactly that neuter and under nothing else).

#### 6. ★ And a fourth: the hazard was NOT REACHABLE through the L1 shell

`read_pushbuffer`'s own rustdoc said *"in L1 this runs under the device read lock"*. **No
L1 entry point ran it at all** — `SharedDevice` had no pushbuffer path, so the only callers
were tests holding a bare `&mut Gpu` and no lock. The doc described an intention.
`SharedDevice::parse_pushbuffer` now exists in exactly the shape the doc claimed (route
phase = rank 0 + the guest read; act phase = the owning proc's rank-1 lock), which is what
lets the mean test drive the hazard through the real lock shell in **both** lock modes.

The premise is asserted, not assumed: `SharedVmm` witnesses `lock::held_depth()` at every
port access and the mean test asserts the span is exactly **`(0, 1)`** — max 1 because the
pushbuffer reads really did run under rank 0, min 0 because the scripting writes really did
run lock-free. Without both ends the whole test could be green about a lock-free path.

#### 7. Bite-checks — including the two that did NOT bite

Thirteen neuters. Every one is reported, because two of the informative results are
failures of *my own instruments*:

| # | neuter | bit |
|---|---|---|
| N1 | `resolve` treats `Device` as RAM | 3 tests (mean + 2 focused) |
| N2 | `resolve` checks only the START address | mean (straddle arm) + 2 focused |
| N3 | drop the un-formable check | bit, but by **panicking inside the neutered code**, not by an assertion — a weak bite, so re-run as N3b |
| N3b | un-formable range **clamped and served** (the realistic mistake) | 2 tests, on their assertions |
| N4 | `punch` drops the split remainder's backing offset | 1 focused test, on an exact offset |
| N5 | restore `map_err(\|_\| GpaRead)` (the original code) | **mean only** — no focused test can see it, which is the argument for the mean test |
| N6 | `MockVmm` skips the proof entirely | mean + `cbfuzz`; **none of the six focused resolver tests bit**, because they exercise `GuestRamMap` directly and are blind to whether any backend uses it |
| N7 | `undeclare` is a no-op | mean + 1 focused |
| N8 | remove the mean test's mid-flight window teardown | mean (revoke arm), `first_refusal: None` |
| N9a/b | the CI gate: a second accessor in a pure crate / a second site in `kayfabe-fwd` | both fire |
| N10 | `resolve` refuses **everything**, RAM included | mean, on the non-vacuity arm |
| N11 | the route phase reads no guest memory | mean, on the non-vacuity arm (so the depth witness was not isolated by it) |
| N12 | the depth witness reports a **constant** | ★ **DID NOT BITE** — see below |
| N13 | the in-lock read is not witnessed | mean, `(0,0)` vs `(0,1)` |

**★ N12 is the finding.** A constant witness passed, because `SharedVmm` derived
`Default` and `Arc<AtomicU32>::default()` is `0` — so the *minimum* watermark started at
its own success value and the lower half of the assertion was **vacuous**. Fixed by
hand-writing `Default` (the same defect the new `MockVmm::default` avoids for the same
reason: a derived `Default` produced an EMPTY region map, in which every GPA refuses).
Re-running N12 after the fix surfaced a second one: *every* port access was in-lock,
because the harness scripted guest memory through `with()` and bypassed the port entirely.
Scripting now goes through `Vmm::gpa_write`, which is both more realistic and what makes
the `(0, 1)` span an observation instead of an artifact. **Two real defects, both in the
instrument, both found only by honestly reporting a non-biting neuter.**

**★ And N10 found a harness HANG.** With the resolver refusing everything, the revoke
prober panicked before signalling, and the main thread spun forever inside `thread::scope`
on a flag nobody would set — the failure read as a wedge, not as an assertion. That is
precisely the shape `Latches`' own `Drop` guard exists for. The spin is now a condvar gate
that **opens on unwind** (`StartGate` + `OpenOnDrop`). Bite-checking found a hang that no
amount of green running would have.

#### 8. Residuals, named

1. **No adapter exists**, so nothing here is measured. The QEMU-side claim is a source read
   at one tag; the *cost* of the leaf lock and of the resolve on the pushbuffer hot path is
   unmeasured and belongs to L2-Q.
2. **The leaf lock's rank is documented, not enforced.** There is no ranked-lock type in
   `kayfabe-vmm` (the port must not depend on the shell), so an adapter that gets it wrong
   trips no gate. The `lock_depth_span` witness proves the *accessor* runs in-lock; nothing
   proves the adapter's own map lock is a leaf. That is an L2 review obligation, named here.
3. **`gpa_write` has no core call site yet**, so its half of the contract is exercised only
   by the harness. It is written into the trait now, before the first caller, deliberately.
4. **Multi-region spans are refused, not served.** Argued from §6.7 (coarse windows), not
   measured against a real guest's descriptors. If a legitimate one ever straddles two
   windows, the fix is a loop that re-proves each step — never a resolve-once-copy-across.
