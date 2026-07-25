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
  - *How it would bite without it:* review P1, verbatim — proc A's 5.4 ms alloc (or
    its 3.5 s worst-case interrupt unwind) holds the device read lock; the completion
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
  on interrupt it still blocks (uninterruptibly, bounded by the worker's EINTR-unwind
  — the C measured ~3.5 s worst case) for the `Interrupted` reply, then surfaces the
  refusal. Per R1 it holds NO lock while waiting (revised in #37 — the original text
  held the proc lock throughout; under R1 that 3.5 s hold would stall every sibling
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
the only state they share is the RM fd, which is kernel-mediated — concurrent ioctls
on one RM client are ordinary host behavior (multithreaded libcuda does it all day).
On the QEMU side the pool is N `&mut`-owned worker handles; checkout (§7.3) transfers
one to the calling thread. Honest note: this re-admits threads into the worker
process that #34 deleted — see §11 B6 for why the deleted *bug class* stays deleted.

**Calibration (owner-directed, explicit).** Design the *interface* for N-in-flight
from day one — checkout/return, per-worker channels, per-worker interrupt, per-worker
reactor sources — but implement a **BOUNDED, statically-sized pool first** (small, on
the order of the vCPU count; the exact default is an L1-M1 tuning constant, not a
design question). Make the pool *dynamically* scaling only when a measured workload
shows the bound hurts. Premature dynamic scaling is a complexity trap: a spawn/reap
policy, thundering-herd wakeups on growth, worker-lifetime races — all cost, no
demonstrated benefit. Pool exhaustion is meanwhile well-behaved backpressure: the
guest thread waits (lock-free, R1) for a worker, exactly as it would wait for the
host ioctl itself — and the poll/completion path needs no worker at all (§3.5), so
sync progress never queues behind the pool.

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
- **B3 (improved by #37, still the honest residual): the system proc is special and
  that's acceptable.** Kernel/CeUtils traffic routes to `gpu.system`, whose isolate
  is as stall-prone as any. A D-state host ioctl on a *user* proc's isolate stalls
  that proc's calling thread (contained, by design). On the **system** proc, the #34
  shape stalled effectively the whole device (the stalled verb held the device read
  lock; every apply/pump queued behind it). Under R1 that coupling is gone: the
  stalled system verb holds NO lock, sibling system verbs proceed on other pool
  workers (§7.2), and apply/pump proceed freely — what stalls is only the specific
  system-proc op whose worker is wedged. Mitigation for that remainder: the #73
  interrupt + a watchdog deadline (`Vmm::defer`) that interrupts and retires a verb
  exceeding its budget, surfacing a loud fault instead of a silent stall. That
  converts an unbounded stall into a bounded failure, which is the best available —
  a D-state host thread is un-killable by anyone; the C wedged the whole GPU on these
  (F5); we wedge one op's requester and *say so*. Flagged as owner decision §10.6.
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
- **B6 (new, #37): N workers re-admit — deliberately and boundedly — the concurrency
  the original design deleted.** The C stub's thread-pool bugs came from a shared
  in-flight slot table and txn demultiplexing over shared channels; the pool keeps
  per-worker 1-deep channels and per-worker `&mut` ownership, so that bug class has
  no home to return to. The bet is that channel-COUNT concurrency is categorically
  safer than channel-MULTIPLEXED concurrency — believed strongly, argued from the
  type system, proven only by the mean test.

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
- **An out-of-band retire does not rebuild the routing maps.** `Spine::retire_proc`
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
