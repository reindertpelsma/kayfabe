# ★★★★★ THE DOORBELL IS A SCHEDULE — the publication lane gets its worker

**STATUS — 2026-09-06 — LIVE.** Built at `w383`. This is the **wiring** that
`publication_off_the_bql.md` §9 lists under *"Designed, NOT built"* and that
`the_async_lane_is_built_and_orphaned.md` §0 names as the headline finding:
*"we have the right shape and nothing runs it."* Both of those parents stay LIVE and are
**not** superseded — the mechanism they describe is unchanged; what changed is that it now
has a producer, a consumer and a thread.

Read with: `publication_off_the_bql.md` (§1 the ordering correction, §3 the design, §4 the
map/revoke asymmetry, §5.3 the obligation this does **not** discharge),
`the_async_lane_is_built_and_orphaned.md` (F1/F2/F2b, which this closes),
`kayfabe-core/src/channel_kind.rs` (`TrapContract`, the type that already said the answer).

## §0 THE RULING

> Owner, 2026-09-06: *"**Never do a blocking call during a doorbell write, quickly re-enter
> the VM.** Instead if you need to do mappings, run the doorbell asynchronously. There is no
> guarantee that a real GPU gives either — the doorbell runs immediately in-line, it's a
> schedule."*

★★★ **The tree already agreed with this in a type, and had done since 2026-08-11.**
`GuestChannelKind::Emulated` declares `TrapContract::ScheduleAndReturn`, whose own doc reads
*"the handler **must not run on the vCPU thread**"*; `may_run_on_the_vcpu_thread()` is the
predicate. `SharedDoorbell::try_ce_submission` **read that contract and reported it as
violated in the same breath**, in a comment that said so: *"Reported, not enforced, and the
type says why."*

⇒ This rung is not a new decision. It is the enforcement of an old one.

## §1 WHAT WAS MEASURED, AND WHY IT IS THE CRITICAL PATH

`[measured w380llm2, real GA106, HEAD 3cacd43]`, one boot:

| | |
|---|---|
| doorbells the guest rang | **15 928** |
| of those, reaching the publication legs | **15 201** |
| `try_ce_submission` kept (the CE shell executor) | **555** |
| `ServedLocally` | **16** |
| **publication wall, summed over the boot** | **359 759 ms = 360 s** |
| mean per publishing doorbell | **23.7 ms** |
| `TRAPWITNESS` | `off_trap_claims=0 inline_exceptions=20893 worst_trap=1733634us` |

The LLM was killed by its own `timeout 600` with the doorbells still being served. **360 s of
that 600 s was the guest's own vCPUs stopped inside our publication pass**, with every other
vCPU and QEMU's main loop stopped alongside them, because every MMIO write arrives with the
BQL held.

⊘ `off_trap_claims=0` is the other half of the same sentence: **not one host RM verb in the
whole boot ran off a trap.** The witness was built to grade exactly this and had never had
anything to grade.

## §2 THE MECHANISM

```
guest MMIO store to the doorbell
  → TrapGuard::enter()                       (unchanged)
  → RegPlane::write → ring_doorbell → SharedDoorbell::ring
  → PublicationQueue::offer(MapPublication::for_doorbell(token))    ← O(1), one leaf mutex
  → DoorbellReport::Scheduled { token, queued }
  → return to VM entry

kayfabe-doorbell-publish worker
  → PublicationQueue::take_blocking()
  → SharedDoorbell::ring_inline(token)       ← THE IDENTICAL FUNCTION, identical order
  → RegPlane::account_doorbell_report(..)
  → PublicationQueue::note_completed()
```

### ★★★ WHAT MOVED IS THE WHOLE BLOCK, NOT ITS INTERNAL ORDER

`ring_inline` is today's `ring` verbatim — routing, `ringproj`, `pt_witness`, `pt_decode`,
`pt_sweep`, `vas_census`, `bindcensus`, `pin_ring`, `operand_join`, `vas_publish`, then
`SharedDevice::doorbell`. **Not one statement inside it changed.** A rung that moved the work
*and* reordered it would make the outcome unattributable, and the order is load-bearing: *"a
mapping published after the ring has been rung is a mapping published after the engine has
already faulted for it."*

### ★★★★★ THE ORDERING GATE IS ON THE FORWARD, AND IT NEEDED NO NEW MACHINERY

The brief that commissioned this asked for *"an epoch/generation per VAS"* to gate the
forward on the VAS being current. **That gate would be redundant, and here is why rather than
an assertion:** publication and the forward are **two adjacent statements of one function**,
and the deferral moves the function, not the statement boundary. There is no path in this
port that rings a host channel *before* the publication that precedes it in the same pass —
`publish_vas_rows` returns, then `SharedDevice::doorbell` is called, on the same thread, with
nothing between them. A per-VAS epoch would be a second encoding of an ordering the call
graph already guarantees, and `a_second_source_of_truth_beside_a_complete_value.md` is this
tree's own ruling on what that costs.

⊘ **And the `Passthrough` case the brief flagged as unhandled does not arise for the same
reason.** The concern was: *"in `Passthrough` the host engine reads the guest's ring directly,
so there is no forward for us to gate."* Measured against the code: the arm that reaches the
publication legs at all is exactly the arm `forwarding_plane_owns_ce` hands to
`SharedDevice::doorbell` — i.e. **there IS a forward, and it is the statement after the
publication.** What `RingAndReturn` licenses is a trap that only *rings*; what this port was
doing was hanging a 24 ms publication under that name. The fix is that the trap no longer
does either — it schedules, and the worker does both in order. **Both contract arms are
discharged by one seam, because both arms went through one function.**

### ⊘ WHY A DEFERRED DOORBELL IS NOT A LATE DOORBELL

The requirement is **publish → *our* host ring**, not *publish → the guest's MMIO store*. The
guest's doorbell write is fire-and-forget by the GPFIFO contract: it reads nothing back, polls
no status, and `publication_off_the_bql.md` §5.2 **checks every arm of `RegPlane::read_inner`**
rather than asserting it — none consults publication state, the host, or an isolate. Both ends
of the real constraint are ours.

## §3 THE FOUR THINGS THE WIRING HAD TO DECIDE, AND WHAT EACH COST

### 3.1 `DoorbellReport::Scheduled` — a fourth arm, not a flag

A `Served` that has not served anything is *"counted as a doorbell, went nowhere, looked
fine"* wearing a boolean — the shape this crate's doorbell doctrine exists to make
unrepresentable, and the same argument that made `ServedLocally` a third arm. `is_served()` is
**false** for it, and it is counted in **neither** `doorbells_served` nor `doorbells_refused`.

⚠ **The invariant changes, and it is stated rather than left to be found.**
`doorbells == served + refused` holds exactly on the disarmed arm and becomes
`doorbells >= served + refused` on the armed one. The difference is precisely
*(coalesced offers) + (entries in flight)*, and both terms print on the `PUBQUEUE` census.
⊘ The worker calls the plane's **own** accounting, so `doorbells_served_forwarded` and every
grading grep built on it keep meaning what they meant; a second set of counters on the worker
side would have made every existing grep silently stop seeing the forwarded arm the day the
lane was armed.

### 3.2 The C ABI — a new VALUE, not a new field

`KAYFABE_DOORBELL_SCHEDULED = 4`, `doorbell_kind = "Pubqueue::Scheduled"` /
`"Pubqueue::Coalesced"`. `KayfabeRegWrite`'s layout is unchanged, so **no ABI bump**. ⊘ An
older C shim would fall into its `else` and print `SERVED` — which is why the C is updated in
the same commit rather than relying on the discriminant being ignored.

### 3.3 The interrupt — the one thing a worker has no wire for

On the trap, a raise leaves through `WriteOutcome::raise_cpu_intr`. A worker has no such wire.

★ **The deferred population cannot need one, by construction**: the fall-through arm's only
reports are `Served` and `Refused`, and `RegPlane::ring_doorbell` already answers `false` to
every raise for both. Only `ServedLocally` announces anything, and it is produced by exactly
one site (`try_ce_submission`).

⊘ **It is still handled, because *"it cannot happen"* and *"it is handled"* are different
states and this tree has paid for the first wearing the second's face.** If the CE shell
executor ever claims a deferred doorbell, `RegPlane::announce_deferred_local_completion` runs
the same two acts in the same order and the worker delivers the vector itself through
`Vmm::raise_irq(IrqSpec::Msix(0))` → `nvkvm_op_signal_msix` → `nvkvm_deliver_vector`, whose
`BQL_LOCK_GUARD()` is the **conditional** form and is therefore legal from a non-vCPU thread.
The line says so, loudly, once — so a boot can tell that it happened.

### 3.4 The thread — shaped like the one that already ships

`DoorbellPublishThread` is `ObserverThread` with the reactor removed: a stop that is **ours**
and a `JoinHandle` that is **joined**. Started in `Regs::attach_ram` (before that instant
there is no guest memory to read a page table out of), stopped and joined in
`Regs::detach_ram` **beside** `stop_completion_observer`, before `set_ram(RefusingRam)` and
before `ce.vmm` is cleared — because the loop reads guest RAM through its own `QemuVmm` and
the hypervisor releases the regions behind that handle once `detach_ram` returns.

⊘ `queue.stop()` **then** join, in that order: `take_blocking` returns `None` only when the
queue is both empty *and* stopping, so shutdown **drains** what is outstanding rather than
abandoning it.

## §4 ⚠ THE FAILURE THIS RUNG CAN HAVE THAT LOOKS LIKE HEALTH

**A port that is armed with no worker draining the lane accepts every doorbell and executes
none of them**, and its `PUBQUEUE depth=` reads as a healthy backlog. That is the single worst
state available here, so the spawn failure is the loudest line in the file
(`⊘⊘⊘ WORKER FAILED TO START … THIS BOOT IS UNMEASURABLE`), and the *disarmed* arm prints its
own `⊘ NO WORKER — arm=off (the control)` so that *"no worker because off"* and *"no worker
because the spawn failed"* are never the same silence.

⊘ `Offered::Full` runs the body **inline** — i.e. exactly what the disarmed arm does, never
worse than the status quo — and `PUBQUEUE refused=N` says so. There is no arm here that
drops a doorbell.

## §5 ⚠ WHAT THIS DOES **NOT** DO — read before grading it

- ⊘⊘ **`inline_exceptions = 0` is NOT reachable by moving the doorbell, and that is a
  property of the design rather than of this rung.** The witness counts host RM verbs issued
  under the BQL from **any** trap, and `publication_off_the_bql.md` §4 rules that the
  **revocation** direction must stay synchronous — deferring an unmap is a leak window, not a
  latency choice. `Regs::write`'s budgeted drain therefore issues host verbs on the vCPU **by
  design**, and `MappedFb::write`'s staged frees (F4) do too. ⇒ **Read the delta, not the
  floor.** The instrument's own printed target is `inline_exceptions=0`; that target is
  correct only for the *doorbell's* share of it, and the census does not currently split the
  two. Splitting it is the obvious follow-up and is not done here.
- ⊘ **It does not make publication cheaper.** 360 s of publication is still 360 s of
  publication; what changes is that the guest is running during it and that repeat doorbells
  on one token **coalesce** into one pass. A workload that rings, blocks on the completion and
  rings again gets no coalescing and no overlap — for that shape this rung is neutral by
  construction.
- ⊘ **Off-BQL is not off-contention.** `the_publish_trigger_measured.md` §measured:
  `worst_tick_us = 118 203`, `vcpu_skipped = 4438` — the publication pass takes the device's
  rank-0 write guard, and a vCPU that needs it still waits. The vCPU is no longer *doing* the
  work; it can still *queue behind* it.
- ⊘ **The torn-ring-read obligation is NOT discharged** (`publication_off_the_bql.md` §5.3).
  The worker reads the guest's GPFIFO **while the guest is running**. That is already illegal
  for the guest against real hardware — the GPU DMA-reads the pushbuffer asynchronously — so
  the deferral is *more* faithful, not less; but our decoder's behaviour on a mid-flight
  rewrite is **UNMEASURED, not safe**.
- ⊘ **RCU thread registration is unexamined.** `memory_region_get_ram_ptr` takes
  `RCU_READ_LOCK_GUARD()` and `rcu_register_thread` appears nowhere in `qemu/hw/misc/nvkvm/`.
  The `memory_region_ref` taken in `region_add` is what keeps the region alive, and the
  completion observer has relied on exactly this since w326 — this rung inherits the
  assumption rather than introducing it, and **nobody in this tree has written the argument
  down.** Named here so the second thread does not make it twice as invisible.

## §6 THE ARM

`KAYFABE_DOORBELL_ASYNC=off|on`, default `off`. ⊘ Two arms and no `assert` control, because
the disarmed arm **is** the control: it runs exactly the code every boot before w383 ran, on
exactly the thread it ran on, and the `PUBQUEUE` census still prints (all zeros) so *"the lane
was disarmed"* is a positive observation rather than an absence.

Runner: `scripts/bench/w383_llm.sh`, one variable against `w380llm2`. Every other arm is
w290p's default byte for byte.

## §7 THE BOOT

See §8 below — filled in from the run, not before it.
