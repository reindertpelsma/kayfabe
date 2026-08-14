# The publication pass's breadth — priced, scoped, and NOT the worst trap

> **STATUS: LIVE — 2026-08-14 (w328).** Branch `w328-scope-the-publication`, base master
> `308799cd`. Bench `vh`, real GA106, host driver open `580.159.04`.
> **Corrects** the reading of `the_publish_trigger_measured.md` §3 and §5.2 that this rung's
> own brief carried — folded in above the text it corrects rather than left beside it.
> Per-boot artefacts: `traces/w328_scope/`. Pre-registration: `scripts/bench/w328_all.sh`'s
> header, written before any boot ran.

---

## 0. ⚠ FIRST: FOUR THINGS WEAR THE WORD "DRAIN", AND THE TWO HEADLINE NUMBERS BELONG TO TWO OF THEM

This is the fifth rung in a row to be sent at the wrong one. The list, with the site that owns
each:

| # | name | site | measured cost |
|---|---|---|---|
| 1 | the **publication CENSUS/JOIN pass** | `publish_vas_rows`, `shim.rs:7904` | **229 passes, 2 529 ms of BQL — CUMULATIVE over the boot** |
| 2 | the **guest-RAM PIN drain** | `measure_guest_ram_pin_rate`, `shim.rs:8216` | **`DRAIN_MS=2792` — ONE trap, 95.8–97.0 % of `worst_trap_us`** |
| 3 | the **retired/disposal drain** | `drain_retired_budgeted` | 40–56 ms (w317 bounded it) |
| 4 | the **doorbell page-table pass** | `pt_decode`/`pt_sweep` | 85.2 → 4.08 ms (w318 dirty-gated it) |

### ★★★★★ AND THE CONFLATION THE BRIEF INHERITED, STATED ONCE

The brief presents *"229 passes, 2 529 ms of BQL, and the LAST ONE REPORTS `published=0`"* and
*"2 879 349 µs worst trap, 95.8–97.0 % of it"* as two facts about **one** pass. They are facts
about **(1)** and **(2)** respectively, and they have **different denominators**:

- **2 529 ms is CUMULATIVE across 229 traps** (~11 ms each).
- **2 879 349 µs is ONE trap** — the single doorbell on which the drain runs.

⇒ *"the pass sweeps every live pid × every VAS key"* is true of **(1)**, and **(1) is not the
worst trap.** The worst trap is **(2)**, and **(2) is already scoped to the ringing channel's
VAS** — by a predicate that has been in the tree since w292.

---

## 1. THE SOURCE ANSWER, BEFORE ANY BOOT — the breadth is NOT what the drain spends

`shim.rs:8409`, unchanged by this rung:

```rust
                // ★★★★★ **w292 — THE ONE SCOPED BUDGET CHANGE, AND IT IS THIS PREDICATE.**
                let doorbelled = drain_target == Some((pid, pdb));
                let cap = if doorbelled {
                    vas_drain_row_limit()      // 65 536, and a 3 000 ms wall clock
                } else {
                    VAS_PINRATE_ROWS           // 256, and NO clock
                };
```

and the assignment that produces the graded number, `shim.rs:8611`:

```rust
                if doorbelled {
                    drain_pinned = rows_pinned;
                    drain_refused = vas_refused;
                    drain_ms = vas_ms;
```

⇒ **`DRAIN_MS` is the doorbelled VAS's own wall time and nothing else's.** The 13 313 rows it
walks are one address space's rows. The other VASes contribute a bounded 256-row sample each,
outside that clock and outside that number.

**So the breadth cannot be 97 % of the worst trap, because 97 % of the worst trap is a number
that excludes the breadth by construction.** Whatever the breadth costs, it is inside the
**87–130 µs·10³ residual** w326 printed — 3.0–4.2 % of the trap.

⊘ **This is a source reading, and a source reading is not a measurement.** §2 measures it.

### 1.1 What the breadth IS, then

Two populations, both real:

- **(1)'s census + joins over every non-doorbelled VAS.** `vas_publish_census` is O(rows of
  that `Vas`) and proc 0 alone holds 6787 rows. This is the 2 529 ms, and w326 measured its
  yield: the last pass reports `published=0 refused=8`, and **all 229 passes fired**
  (`this_doorbell[fired=4 skipped=0]` — w318's dirty gate skipped none on this workload).
- **(2)'s 256-row samples over every non-doorbelled VAS.** Up to 256 real cross-process pins
  per doorbell per VAS, at ~225 µs each — a term that has never been priced separately.

---

## 2. MEASURED — the breadth's own cost, split at the source

`[measured w328, vh, real GA106, host driver open 580.159.04, arm `w328a` = master's behaviour
on this binary, n=3, tags `w328a1..3`]`

| boot | `CUP3_VAL` | `worst_trap_us` | `DRAIN_MS` | residual | drain share | margin | `complete` | `pinned/asked` |
|---|---|---|---|---|---|---|---|---|
| `w328a1` | **43** | 3 048 658 | 2 960 | **88 658 µs** | 97.1 % | **1.01×** | true | 13 313/13 313 |
| `w328a2` | **43** | 3 112 556 | 2 969 | **143 556 µs** | 95.4 % | **1.01×** | true | 13 313/13 313 |
| `w328a3` | **43** | 2 788 831 | 2 681 | **107 831 µs** | 96.1 % | **1.12×** | true | 13 313/13 313 |

⇒ w326's 95.8–97.0 % reproduces at **95.4–97.1 %**, 3/3, on a different day and a different
binary. ⚠ And the margin is **1.01×** on two of three boots — the drain is at **98.7 % and
99.0 % of its 3 000 ms budget**, tighter than the 93 % w326 recorded.

### 2.1 ★★★★★ THE BREADTH IS 0.0084 % OF THE WORST TRAP AND 0.90 % OF THE BOOT

`[measured, `w328a1`, the same boot as row 1 above]`, summed over **all 229** publication
passes:

```
W328SCOPE  CUM over 229 passes:
    target_us = 2 666 358      target_published = 66
    other_us  =    24 349      other_published  =  0        breadth_share = 0.9049 %
W328PIN    CUM over 229 passes:
    other_vases = 0   other_us = 0   other_pinned = 0
```

- **On the worst trap itself: `other_us = 256 µs` of `worst_trap_us = 3 048 658 µs` —
  0.0084 %.**
- **Over the whole boot: 24 349 µs of 2 690 707 µs — 0.90 %.**
- **The pin pass's breadth is exactly ZERO, on 3/3 boots.** No non-doorbelled VAS ever offered
  a single guest-RAM candidate, so the 256-row sample pinned nothing all boot.
- **`other_published = 0` over 229 passes.** The breadth publishes nothing — w326's yield
  claim, confirmed.

⇒ **The brief's (A) is HALF RIGHT AND HALF REFUTED.** *Vestigial in yield* — measured, zero,
all boot. *"Scoping it drops the worst trap far below budget"* — **wrong by a factor of ~11 000
on the worst trap and ~110 on the boot.** ⊘ **My own pre-registered prediction that this would
happen was itself unmeasured when I wrote it**; it is recorded in `w328_all.sh`'s header
before the boots ran, and §2.2 is what it turned into.

### 2.2 ★★★★★ SO WHERE DOES THE 2 529 ms GO? — 328 HOST JOINS, ALL REFUSED, ON 8 RANGES

The doorbelled VAS's own pass is **99.1 %** of it, and it is **bimodal** — the mean is a lie:

```
per-pass target_us:  n=229  min=477  p25=568  median=661  p75=799  p95=58 011  max=143 964
expensive(>10 ms):   n=44   sum=2 524 134 us
cheap   (<=10 ms):   n=185  sum=  142 224 us
```

Correlating each pass's cost against **what it was asked to do** separates the two modes
completely:

| | passes | mean cost | largest table walked |
|---|---|---|---|
| `candidates=0` | **180** | **632 µs** | **18 277 rows** |
| `candidates>0` | **49** | **52 094 µs** | 18 309 rows |

★★★ **The table is the same size in both rows.** ⇒ **the census WALK over 18 277 rows costs
632 µs — 35 ns/row — and is not the cost.** The cost is the join attempts, at
**≈ 6.4 ms per leaf**.

And the census of what those attempts achieved, on the doorbelled pdb `0x201000`:

```
    180  candidates=0  published=0  refused=0
     41  candidates=8  published=0  refused=8        ← 328 REFUSED JOINS
      3  candidates=1  published=1  refused=0
      2  candidates=2  published=2  refused=0
      1  candidates=7  published=7  refused=0
      1  candidates=32 published=24 refused=8
      1  candidates=28 published=28 refused=0
```

⇒ **2 524 134 µs of the boot's 2 690 707 µs of publication BQL is ~400 `join_one_fb_leaf`
attempts, of which 328 are the SAME 8 framebuffer ranges re-offered 41 times and refused every
time** — *"that framebuffer range is already joined"*, which is precisely the host-state
refusal `w318`'s `joined_fb_ranges().len()` term was added to gate on.

### 2.3 ⚠⚠ AND IT IS `gate=off` — NOT THE WORKLOAD

`the_publish_trigger_measured.md:201-203` reads `this_doorbell[fired=4 skipped=0]` on all 229
passes and attributes it to the workload: *"so `w318`'s dirty gate skipped nothing on this
workload"*. The boot's own line says otherwise:

```
... arm=drain W328SCOPE[...] gate=off this_doorbell[fired=4 skipped=0] ...
DIRTY-GATE publish[fired=912 skipped=0 0.0% skipped] witness[fired=229 skipped=0 0.0% skipped]
```

**`gate=off`.** `KAYFABE_DIRTY_GATE_PUBLISH` ships **disarmed** (`shim.rs:14261`,
`None => false`), deliberately and for a stated reason. **The gate skipped nothing because it
was never asked.**

⊘ **A disarmed gate and a gate that never fires print the SAME COUNTER**, and only the arm word
separates them — this tree's own `a_census_zero_needs_a_known_positive` shape, arriving in a
merged doc. ⇒ the arm script now **extracts `gate=`** rather than inferring it.

★ And the question *"what does arming it buy"* was **already answered** and I checked before
spending boots on it: `w318_the_dirty_gate.md:127`, twelve matched doorbells, one variable —
`vas_publish` **45.849 → 0.201 ms/launch, 228×**. Sweep 2 therefore spends its boots on what
w318 did *not* measure: whether the gate **composes** with the scope and the coalescer, and
what it does to `worst_trap_us`.

### 2.4 ⚠ UNATTRIBUTED, AND IT IS THE NEXT RUNG'S BEST LEAD — a REFUSAL that costs 6.4 ms

A `join_one_fb_leaf` attempt costs **≈ 6.4 ms whether it succeeds or is refused**, against
**77 µs** for a successful `map_gpu_va` (`the_drain_cost_is_per_call_not_per_page.md` §2).
**A refusal that costs 83× a success is not a refusal that merely failed a check.** Nothing in
this rung attributes it; it is stated as the measurement it is.

### 2.5 ⊘ MEASURED LIMITS OF THIS INSTRUMENT, stated rather than discovered later

- `W328PIN`'s `other_us` **excludes** a sampled VAS whose candidate list came back empty —
  master `continue`s above the bracket. The excluded term is the `vas_guest_ram_rows` walk,
  and it is bounded above by `W328SCOPE`'s `other_us = 256 µs`, which walks the *same* VASes'
  rows *including* proc 0's 6 787. ⇒ the omission cannot exceed a fraction of a millisecond.
- `breadth_share` on a **single pass** is integer-percent and reads `0%` for anything under
  1 %. The cumulative figure is computed in µs and is the one quoted.
- ⊘ The later drains end at host-**shaped** VAs (`0x7610_3a9f_f000`). That is UVM unified
  addressing — `shape_cannot_discriminate_origin.md`: the GPU VA **is** the process VA. It is
  not a leaked host pointer, and it is recorded here so nobody re-chases it.

---

## 3. THE SCOPING ARM, and what it is worth

<!-- W328-RESULTS-3 -->

---

## 4. ⚠ THE CORRECTNESS HAZARD RUNS THE OTHER WAY, AND IT IS NAMED IN THE CODE

Every other budget in `shim.rs` risks doing **too little work too slowly**. Scoping risks
**not doing the work at all** — and a mapping we decline to publish is a mapping the host MMU
has no directory for, i.e. a GPU fault **indistinguishable by symptom** from
`the_drain_budget_truncation.md`'s pre-existing intermittent.

Three refusals are therefore built in, and two of them are asserted **offline, with no GPU and
no environment variable in the path** (`w328_scope_predicate_tests`, 4/4):

1. **No target ⇒ no scoping.** A doorbell that resolved no channel facts names no VAS.
   Scoping to a VAS we cannot name is scoping to **none**, which would publish nothing at all.
2. **A `SYSTEM_PROC` target ⇒ no scoping.** §12.26: proc 0 is never attempted by either pass.
   Scoping to it leaves every publishable VAS unvisited **while the line still reads
   `scoped=true`** — the favourable-looking absence this tree keeps paying for.
3. **No w318 stamp for a scoped-out VAS.** The stamp asserts *"this census ran to
   completion"*. Stamping one we never looked at tells the next doorbell that a VAS nobody
   examined is clean — a publication silently never performed.

★ Both passes call **one pure function**, `publish_scope_scoped`, so they cannot come to
disagree about which VAS a single doorbell is about. Two sources of truth beside one complete
value is a shape this tree has already paid for one plane over.

⚠ **And the trap w319 pre-registered, which applies to this rung more than to any other:**
anything that makes the doorbell faster makes the truncation **rarer without fixing it**. A
green run is not evidence. Margins are reported as a **multiple of budget at ≥3 points**.

---

## 5. Results

<!-- W328-RESULTS-5 -->

---

## 6. ⚠ WHAT THE BRIEF GOT WRONG, AND WHAT I GOT WRONG — named, because it was asked for

### 6.1 The brief

1. ★★★ **"The publication drain is 2.88 s of held BQL, 97 % of the worst trap, and its last
   pass publishes NOTHING"** conflates **two passes with two denominators**. §0. The 97 % is
   `measure_guest_ram_pin_rate`'s `DRAIN_MS` on **one** trap; the "last pass publishes nothing"
   and the 2 529 ms are `publish_vas_rows` **cumulative over 229 traps**. Both statements are
   true; they are not about the same thing, and (A) is only entailed if they are.
2. ★★★ **"The pass sweeps every live pid × every VAS key ⇒ that is why it is expensive."**
   The sweep is real and it is **0.90 % of the pass and 0.0084 % of the worst trap**. §2.1.
3. ★★ **"229 passes, 2 529 ms of BQL — and w318's dirty gate skipped none on this workload"**
   (inherited from `the_publish_trigger_measured.md:201`). The gate was **`gate=off`**. §2.3.
4. ★ **"`m2hostsem`-style flag reading"** is not in this brief, but the same class is: the
   brief's own taxonomy lists the guest-RAM pin drain (#3, *"w321 coalesced it"*) and the
   whole-VAS publication drain (#1) as **different** drains. They are the **same site** —
   `DRAIN_MS` is what w321 coalesced and what w326 measured at 95.8–97.0 %.

### 6.2 Mine

- ⊘ **My pre-registered prediction in `w328_all.sh` was itself unmeasured when I wrote it.**
  It was a source reading, and I said so in the header before the boots ran. It held — but
  *"a self-correction is a claim like any other and needs its own evidence"* cuts both ways,
  and a prediction that happens to be right is not thereby evidence.
- ⊘ **A comment I wrote in `shim.rs` asserted "the WALK is the whole cost"** — refuted three
  hours later by §2.2's own instrument (632 µs for 18 277 rows). Corrected in place.
- ⊘ **`W328PIN`'s bracket has a hole I did not see until the data arrived**: master
  `continue`s past a sampled VAS with an empty candidate list *above* the bracket, so that
  VAS's walk is unattributed. Bounded in §2.5; not fixed, because fixing it would have changed
  the control mid-sweep.

---

## 7. THE RULING THIS RUNG ASKS FOR

**Do not scope.** The breadth is *free and idle*: it publishes nothing and costs 0.90 % of the
pass. Turning it off buys ~24 ms per boot and takes on the one risk that presents as a GPU
fault. ⇒ `KAYFABE_PUBLISH_SCOPE` ships as an **instrument**, default `all`, and this rung
explicitly **does not** propose flipping it.

★★★ **The two levers that matter are already built, already measured, and both default-off.**
That, not the breadth, is this rung's finding:

| cost | site | the fix that exists | its state |
|---|---|---|---|
| the **worst trap** — 1.01× of budget on 2/3 control boots | `measure_guest_ram_pin_rate` | **w321's coalescer** `KAYFABE_DRAIN_BATCH=coalesce` | merged, **disarmed** |
| the **cumulative BQL** — 2.52 s of refused joins | `publish_vas_rows` | **w318's dirty gate** `KAYFABE_DIRTY_GATE_PUBLISH=on` (228× on `vas_publish`) | merged, **disarmed** |

⚠ **And the standing hazard neither of them addresses**, because it is not a speed problem:
a `join_one_fb_leaf` attempt costs **6.4 ms**, refused or not. §2.4.
