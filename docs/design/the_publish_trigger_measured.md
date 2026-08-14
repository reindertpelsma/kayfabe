# The publish trigger, measured — the guest's TLB invalidate is a BAR0 register, and what that is and is not worth

> **STATUS: LIVE — 2026-08-14 (w326).** The decoder is built, unit-tested and **booted**
> (`kayfabe_device::mmuinval`). The revocation driver is built, unit-tested and booted
> (`kayfabe_qemu_raw::reclaimtick`). ⊘ **The deferred publication lane is still NOT wired** —
> §6 says exactly what blocks it and why this rung did not force it.
>
> **Parents this folds into** (corrections are *in* them, not only here):
> `publication_off_the_bql.md` §6 (its *"the guest's invalidate DOES NOT EXIST HERE"* is
> refuted in place), §4.3.1 (its named defect is fixed in place);
> `blocking_and_completion_model.md` §1 (a new clause-(b) timeout) and §4 (the w323 block's
> last sentence is refuted in place); `crates/kayfabe-device/src/pubqueue.rs` §4 item 1.

---

## 0. The answer in five lines

1. ★★★★★ **The trigger exists.** RM's TLB invalidate on GA106 is a **BAR0 MMIO write to
   `0x00B8_30B0`** — not an RPC, not a pushbuffer method, and with **no GSP branch**. We have
   been receiving it since M5 and dropping it into the unclaimed-offset census.
2. ★★★★★ **Measured on a real `cup3` boot** (`w326m1`, GA106, stock guest, `CUP3_VAL=43`):
   **377 triggers**, **`ALL_PDB` fraction 0.0000**, **`triggers_per_doorbell` 0.785**.
3. ⊘⊘ **The cost argument collapses**, exactly as `gmmu_publication_discipline.md` §8
   pre-registered it might. 0.785 is not ≪ 1. GPU-VAS-only it is 0.302 — a **1.6×** reduction
   in publication passes against the doorbell trigger, not an order of magnitude.
4. ★★★ **Tier 1 is still the right trigger, for two reasons that are not cost**: it is
   **sound** where the doorbell is not (a PTE can change between two GPFIFO commands with no
   doorbell), and it names its **PDB** where a doorbell names only a token.
5. ⊘ **The worst MMIO trap on this boot is 2 879 349 µs and it is the whole-VAS publication
   drain, not the doorbell and not the revocation drain.** The brief's pre-registered
   `~86 700 µs → <1 000 µs` was aimed at a different site.

---

## 1. The register, derived rather than tabled

`GPU_VREG_*` adds `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0x00B8_0000`
(`ogkm-580: kern_gpu_tu102.c:96-100`, `turing/tu102/dev_vm.h:28`), and the usermode window
this device **advertises** sits `DRF_BASE(NV_VIRTUAL_FUNCTION)` = `0x0003_0000` above it:

| BAR0 | register | fields used |
|---|---|---|
| `0x00B8_30A0` | `…_MMU_INVALIDATE_PDB` | `APERTURE 1:1`, `ADDR 31:4`, `ADDR_ALIGNMENT 0xc` |
| `0x00B8_30A4` | `…_MMU_INVALIDATE_UPPER_PDB` | `ADDR 19:0` |
| `0x00B8_30B0` | `…_MMU_INVALIDATE` | `TRIGGER 31`, `ALL_VA 0`, `ALL_PDB 1`, `HUBTLB_ONLY 2`, `REPLAY 5:3`, `INVAL_SCOPE 16:15` |

⊘ **Derived from the advertised `USERMODE` base, not added as a second `ChipProfile` row**, for
`crate::doorbell`'s reason: two independent rows for one fact can drift, and the symptom would
be a guest committing page tables at an offset we answer with a defaulted zero.

⚠ **A correction to this rung's own brief and to
`guest_invalidate_discipline_and_the_publish_boundary.md`:** both say the register is *"448 KiB
below the doorbell we already decode"*. It is **180 KiB** below —
`0x00BB_0090 − 0x00B8_30B0 = 0x2_CFE0 = 184 288 bytes`. The conclusion (same BAR, same window,
already trapped) is unaffected; the number is wrong in both and is pinned by
`mmuinval::tests::the_distance_to_the_doorbell_is_180_kib_not_448`.

---

## 2. ★★★ THE MEASUREMENT — boot `w326m1`, disarmed, one variable

```
MMUINVAL armed=false writes=377 triggers=377 all_pdb=0 all_pdb_frac=0.0000 all_va=377
         hubtlb_only=232 gpu_vas=145 polls=754 pdb_writes=754 distinct_pdbs=8
         doorbells=480 triggers_per_doorbell=0.7854 triggers_at_first_doorbell=66
         worst_hold_us=0 over_budget=0 reentrant=0 pending=false
CUP3_VAL=43  CUP3_RC=0  Xid_lines=0  unserviced_distinct=40
```

The three numbers the brief pre-registered, and one it did not ask for that changes the answer:

| # | pre-registered | measured |
|---|---|---|
| 1 | total `0xB830B0` writes over a `cup3` run | **377** (every one carrying `TRIGGER`; **0** scope-only writes) |
| 2 | writes per `cuLaunchKernel` — *"needs to be ≪ 1"* | **0.785 per doorbell**; **0.302** counting GPU VA spaces only |
| 3 | the `_ALL_PDB` fraction | **0.0000** — all 377 name exactly one PDB, across **8** distinct PDBs |
| ★ | *(not asked)* `HUBTLB_ONLY` | **232 of 377 (61.5 %)** are **BAR** VA spaces, not GPU ones |

★★ **(4) is what makes (2) readable.** Nearly two thirds of the invalidates are `HUBTLB_ONLY`
— BAR1/BAR2 aperture VA spaces, which are not compute-path publications at all. A publish
plane keyed naively on this register would do 377 passes where 145 are relevant; keyed on
`!hubtlb_only` it does 145. **Against the 229 publication passes the doorbell trigger actually
fired on this boot, that is 1.6× — a real but small win.**

### 2.1 The C agrees, which is the cross-check that matters

`gmmu_publication_discipline.md` scanned the committed C captures and recorded
`cap3_matmul_forwarding` (`cup8`, `bad=0 maxerr=0`): **308** invalidates, **177**
`HUBTLB_ONLY=1`, **131** `HUBTLB_ONLY=0`, **9** distinct PDBs, *"every write carried
`TRIGGER=1`, `ALL_VA=1`, `ALL_PDB=0`, `INVAL_SCOPE=NON_LINK_TLBS`"*.

Our Rust port on `cup3`: **377 / 232 / 145 / 8**, `all_pdb=0`, `all_va=377`. ★ Different
workload, different implementation, **same shape and the same 57–61 % BAR-space share.** The C
capture's exact commit word (`0x80010001` with `PDB = 0x02efba50`) is a unit test
(`the_word_the_c_captured_decodes_the_way_the_c_read_it`), so a future field-shuffle fails
offline rather than on a bench.

### 2.2 ★★ `polls = 754 = 2 × 377`, exactly — and that is a non-vacuity check

`kgmmuCheckPendingInvalidates` reads the register once *before* the write (is one already
pending?) and once *after* (has it cleared?). Two per invalidate is therefore the **floor**,
and hitting the floor exactly proves the disarmed plane answered `TRIGGER = FALSE`
immediately and the guest never spun. ⇒ when the lane is armed, `polls ≫ 2 × triggers` is the
signature of the guest waiting on us, and it is measurable without a stopwatch.

---

## 3. ⊘ WHAT THE COST ARGUMENT ACTUALLY WAS, AND WHY IT COLLAPSING DOES NOT MATTER MUCH

`gmmu_publication_discipline.md` §8 wrote its own falsifier: *"count `0xB830B0` writes per
`cuLaunchKernel`. If that ratio is not ≪ 1, the argument collapses"*. It is 0.785. **The
argument collapsed, and the doc that made it is the reason we know.** ★ That is a
pre-registered falsifier firing, which is the cheapest kind of correction this tree gets.

But the ordering the owner gave — *"exact GPU boundary (TLB invalidate) > trap the PTE write >
deferred publish on doorbell > work under the BQL"* — was never a cost ordering. It ranks by
**exactness**, and on that axis nothing changed:

- **The doorbell trigger has a hole and the owner named it.** A PTE can change between two
  GPFIFO commands with **no doorbell**, because the channel is already running
  (`GP_GET != GP_PUT`) and libcuda has no reason to ring. We would never publish it. The
  invalidate closes exactly that: **RM does not change a mapping without one** (see the parent
  doc's per-path table — map *and* unmap; the asymmetry is in the membar, never the invalidate).
- **The invalidate names its PDB; the doorbell does not.** `[measured]` today's
  `publish_vas_rows` iterates **every live pid × every VAS key** on every fire — 229 passes,
  2 529 ms of BQL time, `this_doorbell[fired=4 skipped=0]` on **all 229** (so `w318`'s dirty
  gate skipped nothing on this workload). An invalidate-triggered pass could be scoped to
  **one** of the 8 PDBs. ★★ **That is a much larger win than the 1.6×, and it is available
  only with tier 1.** It is not built here.

⇒ **Verdict: tier 1 for MAP, on soundness and scope. The frequency argument is retired, in
writing, so nobody re-derives it as a cost saving.**

---

## 4. ★★★★★ THE COMPLETION IS THE PROTOCOL'S, AND IT IS AN OBLIGATION

`kgmmuCheckPendingInvalidates_TU102` (`kern_gmmu_tu102.c:69-71`) spin-polls the same register
until `TRIGGER` reads false. Two consequences, pointing opposite ways:

- ★ **The guest is already stopped.** Publication done at this boundary cannot race the
  engine and needs no fence of ours — a quiescence point the protocol hands us free.
- ⚠ **Hold `TRIGGER` and never clear it and the guest hangs.** Not faults: spins to
  `gpuCheckTimeout`, then Xid/reset. The timeout, recovered rather than guessed:
  `GPU_TIMEOUT_DEFAULT` → `pTD->defaultus` → `osGetTimeoutParams` = **4 s (GRAPHICS)** /
  **30 s (COMPUTE)**, re-armed dynamically at `gpuChangeComputeModeRefCount`. **Design against
  4 s**; `INVALIDATE_HOLD_BUDGET_US = 40_000` is this tree's 1 % convention applied to it.

**What guarantees the clear**, stated so it is checkable rather than promised — and no two of
the three are sufficient:

1. **The completion is a `Drop` guard, not a success path.** A publication that panics,
   returns early or is refused still clears. `if ok { clear() }` would make a guest hang the
   punishment for any of our own errors.
2. **The flag is an atomic outside every lock** (`MmuInvalidateLog::pending`). A poisoned
   mutex, or a thread that died holding one, cannot hold it set. The guest's poll takes no
   lock at all.
3. **Refusal degrades to inline.** If the queue refuses, the caller publishes in the same trap
   and clears before returning — i.e. today's behaviour, which is never worse than the status
   quo.

⊘ **And the fourth, which is why this rung is safe:** the plane ships **disarmed**, and
disarmed `read_trigger()` returns `0` unconditionally. **A disarmed plane cannot hang a
guest** — `disarmed_answers_zero_forever_and_can_never_hang_the_guest` is that property.

### 4.1 ⚠ This is the FIRST guest-visible MMIO read that depends on completed work

`publication_off_the_bql.md` §5.2 enumerated all seven arms of `RegPlane::read_inner`, found
every one local, and concluded *"no guest-visible MMIO read depends on completed
publication"*. **That finding is now SCOPED, not refuted**: it remains true of all seven, and
`ReadOutcome::MmuInvalidate` is a deliberate eighth. It has its own `ReadOutcome` variant
rather than being folded into `Gsp` precisely so the exception is greppable.

★ It is safe to make this read mean something *because the guest is already blocking on it*.
We are not introducing a wait; we are answering one the hardware protocol specified.

---

## 5. ★★★★★ THE REVOCATION DRIVER — w323's named defect, fixed

`w323` §4.3.1: the budgeted drain has **exactly one production driver and it is a guest MMIO
write**, so a guest that frees its host-backed objects and then stops touching MMIO leaves a
live host-GPU translation into pages Linux has reused. *A bound discharged only by the
adversary is not a bound.*

Built: `crates/kayfabe-qemu-raw/src/reclaimtick.rs`. The second driver is the existing
**`kayfabe-completion-observer`** thread (250 ms tick), which is off-trap by construction and
can therefore mint an `OffTrap::claim` — the honest constructor that **panics** if it is ever
on a trap, not the counted `at_a_host_verb` exception w323 had to use everywhere else. ⇒ this
is the first production site that retires a census row rather than adding one.

★★★ **The hazard the fix creates, and it is worse than the leak if missed.**
`drain_retired_budgeted` plans under the rank-0 write guard and then issues its host verbs
**with no lock held** — which is exactly what makes it interruptible. Two drivers running
concurrently would both plan and both free the same retired object: a **double disposal of a
host RM object**. One gate, taken **asymmetrically**:

| side | acquire | why |
|---|---|---|
| worker | `lock()` | off-trap; blocking costs nothing and guarantees progress |
| vCPU (`Regs::write`) | `try_lock()`, **never blocks** | under the BQL. A blocking acquire there stops every vCPU and QEMU's main loop until *another thread* finishes host I/O — clause (a) violated by construction |

⊘ There is **no blocking method on the type a trap can reach**, so this is enforced by the API
rather than by a comment. A vCPU that misses the gate skips its drain for that trap only,
which is safe: whoever holds it is spending the same queue right now (`vcpu_skipped=N`).

⚠ **What this does and does not fix.** It makes the drain's **completion** independent of the
guest. It does **not** move the drain's cost off the BQL for traps that win the gate — that
stays tier 2, budgeted at 40 ms, and is correct there. ⊘ And `[measured w326m1]`
`max_drain_us = 53 193`, `max_reap_us = 54 917`: **the retired drain was never the worst
trap.**

### 5.1 ⚠⚠ MEASURED COST — THE TICK TRUNCATES THE PUBLICATION DRAIN, REPRODUCIBLY

`[measured, boots `w326r1`/`w326r2`, tick armed, vs `w326m1`/`w326o1` disarmed]` — and this is
the finding that decides how the arm ships:

| boot | tick | whole-VAS drain | `worst_trap_us` | `CUP3_VAL` |
|---|---|---|---|---|
| `w326m1` | **off** | `pinned=13313/13313 DRAIN_MS=2792` **`complete=true`** | 2 879 349 | 43 |
| `w326r1` | **on** | `pinned=13268/13313 DRAIN_MS=3000` **`complete=false` ⚠ WALL BUDGET HIT** | 3 106 218 | 43 |
| `w326r2` | **on** | `pinned=12728/13313 DRAIN_MS=3000` **`complete=false` ⚠ WALL BUDGET HIT** | 3 130 278 | 43 |
| `w326r3` | **on** | `pinned=13313/13313 DRAIN_MS=2921` **`complete=true`** | 3 033 088 | 43 |

⊘⊘ **AND `n = 3` CHANGED THE READING — at `n = 2` this table said "reproducibly truncates".**
It is **2 of 3**, not 3 of 3, and the third boot completed at `2921 ms`. ★ The correct
statement is therefore not *"the tick truncates the drain"* but:

> **The whole-VAS publication drain was ALREADY at 93 % of its budget on master (2792 / 3000),
> and the tick's ~130–210 ms of rank-0 contention crosses the line about two thirds of the
> time.** The tick is the straw, not the load.

⇒ two consequences, and the second is the more useful one:
1. the tick must get cheaper before it is armed (below), **and**
2. ★★ **the 3000 ms budget has ~7 % of headroom on a workload that already fits.** Any future
   change costing >208 ms of rank-0 time will truncate this drain, and the failure is silent
   in the workload — all four boots returned `CUP3_VAL=43`, `Xid=0`, the same 40 unserviced
   ids, and `w319_attribute.sh ⇒ VERDICT=0 GREEN`. **A truncated publication is a publication
   miss waiting for a different workload**, and nothing in the pass/fail vocabulary sees it.

⚠ This is precisely the campaign's own banked trap, arriving on schedule: *a single-boot
result has a ~20 % false-negative rate*. At `n = 2` I would have shipped "reproducible".

**The mechanism, and it is not mysterious.** `drain_retired_budgeted` takes the **rank-0 write
guard** in its plan phase; so does the publication drain. `[measured]` `worst_tick_us =
118 203` and `vcpu_skipped = 4438` — the worker holds the gate for up to 118 ms at a time and
4 438 vCPU traps found it held. ⇒ off-BQL is **not** off-contention: the tick pays for the
guest's freedom with rank-0 pressure on the very pass it is running beside.

⊘ **So the tick ships DISARMED**, and the next lane owns making it cheap (rate-limit the tick,
yield the rank-0 guard between chunks, or run it only when the publication drain is idle).
★ The defect it fixes is real and the fix is real; what is not yet true is that it is free.

### 5.1.1 ★★★★★ THE CONTROL REFUTED MY OWN PRE-REGISTRATION — **`vcpu_skipped = 2` WITH THE TICK OFF**

Pre-registered for arm O (same binary, tick disarmed): *"`RECLAIM-TICK armed=false
working_ticks=0 vcpu_skipped=0`"*. Measured:

```
RECLAIM-TICK armed=false working_ticks=0 idle_ticks=0 worker_disposed=0 worker_reaped=0
             vcpu_skipped=2 worst_tick_us=0
```

Everything is 0 **except `vcpu_skipped`**. And `ReclaimTick::spend` returns **before** touching
the gate when disarmed (`a_disarmed_spend_never_runs_and_never_takes_the_gate` pins exactly
that), so **no worker took it**. ⇒ **something else was already inside the drain block, twice,
on a boot with no worker thread at all.**

★★★ **The double-disposal hazard this gate was built for ALREADY EXISTS ON MASTER**, and the
disarmed control is its known-positive. Two candidates, and this rung does **not** distinguish
them — both are real and both are refused correctly:

1. **two vCPUs concurrently in `Regs::write`** — which would mean MMIO dispatch is not always
   serialised by the BQL the way this tree assumes it is; or
2. **re-entrancy on one thread** — the drain path re-entering `Regs::write` beneath itself.

⊘ **Whichever it is, `try_lock` is the only safe acquire.** Under (2) a `lock()` would
**deadlock a vCPU against itself** on the first occurrence; under (1) it would stall every
vCPU. The API having no blocking method a trap can reach is therefore not merely hygiene — it
is what keeps this measurement from being a hang.

⚠ **Attributing it is the next lane's, and it is cheap**: print the thread id and a
re-entrancy depth beside `vcpu_skipped`. Until then this is *"two concurrent or re-entrant
entries into the retired drain, on master, measured"* and no more.

★ Note what produced it: a **pre-registered prediction of a control arm's numbers**. The
finding is in the one field that disagreed, on the arm whose whole purpose was to be boring.

### 5.2 ★ THE WORST TRAP IS THE PUBLICATION DRAIN — a four-point attribution, residuals printed

| boot | tick | `worst_trap_us` | `DRAIN_MS` | residual | drain's share | clamped? |
|---|---|---|---|---|---|---|
| `w326o1` | off | 2 820 454 | 2 732 000 | **88 454 µs** | 96.9 % | no |
| `w326m1` | absent | 2 879 349 | 2 792 000 | **87 349 µs** | 97.0 % | no |
| `w326r3` | on | 3 033 088 | 2 921 000 | **112 088 µs** | 96.3 % | no |
| `w326r1` | on | 3 106 218 | 3 000 000 | **106 218 µs** | 96.6 % | ⚠ yes |
| `w326r2` | on | 3 130 278 | 3 000 000 | **130 278 µs** | 95.8 % | ⚠ yes |

⇒ the whole-VAS publication drain accounts for **95.8–97.0 %** of the worst MMIO trap on every
boot, with a residual of **87–130 µs·10³** that is small, stable and slightly larger on the
armed arm. ⊘ **The retired drain is not it** (`max_drain_us` = 40 554 / 53 193 / 56 280) and
neither is the per-doorbell publication pass (w315's 86.7 ms). **Three different things wear
the word *"drain"* in this tree's logs**, and the brief's target was aimed at a fourth.

⚠ Per this campaign's own trap — *a candidate whose magnitude matches your measurement belongs
to the instrument until proven otherwise; ≥3 points, print every residual, refit without the
largest* — `DRAIN_MS` is **budget-clamped at 3000 on two of the five points**, so those two
carry no independent information about the fit. **Refit on the three unclamped points alone**
(`w326o1`, `w326m1`, `w326r3`): residuals 88 454 / 87 349 / 112 088 µs, share 96.3–97.0 %.
★ The fit survives dropping both clamped points **and** the largest residual.

---

## 6. ⊘ WHAT IS NOT BUILT, AND THE STRUCTURAL REASON

**The deferred publication lane is still not wired to `SharedDoorbell::ring`.** w323 declined
to wire it because it could not boot; this rung could boot and declined for a different and
more specific reason, stated so the next lane does not rediscover it:

> ★★★ **`Regs` is a `Box`, not an `Arc`.** It is owned by the QOM object as a raw pointer
> (`shim_unsafe.rs:1075`, `:1268`). `publish_vas_rows` is a `&self` method on it, so **no
> worker thread can hold the thing that publishes.** The fields it needs (`device`, `plane`,
> `exports`, `vas_publish`, `fb_join`) are individually `Arc`s or `Copy`, so the fix is to
> lift them into a cloneable `PublishContext` and make `publish_vas_rows` a free function
> over it — **a real refactor of the publication path, not a wiring change**, and one whose
> whole value is a measured before/after.

⊘ **And a simplification worth recording, because it removes an obligation the brief
imposed.** The brief required giving
`doorbell_reaches_the_completion_observer.rs` and
`doorbell_is_forwarded_without_reading_the_ring.rs` a `PublicationQueue::completed()` barrier,
because an asynchronous lane invalidates their *"call the doorbell, immediately assert the
host verb landed"* shape. **That obligation does not arise for what this rung landed**:
publication is still synchronous, so their shape is unchanged and they are neither relaxed nor
rewritten. ⇒ the barrier becomes the *next* rung's obligation, and it arrives with the lane
that actually needs it rather than ahead of it.

⚠ Note also, for whoever wires it: **`PublicationQueue::completed()` is not a barrier
primitive.** It is a plain monotonic counter with no `wait_until`; `publication_off_the_bql.md`
§10 calls it *"the completion barrier the design already provides"*, which overstates the code
by one method.

---

## 6.1 ⊘ AND THE TRIGGER IS DECODED BUT NOT ARMED — the decision, stated

The decoder ships **arming-capable and disarmed**. `KAYFABE_PUBLISH_PLANE` is read and
reported; nothing sets `MmuInvalidateLog::arm()` in production.

★ **Why, and it is a judgement this doc would rather record than have re-litigated.** Arming
this trigger makes a bug in *our* publication path into a **guest hang** (§4): the guest is
spinning on a register only we can clear, for 4 s, and then takes an Xid/reset. Every other
failure this plane can produce is a GPU fault, which is contained and fail-safe. ⇒ arming it
is the one change here whose blast radius is the whole VM, and the measurement (§3) says it
buys **1.6× fewer publication passes** — a real but small win — while the site that actually
dominates (§5.2, the whole-VAS drain at 95.8–97.0 % of the worst trap) is **untouched by the
trigger choice**.

⇒ **Arm it in the rung that moves the execution site**, where the two changes can be graded
against each other, and not at the end of the rung that measured it. That is the same
discipline `w323` applied to the lane itself, applied one level up.

⊘ **What IS built and watched offline**, so arming later is a wiring change and not a design:
the `TRIGGER` hold, the idempotent completion, the over-budget diagnostic, the reentrancy
counter, and — the one that matters — `disarmed_answers_zero_forever_and_can_never_hang_the_guest`.

---

## 7. What in this rung's brief turned out wrong

- ⊘ **"448 KiB below the doorbell"** — 180 KiB (§1). Wrong in the brief *and* in the research
  tree's doc, which is how it got into the brief.
- ⊘ **"worst trap hold ~86 700 µs → <1 000 µs"** — the boot's worst trap is **2 879 349 µs**,
  and it is the **whole-VAS publication drain** (`DRAIN_MS = 2792` against a 3000 ms budget),
  not the per-doorbell 86.7 ms w315 measured and not the retired drain (53 ms). The target is
  right in spirit and the anchor was the wrong site by a factor of 33.
- ⊘ **"decode it and report writes per `cuLaunchKernel`"** — a per-launch ratio is not the
  decidable quantity, because `cup3` launches once and the publication cost is per *pass*. The
  ratio that decides is **triggers per doorbell** vs **passes actually fired**, and both terms
  are now taken by one observer over one interval so they cannot come to describe two boots.
- ⊘ **"the three pieces are one unit; splitting them leaves an incoherent half-state"** — the
  trigger and the completion are one unit (and are built together), but the **execution site**
  is separable and is blocked on §6's refactor. What shipped is coherent: a measured trigger, a
  protocol-specified completion that ships disarmed, and a revocation driver.
- ★ **"including that tier 1 is reachable at all, which is unmeasured"** — it is reachable, and
  it was already visible in every boot log as three `UNCLAIMED-CENSUS` rows.
