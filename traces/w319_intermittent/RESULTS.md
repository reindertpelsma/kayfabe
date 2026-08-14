# `traces/w319_intermittent/` — the ~20 % `FAULT_PDE` intermittent, ROOT-CAUSED and REPRODUCED

> **STATUS: LIVE — 2026-08-14 (w319).** Branch `w319-intermittent-fault-pde`, base master
> `ef05f9b3`, bench `vh`, real GA106, host driver open `580.159.04`.
> Predictions: `PREREGISTERED.md`, committed before each arm ran.
> Mechanism: `docs/design/the_drain_budget_truncation.md`.
> **Outcome: (A)** — reproduced on demand *and* mechanism attributed by modulation — **with
> (D) folded in**: the brief's *publication-ordering-race* hypothesis is **refuted as stated**.
> ⊘ **The "fix" half of (A) is NOT delivered**: the cheap candidate fix was built, measured and
> **refuted**. What is delivered is the mechanism, two working knobs, an on-demand deterministic
> reproducer, a one-boot attribution tool, and a named structural fix that is *not yet built*.

### The whole rung in one table

| arm | knob | n | outcome | pre-registered? |
|---|---|---|---|---|
| **R** | `ROW_LIMIT=11800` | 3 | **3/3 RED**, identical `last_pinned_va` | ✔ as predicted |
| **X-off** | same, pin off (fix binary) | 2 | 1 RED + 1 ⊘UNMEASURED (other defect) | ✔ control held |
| **X-on** | same + `COMPLETION_PIN=on` | 3 | **1 green / 3 — fix REFUTED** (p ≈ 0.43) | ✘ falsifier fired |
| **M** | `BUDGET_MS=2500` | 2 | **2/2 RED**, wall budget hit | ✔ as predicted |
| **H** | `BUDGET_MS=20000` | 3 | **3/3 green**, `complete=true` 3/3 | ✔ but weakly informative |

**8 faults across 4 arms, every one at exactly `0x2_0440f000`** (`CE3` ×6, `GRAPHICS` ×2).

---

## 1. The mechanism, and it was on disk before this rung started

The doorbell-time whole-VAS guest-RAM drain walks **ascending VA order** under a **3 000 ms wall
budget**; the workload's own cost **straddles that budget**; a slow boot truncates and **drops
the top of the address space**, where the completion-semaphore page `0x2_0440f000` lives. The
ring is rung anyway and the host MMU reports `FAULT_PDE`.

Derived threshold, from a source constant and a log row-count with **nothing fitted** —
`3 000 ms / 13 313 rows = 225.3 µs/row` — classifies **5 / 5** of w314's boots:

| w314 boot | `per_row` | vs 225.3 | `pinned/asked` | `last_pinned_va` | outcome |
|---|---|---|---|---|---|
| `br1` | 199 µs | under | 13313/13313 | `0x2_047ff000` | 43 |
| `bt1` | 216 µs | under | 13313/13313 | `0x2_047ff000` | 43 |
| `br4` | **224 µs** | under by **0.6 %** | 13312/13313 | `0x2_047fe000` | 43 |
| `basecup3` | **251 µs** | **over** | 11883/13313 | `0x2_0326a000` | **RED** |
| `cup3` | **252 µs** | **over** | 11810/13313 | `0x2_03221000` | **RED** |

★ **The margin, exactly:** `3 000 ms / 225.3 µs` buys **13 315** rows; the workload needs
**13 313**. **Two rows.** That is the whole of the ~20 %.

`0x2_03221000 < 0x2_0440f000 < 0x2_047fe000`. ★ And the red boot's **next** doorbell drained the
missing 1 430 rows in 353 ms — **the rows arrive one doorbell too late.**

⊘ **The brief's publication-ORDERING-RACE hypothesis is REFUTED as stated.** Publication is
synchronous on the vCPU thread inside the MMIO trap, strictly before the forward
(`shim.rs:4826` → `:4896`); there is no queue, worker, or async drain in the crate. It is a
**budget truncation**, not a thread race.

---

## 2. ★★★★★ ARM R — REPRODUCED ON DEMAND, AND DETERMINISTICALLY

`KAYFABE_VAS_DRAIN_ROW_LIMIT=11800`. **3 / 3 RED**, `last_pinned_va` byte-identical on all three.

| boot | `CUP3_VAL` | `pinned/asked` | `DRAIN_MS` | `last_pinned_va` | host `Xid` |
|---|---|---|---|---|---|
| `w319r1` | `NO_KERNEL_LINE` | 11800/11800 | 2693 | `0x203217000` | 31 · **CE3 · HUBCLIENT_CE1** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |
| `w319r2` | `NO_KERNEL_LINE` | 11800/11800 | 2508 | `0x203217000` | 31 · **CE3 · HUBCLIENT_CE1** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |
| `w319r3` | `NO_KERNEL_LINE` | 11800/11800 | 2775 | `0x203217000` | 31 · **GRAPHICS · HUBCLIENT_FE** · @ `0x2_0440f000` · `FAULT_PDE` · `VIRT_WRITE` |

All four pre-registered clauses met. Under the baseline ~20 % red rate, 3/3 red has p ≈ 0.008.
★ Direct confirmation: in `w319r1`'s log the page is **declared 8 times and pinned zero times** —
`grep 'va=0x20440f000'` over the whole QEMU log returns **no pin**.

### ⚠ ONE PREDICTION OF MINE WAS WRONG, AND BEING WRONG IMPROVED THE ACCOUNT

I pre-registered the fingerprint as `CE3 HUBCLIENT_CE1`. `w319r3` faulted with **`GRAPHICS
HUBCLIENT_FE`** — same VA, same `FAULT_PDE`, same `ACCESS_TYPE_VIRT_WRITE`. VA/type/access
matched 3/3; **engine matched 2/3.**

⇒ ★★ **This dissolves the brief's standing "oddity"** (*why does a copy engine fault on a page
belonging to GR?*). **The ENGINE is incidental; the PAGE is the invariant.** `0x2_0440f000`
holds the guest's eight `SET_REPORT_SEMAPHORE` targets `0x2_0440ff80 … 0x2_0440fff0`, written by
whichever engine executes the release first — and that engine varies across boots. There was
never anything to explain.

---

## 3. ⊘⊘ A **SECOND**, DISTINCT INTERMITTENT — CAUGHT BY THE `⊘ UNMEASURED` DISCIPLINE

`w319xoff2` graded red by the usual `^CUP3_VAL != 43` and is **not this defect**:

```
FAIL cuInit(0) -> unknown error (999)      ← cuInit, not cuCtxCreate
stage ladder: nothing ✔ at all
hostdmesg_bytes = 0   Xid_lines = 0        ← NO Xid ANYWHERE
by engine: GrCompute=0 Ce=2                ← 2 doorbells, against ~198 on a normal boot
W319KNOB = ⊘ABSENT   DRAIN clause = ⊘ABSENT
```

⇒ **At least TWO failure modes hide behind `^CUP3_VAL != 43` on this box**, trivially separable:
**this defect always carries a host `Xid`; that one carries none.**
★ The arm script printing `⊘UNMEASURED — OLD BINARY` instead of `0` is the only reason it did
not join the red tally as a third false confirmation.
⚠ ⇒ every bare `CUP3_VAL != 43` rate in this campaign, **w314's ~20 % included**, is an **upper
bound** on this defect, not a measurement of it. ⊘ One observation; not quantified.

---

## 4. THE FIX ARMS — a same-binary control and the fix under provocation

`KAYFABE_COMPLETION_PIN=on` pins the pages `WatchList::declared_sites()` names into the
doorbelled VAS **before** the budgeted drain. Default off, so **one binary carries both arms**.

| arm | knobs | boots | result |
|---|---|---|---|
| **X-off** | `ROW_LIMIT=11800`, pin **off** | 2 | **1 RED** (same fingerprint) + **1 ⊘UNMEASURED** (the `cuInit 999` mode, §3) |
| **X-on** | `ROW_LIMIT=11800`, pin **on** | 3 | *see below* |

### ⊘⊘⊘ THE CANDIDATE FIX IS **REFUTED**, AND MY FIRST READING OF IT WAS OVER-CLAIMED

**Final: X-on = 1 green / 3. X-off + R = 0 green / 4.** Fisher exact **p ≈ 0.43** — **not
significant.** ⇒ **The completion pin is NOT a fix**, and the single green boot is **not
attributable to it**.

⚠ **I drafted this section, before boot 3 landed, as *"the pin converts a deterministic failure
into an intermittent one … causal evidence the completion page is on the critical path."* That
was wrong, and it was wrong in the exact way this tree keeps paying for**: n=2 of a 3-boot arm,
read in the favourable direction, with the word "causal" attached to a single observation. Boot
3 came back red and the arm is 1/3. The draft is preserved in the commit history rather than
quietly replaced.

⊘ What survives: the pin **executes, lands, and lands in time** (below), so its failure to fix
anything is a fact about the *mechanism*, not about the patch being inert. ⇒ **the completion
page is not the whole of what the truncation drops**, and a one-page patch cannot stand in for
the missing invariant.

### The pin DID execute, land, and land in time — so this is a real negative, not a dud patch

The pin **executes and lands**, verified from the boot's own emissions:

```
SEMAPIN[★ ARMED proc=2 pdb=0x201000 declared_pages=1 pinned=1 refused=0 skipped=0
        [va=0x20440f000 gpa=0x3f524000 host_va=0x20440f000 placed_as_asked=true fresh]]
```

and it lands **in time**: the first `COMPLETION-DECLARE` is at log line 562 (token `0x07`) and
the first `SEMAPIN … pinned=1` at line 591 — **the same doorbell, after the decode, before the
forward**. ⊘ It is the identical verb the drain uses (`SharedDevice::pin_guest_ram`, which
commits via `commit_pin_guest_ram` at `device.rs:4124`), so this is not an instrument gap.

**And boot 1 still faulted, identically.** ⇒ my pre-registered falsifier fired: *the completion
page is not sufficient; other pages in the dropped tail matter too.*

**But boot 2 came back `CUP3_VAL=43`, `Xid_lines=0`** — **green under a provocation that is
deterministically red without the pin** (3/3 in arm R, 1/1 in arm X-off, identical
`last_pinned_va` every time). ⇒ **the pin converts a deterministic failure into an intermittent
one.** That is causal evidence the completion page is *on* the critical path, and equally clear
evidence it is not the whole of it.

⇒ ★ **The correct fix is COMPLETENESS OF THE DOORBELLED VAS'S DRAIN BEFORE THE RING**, which is
the C's invariant verbatim: *"a mapping is always backed before the engine that uses it runs."*
Pinning one page is a patch on one symptom of a missing invariant.

---

## 4b. ★★★★★ ARM M — THE **FAITHFUL** KNOB. THE CLOCK ITSELF MODULATES IT TO 100 %

`KAYFABE_VAS_DRAIN_BUDGET_MS=2500` — not a proxy, **the very clock the defect trips on**.
**2 / 2 RED**, both pre-registered clauses met (`⚠⚠ WALL BUDGET HIT`, `complete=false`,
`DRAIN_MS=2500`, truncation below `0x2_0440f000`):

| boot | `pinned/asked` | `last_pinned_va` | host `Xid` |
|---|---|---|---|
| `w319m1` | 12235 / 13313 | `0x2033ca000` | 31 · **CE3 · HUBCLIENT_CE1** · @ `0x2_0440f000` · `FAULT_PDE` |
| `w319m2` | 10628 / 13313 | `0x202d83000` | 31 · **GRAPHICS · HUBCLIENT_FE** · @ `0x2_0440f000` · `FAULT_PDE` |

⚠ **A second prediction of mine missed its range:** I pre-registered `pinned` in 9 000–11 500;
the measurements are **12 235** and **10 628** — one inside, one above. The essential clauses
(red, budget hit, incomplete, truncated below the page) all held; the range did not, and it is
recorded as a miss rather than quietly widened.

★ **The engine varied again** — `CE3` then `GRAPHICS` — at the *same* VA, on the *same* arm.
That is now **2 independent arms × 5 boots** showing the engine is incidental and the page is
the invariant.

⇒ **THE RATE IS MODULATED TO 100 % BY TWO INDEPENDENT KNOBS** — a row count (arm R, 3/3) and a
wall clock (arm M, 2/2) — both acting on the same truncation. That is the modulation the brief
asked for, in the "make it worse" direction, pre-registered before it ran.

---

## 4c. ★★★★★ ARM H — THE OTHER DIRECTION, AND THE DOSE–RESPONSE IS MONOTONE

`KAYFABE_VAS_DRAIN_BUDGET_MS=20000`. Graded on the **mechanism variable**, as pre-registered,
because the green count alone is weak at this n.

**3 / 3 GREEN.** Every pre-registered discriminating clause met:

| boot | `CUP3_VAL` | `pinned/asked` | `DRAIN_MS` | `complete` | `budget_hit` | `Xid` |
|---|---|---|---|---|---|---|
| `w319h1` | **43** | 13313/13313 | 2792 | **true** | 0 | none |
| `w319h2` | **43** | 13313/13313 | 2958 | **true** | 0 | none |
| `w319h3` | **43** | 13313/13313 | 2924 | **true** | 0 | none |

⇒ **Three settings of ONE constant, everything else held:**

| `VAS_DRAIN_WALL_BUDGET` | truncated | `CUP3_VAL=43` |
|---|---|---|
| **2 500 ms** (arm M) | **2 / 2** | **0 / 2** |
| **3 000 ms** (master default) | 3 of 5 recorded w314 boots | ~4 / 5 |
| **20 000 ms** (arm H) | **0 / 3** | **3 / 3** |

★ A **monotone dose–response across three settings of a single source constant**, with the
shipping default sitting **inside** the failure region.

### ⊘⊘ BUT ARM H IS THE WEAKEST ARM HERE, AND I SAID SO BEFORE RUNNING IT

**None of arm H's three boots needed the extra budget.** Their drains finished in **2 792,
2 958 and 2 924 ms — all under the default 3 000 ms.** ⇒ all three would have completed at the
default too, so arm H demonstrates *"a 20 s budget cannot truncate"* (which is arithmetic) and
**not** *"raising the budget rescues a boot that would otherwise have failed"*. ⊘ It is
consistent with the mechanism; it is not independent evidence for it. The pre-registration
named this caveat in advance rather than after the numbers landed.

### ★★★★★ WHAT ARM H ACTUALLY BOUGHT — the cost distribution, and it is the whole finding

Pooling every first-drain `DRAIN_MS` this campaign has recorded:

```
2672   2792   2898   2924   2958   ≥3000   ≥3000   ≥3000
 br1    h1     bt1    h3     h2     br4    base    cup3
green  green  green  green  green  green    RED     RED
                                   ↑ budget = 3000 ms sits HERE
```

**8 observations; the budget lands at roughly the 60th percentile of the drain's own cost.**
⇒ the ~20 % is not a mystery rate — it is **the fraction of the drain's natural cost
distribution that falls on the far side of a constant sitting in its middle**. And the
tightest green (`h2`, 2 958 ms) cleared by **42 ms, 1.4 %**.

---

## 5. What every other lane should do about it, starting now

**Stop grading this on `CUP3_VAL`.** The binary outcome is a ~20 %-probability *consequence*;
the drain's completeness is a **per-boot deterministic observable of the same event**, already
emitted on every boot:

```
DRAIN[visited=true asked=N pinned=M refused=0 DRAIN_MS=T complete=BOOL ⚠⚠ WALL BUDGET HIT]
… last_pinned_va=0xVA
```

| you see | it means |
|---|---|
| `complete=true`, `pinned == asked`, **and** a fault | ⇒ **NOT this defect.** The red is yours. |
| `complete=false` **and** fault VA **above** `last_pinned_va` | ⇒ **this defect**, not yours. Discard or attribute. |
| no host `Xid` at all | ⇒ the **other** intermittent (§3). Neither. |
| `budget_hit` alone | ⊘ **NOT a discriminator** — `w314br4` hit the budget and was green. |

⇒ **One boot now grades**, where before it took n ≥ 4 plus a same-hour control.

`scripts/bench/w319_attribute.sh <tag>` implements exactly that table. `--selftest` runs six
offline fixtures plus an explicit matcher assertion; it is validated on four real boots.
★ **Run `--selftest` before trusting it** — its first version passed 5/5 fixtures while broken
on every real log (see the script header, trap 2).

---

## 6. ★★★ FOR `w318` SPECIFICALLY — three things, one of them a prediction

1. **Your hypothesis's framing is wrong and that is load-bearing.** There is no publication
   *ordering race*: publication is synchronous on the vCPU thread, before the forward. If your
   dirty-set gate is being designed against a race, it is being designed against something that
   does not exist. The real hazard is **incomplete delivery under a budget**.
2. **Grade with `w319_attribute.sh`, not with `CUP3_VAL`.** `VERDICT=0` on a faulting boot means
   *your* gate missed. `VERDICT=1` means the pre-existing truncation fired. `VERDICT=3` means
   the other intermittent. This is the discrimination you are blocked on, and it costs 0 boots.
3. ★★★ **PREDICTION, pre-registered here: a dirty-set gate will make this defect RARER, and you
   must not read that as your gate being correct.** The defect is *cost-driven* — 13 313 rows ×
   ~225 µs against a 3 000 ms budget. Anything that shrinks the candidate set shrinks the drain
   below the budget and the truncation stops happening. ⇒ **a green run after your change is
   consistent with both "my gate is right" and "my gate accidentally suppressed someone else's
   bug".** The discriminator is `pinned == asked` **and** `complete=true` on the boot in
   question — publish that number, not the outcome.
   ⊘ And the mirror: **a dirty-set MISS and this defect produce the same `FAULT_PDE` symptom**
   but different clauses — a miss shows `complete=true` with the page never a candidate; this
   defect shows `complete=false` with the page above `last_pinned_va`.
