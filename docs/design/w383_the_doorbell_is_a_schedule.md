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

### ⊘⊘⊘ THE ORDERING GATE — **I ARGUED IT WAS REDUNDANT AND THE HARDWARE SAYS OTHERWISE**

> ⚠ **This section is a correction of itself.** The paragraph below the rule was written
> before the boots and is retained, because the refutation needs its subject. Read the
> correction first.

**What I wrote, and it is wrong:** *"publication and the forward are two adjacent statements
of one function, so gating the forward on a per-VAS epoch would be redundant; and the
`Passthrough` case the brief flagged does not arise, because there IS a forward and it is the
statement after the publication."*

**What the boots say** (§7, §9): deferral is safe for `cup3` — one process, one channel, few
submissions — and **faults for the LLM**, with the *same two Xids on each of two consecutive
python processes*: `CE2 … FAULT_PDE @ 0x724b_9ce00000` and `GR0_PBDMA0 … FAULT_PTE @
0x2_0440f000`. Both are *"the mapping was not there when the engine ran"*, and neither is a
coalescing artefact — the coalescing was already off.

**⊘ THE PREMISE THAT IS FALSE: our forward is not the only thing that starts the engine.**
The host channel is born **over the guest's own USERD page** — this tree's own comment, at
`shim.rs`' `GrCursorWatch`: *"after leg B the host channel is born over this same page
(`GR-BIRTH … userd=GUEST-USERD`), which is precisely why reading it answers a question about
the **host** engine's progress."* And `exec.scheduled.insert(plan.chan)`
(`kayfabe-fwd/src/lib.rs`) is **monotone**: a channel we schedule once stays on the host
runlist.

⇒ ★★★★★ **From the SECOND submission on a channel, the guest's own `GP_PUT` store into the
adopted USERD is what starts the host PBDMA, and our doorbell trap is not in that path at
all.** Inline publication was never *ordered* before the engine — it was merely **microseconds
behind the guest's `GP_PUT`, with the guest halted**. Deferral turns those microseconds into
the queue's latency, and the race becomes reachable.

⇒ **The brief's warning was right and my dismissal of it was wrong**, and it was right for a
sharper reason than either of us stated: it is not that `Passthrough` has *no forward to
gate*; it is that **the forward is not the trigger**. `TrapContract::RingAndReturn` says
exactly this in one word — the trap's whole content is a *ring*, because by then the engine is
already the guest's to start.

⇒ ★★★ **And it names the next rung, which is the owner's own preference ordering**
(`publish_trigger_preference_ordering.md`, 2026-08-14): *"exact GPU boundary (TLB invalidate)
> trap the PTE write, as little as possible > deferred publish on doorbell > work under the
BQL"*. The doorbell is **third**. The first exists, is decoded, and has **no consumer**:
`crate::mmuinval` (w326) decodes `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` at BAR0
`0x00B8_30B0`, the guest **blocks** on it by protocol, and `WriteOutcome::invalidate` is
carried out of the plane with **nothing in the shim reading it**
(`grep -rn '\.invalidate' crates/kayfabe-qemu-raw/src/` → one doc comment, no consumer). A
publication driven from *there* precedes the guest's `GP_PUT` by construction, which is the
property the doorbell can never have.

⊘ **What would falsify the mechanism above:** a boot in which a channel's *first* submission
faults the same way (the engine cannot be running before we schedule it), or one in which the
fault survives with the deferral removed and everything else held. The second is exactly the
`arm=off` control in §9; the first has not been run.

---

*The pre-boot argument, retained as the refutation's subject:*

The brief that commissioned this asked for *"an epoch/generation per VAS"* to gate the
forward on the VAS being current. **That gate would be redundant, and here is why rather than
an assertion:** publication and the forward are **two adjacent statements of one function**,
and the deferral moves the function, not the statement boundary. There is no path in this
port that rings a host channel *before* the publication that precedes it in the same pass —
`publish_vas_rows` returns, then `SharedDevice::doorbell` is called, on the same thread, with
nothing between them.

⊘ And the `Passthrough` case the brief flagged as unhandled does not arise for the same
reason: the arm that reaches the publication legs at all is exactly the arm
`forwarding_plane_owns_ce` hands to `SharedDevice::doorbell` — i.e. **there IS a forward, and
it is the statement after the publication.**

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

## §7 ★★★★★ THE MEASUREMENT THAT SPLIT THE RUNG IN TWO — cup3, three boots, one variable

**Pre-registered before the runs**: the arm is one word; everything else is w290p's default
byte for byte; `cup3` is the known-positive (`^CUP3_VAL=43`, un-forgeable — a 4×4 of ones
squared has every element 4, and `out = in*3+1` with `in = 14` has exactly one right answer).

| arm | `^CUP3_VAL` | `CUP3_RC` | host `Xid` | `TRAPWITNESS` |
|---|---|---|---|---|
| `off` — inline on the vCPU (**the control**) | **43** | 0 | **0** | `off_trap_claims=0 inline_exceptions=856` |
| `on` — deferred **and coalescing** | **ABSENT** | 1 | **2** | `off_trap_claims=3288 inline_exceptions=35` |
| `nocoalesce` — deferred, one execution per doorbell | **43** | 0 | **0** | `off_trap_claims=2812 inline_exceptions=35` |

The `on` arm died at `FAIL cuCtxCreate(&ctx,0,d) -> unknown error (999)` with two `Xid 31`
MMU faults: `ENGINE CE2 HUBCLIENT_CE0 faulted @ 0x7eca_d4e00000 … FAULT_PDE
ACCESS_TYPE_VIRT_WRITE` and `ENGINE GR0_PBDMA0 HUBCLIENT_ESC faulted @ 0x2_0440f000 …
FAULT_PTE ACCESS_TYPE_VIRT_READ` — both *"the mapping was not there when the engine ran"*,
and the second is the **GR report-semaphore page** this campaign has met before.

⇒ ★★★★★ **DEFERRING IS FINE. COALESCING IS NOT.** The two arrived as one change and were
therefore one red; `nocoalesce` is the variable that separates them, and it separated them in
a single 90-second boot.

### ⊘ WHERE `pubqueue` §2 IS WRONG — the premise was verified against the wrong consumer

§2 rests on *"the submission cursor is read at EXECUTION time, not latched at trap time"*.
That is **true and verified** of `kayfabe_rt::ceutils::run_submission`, which reads forward
from its `GpCursor` *while the entries decode*. It was **asserted, never measured**, of the
**forwarding** path — `SharedDevice::doorbell` → `kayfabe_fwd::read_gpfifo_ring` — and the
forwarding path is the one the failing arm exercises.

`[measured]` for a comparable doorbell count the control ran **229** publication passes and
the coalescing arm ran **53**: 200 doorbells' worth of forwarding was folded away, and the
engines faulted on ranges the folded-away passes would have carried.

⚠ **The lesson is not "coalescing is impossible".** It is that *"N doorbells are one act"* is
a claim about a **specific consumer**, and the queue made it about all of them.
`DoorbellAsyncArm::On` survives **by name only**, as this rung's negative control; re-enabling
it needs a per-consumer justification and a boot.

★★★ **And note what nearly happened**: `on` was the only armed arm when the first boot ran.
Without `nocoalesce` the result would have been *"deferring the doorbell breaks cup3"* — a
true sentence about the boot and a false one about the mechanism, and it would have retired
the owner's ruling on one measurement of a confounded pair.

## §8 ⊘ TWO OF THE FOUR GRADED NUMBERS DO NOT MEASURE WHAT THE BRIEF EXPECTED

Both are stated here rather than in a footnote, because in both cases the number moved
correctly and the *target* was wrong.

### 8.1 `inline_exceptions` — the doorbell's share went to **zero**, and 35 is somebody else's

`[measured, cup3]` the **first** `TRAPWITNESS` line of the control boot already reads
`inline_exceptions=35`, before the first doorbell, and the armed arms end the boot at
**exactly 35**. ⇒ every one of the control's remaining 821 was the doorbell's, and the lane
took all of them: **856 → 35, and the residual is entirely pre-doorbell.**

⊘ **`inline_exceptions = 0` is not reachable by moving the doorbell, by design.** The witness
counts host RM verbs issued under the BQL from *any* trap, and §4 of the parent rules that the
**revocation** direction must stay synchronous. The census does not currently split
*doorbell* from *not-doorbell*, which is why its printed target reads as unmet on a boot that
met it completely. **Splitting it is the obvious follow-up.**

### 8.2 `worst_trap` — it is not the doorbell's, and this rung cannot move it

| arm | `worst_trap` |
|---|---|
| `off` | 1 745 862 µs |
| `on` | 1 756 491 µs |
| `nocoalesce` | 1 780 811 µs |

⊘⊘ **Unchanged across all three arms, and — measured — already at its final value on the
FIRST `TRAPWITNESS` line of the boot**, i.e. **before any doorbell has been rung**, never
moving again. `grep -o 'worst_trap=[0-9]*us' | uniq` over a whole boot returns **one** value.

⇒ The 1.75 s trap is a single early guest-driver-init MMIO write, and attributing it to
publication — as the brief that commissioned this rung did, and as the number's position
beside `inline_exceptions` invites — is wrong. `max_reap_us` tops out at **14 939 µs** and
`DRAIN-DEFER` returns to zero, so it is not the disposal either. **It is unattributed, and
attributing it needs `kftime` armed on the register path, not another doorbell rung.**

## §10 ★★★★★ THE LATENCY WAS NEVER THE THREAD — **w330's DIRTY-GATE DEFAULT WAS UNREACHABLE**

Found while looking for why the LLM boot spends 360 s in publication, and it is a bigger
number than anything this rung moves.

`[measured w380llm2, the LLM boot this whole campaign is about]`:

```
DIRTY-GATE publish[fired=60800 skipped=0 0.0% skipped]
```

**A 0 % skip rate reads as *"every VAS was dirty every time"*. It is actually *"the gate was
never consulted."***

```rust
// crates/kayfabe-qemu-raw/src/shim.rs — BEFORE w383
pub fn dirty_gate_from(value: Option<&str>) -> Result<bool, ..> {
    // ★★★★★ w330 — DEFAULT MOVED off → ON, on measurement.
    None | Some("on") => Ok(true),
    ...
}

fn selected_dirty_gate(var: &str) -> bool {
    match std::env::var_os(var) {
        None => false,                      // ⊘⊘⊘ the None arm above is NEVER REACHED
        Some(v) => dirty_gate_from(Some(..)).unwrap_or(false),
    }
}
```

⇒ ⊘⊘⊘ **`dirty_gate_from(None)` has no caller.** w330's measured default change — *"median
18 741 → 2 197 µs (8.5×), p90 86 104 → 4 431 µs (19.4×), `^CUP3_VAL=43` held on every armed
boot"* — has **affected nothing since it was made**, and `KAYFABE_DIRTY_GATE_PUBLISH` appears
nowhere in `scripts/bench/w290p_run.sh`, so no boot this harness ever ran set it either.

★★★ **Two statements of one default, four screens apart in one file, and the caller silently
won.** The doc comment on `DIRTY_GATE_PUBLISH_ENV` still read *"Off by default"* while the
enum beneath it read `on`. Nothing was inconsistent enough to fail a build, a test, or a
census — and the census the gate prints is satisfied identically either way, because
`skipped=0` is what a disarmed gate and a permanently-dirty world both produce.

⚠ **This is the exact class the parent repo's `CLAUDE.md` opens with**: *"a correct document
that stopped being true and did not say so"*, here in code rather than prose, and
`a_blocker_i_declared_was_already_fixed.md`'s *"the recurring defect is CURRENCY, not
wrongness"* one layer down. It also fired the way that class always fires: **the instrument
that would have caught it printed a number that was true of both worlds.**

**Fixed at w383, in the direction the code already stated**: `selected_dirty_gate` now does
nothing but hand the environment to `dirty_gate_from`, which becomes the only statement of the
default; and `w290p_run.sh` exports both gates (defaulted `on`, overridable, echoed in the
arming record) so an ablation is expressible from the caller.

⇒ ★★★ **The LLM's 360 s of publication was paid with the switch that removes it in the OFF
position, and the switch had been recorded as ON for a month.** Whether arming it is
sufficient for `LLM_TOKENS > 0` is §11's boot; what is already established is that **the
doorbell's latency problem had a cheaper cause than the one this rung was commissioned to
fix**, and that neither would have been found without the other — the thread hunt is what
walked past the census.

## §11 ⊘⊘ A TEARDOWN-ONLY CENSUS IS UNREACHABLE ON THIS BENCH — measured, and it is not new

`grep -c "COMPLETION-OBSERVER stopped"` over a whole boot returns **0**, and so does
`grep -c "worker STOPPED and JOINED"`, while both *"started"* lines are present exactly once.
`boot_capture.sh` **kills** QEMU; `Regs::detach_ram` is never reached.

⇒ Every number that only prints at teardown — the observer's `declared/reads/verdicts`
census **and**, as first written, this rung's `PUBQUEUE` census — has been **structurally
unreachable on the bench since w326**, and its absence has read as *"nothing to report"*.
⚠ The `stop`-then-**join** ordering that both threads document as *"not optional, because the
loop reads guest RAM the hypervisor is about to release"* has, for the same reason, **never
executed on this bench**. That is safe only because SIGKILL takes the reader with it.

**Fixed for the lane**: the `PUBQUEUE` census now rides the `PT-DECODE` line every doorbell
prints, which is also the only place it can answer the question a deferred lane actually
raises — *"is the worker falling behind the guest?"* — since at teardown the queue is drained
by construction and a final depth is always `0`.

⊘ The observer's teardown census is left alone; naming it here is the whole remedy this rung
owes it.
