# L1 concurrency design — threads, locks, and completion flow (decision #34)

**Status:** design for review, 2026-07-25 · The FIRST L1 step, written **before any L1
code** (owner mandate: design-first on the highest-risk seam). Scope: how the Linux OS
layer drives the proven L0 core (`core_state_and_consolidation.md` at `1e8d55b`) under
real concurrency — N vCPUs, blocking host ioctls, interrupt delivery — without breaking
a single core invariant.

**Companion docs:** `core_state_and_consolidation.md` (§2 the port surface, §3 the
invariants — the hand-off contract this doc implements), `execution_plane.md` (§2.4
completion patterns a–e), the crate docs of `nvkvm-core`/`nvkvm-vmm`/`nvkvm-isolate`
(the compile-time-asserted concurrency contract, decision #17), and the C memory ledger
cited per-failure below. C-repo cites are prefixed `C:`.

**How to read this doc:** §1 is the inherited law. §2 is the recommended architecture in
one picture. §3–§8 are the six decisions, each with recommendation → rationale →
alternatives → what's genuinely open. §9 is the focused list of owner calls. §10 is the
honesty section: the bets, and the single biggest risk.

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
   multi-vCPU interleavings *sound* — §7.)
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
    one tiny audited raw module (mmap/volatile — §8.3); logic crates stay `forbid`.

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

Four thread roles, two locks, one queue. Everything else is a refinement.

```text
 vCPU threads (N, owned by the VMM)          watcher thread (1)            executor (1)
 ───────────────────────────────────         ──────────────────            ─────────────
 KVM exit → Device::mmio_read/write          epoll_wait on:                drains CoreEvent
   │  decode trap (pure)                       - isolate sockets (HUP)     inbox in order;
   │  ┌───────────────────────────┐            - host os-event fds         runs Device::event
   ├─►│ device lock (RwLock)      │            - timerfd (defer deadlines)   - Deferred(…)
   │  │  read: route/resolve      │          readiness → CoreEvent           - IsolateComplete
   │  │  write: apply/refresh/    │          → push inbox, wake executor     - LockedRegionFault
   │  │         pump/poll/drained │          (touches NO core state)       calls core under the
   │  └───────────────────────────┘                                        same two locks as a
   │  ┌───────────────────────────┐                                        vCPU thread would
   ├─►│ per-Proc lock (Mutex)     │
   │  │  &mut Proc entry points:  │          isolate workers (1 process per (Proc,GpuId))
   │  │  publish_backing, the     │          ─────────────────────────────────────────────
   │  │  doorbell act-phase,      │          sandboxed, unprivileged, single-threaded
   │  │  RmBackend verbs (MAY     │◄────────► verb loop: recv → real RM ioctl → reply
   │  │  BLOCK — confined here)   │  socket   + signal handler for interrupt (#73 pattern)
   │  └───────────────────────────┘  (sync,   strictly one verb in flight (by &mut
   │  write GSP reply, resume vCPU   1-deep)  construction — §6.2)
```

- **vCPU threads** enter at trap dispatch, do short pure work under the device lock,
  and — for per-proc work — drop to the owning proc's lock, where a **bounded blocking
  isolate verb is permitted**. The guest is blocked awaiting the RPC/trap reply anyway;
  synchronous is the honest shape (§4).
- **The device lock** (`RwLock`) guards the device-global spine: `RmGraph` mutation
  (`Gpu::apply`), projection refresh, routing maps, per-target `DeliveryPlane`
  pump/poll/drained, target minting. Its write sections are **pure logic only — the
  cardinal rule is that no host I/O ever blocks under it** (rule R1, §3.3).
- **Per-proc locks** confine blocking to the proc that asked for it: the #14
  blast-radius boundary reused as the concurrency boundary. Two procs' doorbells,
  publications, and host allocs proceed in parallel; a wedged host ioctl stalls only its
  owner (with one honest residual — §10).
- **The watcher** is the F3 fix done right: one `epoll_wait` (blocking, zero busy-poll)
  over isolate sockets + host os-event fds + the defer timer; it converts readiness into
  `CoreEvent`s and touches no core state. It is the ONLY producer to the executor inbox
  besides `Vmm::defer`.
- **The executor** is the serialized executor `Vmm::defer` names (in QEMU: the main
  loop/BH context; in the harness: an explicit loop). It runs `Device::event` for every
  `CoreEvent` — deferred reaps, isolate completions, lock faults — under the same locks
  as any vCPU thread. Isolate I/O completes here, never by re-entry from an isolate
  thread (inherited law 9).
- **Isolate workers** are processes, not threads: one per `(Proc, GpuId)`, spawned by
  the factory with the Mode-1 stub posture (namespaces, pivot_root, seccomp, cleared
  env/fds, unprivileged uid). The wire protocol is strictly one-request-one-reply,
  single in-flight (§6.2), interruptible (§5.4).

Lock order, total and one-way: **device → proc → leaf** (inbox/recorder). Never
acquire the device lock while holding a proc lock; no lock held across a thread join or
barrier. (The same discipline `tests/tests/concurrency_stress.rs` already documents and
enforces for the mock harness.)

---

## 3. Decision 1 — the driving model: how N vCPUs invoke the core

**★ THE central decision. Owner confirmation requested (§9.1).**

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

1. **The blocking-ioctl reality decides it.** `RmBackend` verbs are real host RM ioctls
   over a socket to a sandboxed worker: microseconds typically, milliseconds for allocs
   (the C measured alloc ~5.4 ms), *unbounded* in the failure case (D-state — the C's
   wedge class). Under (b)/(c) each such call holds the whole device. Under (a) it holds
   exactly the proc that asked. Confinement — not elimination — of blocking is the
   design (see §4 for why not full-async).
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

### 3.3 The rules that make (a) sound (the load-bearing part)

- **R1 — no host I/O under the device lock. Ever.** Device-lock write sections are pure
  core logic + at most `Vmm::raise_irq` (an eventfd/irqfd write — non-blocking, bounded;
  explicitly permitted). Every `RmBackend` verb runs under a proc lock only.
- **R2 — proc ops hold the device READ lock for their duration** (including the
  blocking verb), plus their proc `Mutex`. This is what makes "device write lock"
  mean "quiesced": a writer (apply/refresh/pump) waits out in-flight proc ops and then
  sees a consistent world with no proc mid-mutation. Readers (other procs' ops, routing
  lookups) are unaffected by a blocked peer.
- **R3 — lock order device → proc, one-way, no exceptions.** A proc op acquires
  read-device then its proc; cross-proc ops acquire write-device (and thereby need no
  proc locks — R2 guarantees exclusivity). Deadlock-free by construction.
- **R4 — the doorbell path is route/act split.** Route (token decode, `by_vchid`
  lookup) under the device read lock → act (materialize-on-first-touch, working-set
  gate, ring) under the proc lock. See §3.4.

### 3.4 The honest cost: a core-shape change, requested by design discussion

Today `nvkvm_fwd::handle_doorbell` takes `&mut Gpu`, and `Gpu` owns its `Proc`s as
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

---

## 4. Decision 2 — execution substrate: OS threads, not async, not a new state machine

**Owner-flagged decision (explicitly called out earlier). Confirmation requested (§9.2).**

### 4.1 Options

- **(a) OS threads with a small fixed role set** (§2: vCPU threads + watcher + executor
  + isolate worker processes). Blocking is expressed as blocking; the thread inventory
  is enumerable and appears in the §8 contract table.
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
  §7, where it belongs.)

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
| host RM verb (socket round-trip to isolate) | calling vCPU thread (or executor), under proc lock only | the host ioctl itself + the #73 interrupt path (§5.4) |
| wait for host os-event / isolate death / defer deadline | watcher, in `epoll_wait` | event-driven; no bound needed |
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
   - the isolate armed a host os-event fd; readiness fires in the **watcher's** epoll →
     `CoreEvent::IsolateComplete{session, cookie}` → executor. (The F3 producer,
     finally real. Arm-at-registration, so there is no per-wait arming round-trip.)
   - a parse/exec path observes a `SemRelease`/fence advance synchronously (vCPU
     thread, under the owning proc's lock) → `CompletionQueue::observe` /
     `FenceArms::observe` right there.
   Observation is always per-proc state under the proc lock — never gated on any other
   proc (F2).
2. **Posting.** `deliver_completions(gpu, vmm, target)` — the pump — runs under the
   **device write lock** (it composes across procs' queues and consults the per-target
   drain gate; it is pure + one `raise_irq`, microseconds — R1-compliant). It is
   invoked on **edges only**:
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
watcher epoll (blocking wait, zero cost while idle) → `IsolateComplete` → executor →
`observe` + pump → IRQ. Every hop is edge-driven. The C's failure was a missing
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
  refusal. The proc lock is held throughout — correct, because the proc is dying
  anyway and confinement means nobody else waits on it.
- Trigger points: proc retire (guest process death detected via the RM protocol),
  guest reset, isolate watchdog (§10). `Isolate::retire()` = send interrupt if
  in-flight + refuse new verbs; reap (waitpid, namespace teardown) is deferred to the
  quiesce point per L10.

---

## 6. Decision 4 — isolate driving

### 6.1 One process per `(Proc, GpuId)`, driven synchronously by its owner's threads

Options considered: thread-per-isolate on the QEMU side (a dedicated relay thread per
isolate), a shared verb thread-pool (the C stub's shape), async socket I/O, or —
recommended — **no dedicated QEMU-side threads at all**: the calling thread (vCPU or
executor) that holds the proc lock does the socket round-trip itself.

Rationale:
- The verb surface is control-plane (alloc/map/schedule/ring/control) — short, and
  issued while the guest is blocked on the corresponding RPC/trap anyway. A relay
  thread would add a hop to every verb and create *idle thread × isolate count*
  scaling for nothing.
- **Deliberately absent from the verb surface: blocking host waits.** Waiting is done
  by the guest (passthrough sema) or by the watcher (os-event fd). A verb that could
  block indefinitely by *design* (as opposed to by host failure) would break the
  confinement story — refuse to add one; that's a design-discussion tripwire.
- Backpressure is inherent: the caller blocks, the guest's RPC stalls, the guest slows
  down. No queue to size, no overflow policy to invent.

### 6.2 The wire protocol is single-in-flight — by construction, keep it that way

`RmBackend` is `Send`-only and reachable solely through `Isolate::rm(&mut self)`
(the documented exception): **a shared reference to a backend is unrepresentable, so at
most one verb per isolate is ever in flight.** The wire protocol should *assume* this —
strict request/reply, 1-deep, no txn multiplexing (txn ids exist only for the interrupt
handshake, §5.4) — and the worker stays single-threaded (verb loop + signal handler).
This deletes the C stub's whole thread-pool/`worker_inflight_txn[]`/slot-mapping
apparatus, which was itself a bug source. The type system already paid for this
simplification; collecting it is free.

Flagged honestly: single-in-flight per isolate means a proc cannot overlap two of its
own host verbs. Per the traffic analysis (control-plane only; data plane is
passthrough) this costs nothing measurable; if a real workload ever proves otherwise,
the fix is per-channel sub-verbs batched into one request — not multiplexing.

### 6.3 Mapping to the shard; spawn; reap

- The isolate lives *in* the `Proc` (`Proc::isolates[gpu]`), so the proc lock IS the
  isolate lock — one lock, not two, no ordering question.
- Spawn (`IsolateFactory::spawn`) happens inside `Gpu::apply`/refresh (lazy, at target
  materialization) — which runs under the **device write lock**, and `fork+exec` of a
  sandboxed worker is not pure logic. This is a real R1 tension, resolved by making
  spawn **two-phase**: the factory under the lock only *reserves* (allocates the
  session slot, records intent — cheap); the actual fork/exec/namespace setup runs
  lazily at the first verb (proc lock, R1-compliant) or on the executor. The
  `IsolateFactory` doc already permits "spawn (or lazily reserve)" — L1 chooses
  reserve.
- Reap: `retire()` eager (interrupt + refuse, §5.4); `waitpid` + teardown at the
  quiesce point the adapter declares (GSP re-handshake / idle), on the executor, via
  `CoreEvent::Deferred(DeferredReap)` → `Gpu::reap_retired()` (which also recycles the
  GPA arenas — the #80 leak's fix). Worker death out-of-band (crash) is a watcher HUP
  → `CoreEvent` → retire the proc loudly (its completions die with it — the guest
  tore down or the sandbox failed; either way MISS=FAULT posture, no resurrect).

---

## 7. Decision 5 — deterministic testability (non-negotiable)

The core's whole value is mock-testable determinism; L1 must not be the layer where
that dies. The design principle: **the threads are a shell; everything they do is a
pure function the tests call directly.**

### 7.1 The thin-waist rule

All L1 *logic* — trap decode → core call → reply composition; readiness → `CoreEvent`
mapping; event dispatch; the pump-edge selection of §5.2 — lives in plain synchronous
functions with no thread, clock, or fd types in their signatures (they take
`&mut dyn Vmm`, core types, and byte slices). The threaded production shell (the ~few
hundred lines that own epoll fds, locks, and thread spawns) only moves bytes between
the OS and these functions. **Tests exercise the same functions as production** — the
only untested-by-determinism residue is the shell itself, which is covered by the
threaded stress tier (T2) and kept too small to hide logic in.

### 7.2 The three test tiers

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
  the lock discipline (R1–R4) are validated — including mean tests: a scripted
  `MockRmBackend` that *stalls* a verb (proving other procs progress — the
  §3 confinement claim, as a test), a scripted interrupt mid-verb (the §5.4 handshake),
  and a kill-the-worker HUP (the §6.3 death path).
- **T3 — loom: not applicable by rule.** §4.2 forbids atomics/lock-free in L1 logic;
  if that rule is ever revisited, a loom model of the new path is the mandatory toll.

### 7.3 Clock discipline

No wall-clock read anywhere in L1 logic (the workspace already has no wall clock in
any logic crate — extend the rule): time enters only as `Vmm::now()` and leaves only
as `Vmm::defer(after, ev)`. The production shell derives its epoll timeout from the
earliest pending defer deadline — the one place wall time exists, inside the shell,
untestable and deliberately trivial.

---

## 8. Decision 6 — the explicit thread-safety contract per interface

Owner's rule: thread safety is assumed unless documented otherwise — so every L1
interface states its contract. This table is normative; each row becomes a rustdoc
header on the interface when L1 code lands.

| Interface | Send/Sync | Who calls, from where | Concurrency contract |
|---|---|---|---|
| `Device::{mmio_read, mmio_write, event}` | (trait on `Gpu`) | vCPU threads (traps), executor (events) | **serialized per device by the adapter's locks** (§3); entry implies device lock held per R2-R4. Never called from watcher or isolate context |
| `Vmm` impl (the real one) | `Send`, not `Sync` (as declared) | only as `&mut dyn` from within a core entry, i.e. under the caller's locks | one caller at a time by `&mut`; impl may keep fds/buffers without internal locks. `raise_irq` must be non-blocking (R1); `defer` must be deadline-ordered + deterministic (MockVmm-pinned semantics) |
| `RmBackend` impl (socket client) | `Send`, not `Sync` (the documented exception) | exactly one thread at a time, via `Isolate::rm(&mut)`, under the owning proc's lock | MAY block (the one sanctioned blocking site, §4.3); MUST be interruptible (§5.4); MUST NOT be callable after `retire()` (refuse loudly) |
| `Isolate` / `IsolateFactory` | `Send + Sync` (supertraits, core-stored) | proc-lock holders / device-write-lock holders (spawn=reserve only, §6.3) | mutation `&mut`-exclusive per the core contract; `is_retired()` is a pure read |
| executor inbox (`CoreEvent` queue) | the ONE concurrent L1 structure | producers: watcher, `Vmm::defer` impl; consumer: executor | `std::sync::mpsc` (or Mutex<VecDeque>+Condvar); no capacity-unbounded growth from guest input (events are per-fd/per-deadline, not per-guest-byte); FIFO per producer, total order defined by the executor's drain |
| watcher thread | owns its epoll fd set exclusively | nobody calls into it; it only produces inbox events | touches ZERO core state — enforced by giving it no reference to the device, only the inbox sender + fd registry |
| isolate worker (process) | n/a (process boundary) | its isolate's socket only | single-threaded; single in-flight; interrupt via signal; its handle namespace dies with it |
| the raw module (below) | per-item | adapter crates only | the only `unsafe` in the workspace |

### 8.1 `unsafe` policy (decision #16/#16b, applied to L1)

- Every L1 logic crate: `#![forbid(unsafe_code)]`, same as the core. The CI grep-gate
  extends to L1.
- **One audited raw module** (working name `nvkvm-linux-raw`) for the operations that
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

## 9. OWNER DECISIONS TO CONFIRM

Genuinely open calls, in review order. Everything not listed here I consider settled
by the inherited contract + the rationale above (push back anywhere, of course).

1. **★ The sharding model and its core-shape cost (§3).** Recommended: hybrid (a) —
   device `RwLock` + per-`Proc` `Mutex`, rules R1–R4 — **including the core refactor
   now** (route/act split of `handle_doorbell` + the `Gpu` ownership split), with
   L1-M1 shipping the degenerate one-lock configuration. The genuine alternative is
   (b)-first with the refactor deferred to the concurrency milestone — less core churn
   now, retrofit risk later. This is THE decision of the doc.
2. **★ Threads, not async (§4).** Recommended: fixed-role OS threads, no tokio, no
   atomics/lock-free in L1 logic (loom stays out by rule). You called this one out —
   please confirm or contest the rejection rationale.
3. **Blocking verbs on vCPU threads (§4.3/§6.1).** Recommended: yes — synchronous
   `RmBackend` round-trips on the calling vCPU thread under the proc lock (the guest
   is blocked on the RPC anyway), rather than bouncing every verb to the executor.
   The alternative (executor-only verbs) buys a cleaner "vCPU threads never block on
   sockets" story at the cost of a cross-thread hop per verb + a busier executor.
4. **Single-in-flight isolate wire protocol (§6.2).** Recommended: accept the
   `&mut`-derived simplification as the protocol spec (1-deep request/reply,
   single-threaded worker, no txn multiplexing). Cheap to confirm now, expensive to
   retrofit multiplexing later if I'm wrong about the traffic — say if you want the
   protocol framed to leave a multiplexing door open.
5. **The pump's backstop timer (§5.2).** Recommended: edge-driven pump only, plus a
   defer-armed backstop while completions are outstanding. Confirm the posture that a
   *periodic* redeliver sweep is forbidden (F1) — this is the kind of "harmless
   safety poll" that historically creeps in during a debugging session.
6. **The §10 residual** (system-proc stall containment + watchdog): accept as residual
   with the stated mitigation, or require a stronger story before L1-M1.

---

## 10. Honesty — the bets, and the biggest risk

**Bets this design makes, named:**

- **B1 (the biggest): synchronous-confined beats asynchronous-everywhere.** The design
  bets that `RmBackend` verbs are short and bounded enough that *confining* blocking
  (per-proc) suffices, and full async (state-machine-ify the core's forward paths,
  complete every verb via `CoreEvent::IsolateComplete`) is not needed. If the bench
  falsifies this — a common host verb with unbounded/long latency on the critical
  path — the fallback is per-verb async through the **already-typed**
  `IsolateComplete{session, cookie}` seam (it exists in `CoreEvent` today, unwired,
  for exactly this), at the cost of a plan/commit split of that verb's core path.
  The seam is held open deliberately; the doc's bet is that we never need it.
- **B2: the trap-minimization premise holds**, i.e. the contended lock paths stay
  off the steady-state hot path because the hot path has ~no traps. This was proven
  for the C bare-metal (`C: mode2_baremetal_32`) but the Rust L1 will first run under
  the same nested-virt bench where vmexit costs dominate everything — perf conclusions
  from the bench must be read through that filter (the C's rom-device lesson: a
  correct trap-elimination showed zero exit-count win under nested virt).
- **B3: the system proc is special and that's acceptable — the honest residual.**
  Kernel/CeUtils traffic routes to `gpu.system`, whose isolate is as stall-prone as
  any. A D-state host ioctl on a *user* proc's isolate stalls that proc (contained,
  by design); on the **system** proc it stalls kernel-traffic forwarding — effectively
  the device (and under R2, an apply waiting on the write lock behind it). Mitigation:
  the #73 interrupt + a watchdog deadline (`Vmm::defer`) that interrupts and retires a
  verb exceeding its budget, surfacing a loud fault instead of a silent stall. That
  converts an unbounded stall into a bounded failure, which is the best available —
  a D-state host thread is un-killable by anyone; the C wedged the whole GPU on these
  (F5); we wedge one proc and *say so*. Flagged as owner decision §9.6.
- **B4: scripted-order T1 testing is a faithful proxy for real interleavings.** This
  rests entirely on the core's order-independence (inherited law 1) plus the thin-waist
  rule (§7.1). It is a *good* bet — the determinism differential suite is exactly this
  argument, already green — but its blind spot is the shell (lock acquisition order,
  condvar wakeups), which only T2/TSan covers. Keeping the shell small is therefore a
  correctness strategy, not a style preference; shell growth in review is a smell.

**The single biggest risk, stated plainly:** B1/§3.4 together — the design commits to
a core ownership refactor *and* a lock discipline on the strength of pure-logic
reasoning and mock stress tests, before any real host ioctl latency distribution has
been measured under this architecture. If real RM verb latencies are much worse or
much *weirder* than the C's measurements (5.4 ms allocs, µs controls), the per-proc
confinement story stays *correct* but the perceived per-proc latency could disappoint,
and pressure will mount to widen concurrency (multiplexed isolates, async verbs) —
each widening re-opening exactly the race classes the C bled on. The discipline this
doc asks the owner to hold us to: widen only through the named seams (B1's fallback,
§6.2's batching), each as its own reviewed design change — never in a debugging
session.
