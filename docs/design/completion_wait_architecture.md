# The completion-wait architecture — what exists, what serialises, what is unbuilt

**Status:** read-only audit, 2026-08-09. ⚠ **HEAD moved mid-audit.** The reading began at
`014ea07`; the bench agent landed `25295aa` (§16.19) and `dbf853a` (§16.20) while it was in
progress, touching `kayfabe-qemu-raw/src/shim.rs`, `kayfabe-rt/src/ceutils.rs` and
`kayfabe-fwd/src/lib.rs` — all files cited here. **Every line number and every negative
search below was therefore RE-RUN against `dbf853a` and is quoted at `dbf853a`.** Only
`shim.rs` shifted (`try_ce_submission` 2603 → 2615, and seven siblings); `ceutils.rs` and
`kayfabe-fwd/src/lib.rs` were unchanged at every cited line, and no negative gained a
caller. ⊘ No build, no boot, no bench — every claim below is from
source reading with `file:line`, and every "no caller" claim was produced by GNU
`/bin/grep -rn --include=*.rs … crates/ qemu/` with a **passing positive control** in the
same invocation (see §0.1). `grep` in this environment is a shell function wrapping
`ugrep`, and `--include` placed after the pattern is silently taken as a **path operand**
(`rc=2`, a warning on stderr) — so the searches behind this document were re-run through
`/bin/grep` explicitly.

**Claim-ledger accounting** `[MEASURED 2026-08-09 @ rev dbf853a]` — `scripts/claim_ledger.py`
read `428 / 69 / 18` (unattributed / conflated / bare-hw) both **with and without** this
file present, so this document adds **zero** to every gated number. ⚠ Those three are
already above their committed ceilings (`UNATTRIBUTED_BAR = 381`,
`scripts/claim_ledger.py:477`) and above the `420/68/18` residue recorded in this
campaign's brief — that drift is **pre-existing and is not mine**; it was measured here by
moving this file out of the tree and re-running. ⊘ And note how the gate's verdict was
nearly lost: `python3 scripts/claim_ledger.py --gate | tail -25; echo rc=$?` prints `rc=0`
because `$?` is **`tail`'s**. The script's own exit is `1`. A gate read through a pipe
cannot fail.

---

## 0. THE HEADLINE

> **There is no completion-wait architecture. There is one synchronous inline executor on
> the vCPU thread, and everything else that looks like a wait plane has zero production
> callers.**

`[MEASURED — grep, §0.1]` In the shipping composition a guest doorbell is an MMIO write
that runs, on the vCPU thread, under the QEMU BQL, the register plane's unranked FSM
mutex, the shim's guest-memory mutex and the device read lock:

```
guest MMIO write to the doorbell
  └─ nvkvm.c:  BQL held  (qemu/hw/misc/nvkvm/nvkvm.c:279-283, 295)
     └─ RegPlane::ring_doorbell                    (crates/kayfabe-device/src/plane.rs:2740)
        └─ SharedDoorbell::ring                    (crates/kayfabe-qemu-raw/src/shim.rs:2514)
           └─ try_ce_submission                    (shim.rs:2615)
              ├─ ce.vmm.lock()                     (shim.rs:2642)      ← unranked mutex
              └─ RegPlane::ce_session              (plane.rs:1676-1692)← unranked FSM mutex
                 └─ SharedDevice::with_pushbuffer  (shim.rs:2657)      ← rank-0 device read lock
                    └─ ceutils::run_submission     (crates/kayfabe-rt/src/ceutils.rs:429)
                       ├─ cpu_ce::execute_ours_spans  (ceutils.rs:629) ← THE COPY, on this thread
                       └─ cpu_ce::write_resolved_completion (ceutils.rs:576, 656)
                          ├─ write every payload   (cpu_ce.rs:327-332)
                          └─ vmm.raise_irq(Msix(0))(cpu_ce.rs:336)  ← IRQ #1
           └─ announce_completion                  (plane.rs:2774, 2822)
              └─ cpu_intr.latch(vector)            (plane.rs:2845)
                 → WriteOutcome::raise_cpu_intr    (plane.rs:2788)
                    → nvkvm_deliver_vector(0)      (nvkvm.c:428, 291)  ← IRQ #2
```

There is no queue, no registration, no thread hand-off, no deferral, and nothing that
waits. **The op finishes before the MMIO write returns.** That is a perfectly coherent
design for the one thing it does today (an emulated CE copy on the CPU), and it is the
reason "does the wait strategy survive N ops" currently has no answer: there is no wait.

---

### 0.1 The searches, and their controls

`[MEASURED 2026-08-09]` Positive control, same invocation shape as every negative:

```
$ /bin/grep -rn --include=*.rs "run_submission" crates/ qemu/ ; echo rc=$?
crates/kayfabe-rt/src/ceutils.rs:429:pub fn run_submission(
crates/kayfabe-qemu-raw/src/shim.rs:2658:    kayfabe_rt::ceutils::run_submission(ce, pb, vmm, chan, cursor)
rc=0
```

The control finds a real production call site across two crates, so the file set and the
pattern engine are both live. Against that same file set (`crates/` + `qemu/`, i.e.
**production only — `tests/` deliberately excluded**), each of the following returns
**only its own definition** and no call site:

| symbol | production call sites | definition |
|---|---|---|
| `Reactor::new` | **0** | `crates/kayfabe-shell/src/reactor.rs:240` |
| `Executor::new` | **0** | `crates/kayfabe-rt/src/executor.rs:66` |
| `SharedDevice::register_source` | **0** | `crates/kayfabe-rt/src/device.rs:904` |
| `Registrar::arm_counter` | **0** | `crates/kayfabe-shell/src/sources.rs:398` |
| `Registrar::arm_channel` | **0** | `crates/kayfabe-shell/src/sources.rs:415` |
| `SharedDevice::submit_ring` | **0** | `crates/kayfabe-rt/src/device.rs:1657` |
| `kayfabe_fwd::submit_ring` | **0** | `crates/kayfabe-fwd/src/lib.rs:4143` |
| `SharedDevice::forward_ce` | **0** (only from `submit_ring`, itself 0) | `device.rs:1619` |
| `SharedDevice::decode_pt_writes` | **0** | `device.rs:1694` |
| `kayfabe_fwd::deliver_completions` | **0** | `kayfabe-fwd/src/lib.rs:2002` |
| `kayfabe_fwd::poll_completions` | **0** | `kayfabe-fwd/src/lib.rs:2015` |
| `kayfabe_fwd::plan_ce` | **0** (only from `forward_ce`) | `kayfabe-fwd/src/lib.rs:3785` |

⊘ This is *not* a claim that the code is wrong or dead. `Reactor` and `Executor` are
carefully built, thoroughly tested (`tests/tests/reactor_os.rs`, `tests/tests/rt_shell.rs`)
and are the shape the wait plane should eventually take. The claim is narrower and load
bearing: **an instrument that compiles is not one that runs**, and today none of them is
on any path a guest can reach.

---

## 1. Inventory of every wait path that exists

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` Classified as the brief asks: busy spin / blocking syscall / poller
registration / synchronous inline execution.

| # | site | shape | reachable from a guest today? |
|---|---|---|---|
| W1 | `ceutils::run_submission` → `cpu_ce::execute_ours_spans` (`ceutils.rs:629`, `cpu_ce.rs:151`) | **synchronous inline execution** — a `while off < len` memcpy loop in 64 KiB chunks (`cpu_ce.rs:40, 177, 209`) | **YES.** The only one. |
| W2 | `cpu_ce::write_resolved_completion` (`cpu_ce.rs:321-339`) | **synchronous inline** write-then-signal; the IRQ is `Vmm::raise_irq(COMPLETION_VECTOR)` = `IrqSpec::Msix(0)` (`kayfabe-fwd/src/lib.rs:103`) | **YES**, from W1 |
| W3 | `RegPlane::announce_completion` (`plane.rs:2822`) | **synchronous inline** latch of the engine's `vectorNonStall` into the CPU-INTR tree, then a second vector delivery via `nvkvm.c:428` | **YES**, from the `ServedLocally` report |
| W4 | `HostRmBackend::await_semaphore` (`crates/kayfabe-isolate-host/src/rm.rs:2897-2915`) | **1 ms sleep-poll** on a real GPU-written semaphore, deadline `CE_COPY_TIMEOUT = 2 s` (`rm.rs:620`) | **NO** — see §4(b) |
| W5 | `ChildIsolate::call` (`crates/kayfabe-isolate-host/src/isolate.rs:343-360`) | **blocking syscall** — `write_frame` then `read_frame` on the isolate socket; the calling thread is parked in `read` for the whole verb | reachable in principle from `SharedDevice::doorbell`; **NO** in the default build (§5) |
| W6 | `PoolGate::wait_for_return` (`crates/kayfabe-rt/src/device.rs:377-398`) | **condvar wait** — backpressure when every worker of a `(Proc, GpuId)` isolate is checked out; opens a `BlockingSection` first, holds **zero** ranked locks | same as W5 |
| W7 | `Reactor::run_with` → `Poller::wait` (`crates/kayfabe-shell/src/reactor.rs:303-308`, `kayfabe-linux-raw/src/epoll_unsafe.rs:189-212`) | **poller registration** — level-triggered `epoll_wait`, N tokens, one thread | **NO — zero production callers** |
| W8 | `Parker` / `ExecutorWaker` (`crates/kayfabe-rt/src/executor.rs:154`) | **condvar wait** | **NO — zero production callers** |
| W9 | isolate child watchdog (`crates/kayfabe-isolate-host/src/isolate.rs:727-735`) | dedicated thread, `recv_timeout` + blocking `read` | only under a real isolate plane |

⚠ **W1 has no byte-count bound.** `execute_ours` copies `span.sub.len` in 64 KiB chunks
with no ceiling (`cpu_ce.rs:176-221`). The bounds that do exist are on *fragmentation and
method words*, not on bytes moved: `MAX_ENTRIES_PER_DOORBELL = 8` (`ceutils.rs:67`),
`MAX_PUSH_RANGE_BYTES = 1 MiB` / `MAX_PUSH_TOTAL_BYTES = 8 MiB` (`kayfabe-fwd/src/lib.rs:2529,
2536`), `MAX_CE_SPANS = 4096` (`kayfabe-fwd/src/lib.rs:3188`). `[INFERRED]` A guest that
declares one `LINE_LENGTH_IN` over a large contiguous mapped run therefore holds the BQL,
the FSM mutex and the device read lock for the duration of an arbitrarily large memcpy. On
today's workload (the CeUtils scrubber and `channel_init`) this is small; it is a scale
property, not a live bug, and it is stated here because nothing in the tree states it.

---

## 2. ★★★ THE CONCURRENCY QUESTION — what happens with N ops in flight

### 2.1 Answering the brief's four sub-questions

**Do waits serialise?** `[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` They do not serialise — they *do not exist*. What
serialises is the **work**, and it serialises three times over, at three different
granularities, each of which is by itself sufficient:

1. **The QEMU BQL.** `qemu/hw/misc/nvkvm/nvkvm.c:279-283` states it explicitly: since
   `MemoryRegionOps::global_locking` was removed, *"an MMIO write from a vCPU reaches here
   with the BQL already held"*. So doorbell #2 on vCPU 1 cannot begin until doorbell #1 on
   vCPU 0 has completed its entire copy. **N ops = N× latency, and worse: every unrelated
   MMIO in the whole VM queues behind the copy too.**
2. **`Mutex<PlaneState>`** (`plane.rs:1684`, taken by `ce_session` for the whole closure).
   This is the same mutex every register access takes, and
   `tests/tests/unranked_locks.rs:50-55` classifies it as *"★★★ THE HAZARD … ⊘ NOTHING may
   block beneath it: a wait here stalls every vCPU's register access, and the R1 witness
   will not say so."* The CPU copy runs beneath it.
3. **The shim's `ce.vmm` mutex** (`shim.rs:2642`), held across the same region, and the
   rank-0 device read lock taken inside it (`shim.rs:2657`).

**Is anything thread-per-op?** `[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` No. `/bin/grep -rn "thread::spawn" crates/`
outside `tests/`, `mocks/` and `bin/` finds exactly three production spawns, none per-op:
the isolate **child process**'s control thread and worker threads
(`kayfabe-isolate-host/src/child.rs:185, 191`) and the isolate spawn watchdog
(`kayfabe-isolate-host/src/isolate.rs:727`). ⊘ **The shim and the vmm-qemu crates spawn no
threads at all.** The concurrency the design describes comes entirely from *the vCPU
threads QEMU already has*, and they are all funnelled through the BQL on this path.

**Can the poller carry per-op completion sources (one thread, N registrations)?**
★ **Yes — and it is BUILT, TESTED, and UNREACHABLE.** This is the most useful single
finding for the owner's question, because it means the answer is "wire it", not "design
it":

- `Registrar::arm` / `arm_counter` / `arm_channel` (`kayfabe-shell/src/sources.rs:308, 398,
  415`) register an arbitrary number of host descriptors against one `Poller`, each under
  its own `CompletionSource` token, with a descriptor budget (`SourceBudgetExhausted`).
- `Reactor::run_with` (`reactor.rs:303`) does one `epoll_wait`, resolves every ready token,
  drains counters, pushes one `SourceSignal` per unit of work, and wakes the executor —
  **one thread, N registrations**, exactly the shape the owner suspects.
- Two source *shapes* are already distinguished: `SourceShape::Counter` (a quantity — drain
  N, push N) and `SourceShape::Terminal` (a fact — fires once, then unwatched),
  `sources.rs:150-156`. `SourceKind::Worker` is `Terminal` and means *"the worker is
  GONE"*; `SourceKind::OsEvent { proc, gpu, ev }` is the `Counter` shape and is exactly the
  per-op completion source (`kayfabe-core/src/reactor.rs:124-179`).
- The F1 anti-spin gate is a **quantity**, not an absence: `MAX_UNPRODUCTIVE_STREAK = 16`
  consecutive ready-but-undrainable waits is a loud `ReactorFault::UndrainableSource`
  naming the token (`reactor.rs:69, 341-357`). ★ This is the rare instrument that cannot
  pass vacuously — an infinite busy-poll *fails* it rather than satisfying it.

⇒ `[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` The shape is **reachable-in-principle and unreached-in-fact**. Nothing
constructs a `Reactor`, nothing constructs an `Executor`, nothing calls `register_source`,
nothing calls `arm_counter`/`arm_channel` outside `tests/`.

**Does any wait hold the device lock?** `[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` The *waits* are clean; the *work* is
not.

- W6 (`PoolGate::wait_for_return`) is the model citizen: it opens a `BlockingSection`,
  asserts zero ranked locks, and re-enters the whole op from the top after waking, with
  full R5 re-validation (`device.rs:377-398`, and the doc comment at `device.rs:335-352`).
- W5 (`Worker::execute`) asserts lock-freedom at the door
  (`kayfabe-isolate/src/lib.rs:1770-1771`, `assert_lock_free("issuing a host RM verb")`).
- ⚠ **But W1/W2/W3 run under three locks the witness cannot see.** `assert_lock_free` is a
  mask over *ranked* locks only; `kayfabe-util/src/lockwitness.rs:9-21` says so in its own
  words — *"a `std::sync::Mutex` nobody ranked is invisible to it, so `assert_lock_free`
  passes **vacuously** while such a lock is held"* — and cites a 2026-08-06 near-miss where
  an agent was about to rely on it beneath exactly `RegPlane`'s FSM mutex.

### 2.2 ⚠ A gap in the unranked-lock gate itself

`[MEASURED]` `tests/tests/unranked_locks.rs:76` scopes the enumeration to
`VCPU_PATH_CRATES = ["kayfabe-device", "kayfabe-rt"]`. But **`kayfabe-qemu-raw` is on the
vCPU path** — it *is* the MMIO handler — and it holds two unranked mutexes across the whole
CE submission:

```rust
// crates/kayfabe-qemu-raw/src/shim.rs:2480-2486
struct CeShellState {
    vmm: std::sync::Mutex<Option<QemuVmm>>,
    cursors: std::sync::Mutex<BTreeMap<(u32, u32), kayfabe_rt::ceutils::GpCursor>>,
}
```

Neither appears in `UNRANKED_VCPU_PATH_LOCKS`, and neither can, because the scanner never
walks that crate. The gate is green and the class walks past it — which is the exact
instrument-defect shape the file's own header is about
(`tests/tests/unranked_locks.rs:9-21`). ⇒ **Adding `"kayfabe-qemu-raw"` to
`VCPU_PATH_CRATES` is a one-line change that would force both to be classified.** It is
the cheapest recommendation in this document.

### 2.3 What L1 invariant #37 actually enforces

`[MEASURED]` The invariant, verbatim (`docs/design/l1_concurrency.md:512-513`):

> **A blocking GPU-work verb issued by guest thread A must not stall guest thread B of the
> same process — in particular B's poll / event-wait / completion paths.**

It is claimed to rest on three mechanisms (`l1_concurrency.md:516-543`):

1. **R1 (no blocking call under any lock).** Enforcement is *"a thread-local lock-depth
   counter … asserted zero at every blocking-verb entry"* — real, always-on (not
   `debug_assert`, `lockwitness.rs:46-51`), and asserted at the one door
   (`kayfabe-isolate/src/lib.rs:1771`). ⇒ **Enforced at RUN TIME, for ranked locks only.**
2. **N workers per isolate.** ⊘ **The doc RETRACTS this itself** (`l1_concurrency.md:520-533`,
   *"★★ RETRACTED (2026-07-27, doc audit) — mechanism 2 is false"*): RM takes the per-client
   lock `LOCK_ACCESS_WRITE` at every ioctl entry, so N workers are N *queued* host threads,
   not N concurrent RM ops. `DEFAULT_POOL_WORKERS = 4` and the comment at
   `kayfabe-isolate/src/lib.rs:793-810` says the pool buys *liveness/latency isolation*, not
   throughput.
3. **The poll path is structurally independent of the RM-verb path** — *"a completion
   reaches B via the reactor (§6): host os-event fd → reactor → dispatch → observe + pump →
   IRQ"*. ⊘ **That path does not exist in any shipping build.** The reactor is never
   constructed; `pump_completions` / `completion_poll` / `completions_drained`
   (`device.rs:639, 650, 664`) have zero production callers.

⇒ **The honest answer to "is #37 enforced at compile time, test time, or not at all" is:
its *lock discipline* is enforced at run time (ranked locks only) and at test time
(`tests/tests/l1_concurrency*`, the bite table at `l1_concurrency.md:6506` shows 9 red tests
+ 3 R1 panics when `defer_isolate` is mutated to spawn inline). Its *completion*
half — mechanism 3, the half the invariant is actually about — is enforced NOWHERE,
because the plane it quantifies over is unreached.** And on the one path a guest does
reach, the invariant is moot for a stronger reason than compliance: the whole VM is
serialised on the BQL, so thread B is stalled behind thread A regardless of what any lock
in this tree does.

---

## 3. The escalation policy — spin-briefly-then-sleep

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` **UNBUILT.** There is exactly one spin-then-sleep loop in the tree and it is
neither escalating nor guest-facing:

```rust
// crates/kayfabe-isolate-host/src/rm.rs:2903-2909
let deadline = Instant::now() + timeout;
let mut semaphore = self.ring_load_u32(chan, sem_offset)?;
while semaphore != payload && Instant::now() < deadline {
    std::thread::sleep(Duration::from_millis(1));
    semaphore = self.ring_load_u32(chan, sem_offset)?;
}
```

It sleeps 1 ms **first**, never spins hot, and gives up at `CE_COPY_TIMEOUT = 2 s`
(`rm.rs:620`). Its own doc comment is explicit that this is a rung property, not a design:
*"★ Polling, not waiting on an interrupt: this rung deliberately has no event delivery, and
a poll cannot mistake 'we were never woken' for 'it never landed'"* (`rm.rs:2891-2896`).

**Where the escalation would go, if it is wanted:** the seam already exists and is named.
`Registrar::arm_counter` (`sources.rs:398`) hands back the write end of an eventfd and its
doc says exactly this: *"the reactor owns the readable side, and the returned handle is what
the **isolate's relay thread** writes when one of its own nvidia descriptors fires. Every
nvidia descriptor stays inside the sandbox."* So the escalation is:

- **spin phase** — inside the isolate child, beside `await_semaphore` (`rm.rs:2897`);
- **hand-off** — the child's relay thread `signal()`s the `Notifier` obtained from
  `arm_counter`;
- **sleep phase** — the parent's single `Reactor` thread blocks in `Poller::wait`
  (`reactor.rs:307`) with N such counters registered.

Every one of those pieces is written. None of them is connected.

---

## 4. The (a)/(b)/(c) split, checked against the code

*(Scope for this whole section: source read + `/bin/grep` runs on 2026-08-09 at rev `dbf853a`; ⊘ no boot, no bench. The C-side citations are readings of `src/qemu/nvkvm_gpu_emul.c` in the C artifact.)*

### (a) Kernel ops WE EMULATE — **built and live. And the brief's own C-oracle caveat does NOT hold for this arm.**

*(Basis: source read at rev `dbf853a`, 2026-08-09, plus the C artifact's own `src/qemu/nvkvm_gpu_emul.c` and `docs/design/mode2_execfwd_keystone_plan.md`.)*

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` Built, wired, and on the live path: `run_submission` (`ceutils.rs:429`) does
the copy on the CPU (`ceutils.rs:629`) and only then writes the finishPayload and raises
(`ceutils.rs:576, 656` → `cpu_ce.rs:327-337`). The ordering discipline is structural, not
conventional: `write_resolved_completion` writes **every** payload before raising, and
raises **nothing** on any refusal (`cpu_ce.rs:326-338`), and the resolve/write split exists
because the resolver and the store are the same `&mut` object (`cpu_ce.rs:264-276`).

⊘ **But the brief's parenthetical "(a) has no C oracle" — which my own earlier framing
implied — is wrong, and the C proves it.** The C artifact *had* an emulated CPU copy engine
that moved the bytes and released the semaphore:
`C: src/qemu/nvkvm_gpu_emul.c:985` declares `nvkvm_t_ce_emul_ns` / `nvkvm_t_ce_emul_calls`,
described as *"emulated-CE LAUNCH_DMA byte copy"*, and `C: :6312` times it per launch. So
(a) has a **behavioural** C oracle — a known-good reference for "CPU moves the bytes, then
writes the payload the guest polls" that a real NVIDIA driver accepted end to end. That
strengthens, not weakens, the owner's split: **(a) is the arm with the most evidence behind
it, not the least.**

⚠ **The risk the brief names — "does the guest wake" — has a concrete defect today, and it
is an ordering one.** Two vectors are raised per served submission, both vector 0:

| | site | what it carries |
|---|---|---|
| IRQ #1 | `cpu_ce.rs:336` → `QemuVmm::raise_irq` (`kayfabe-vmm-qemu/src/lib.rs:2261-2268`) → `nvkvm_op_signal_msix` (`nvkvm.c:1153`) → `msix_notify` | **nothing pending in the interrupt tree yet** |
| IRQ #2 | `plane.rs:2845` `cpu_intr.latch(vector)` → `raise_cpu_intr` (`plane.rs:2788`) → `nvkvm.c:428` → `nvkvm_deliver_vector(0)` | the LEAF pending bit, then the message |

`announce_completion` runs **after** `port.ring(token)` returns (`plane.rs:2746-2747` then
`:2773-2774`), so IRQ #1 is always delivered before the pending bit that explains it is
latched. `nvkvm.c:443-447` states the hazard in its own words for the *other* vector — *"a
bare message with nothing pending sends the guest's ISR looking for an interrupt that is
not there"* — and does not notice that the completion path already sends one.

⊘ To be precise about severity: this is **not** a forged completion. The bytes are in place
and the payload is written before either raise, so the data is never behind the signal.
`[INFERRED]` The consequence is an unattributable interrupt: an ISR that runs before the
latch reads `LEAF & LEAF_EN_SET` as zero and returns without attribution. It is invisible
today because the guest **polls** (`channelWaitForFinishPayload`), which is exactly the
brief's trap — *"it works right until the guest sleeps instead of spinning."* The two wires
are deliberate (`nvkvm.c:1143-1147`, *"they stay two"*); their **order** is not argued
anywhere.

### (b) Ops FORWARDED to the real host GPU — **the single most important line in this report**

> `[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` **Exactly one function in the tree observes a real host GPU completion —
> `HostRmBackend::await_semaphore`, `crates/kayfabe-isolate-host/src/rm.rs:2897-2915` — and
> it is unreachable from any guest action in any build.**

The call chain and where it is cut:

```
await_semaphore  (rm.rs:2897)          ← observes real silicon
  ← ce_copy_outcome (rm.rs:2930, 2978)
  ← RmBackend::ce_copy (rm.rs:2356)
  ← Worker::execute, VerbPlan::CeSplit arm (kayfabe-isolate/src/lib.rs:1917-1932)
  ← the ONLY constructor of VerbPlan::CeSplit: kayfabe_fwd::plan_ce (kayfabe-fwd/src/lib.rs:3698, 3785)
  ← SharedDevice::forward_ce (device.rs:1630)        ← 0 production callers
  ← SharedDevice::submit_ring (device.rs:1665)       ← 0 production callers
  ← ✗ nothing
```

The cut is **structural, not incidental**: `plan_ce` is the only site that mints a
`CeSplit`, and its only two callers are `forward_ce` and `kayfabe_fwd::submit_ring`, both of
which are reached only from `tests/`.

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` The second cut is the composition root. `selected_isolate_plane`
(`shim.rs:4244-4251`) returns `IsolatePlane::Stillborn` unless `KAYFABE_ISOLATES` is set,
and `Stillborn` means, in its own declared words, *"this build has no forwarding plane: the
object model accepts protocol facts and **no host verb can be issued**"*
(`STILLBORN_WHY`, `shim.rs:4150-4151`). The shim keys `local_ce_is_the_only_executor` on
exactly that (`shim.rs:3452`).

⇒ `[MEASURED 2026-08-09 @ rev dbf853a — call-graph search, plus the C's own `m2hostsem` run cited below]` **The brief's (b) claim is confirmed and then some: it is not that nothing observes a
real host completion — something does, correctly, with a bounded deadline and a
three-fact outcome (`semaphore`, `gp_get`, `gp_put`, `rm.rs:2911-2915`) — it is that the
observer is severed from the guest at two independent places.** ★ That is much better news
than "unbuilt": the observation primitive already exists and already refuses to conflate
*"never woken"* with *"never landed"*.

⊘ And the C is silent here for a **measured** reason, not an architectural one:
`C: docs/design/mode2_execfwd_keystone_plan.md:245-258` records the `m2hostsem=on` negative
control — with QEMU's stub completion write gated off, *"the CE scrubber wait **TIMES
OUT** … → `RmInitAdapter failed` → **cuInit 999**. So the **host GPU is NOT writing the
completion semaphore** — QEMU's Phase-B stub write was the ONLY thing satisfying the
scrubber wait."* The C forged because its forwarding never executed, not because it chose
to forge.

### (c) Userspace passthrough (blind arm → interrupt → eventfd) — **UNBUILT**

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` No part of this chain is wired. The arm side (`Registrar::arm_*`) has zero
production callers; the interrupt-in side (the isolate relay thread that would `signal()`
a `Notifier`) does not exist — `/bin/grep -rn "thread::spawn" crates/` finds no relay
thread, only the isolate child's own control/worker threads
(`kayfabe-isolate-host/src/child.rs:185, 191`); and the eventfd side (`Poller`, `Notifier`)
is fully built and never registered from production.

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` One additional fact bounds how far this can be deferred: the interrupt tree is
already load-bearing for boot. `nvkvm.c:266-271` records that `RmInitAdapter` refuses to
finish without a working loopback self-test — the driver writes `CPU_INTR_LEAF_TRIGGER` and
spins ~4.3 s for its own ISR — measured as `RmInitAdapter failed! (0x11:0x45:2134)` in
`/workspace/bench/run_stateload2_dmesg.log:35`. So delivery works; what is missing is a
*source* other than the vCPU's own trap.

---

## 5. Multi-GPU / multi-proc — does the wait design assume one of each?

`[MEASURED 2026-08-09 @ rev dbf853a — a search that RAN or a source read, ⊘ never a bench run]` **The core does not. The shell does.**

**The core is genuinely multi-GPU and multi-proc**, and the axis is carried everywhere:
`PoolGate` keys its waits by `GpuId` and explains why it is *not* keyed by `(ProcId,
GpuId)` — a monotonic `ProcId` would let a churning guest grow the map without bound
(`device.rs:320-326`). `pump_completions` / `completion_poll` / `completions_drained` all
take a `GpuId` (`device.rs:639, 650, 664`). The executor's `CompletionRedeliver` arm was
**fixed** on 2026-07-27 from a hardcoded `GpuId::ZERO` to the target the edge names, with
the reason stated: *"on a two-GPU proc there was no other edge to be pumped by, and GPU1's
undelivered batch could never be re-fed"* (`executor.rs:89-99`). The completion policy crate
is per-`Proc` by construction and its `DeliveryPlane::on_poll` re-posts *the polling proc's
own* pending, so #14's round-8 starvation is impossible by construction
(`crates/kayfabe-completion/src/lib.rs:10-19`).

**The shell assumes one GPU, by a constant:**

```rust
// crates/kayfabe-qemu-raw/src/shim.rs:2498
const DOORBELL_TARGET_GPU: kayfabe_rt::GpuId = kayfabe_rt::GpuId(0);
```

Used at `shim.rs:2524`, `:2606`, `:2732`. It is the **only** `GpuId(` literal in the crate.
`[INFERRED]` This is honest for a device model where one QEMU `nvkvm-gpu` device is one
GPU — the constant's own doc says so — but it does mean the *wait* question has never been
posed across GPUs: two devices are two `SharedDoorbell`s, two `CeShellState`s, two `ce.vmm`
mutexes… and **one BQL**. Cross-GPU concurrency is therefore zero on this path regardless of
how the core is keyed.

**Multi-proc on the live path:** `try_ce_submission` keys its GPFIFO cursors by
`(facts.proc.0, facts.chan.0)` (`shim.rs:2633`), so per-process ring positions are correct.
But the cursor map is behind one global mutex (`shim.rs:2480-2486`) and the copy runs under the
one FSM mutex, so **N processes serialise exactly as N threads of one process do**. The
per-proc sharding the core builds (`ExclusiveProcs`, `RankedMutex<Proc>`) buys nothing here
because the path never reaches it.

---

## 6. What is unbuilt — the list, so nothing is inferred from silence

| thing | state | where it would go |
|---|---|---|
| any wait at all on the guest-facing path | **UNBUILT** (synchronous inline) | `ceutils::run_submission` returns a completion instead of a pending op |
| observing a forwarded host completion | **BUILT, SEVERED** (`rm.rs:2897`) | reconnect `plan_ce` → `forward_ce` → a production caller |
| spin-briefly-then-sleep escalation | **UNBUILT** | `rm.rs:2897` (spin half) + `sources.rs:398` (sleep half) |
| relay thread inside the isolate that signals an eventfd | **UNBUILT** | `kayfabe-isolate-host/src/child.rs`, beside the control thread at `:185` |
| reactor loop running in production | **BUILT, UNCONSTRUCTED** | a composition root beside `Regs::new` (`shim.rs:3373`) |
| executor thread running in production | **BUILT, UNCONSTRUCTED** | same |
| completion pump/poll reaching the guest | **BUILT, UNCALLED** (`device.rs:639-664`) | the executor's drain loop |
| GSP status-queue interrupt delivery | **REFUSED BY NAME** (`nvkvm.c:453-460`) | needs a pending bit in the interrupt tree first |
| `kayfabe-qemu-raw` in the unranked-lock gate | **UNBUILT** | `tests/tests/unranked_locks.rs:76` |

---

## 7. The recommended shape, and why

### R1 — Make "observe, then complete" the only expressible order, by type

⊘⊘ The hard constraint says: never complete work that did not run, and the C's
`m2hostsem` negative control (§4(b)) is what a violation costs. Today the ordering is
enforced by **statement order inside one function** (`cpu_ce.rs:326-337`) plus a doc
comment. That is correct and it is fragile: nothing stops a future arm from raising first.

**Shape:** introduce a `Completion` value that only an *observation* can mint, and make the
signal a method on it.

```rust
/// Minted ONLY by an observer. No public constructor, no Default, non_exhaustive.
pub struct Observed { /* what was seen, and by whom */ }

impl Observed {
    fn by_our_cpu_ce(bytes_moved: u64) -> Observed;      // (a) — we ran it
    fn by_host_semaphore(o: SubmitOutcome) -> Observed;   // (b) — silicon wrote it
}

/// The ONLY function that writes a payload, and the ONLY one that raises.
pub fn complete(o: Observed, …) -> Result<Raised, FwdFault>;
```

This is the same trick `VerbPlan::gated_doorbell` already plays: `VerbPlan::Doorbell` is
`#[non_exhaustive]` with no struct expression outside its crate, so *"a `VerbPlan::Doorbell`
cannot exist without the gate having run"* — and the residual is stated honestly rather than
claimed away (`kayfabe-fwd/src/lib.rs:1973-1985`). ★ Reuse that pattern verbatim; it is
already proven in this tree and already has a `tests/ui/` compile-fail pin.

**Falsification:** delete the `Observed` argument from `complete` and give the raise path a
second entry that takes only an address+payload. If the suite stays green, the type did no
work. ★ And check the *check*: `cargo fmt` collapsing lines has already made a bite-check
match no text in this campaign — assert the file actually changed before trusting red.

### R2 — Fix the two-raise ordering before anything sleeps

Move `announce_completion`'s latch **before** the vector goes out, or drop IRQ #1 entirely
and let `raise_cpu_intr` be the single wire. `[INFERRED]` The second is simpler and the
comment at `nvkvm.c:1143-1147` argues the two wires must stay distinguishable — which is a
counter-argument to merging them, not to *ordering* them. Minimum viable: latch first, then
raise, then raise again is harmless.

**Falsification:** a test that asserts, at the moment `signal_msix` is called, that the
CPU-INTR tree has a pending LEAF bit for the announced vector. It must be red today. If it
is green today, my reading of `plane.rs:2746-2774` is wrong and this recommendation is void.

### R3 — Wire the reactor before, not after, (b) is reconnected

★ The owner's suspected shape (one thread, N per-op registrations) is the right one **and it
already exists**. The work is a composition root, not a design. Do it *before* reconnecting
`forward_ce`, because the moment (b) is live, `await_semaphore`'s 1 ms sleep-poll runs on a
worker thread inside `Worker::execute` — which is lock-free (`kayfabe-isolate/src/lib.rs:1771`)
but is *one of four* (`DEFAULT_POOL_WORKERS = 4`), so five concurrent CE copies means the
fifth guest thread parks in `PoolGate::wait_for_return` for up to `CE_COPY_TIMEOUT = 2 s`.
That is bounded backpressure and it is correctly built — but it is not a wait *architecture*,
it is a queue depth of four.

**Falsification:** drive 8 concurrent submissions against a mock whose `ce_copy` blocks, and
assert `PoolWaits::peak_waiters` reaches 4 and that no thread's `assert_lock_free` fires.
The counters are already there (`device.rs:300-317`). If `peak_waiters` never exceeds 1, the
ops are serialising somewhere upstream and the pool is not the constraint — which is what I
expect on today's BQL-bound path, and would itself be the finding.

### R4 — Get the copy out from under the FSM mutex and the BQL

`[INFERRED]` This is the one that decides whether N ops cost N× latency, and it is upstream
of every wait-strategy question. As long as the CE copy runs inside
`RegPlane::ce_session`'s `Mutex<PlaneState>` on the vCPU's own MMIO trap, no completion
design can help: op #2's *doorbell write* cannot even be delivered until op #1's bytes have
moved. The minimal change that preserves today's semantics is to have `try_ce_submission`
resolve under the session, then **release** and execute, then re-acquire to commit the
cursor — the same plan/execute/commit shape `verb_op` already uses (`device.rs:1237-1340`),
with `commit_ce`-style re-validation covering the gap.

⚠ **Do not do this until R1 exists.** Splitting the phases creates, for the first time, a
window in which the completion write is separated from the copy — which is precisely the
window a forged completion fits through.

**Falsification:** hold the session across a 100 ms mock copy on vCPU 0 and time an
unrelated BAR0 register read on vCPU 1. If it does not block, my reading of
`plane.rs:1684` / `nvkvm.c:279-283` is wrong. ⊘ This one needs the bench; it is stated as
the falsification, not as a result.

### R5 — Add `kayfabe-qemu-raw` to `VCPU_PATH_CRATES`

One line, `tests/tests/unranked_locks.rs:76`. It will fail immediately with two
unclassified mutexes, which is the correct outcome: someone has to write down what may
block beneath `CeShellState::vmm` and `CeShellState::cursors`.

**Falsification:** if adding the crate leaves the gate green, the scanner is missing a
spelling — the same defect the file's own header records finding in itself
(`unranked_locks.rs:130-137`).

---

## 8. What I could not determine

- `[NOT MEASURED]` Whether the guest's `channelWaitForFinishPayload` on the live path ever
  *sleeps* rather than spins. Everything in §4(a) about the interrupt being unattributable
  is latent until it does. This needs a boot with a `dmesg` capture, and the C's own trap
  applies: the serial log is not where the driver's output is.
- `[NOT MEASURED]` Whether any real workload asks for a CE copy large enough for the
  unbounded-`len` loop (§1, W1) to matter. Today's callers are the scrubber and
  `channel_init`.
- `[NOT MEASURED]` Whether `KAYFABE_ISOLATES=real` builds are ever run against a guest that
  reaches `forward_ce`. The severing at `plan_ce` says no path exists, but I did not read
  every `bin/` target; `rmladder` (`kayfabe-isolate-host/src/bin/rmladder.rs:1930`) drives
  `prove_ce_copy` directly and is a host-side tool, not a guest path.
