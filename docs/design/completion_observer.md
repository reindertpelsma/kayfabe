# The completion observer — the two severance points, RE-READ at `d9136d7`, and what was built

**Rung:** `w226` / branch `completion-observer`, based on `master` at `d9136d7`.
**Scope:** the observer and its wiring. ⊘ **Not** the memory crossing (#238 step 3, another
agent's rung, another branch); where mapped guest RAM is needed it is asserted by name.

---

## 0. ★★★★★ WHAT I REFUTED FIRST — including the brief

### 0.1 ⊘ The brief's central premise is STALE: severance #1 was closed ten commits ago

The brief instructed: *"Your first job is not to build. It is to find those two severance
points and name them precisely … Do not start writing an observer until you have both, in
writing."* Its citation is audit `026374c`
(`docs/design/completion_wait_architecture.md` §4(b)), which named:

1. `plan_ce` / `forward_ce` / `submit_ring` with **zero production callers**;
2. `selected_isolate_plane` returning `IsolatePlane::Stillborn` by default.

`[MEASURED at d9136d7, /bin/grep with a passing positive control]` **Both are already
resolved on the base commit, and one of them was refuted as a severance at all:**

| the audit's cut | state at `d9136d7` | evidence |
|---|---|---|
| `forward_ce` has 0 production callers | ⊘ **CLOSED** at `e7bed44` (in `master`, 10 commits before my base) | `crates/kayfabe-rt/src/device.rs:1769` `self.forward_ring(vmm, out.proc, out.chan)?` ← `SharedDevice::doorbell`; `device.rs:1933` `self.forward_ce(...)`; `device.rs:2076` `kayfabe_fwd::plan_ce(...)` |
| `Stillborn` is a severance | ⊘ **REFUTED**, by `e7bed44`'s own commit message: *"a **build selector working as designed**"* — and `2692e7b` (`w220`), `cd930d6` (`w221`), `52a55d6` (`w222`) are three real GA106 boots with `KAYFABE_ISOLATES=real` | `docs/design/gpu_promote_ctx.md` §16.80.1-2 |

⇒ The positive control for this reading is that `run_submission` is found by the same
invocation in the same two crates, so the file set and the pattern engine are live.

★ **The finding this leaves is better than the brief's, not worse**: the completion plane's
severance has *moved*, and the audit's page no longer describes where it is.

### 0.2 ⊘ REFUTED: *"a green C↔Rust diff says nothing here"* — there is no diff to be green

The brief is right that the C forges completions and right that it therefore cannot be an
oracle for this plane. But it frames that as a caveat on a comparison. There is no
comparison: the C's `nvkvm_gpu_emul.c` never calls `nvkvm_isolate_poll` at all, so the diff
in question was never runnable. ⊘ Nothing here was decided by *"what did the C do"*.

### 0.3 ★ The brief's ruling holds, and is SHARPER than stated: `AWAKEN_ENABLE = 0` means the VMM owes **nothing** here

The brief derives *"the VMM owes the NOTIFICATION, not the WRITE"*. For **this** completion
that is still one step too generous. `[measured 2026-08-10, boot `w218_cb6adcc_grfull`,
recorded in `gpu_promote_ctx.md` §16.79]` the operand is `AWAKEN_ENABLE = FALSE`
(`ogkm-580: clc7c0.h:745-747`) — the guest is asking for **no interrupt**, and it spins in
userspace (`state=Rl`, `RIP` in `[vdso]`). ⇒ On this completion the VMM owes neither the
write **nor** the notification. What it owes is that the work RUNS and that it can **say**
whether the value landed. That is why this rung builds an *observer* and not a *deliverer*.

---

## 1. ★★★ THE TWO SEVERANCE POINTS AS THEY ARE AT `d9136d7`

Both are named with `file:line`, with what is lost at each. Both are about the **GR channel
that actually waits** — `cuCtxCreate`'s `GrCompute` token `0x00000007`, rung 86 times.

### S1 — ROUTING: the GR doorbell never reaches the observer's chain at all

**`crates/kayfabe-qemu-raw/src/shim.rs:3573-3592`** (`SharedDoorbell::try_ce_submission`):

```rust
let route = facts.route();
if route != kayfabe_rt::DoorbellRoute::CpuCe {
    self.dump_gr_pushbuffer_once(token, &facts);
    return Some(refused(token, FaultTag("Route::NotACopyEngineChannel"), …));
}
```

`SharedDoorbell::ring` (`shim.rs:3047`) calls `try_ce_submission` **first and
unconditionally**, and returns at the `if let Some(report)` immediately below it
(`shim.rs:3075-3077`). So a `Some(..)` here is **terminal**.

**What is lost:** every `GrCompute` doorbell — `[measured 2026-08-10, boots `w218_cb6adcc_grfull` /
`w220` / `w221_49dc3ec_grfwd` / `w222_346921b_gate`]`
**86 of 448** — never reaches `SharedDevice::doorbell`, therefore never reaches
`forward_ring` → `parse_pushbuffer` → `forward_ce` → `plan_ce` → `Worker::execute` →
`RmBackend::ce_copy` → `await_semaphore`. The observer the audit found is intact and the
channel that is waiting cannot get to it.

⊘ **And the refusal is RIGHT and stays.** Its own comment gives the reason and `§16.80.1`
measured it: GR forwarding needs a host channel that *shadows* the guest's ring plus the
`OS_DESCRIPTOR` primitive, neither of which is built, and opening that gate to make a
falsifier fire is the hostile-guest boundary `same_class_id_opposite_directions` records.
**A true refusal outranks a forwarded no-op.** This rung does not touch it.

### S2 — THE OPERAND: the one observer that exists cannot be asked about the guest's address

**`crates/kayfabe-isolate-host/src/rm.rs:538`, `:3358`, `:3374`, `:3380`**:

```rust
const SEMAPHORE_OFFSET: u64 = 0x2000;                     // :538   — OURS
let sem_va   = parts.ring_va + SEMAPHORE_OFFSET;          // :3358  — OUR channel's ring
let payload  = ce_chan.next_payload;                      // :3352  — OUR counter
self.ring_store_u32(chan, SEMAPHORE_OFFSET, 0)?;          // :3374
let outcome  = self.await_semaphore(chan, SEMAPHORE_OFFSET, payload, CE_COPY_TIMEOUT)?; // :3380
```

`HostRmBackend::await_semaphore` (`rm.rs:3299`) is a correct, bounded, three-fact observer —
and it observes **a word we chose, at an offset we chose, holding a payload we minted**,
inside a host channel we built. It is structurally incapable of answering *"did `1` appear at
guest VA `0x2_0440fff0`?"*.

**What is lost:** the guest's declared completion operand — VA, payload, `AWAKEN_ENABLE`,
`STRUCTURE_SIZE` — is not carried anywhere on the forwarding path. `parse_pushbuffer`
produces `CeSpan`s only. The GR method stream *is* read, at `shim.rs:3391`
(`dump_gr_pushbuffer_once`), and that function's own docs say it *"decides nothing and names
nothing"* — it is print-only and bounded to **2** dumps per device life.

⇒ ★ **The severance is not "no observer exists". It is that the only observer is keyed to
OUR completion and the guest's completion is never turned into a fact.** That is what this
rung builds.

---

## 2. WHAT WAS BUILT

### 2.1 The operand becomes a fact — `crates/kayfabe-rt/src/completion_watch.rs`

`DeclaredCompletion` — `va`, `payload`, `four_words`, `awaken`, `operation`, `subch`,
`class_id`. **No public constructor, no `Default`, `#[non_exhaustive]`**: the only way to
obtain one is `decode_report_semaphore`, which reads it out of the guest's own method words.
Same trick as `VerbPlan::gated_doorbell`, for the same reason — the whole correctness
argument for passthrough is *"the payload is the guest's literal"*, so a `DeclaredCompletion`
that did not come from guest bytes must not be expressible.

The decoder tracks `SET_OBJECT` **per subchannel** and answers only for `AMPERE_COMPUTE_B`.
⊘ Not fussiness: `[measured, `w218`]` this very pushbuffer binds `0xc7c0` to subchannel 1 and
`0xc7b5` to subchannel 4, and on the copy engine `0x1b00` is not a report semaphore. A test
asserts the collision is refused, and biting the gate turns it red (§4).

### 2.2 The observer — reader-only, by type

`WatchList::sweep` takes `&mut dyn FnMut(u64, &mut [u8; 4]) -> Result<(), String>`. It is
handed a **reader**. It cannot write, cannot raise and cannot resolve — those capabilities
are not in the type it is given. ⊘ That is the structural guarantee behind *"do not build a
semaphore writer"*: this code is unable to forge a completion even if a later edit wanted it
to. `grep` the module for `gpa_write`; there is none.

Four verdicts, and the distinctions are the point:

| verdict | means |
|---|---|
| `Observed` | the declared payload appeared at the declared address. **We read it; we did not write it.** |
| `NotObserved` | the address WAS readable, the deadline passed, the value never appeared — **a statement about the completion plane**, and it prints `last_seen` because `0` and *"never read"* are different facts |
| `Unobservable` | the address did not resolve, so **nothing was read** — **a statement about the address table**, and it must never be counted as evidence the work did not run |
| `ReadRefused` | our own read failed at a site that HAD resolved — **a statement about the instrument** |

⊘ `Unobservable` and `NotObserved` are never merged. *"We could not look"* and *"we looked
and it was not there"* are the two answers this whole rung exists to keep apart.

### 2.3 The split — and it does NOT make a tenth blocking site

| phase | thread | what it does |
|---|---|---|
| **declare** | the **vCPU**, at `shim.rs:3577` (`declare_gr_completion`), inside the ring read it already performs | decode the operand, resolve the VA **once**, insert into a leaf map, poke an eventfd. No host verb, no pool checkout, nothing that can park. |
| **observe** | the **reactor** thread (`observer_loop`, `shim.rs`) | `epoll_wait`, then `gpa_read` each declared address and emit verdicts. **No ranked lock, no device lock, no address table.** |

★ This is the same plan/execute split `verb_op` uses one layer down, and it is why the nine
blocking sites on the guest-facing path stay nine.

⚠ Every lock the declare phase took — the memory-plane mutex, the plane's CE session, the
rank-0 device read — is released **before** the declare and before the poke.
`Notifier::signal` asserts lock-freedom (R1), so a future edit that moves the poke under a
lock panics loudly instead of deadlocking quietly.

### 2.4 ★★★★★ THE FIRST PRODUCTION `Reactor` IN THIS TREE

`completion_wait_architecture.md` §0.1 measured `Reactor::new`, `Executor::new`,
`register_source`, `arm_counter`, `arm_channel`, `deliver_completions`, `poll_completions`
at **zero production call sites**, and §7 R3 states the consequence: *"the owner's suspected
shape (one thread, N per-op registrations) is the right one **and it already exists**. The
work is a composition root, not a design."*

`Regs::start_completion_observer` is that composition root. It builds a real `Poller`, a real
`Registrar`, **arms a real counter source** (`SharedDevice::register_source(SourceKind::Notify)`
→ `Registrar::arm_counter` — both previously at zero production callers), constructs the
`Reactor`, and spawns one thread. `Reactor::new`, `register_source` and `arm_counter` now
have exactly one production call site each.

⊘ Gated on the **same** `host-isolates` feature as the forwarding plane, so the default
archive's dependency graph is byte-for-byte master's and gains no thread and no descriptor.

⚠ **`kayfabe-linux-raw` has no `timerfd`** — deliberately; its own module docs say the day it
becomes real is *"when something outside a test has to be woken by a deadline nobody is
waiting on"*. **That day is this loop.** The stand-in is `PollTimeout::Millis(250)`, named as
a stand-in (`OBSERVER_TICK_MS`), and a `timerfd` source is the correct successor.

### 2.5 ⚠ Reading guest RAM off the vCPU thread — the argument, stated not assumed

`QemuVmm` is `Clone + Send + Sync` (its crate's own `assert_send_sync!`), holds only an
`Arc<Plane>` of leaf mutexes, contains no raw pointer, and calls no `bql_lock` — the
adapter's crate docs state there is not one call to it in the whole crate. The C side is a
`memcpy` off `memory_region_get_ram_ptr` guarded by `memory_region_is_ram`
(`qemu/hw/misc/nvkvm/nvkvm.c:1159`), chosen over `address_space_rw` precisely so it takes no
global lock. The copy runs inside the plane's `view` mutex.

⊘ **What that argument does not cover:** the foreign region's liveness rests on a
`memory_region_ref` taken in topology callbacks that DO arrive under the BQL. So the thread
is stopped **and joined** in `Regs::detach_ram`, before anything else is torn down — a reader
still running against a machine that has released its slots is the one hazard here, and it is
closed by **ordering**, not by hope.

---

## 3. ⊘ WHAT THIS DOES NOT DO — read before citing it

1. ⊘ **It never writes a semaphore and never raises a vector.** Not a policy — a type
   property (§2.2).
2. ⊘ **It does not open S1.** The `Route::NotACopyEngineChannel` refusal is untouched. GR
   work still does not run, so an `Observed` verdict on the GR channel is **not expected**
   and would itself be a finding worth chasing.
3. ⊘ **It does not build the memory crossing.** #238's guest-RAM grant is another agent's
   rung on another branch. Where this rung needs an address the table cannot bind, it
   answers `Site::Unresolved` **carrying the walk's own name for the refusal** and reads
   nothing. `[NOT MEASURED]` at the time of writing whether `0x2_0440fff0` binds at all;
   `§16.73`'s ruling (*"the table is INCOMPLETE"*) and four rungs of `RING-VA-UNBOUND` say
   the honest prior is that it does not — in which case the boot's verdict is
   `UNOBSERVABLE`, and that row is **about the address table**, not about the completion.
4. ⊘ **`cup2` is not expected to pass** and its timeout is not a failure condition of this
   rung.
5. ⊘ **Nothing about arm (c)** — userspace passthrough — is built or claimed.

---

## 4. THE BITES — the negative controls, all three run, all three red, all restored

| bite | what it removed | what went red |
|---|---|---|
| **B1** | `self.declare_gr_completion(token, &facts);` in `try_ce_submission` | `a_gr_channel_is_refused_by_route_and_the_engine_object_is_what_moves_it` — *"★★★ THE SEVERANCE: a GR doorbell must reach the completion observer's declare path. Saw `WatchStats { attempts: 0, … }`"*, `left: 0 right: 1` |
| **B2** | the decoder's per-subchannel class gate (`if false && bound[subch] != …`) | `a_run_on_a_subchannel_bound_to_another_class_is_not_decoded` **and** `an_unbound_subchannel_is_not_assumed_to_be_compute` — a `0xc7b5` run decoded as a compute report semaphore |
| **B3** | the payload comparison (`if true \|\| v == w.decl.payload`) | three tests, incl. `the_observer_reads_and_never_writes_and_says_when_the_value_appears` and `a_deadline_that_passes_with_the_wrong_value_is_not_observed…` |

★ **B1 is the one that matters**: it is the severance itself, asserted at the **caller**, in
the shape `a_flag_is_not_progress` demands. And it asserts `attempts`, not `declared` —
because the fixture attaches no guest memory, so nothing *can* be declared, and *"never
reached"* vs *"reached and found nothing"* is precisely the distinction a single counter
destroys.

## 5. ★ AND A GATE CAUGHT ME — `tests/tests/unranked_locks.rs`

The two new mutexes (`WatchList`'s `Mutex<Inner>` in `kayfabe-rt`, `Mutex<Option<ObserverThread>>`
in `kayfabe-qemu-raw`) turned that gate **red on first run** and had to be classified with an
explicit blocking ruling. ⊘ Worth recording because `completion_wait_architecture.md` §2.2
recommended widening that scanner's scope to `kayfabe-qemu-raw` as *"the cheapest
recommendation in this document"* — it has since been done, and it fired on the first new
lock to arrive. `WatchList`'s row is the first in the table held by **two** threads, and its
ruling pins what a future reader inside the sweep may cost.

---

## 6. THE FALSIFIER — written BEFORE the boot

`[to be scored at `w226`, `KAYFABE_ISOLATES=real`, CE executor `local`, on `vh2`]`

⊘ **A limit of the instrument, named in advance so it is not discovered as a result:** the
observer's thread and its verdicts print to the device's own stderr (`run_<tag>_qemu.log`),
not to the guest's `dmesg`. A boot whose QEMU log lacks `COMPLETION-OBSERVER started` did not
run the observer at all, and every `COMPLETION-WATCH` absence in that boot is **absent by
construction**. That is why the start line, the stop line and `WatchStats` are all printed:
`reads=0` with `declared>0` means *the loop never ran*, not *nothing appeared*.

| outcome | what the log shows | reading |
|---|---|---|
| **A — UNOBSERVABLE** (★ the predicted one) | `COMPLETION-DECLARE … → DECLARED va=0x20440fff0 payload=0x00000001 awaken=0 four_words=1 … site=Unresolved(…)` then `COMPLETION-WATCH … → UNOBSERVABLE` | the guest's poll target does not bind in our address table. **The memory crossing is the named dependency**, quantified for the first time at the guest's own address. §16.73's ruling corroborated from a new direction |
| **B — NOT-OBSERVED** | `site=GuestRam { gpa: … }`, then `→ NOT-OBSERVED samples=N last_seen=0x…` | ★ **strictly better than A**: the address DOES bind, we can read the exact word `cuCtxCreate` polls, and it never changes. The wall moves from *addressing* to *execution*, and S1 is then the only thing between the guest and a real completion |
| **C — OBSERVED** | `→ OBSERVED after=…ms` | ⚠ **unexpected, and the first thing to distrust is this instrument.** Nothing in this build runs GR work. Either something else in the VMM is writing that page, or the decode is pointing at the wrong address. Do NOT report it as progress without finding the writer |
| **D — NOT DECLARED** | `COMPLETION-DECLARE … ⊘ NOT DECLARED: <reason>` | the declare path ran and could not get to the operand. The reason is printed; it is a fact about the ring read, not about the completion |
| **E — the observer never started** | no `COMPLETION-OBSERVER started` line | ⊘ **absent by construction**; nothing below may be cited. Read the `⊘ NOT STARTED` reason |
| **F — the census moves** | `doorbells` ≠ `191/183/8` or `GrCompute=8 Ce=183` | ⚠ this rung perturbed the control plane and that is a regression to explain before anything else is read. The declare path is print-and-insert only and must not move a doorbell |
| **G — a hang** | the boot wedges where `w222` did not | first suspect is the new thread: the join in `detach_ram`, or the sweep's reader under `WatchList`'s guard. ★ A slow boot is not a crash (~20-25 s to a login prompt) |

★ **F is the falsifier that costs me something.** Every prior rung on this line reported
`191/183/8` and `GrCompute=8 Ce=183` identically; if those move, the finding is about my
change and not about the completion plane.
