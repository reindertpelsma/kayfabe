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
| F1 | **0x110094 poll storm** (`C: mode2_execfwd_layer2`) | guest spin-polls a GSP status reg; every read a nested-virt vmexit; ~40k exits per phase | never busy-poll — on either side. Guest-side: read-native overlays (Vmm cap 7, an L2 fill). **Our side: every L1 wait is event-driven (epoll/condvar/deadline), never a spin loop** |
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
  F3 fix done right: one `epoll_wait` (blocking, zero busy-poll) over the registered
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

- **No busy-polling anywhere in L1 (F1).** Every wait is `epoll_wait`, a condvar, or a
  deadline. The completion pump is edge-driven (§5.2); there is no periodic "scan
  everything" thread. If a backstop timer proves necessary it is a `Vmm::defer`
  deadline, armed only while something is outstanding, cadence bounded — never a spin.
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
exist, because a `Proc` **is** a dup-connected component — a dup between two clients makes
them the same `Proc`. A tracing GC would re-derive what the graph states and would make
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
