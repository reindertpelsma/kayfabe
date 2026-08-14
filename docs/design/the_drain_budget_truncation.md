# The drain-budget truncation — the ~20 % `FAULT_PDE` intermittent, root-caused

> **STATUS: LIVE — 2026-08-14 (w319).** Branch `w319-intermittent-fault-pde`, base master
> `ef05f9b3`. Bench `vh`, real GA106, host driver open `580.159.04`.
> Supersedes nothing; **answers** `traces/w314_confirm/README.md` §2's *"⚠ Unattributed: I did
> not find the cause. It is a rate, not a mechanism."*
> Pre-registration and per-boot artefacts: `traces/w319_intermittent/`.

---

## 1. The finding, in one paragraph

The doorbell-time publication pass drains the doorbelled VAS's guest-RAM rows **in ascending
VA order**, inside the MMIO trap, bounded by a **3 000 ms wall clock**
(`VAS_DRAIN_WALL_BUDGET`, `crates/kayfabe-qemu-raw/src/shim.rs`). The drain's own cost on this
workload is **13 313 rows × 199–280 µs = 2.65–3.73 s**, which **straddles that budget**. On a
boot that lands on the slow side, the loop `break`s and every row after that point is *never
attempted* — and because the walk is ascending, **what it drops is always the TOP of the
address space**, which is where the guest's completion-semaphore page `0x2_0440f000` lives.
The ring is then rung anyway; an engine writes the semaphore; the host MMU finds no page
directory and reports `Xid 31 … FAULT_PDE ACCESS_TYPE_VIRT_WRITE`. `cuCtxCreate` returns 719.

**It is not a race between threads.** Publication is synchronous on the vCPU thread, strictly
before the forward (`shim.rs:4826` → `shim.rs:4896`); there is no queue, no worker and no async
drain anywhere in the crate. ⇒ **the "publication ordering race" framing is refuted**; the
defect is a *budget truncation*, and the difference decides the fix.

---

## 2. ★★★★★ THE EVIDENCE WAS ALREADY ON DISK — w314 COMMITTED IT AND NOBODY READ IT

No boots were spent to attribute this. Every number below comes from
`traces/w314_confirm/run_*_qemu.log.gz`, committed the same day the flake was reported as
unattributed.

| boot | `CUP3_VAL` | `pinned / asked` | `DRAIN_MS` | `last_pinned_va` | budget |
|---|---|---|---|---|---|
| `w314basecup3` (clean master) | **RED** | 11 883 / 13 313 | 3000 | `0x2_0326a000` | ⚠⚠ **HIT** |
| `w314cup3` (branch) | **RED** | 11 810 / 13 313 | 3000 | `0x2_03221000` | ⚠⚠ **HIT** |
| `w314br1` | 43 | 13 313 / 13 313 | 2672 | `0x2_047ff000` | no |
| `w314br4` | 43 | 13 312 / 13 313 | 3000 | `0x2_047fe000` | hit, **1 row short** |
| `w314bt1` | 43 | 13 313 / 13 313 | 2898 | `0x2_047ff000` | no |

**`0x2_03221000 < 0x2_0440f000 < 0x2_047fe000`.** The faulting page lies strictly between where
the reds stopped and where the greens reached. Every red hit the budget; no green that reached
the page did.

### ★★★★★ THE MARGIN IS 0.6 %, AND A THRESHOLD DERIVED FROM THE SOURCE SEPARATES 5/5 BOOTS

The budget affords `3 000 ms / 13 313 rows = **225.3 µs/row**`. That number is computed from a
constant in the source and a row count in the log — **nothing is fitted**. Against each boot's
own reported `per_row` for the first drain:

| boot | `per_row` | vs 225.3 µs | outcome |
|---|---|---|---|
| `w314br1` | 199 µs | **under** | 43 |
| `w314bt1` | 216 µs | **under** | 43 |
| `w314br4` | **224 µs** | **under by 1.3 µs (0.6 %)** | 43 — and it stopped **one row** short |
| `w314basecup3` | **251 µs** | **over** | **RED** |
| `w314cup3` | **252 µs** | **over** | **RED** |

**5 / 5 correctly classified.** ⇒ this is not a correlation noticed after the fact; the
threshold was derivable before looking at any outcome. The whole ~20 % is the fraction of boots
whose per-row host-pin latency lands above **225 µs** — and the tightest green boot cleared it
by **0.6 %**. ⊘ A budget sized *"~8× above the measurement"* at w292 (1 075 rows) is sized at
**1.00×** for the population it actually meets (13 313 rows). The budget did not drift; **the
workload grew into it**, and nothing was watching the ratio.

★ **`w314br4` is the keystone.** It hit the budget too — and came back **green**, because it
stopped **one row short**, at `0x2_047fe000`, which is *past* the fault page. ⇒ **budget-hit is
not the failure; budget-hit BELOW `0x2_0440f000` is.** A grader keying on `budget_hit` alone
would have called `br4` red and been wrong.

### ★★★ AND THE WINDOW IS EXACTLY ONE DOORBELL

In the red boot's own log the **next** doorbell drains the remaining **1 430 rows in 353 ms**,
`complete=true`, reaching `0x2_047ff000`. The rows are not lost — **they arrive one doorbell too
late.** That is precisely the invariant this campaign quotes from the C artifact and has never
enforced: *"a mapping is always backed **before the engine that uses it runs**."*

### ⊘⊘ THE SIZING ERROR — THE BUDGET WAS SIZED ON A **RESIDUAL** AND IS SPENT ON A **TABLE**

`VAS_DRAIN_WALL_BUDGET`'s own doc says it is *"sized from the MEASUREMENT, and deliberately ~8×
above it"*, citing `[measured w291, boot w290ppinrate]`: **1 075 rows × 276–338 µs = 0.30–0.36 s**.

⊘ **But 1 075 was the END-OF-BOOT UN-PINNED RESIDUAL on the `both` arm**, where the bounded
256-row/doorbell sample had already pinned the rest across many doorbells. The `drain` arm
applies the same budget to `vas_guest_ram_rows` at the **first** doorbell, when **nothing is
pinned yet** — so what it actually meets is the **whole table: 13 313 rows**.

⇒ **12.4× the population the budget was sized against.** The 8× margin was consumed exactly
once, silently, by a change in *which* rows the pass walks — not by any drift in the budget or
in per-row cost. ★ The durable lesson: **a budget sized against a RESIDUAL is not a budget for a
POPULATION**, and nothing in the tree was watching the ratio. `br4` clearing by 0.6 % is what
"the margin is gone" looks like from inside a green boot.

---

## 3. Reproduced on demand — and it is DETERMINISTIC, not a rate

`KAYFABE_VAS_DRAIN_ROW_LIMIT=11800` (added this rung, default-off) truncates the drain at a
**row count** instead of a clock. `[measured w319, vh, n=3, tags `w319r1..3`]`:

| boot | `CUP3_VAL` | `pinned/asked` | `DRAIN_MS` | `last_pinned_va` | host `Xid` |
|---|---|---|---|---|---|
| `w319r1` | `NO_KERNEL_LINE` | 11 800 / 11 800 | 2693 | `0x203217000` | 31 · **CE3 · HUBCLIENT_CE1** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |
| `w319r2` | `NO_KERNEL_LINE` | 11 800 / 11 800 | 2508 | `0x203217000` | 31 · **CE3 · HUBCLIENT_CE1** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |
| `w319r3` | `NO_KERNEL_LINE` | 11 800 / 11 800 | 2775 | `0x203217000` | 31 · **GRAPHICS · HUBCLIENT_FE** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |

**3 / 3 red**, `last_pinned_va` **byte-identical** on all three. ⇒ this is an on-demand
reproducer, not a 20 % sample. Under the baseline ~20 % red rate, 3/3 red has p ≈ 0.008.

### ★★ AND IT ANSWERS THE "ODDITY" NOBODY COULD EXPLAIN

The standing puzzle was why a **copy engine** (`CE3 / HUBCLIENT_CE1`) reports a fault on a page
described as the *GR* completion semaphore's. `w319r3` shows **`GRAPHICS / HUBCLIENT_FE`
faulting at the same VA on the same arm**. ⇒ **the ENGINE is incidental and the PAGE is the
invariant.** Whichever engine executes the release first writes `0x2_0440f000`, and whichever
one that is takes the fault. There was never anything to explain about CE-vs-GR; the page is
the shared completion-semaphore page (`completion_watch.rs:703-705`: eight
`SET_REPORT_SEMAPHORE` targets `0x2_0440ff80 … 0x2_0440fff0`, all `Site::GuestRam`).

⚠ **My own pre-registration named `CE3 HUBCLIENT_CE1` as the predicted fingerprint and did not
anticipate the engine varying.** The VA, fault type and access type matched 3/3; the *engine*
matched 2/3. Recorded as a partial miss of my prediction that turned out to strengthen the
account rather than weaken it.

---

## 3b. ⊘⊘ A **SECOND**, DISTINCT INTERMITTENT — AND IT IS NOT THIS ONE

`[measured w319, vh, tag `w319xoff2`]` one boot came back `CUP3_VAL=NO_KERNEL_LINE CUP3_RC=1`
— **red by the usual grade** — and was **not this defect**:

```
FAIL cuInit(0) -> unknown error (999)          ← cuInit, NOT cuCtxCreate
stage ladder: ✘ CTX OK  (nothing ✔ at all)
hostdmesg_bytes = 0      Xid_lines = 0         ← NO Xid ANYWHERE
by engine: GrCompute=0 GrGraphics=0 Ce=2       ← 2 doorbells, against ~198 on a normal boot
W319KNOB = ⊘ABSENT   DRAIN clause = ⊘ABSENT    ← the drain never ran; nothing to truncate
```

⇒ **At least TWO failure modes hide behind `^CUP3_VAL != 43` on this box**, and they are
trivially separable: **this defect always carries a host `Xid`; that one carries none.**

★ **The `⊘ UNMEASURED, never 0` discipline is what caught it.** The arm script prints
`⊘UNMEASURED — OLD BINARY` rather than a zero for an absent knob line, so the boot announced
itself as *not comparable* instead of silently joining the red tally. A grader that printed `0`
would have counted it as a third confirmation of the wrong thing.

⚠ **Consequence for every rate in this campaign, including w314's ~20 %:** a bare
`^CUP3_VAL != 43` count is an **upper bound** on this defect's rate, not a measurement of it.
⊘ Not quantified here — one observation, reported as an observation.

---

## 4. ★★★ THE REGRESSION IS NAMED — an unconditional pin became a conditional one

`SharedDoorbell::pin_completion_guest_ram` pinned **exactly this page**, unconditionally, on
every doorbell. `shim.rs:3851` still records what it bought:

> *"`[measured, w266, real GA106, both arms]` pinning the completion page took the host GPU's
> eight `Xid 31 … @ 0x2_0440f000 ACCESS_TYPE_VIRT_WRITE` to **zero**"*

It was **deleted at w304** (`f20ab952`, *"DELETE THE FIVE INERT RELAXATIONS"*) on this argument:

> *"`VAS_PUBLISH=drain` pins through the identical verb over a candidate set that is a **strict
> superset** … Deleting these removes a redundant path, not a capability."*

⊘ **The claim is true of the candidate SET and false of the DELIVERY.** The superset is drained
under a clock, so membership in it does not imply being reached. The deletion converted an
unconditional guarantee into one contingent on five per-doorbell facts — the arm being `drain`,
a `vas_pdb` resolving, the proc not being `SYSTEM_PROC`, the row being in `vas.table` at that
instant, and **neither budget binding first**. ⇒ a lesson this tree already owns, one plane
over: *a strict-superset argument about a SET says nothing about a bounded TRAVERSAL of it.*

---

## 5. The fix, and why it is not "raise the budget"

Raising `VAS_DRAIN_WALL_BUDGET` works and is the wrong fix. The drain is held under the QEMU
BQL with **every vCPU halted**, and `[measured w314]` the surrounding disposal already consumes
**2.65–2.92 s of a 4 s `scrubberDestruct` budget (73 %)**. Completeness bought with more BQL
spends headroom that is nearly gone, and it scales with the guest's table, not with need.

### ⊘⊘⊘ THE OBVIOUS CHEAP FIX WAS TRIED AND **REFUTED** — and that is the useful half

`KAYFABE_COMPLETION_PIN=on` (added and measured this rung, default off) pins the pages
`WatchList::declared_sites()` names — the completions **the guest itself declared** — into the
doorbelled VAS *before* the budgeted drain. One page, one pin, restoring what w304 deleted.

It **executes, lands at the right VA, and lands in time** (`SEMAPIN[… declared_pages=1 pinned=1
… placed_as_asked=true fresh]`, on the same doorbell as the declaration, after the decode and
before the forward), through the identical verb the drain uses.

**And it does not fix the defect.** Under a truncation that is 0 green / 4 without it, the pin
arm returned **1 green / 3** — Fisher exact **p ≈ 0.43, not significant**.
⇒ **the completion page is not the whole of what the truncation drops.** A one-page patch
cannot stand in for a missing invariant, and the negative is worth more than the patch: it says
the fix must be about **completeness**, not about any particular page.

### ★ So what the fix has to be

The invariant is the C's, verbatim: **a mapping is always backed before the engine that uses it
runs.** Three ways to hold it, in increasing order of how much they deserve:

1. **Raise / remove the budget for the doorbelled VAS's first drain.** Immediate, ~0.7 s more
   BQL on top of 3.0 s. ⊘ Spends clause-(b) headroom that w314 measured at 73 % consumed.
   Mitigation, not a fix.
2. ★★★ **Make the pins cheap enough that completeness is affordable.** The cost driver is
   **13 313 individual host round trips at ~225 µs each**. A batched/bulk pin verb collapses
   the drain from ~3 s to a fraction of it, and it fixes the clause-(b) BQL problem *at the
   same time*. **This is the structural answer and it is the one to build.**
3. **Refuse to ring on an incomplete drain.** Holds the invariant by construction, but the
   guest has no retry contract here; architecturally the largest change.

⊘ Nothing is merged. The knobs are instruments, default-off, byte-identical to master when
unset. Results and per-boot artefacts in `traces/w319_intermittent/RESULTS.md`.

---

## 6. ★★★★★ WHAT EVERY OTHER LANE SHOULD DO ABOUT IT, STARTING NOW

**Stop grading this on `CUP3_VAL`.** The binary outcome is a ~20 %-probability *consequence*;
the drain's own completeness is a **per-boot deterministic observable of the same event**, and
it is already emitted on every boot:

```
DRAIN[visited=true asked=N pinned=M refused=0 DRAIN_MS=T
      W319KNOB[budget_ms=… row_limit=…] complete=BOOL ⚠⚠ WALL BUDGET HIT]
… last_pinned_va=0xVA
```

- `complete=true` **and** `pinned == asked` ⇒ the pre-existing intermittent **did not fire on
  this boot**. A red boot here is *yours*.
- `complete=false` ⇒ the drain was truncated. Compare `last_pinned_va` against the VA in the
  `Xid`: if the fault VA is **above** `last_pinned_va`, the red is **this defect**, not yours.
- ⊘ `budget_hit` alone is **not** the discriminator — `w314br4` hit the budget and was green.
  The discriminator is `last_pinned_va` **versus the faulting VA**.

⇒ **One boot now grades**, where before it took n≥4 plus a same-hour control. Every lane
currently paying the 4-boot tax can stop paying it.
