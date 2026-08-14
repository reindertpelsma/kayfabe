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
