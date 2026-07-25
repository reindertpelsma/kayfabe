---
title: "kayfabe Mode-2 — L1 Concurrency Architecture"
subtitle: |
  Review summary of the PLANNED design — for owner flaw-hunting.
  Sources: `docs/design/l1_concurrency.md` (decision #34) and
  `docs/design/core_state_and_consolidation.md` (#32), grounded against
  `nvkvm-core/src/gpu.rs` + `nvkvm-fwd/src/lib.rs` at HEAD `488117c`.
date: "2026-07-25 · status: DESIGN, pre-code — the route/act core refactor is in flight; no L1 code exists yet"
geometry: margin=2.1cm
fontsize: 10pt
mainfont: "DejaVu Sans"
monofont: "DejaVu Sans Mono"
colorlinks: true
header-includes:
  - \usepackage{pdflscape}
  - \usepackage{graphicx}
  - \usepackage{needspace}
---

# 1. What this is, and why L1 concurrency is the highest-risk seam

**kayfabe** is the Mode-2-only Rust rewrite of `nvkvm`: an **unmodified guest** runs the
stock NVIDIA kernel driver against an emulated GPU + faked GSP; we recover the guest's
*intent* (RM allocs, page-directory binds, doorbells, pushbuffer methods) and forward
real compute to a host GPU through **unprivileged, per-guest-process host isolates**.
The pure logic core (L0) is done and heavily gated (143 tests, 99.2% mutation kill,
TSan-green stress). **L1 is the Linux OS layer that drives that core under real
concurrency** — N vCPU threads, blocking host RM ioctls, interrupt delivery — and it is
the highest-risk seam because the C research artifact's worst bugs were exactly here:
the 0x110094 busy-poll storm (F1), the #14 round-8 completion starvation (F2), the
Mode-1 blocking-sync no-op producer (F3), the abandon-on-signal UAF (F4), de-facto
whole-bench serialization (F5), and the teardown reap hang (F6). The design's stated
goal is to make each of these *structurally impossible* — or explicitly, honestly
residual. This document summarizes the planned design faithfully, including its bets
and open questions, so the owner can hunt for flaws before code lands.

**In one line:** four thread roles, two locks, one queue — vCPU trap threads + one
epoll watcher + one serialized executor + per-`(Proc, GpuId)` isolate worker
*processes*; a device `RwLock` over the pure spine + one `Mutex<Proc>` per guest
process; a single `CoreEvent` mpsc inbox.

# 2. The architecture diagram

The next page shows the whole planned architecture: guest → trap threads → the two
lock domains → isolate workers → host GPU, and the completion path back (watcher →
inbox → executor → pump → SWGEN0 IRQ). The two numbered paths are walked step-by-step
in §4. (The diagram is a faithful rendering of `l1_concurrency.md` §2; the route/act
doorbell split it shows is the *planned* shape — today's `handle_doorbell` still takes
`&mut Gpu`, and the in-flight core refactor is what produces the split.)

\begin{landscape}
\thispagestyle{empty}
\begin{center}
\includegraphics[width=\linewidth,height=\textheight,keepaspectratio]{l1_architecture_diagram.png}
\end{center}
\end{landscape}

## 2.1 The four thread roles, precisely

| Role | Count | Does | May block on |
|:--|:--|:------|:----|
| **vCPU trap threads** | N (VMM-owned) | KVM exit → decode trap (pure) → core call under locks → GSP reply → resume | the isolate socket verb, **under proc lock only** |
| **watcher** | 1 | one blocking `epoll_wait` over isolate sockets (HUP), host os-event fds, timerfd; converts readiness → `CoreEvent`; **touches zero core state** (holds no device reference) | `epoll_wait` itself |
| **executor** | 1 | drains the `CoreEvent` inbox in order; runs `Device::event` under the *same* locks a vCPU thread would (isolate completions, deferred reaps, lock faults) | inbox condvar |
| **isolate workers** | 1 *process* per `(Proc, GpuId)` | sandboxed (namespaces, pivot_root, seccomp, unprivileged uid); single-threaded verb loop: recv → real RM ioctl → reply; strictly 1-deep; EINTR-interruptible (#73) | the real host ioctl |

Anything else that blocks is **forbidden by rule** — a new blocking site is a design
change, not a patch. Guest semaphore waits happen on the *guest's own vCPU* against
passthrough pages (no L1 thread at all — the trap-minimization payoff).

# 3. The lock discipline (R1–R4) — where deadlocks and races would hide

Two locks: the **device `RwLock`** over the pure spine (`RmGraph` + `apply`,
projection refresh, routing maps `by_pdb`/`by_vchid`, `GpuTarget` minting, the
completion pump) and **one `Mutex<Proc>` per guest process** (its `Vas`+AddressTable,
Channels+ExecPlane, GPA arena, CompletionQueue, and its isolates — the isolate lives
*in* the `Proc`, so the proc lock IS the isolate lock).

| Rule | Statement | What it prevents | How it could fail (scrutinize here) |
|:--|:------|:-----|:-----|
| **R1** | **No host I/O under the device lock, ever.** Write sections are pure logic + at most `Vmm::raise_irq` (non-blocking eventfd). | a slow/wedged host ioctl freezing the whole device (the C's F5 class) | any future "small" verb sneaking into `apply`/refresh — e.g. isolate **spawn** (resolved by two-phase reserve-then-lazy-fork; verify no other creep) |
| **R2** | Proc ops hold the **device READ lock for their whole duration** (including the blocking verb) + their proc Mutex. A device *writer* therefore waits out all in-flight proc ops and sees a quiesced world. | writers observing a proc mid-mutation; readers being blocked by a peer's stall | **the read-lock is held across a blocking host verb** — see §6.3 pressure point P1: every device *write* (incl. the completion pump) queues behind the slowest in-flight verb of ANY proc |
| **R3** | Lock order **device → proc → leaf** (inbox/recorder), total, one-way. No exceptions; cross-proc ops take write-device and thus need no proc locks. No lock across a thread join/barrier. | deadlock | any path acquiring the device lock while holding a proc lock — e.g. an act-phase that discovers it needs a routing update; the executor's observe→pump sequence must *release* the proc lock before taking write-device |
| **R4** | The doorbell is **route/act split**: route (token decode, `by_vchid` lookup) under device READ; act (working-set gate, materialize, schedule, ring) under the proc Mutex. | doorbell taking write-lock-shaped exclusivity; a second ungated ring path | the route→act handoff: proc retire between route and act (handled by the act-phase `is_retired` check — verify it stays first) |

Supporting rules: **no atomics, no lock-free structures, no hand-rolled sync in L1
logic** — `std::sync` only (keeps TSan meaningful, keeps loom out by rule); **no
busy-poll anywhere** — every wait is `epoll_wait`, a condvar, or a `Vmm::defer`
deadline armed only while something is outstanding.

# 4. The two hot paths, step by step, with lock state

## 4.1 A doorbell ring (route → act)

Grounded in today's `nvkvm_fwd::handle_doorbell` (the ONE gated ring path — nothing
else may ever call `RmBackend::ring_doorbell`), refactored per the plan into
`route_doorbell(&Gpu, token)` + `exec_doorbell(&mut Proc, …)`:

| # | Step | Thread | Locks held |
|:--|:------|:--|:-----|
| 1 | Guest writes the doorbell register → KVM exit | vCPU | none |
| 2 | Decode trap; recover the working set (pure) | vCPU | none |
| 3 | **ROUTE:** `arch.decode_doorbell(token)` (hostile → `MalformedToken`); `by_vchid[(target_gpu, vchid)]` → `(ProcId, ChanId)` | vCPU | device **READ** |
| 4 | Acquire the owning proc's Mutex; `is_retired` check (refuse if dying) | vCPU | device READ + proc |
| 5 | **ACT — gate:** `gate_working_set` against the channel's declared `Vas`'s AddressTable — any unbound or unpublished (`host_va = None`) VA is a loud **MISS=FAULT** *before* any host op | vCPU | device READ + proc |
| 6 | **ACT — materialize (first touch only):** `ensure_host_vas` → `rm.alloc_channel(hvas, engine)` (engine tag load-bearing — the C's wrong-runlist 401 designed out) → `rm.schedule` — **blocking socket round-trips to this proc's own isolate** (per-proc ExecPlane `scheduled` set: no global one-shot, #12's CTX2 bug designed out) | vCPU | device READ + proc (blocking **confined** here) |
| 7 | `rm.ring_doorbell(host_token)` — the actual ring (blocking verb, same confinement) | vCPU | device READ + proc |
| 8 | Release proc, release device READ; compose GSP reply; resume vCPU | vCPU | none |

Flaw-visibility notes: the *only* lock a stalled step 6/7 verb holds besides its own
proc is the device **read** lock — other procs' doorbells (readers + different proc
Mutex) proceed; a device **writer** does not (§6.3-P1). There is no wait-loop anywhere
in the path — every step either completes, faults loudly, or blocks in exactly one
sanctioned place (the socket), which is interruptible (#73 pattern).

## 4.2 A completion delivering back to the guest

Two observation routes, then one pump:

| # | Step | Thread | Locks held |
|:--|:------|:--|:-----|
| 0a | *(passthrough route — patterns a/c/e-read)* host GPU writes the semaphore into a passthrough guest page; the guest's own poll sees it. **No L1 thread, no lock, no trap.** | — | — |
| 0b | *(interrupt route — patterns b/d)* host RM signals the os-event fd the isolate armed at registration | host | — |
| 1 | Watcher's `epoll_wait` returns; maps readiness → `CoreEvent::IsolateComplete{session, cookie}`; pushes inbox, wakes executor. (Watcher holds no locks, touches no core state.) | watcher | none |
| 2 | Executor drains the inbox in order; dispatches the event | executor | none |
| 3 | **Observe:** `CompletionQueue::observe` / `FenceArms::observe` on the owning proc — always per-proc state, never gated on any other proc (the F2 fix) | executor | device READ + proc |
| 4 | Release proc + read lock; acquire device **WRITE**; **pump:** `deliver_completions(gpu, vmm, target)` composes the batch across procs' queues, consults the per-target drain gate (one batch outstanding per GPU's GSP queue — a *transport* gate, never an *observation* gate) | executor | device **WRITE** |
| 5 | Inside the pump: `Vmm::raise_irq(SWGEN0)` — non-blocking eventfd write (the one sanctioned side effect under the write lock) | executor | device WRITE |
| 6 | Guest takes the IRQ, reads its queue, acks | guest vCPU | — |

The pump runs on **edges only** — four of them: (i) after an observation lands (step
3→4, same thread); (ii) the guest's *own* `poll_completions` RPC (the poller's vCPU
thread re-posts the poller's un-acked events regardless of anyone else's activity —
the starvation fix); (iii) IRQSCLR → `completions_drained` → pump once more; (iv)
`Deferred(CompletionRedeliver)` — a `Vmm::defer` backstop armed **only while
completions are outstanding**, never periodic-forever. The explicit F2 checklist of
forbidden shapes: no delivery driven solely from another proc's doorbell; no
`any_completed`-style global gate before `observe`; no single delivery thread
round-robining procs behind one queue.

## 4.3 Signal interruptibility and teardown (the F4/F6 paths, briefly)

Every verb carries a txn id; on interrupt the worker's no-`SA_RESTART` handler makes
the blocked ioctl return `EINTR` and the worker replies `Interrupted{txn}` on the
normal reply path. The requester **never abandons the reply buffer** (the C's UAF): it
blocks — proc lock held, bounded by the worker's EINTR-unwind, C-measured ~3.5 s worst
case — for the reply, then surfaces the refusal. `Isolate::retire()` = interrupt +
refuse new verbs (eager); `waitpid` + namespace teardown is deferred to the
adapter-declared quiesce point on the executor via `Gpu::reap_retired()` (L10 —
never inline in a teardown trap). Worker crash = watcher HUP → retire the proc loudly;
its completions die with it (MISS=FAULT posture, no resurrection).

# 5. The invariants L1 must not break (inherited from the core)

1. **Order-independence / protocol-not-trace** — deliver events from any thread in any
   order, but the SAME facts: no dedup, no drop, no synthesis. (This is also what makes
   scripted single-thread testing of interleavings *sound*.)
2. **MISS = FAULT** — never catch-and-guess, never retry a `Miss` into another VAS.
3. **Per-`(GpuId, ·)` isolation (I1)** — no isolate/arena shared across `(Proc, GpuId)`.
4. **Completion integrity (I2)** — per-proc queues; re-delivery off the owner's own
   poll; forge types to the system proc only; fence-jump refusals are final.
5. **Refcount soundness (I3)** — never cache resolutions across frees.
6. **DoS containment (I4)** — `apply` refusals stay contained; never tear down the
   device; never let one guest's refusal path serialize another's progress.
7. **The one gated ring** — all doorbells via `handle_doorbell`; nothing else calls
   `RmBackend::ring_doorbell`.
8. **Retire-eager / reap-deferred (L10)** — L1 declares the quiesce point and calls
   `reap_retired` there; not inside teardown, not never.
9. **The concurrency contract (#17)** — never cross-proc-serialize per-proc work
   (esp. completion delivery); isolate I/O completes via `CoreEvent`s on the executor,
   never re-entrantly; the deterministic single-thread test mode stays viable.
10. **Purity + `forbid(unsafe_code)`** — OS code in adapter crates; `unsafe` only in
    one audited raw module (`nvkvm-linux-raw`: mmap, volatile shared-page access, KVM
    ioctls) whose API review is part of L1's exit gate.

\needspace{8\baselineskip}

# 6. Risks, bets, and open decisions (read this section hardest)

## 6.1 The named bets

- **B1 — THE BIGGEST: synchronous-confined blocking beats async-everywhere.** The
  design bets `RmBackend` verbs are short/bounded enough (C measured: µs controls,
  ~5.4 ms allocs) that *confining* blocking per-proc suffices, and full async
  (state-machine-ifying the core's forward paths) is unnecessary. **Falsifier:** a
  common host verb with long/unbounded latency on the critical path. **Fallback seam,
  held open deliberately:** per-verb async through `CoreEvent::IsolateComplete{session,
  cookie}` — already typed in the core today, unwired — at the cost of a plan/commit
  split of that verb's path. The bet is that this seam is never needed.
- **B2 — the trap-minimization premise holds:** contended lock paths stay off the
  steady-state hot path because the hot path has ~zero traps (passthrough
  pushbuffers and semas). Proven for the C on bare metal; but the Rust L1 will first
  run under
  **nested virt where vmexit costs dominate** — perf conclusions from that bench must
  be read through that filter, or a correct design will look falsely slow (the C's
  rom-device lesson).
- **B3 — the system proc is special, and that is the honest residual:** kernel/CeUtils
  traffic routes to `gpu.system`, whose isolate is as stall-prone as any. A D-state
  ioctl on a *user* proc stalls that proc (contained, by design); on the **system**
  proc it stalls kernel-traffic forwarding — effectively the device (and, under R2,
  any `apply` waiting on the write lock behind it). Mitigation: #73 interrupt + a
  `Vmm::defer` watchdog that retires a verb exceeding its budget — converting an
  unbounded stall into a bounded, *loud* failure. (A D-state host thread is
  un-killable by anyone; the C wedged the whole GPU on these — this design wedges one
  proc and says so.)
- **B4 — scripted-order T1 testing is a faithful proxy for real interleavings.** Rests
  on core order-independence + the thin-waist rule (all L1 *logic* in plain sync
  functions; the threaded shell only moves bytes). Blind spot: the shell itself (lock
  acquisition order, condvar wakeups) — covered only by the T2 threaded-stress tier +
  TSan. Keeping the shell small is therefore a **correctness strategy**; shell growth
  in review is a smell.

## 6.2 The owner-decision ledger (`l1_concurrency.md` §9)

| # | Decision | Status |
|:--|:------|:---|
| 9.1 ★ | Sharding model (a): device `RwLock` + per-`Proc` `Mutex`, R1–R4, **including the core route/act + `Gpu` ownership refactor now**; L1-M1 ships the degenerate one-global-lock configuration; the 2×-concurrent #14 gate flips on real sharding | **CONFIRMED — refactor-now** (the split is in flight in the core at this writing) |
| 9.2 ★ | Fixed-role OS threads; no tokio; no atomics/lock-free in L1 logic (loom out by rule) | **CONFIRMED — OS threads** |
| 9.3 | Blocking verbs on the calling vCPU thread under the proc lock (vs. bouncing every verb to the executor) | recommended; open to contest |
| 9.4 | Single-in-flight isolate wire protocol (1-deep request/reply, single-threaded worker, no txn multiplexing — txn ids exist only for the interrupt handshake). Escape hatch if traffic disproves: per-channel sub-verbs batched into one request, **not** multiplexing | recommended; cheap to confirm now, expensive to retrofit |
| 9.5 | Edge-driven pump only + defer-armed backstop; a *periodic* redeliver sweep is **forbidden** (F1) — the "harmless safety poll" that historically creeps in mid-debug | recommended posture; needs explicit owner buy-in |
| 9.6 | Accept B3 (system-proc stall) as residual with interrupt+watchdog, or demand a stronger story before L1-M1 | **open** |

## 6.3 Pressure points — where this summary's author would look for flaws

*(These are derived observations from reading the design against the code — the
places where the argument is thinnest, offered as a reviewer's aid.)*

- **P1 — R2 couples every device WRITE to the slowest in-flight verb.** Proc ops hold
  the device *read* lock across their blocking verb; `Gpu::apply`, projection refresh,
  and — critically — the **completion pump** are *writers*. So proc A's 5.4 ms alloc
  (or its 3.5 s worst-case interrupt unwind, §4.3 — proc lock *and* read lock held
  throughout) delays **every proc's completion delivery** and all control-plane
  progress, not just proc A's. The confinement story is airtight for proc-vs-proc
  reader ops but *not* for writer latency. Related: `std::sync::RwLock` makes no
  fairness guarantee — sustained reader traffic could starve writers on some
  platforms. If there is a hole in "confined blocking," it is here; ask whether the
  pump really needs the write lock, or whether R2's read-hold really must span the
  blocking verb.
- **P2 — the observe→pump lock transition (§4.2 steps 3–4).** Correctness depends on
  *releasing* proc+read before taking write (R3). This is a protocol, not a type-system
  guarantee; one convenience refactor that pumps while still holding a proc lock is a
  deadlock. Worth demanding a debug-assert or a lock-witness type in the shell.
- **P3 — the route→act window (R4).** Between routing under the read lock and acting
  under the proc lock, the world can change (retire, channel teardown). The design
  handles retire via the act-phase check; enumerate the *other* facts route produced
  (ChanId validity, target GPU) and confirm each is re-validated or stable-by-ownership
  under the proc lock.
- **P4 — the single executor as a completion funnel.** All interrupt-route completions
  (§4.2) serialize through one thread that also runs reaps and lock faults. A slow
  `Device::event` (e.g. a reap that takes the write lock behind P1) delays every other
  proc's os-event delivery — an echo, in miniature, of the F2 shape the design
  otherwise kills. The mitigation is that observation edges also fire from vCPU
  threads (poll/IRQSCLR); check that every completion pattern has a non-executor edge
  or an acceptable latency bound.
- **P5 — the degenerate one-lock stage.** L1-M1 ships with the global-lock
  configuration "flipped later" to real sharding by the #14 gate. Config flips that
  change lock granularity late are exactly where retrofit races appear; the T2 stress
  tier must run *both* configurations from day one, or the sharded mode is the
  untested one when it matters.

# 7. Provenance and method

Written read-only from `l1_concurrency.md` (§§0–10) and
`core_state_and_consolidation.md` (§§1–6) at `488117c`, with the doorbell/publish/
completion entry points verified against `crates/nvkvm-fwd/src/lib.rs` and the
`Gpu`/`Proc` ownership against `crates/nvkvm-core/src/gpu.rs`. The diagram
(`l1_architecture_diagram.py` → `.png`, matplotlib) renders `l1_concurrency.md` §2.
Sections 1–5 and 6.1–6.2 summarize the docs' own claims; §6.3 is this summary's
analysis and is marked as such. Nothing here amends the design — discrepancies found
in review should be filed against `l1_concurrency.md`.
