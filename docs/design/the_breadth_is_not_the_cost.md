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

`[measured, arm `w328a`, n=3]`, summed over **all** of each boot's publication passes
(`traces/w328_scope/census/w328a*_census.txt`):

| boot | passes | `target_us` | `target_published` | `other_vases` | **`other_us`** | **`other_published`** | **breadth share** |
|---|---|---|---|---|---|---|---|
| `w328a1` | **229** | 2 666 358 | 66 | 3 | **24 349** | **0** | **0.9049 %** |
| `w328a2` | **229** | 2 489 089 | 63 | 3 | **23 408** | **0** | **0.9317 %** |
| `w328a3` | **229** | 2 544 533 | 66 | 3 | **23 814** | **0** | **0.9272 %** |

and the **pin** pass's breadth, on the same three boots:

```
W328PIN  CUM over 229 passes:   other_vases = 0   other_us = 0   other_pinned = 0     (3/3)
```

- **On the worst trap itself: `other_us = 256 µs` of `worst_trap_us = 3 048 658 µs` —
  0.0084 %.**
- **Over the whole boot: 0.9049 % / 0.9317 % / 0.9272 % — n=3, spread 0.03 pp.**
- **The pin pass's breadth is exactly ZERO, on 3/3 boots.** No non-doorbelled VAS ever offered
  a single guest-RAM candidate, so the 256-row sample pinned nothing all boot.
- **`other_published = 0` on 3/3 boots, over 687 passes in total.** The breadth publishes
  nothing — w326's yield claim, confirmed and extended from one boot to three.
- ⊘ **`passes = 229` on all three boots, to the digit.** The doorbell count is workload-fixed
  here, which is why the three cumulative figures are comparable at all.

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

## 3. THE SCOPING ARM, and what it is worth — measured, and it is worth nothing

`[measured w328, vh, real GA106, arm `w328s` = `KAYFABE_PUBLISH_SCOPE=doorbelled`, n=3]`

| boot | `CUP3_VAL` | `worst_trap_us` | `DRAIN_MS` | margin | `complete` | `pinned/asked` | attributor |
|---|---|---|---|---|---|---|---|
| `w328s1` | 43 | 3 164 107 | **3 000** | **1.00×** | ⚠ **false** | **12 433**/13 313 | `VERDICT=0` |
| `w328s2` | 43 | 2 860 308 | 2 775 | 1.08× | true | 13 313/13 313 | `VERDICT=0` |
| `w328s3` | 43 | 2 989 376 | 2 904 | 1.03× | true | 13 313/13 313 | `VERDICT=0` |

**`W328PIN[arm=doorbelled scoped=true scoped_out=1 other_vases=0 other_us=0 other_pinned=0]`
on all three** — the arm word is in the boot's own line, so `scoped=true` is a measurement and
not an inference from the launcher's environment.

| | arm A (control) | arm S (scoped) |
|---|---|---|
| mean `worst_trap_us` | **2 983 348** | **3 004 597** |
| mean `DRAIN_MS` | **2 870** | **2 893** |

⇒ **+0.7 % and +0.8 %. Indistinguishable.** Scoping the breadth does not move the worst trap,
which is exactly what §2.1's 0.0084 % predicts and is the pre-registered outcome for this arm.

### 3.0 ★★ THE SCOPE DEMONSTRABLY EXECUTED — and it is SMALLER THAN THE NOISE IT WOULD REMOVE

The scoped boots' own censuses (`traces/w328_scope/census/w328s*_census.txt`) show the
mechanism firing, so *"it changed nothing"* is a measurement and not a silent no-op:

| | arm A (`w328a1`) | arm S (`w328s1`) |
|---|---|---|
| `W328SCOPE scoped_out` (cum) | **0** | **687** = 229 passes × 3 VASes |
| `W328PIN scoped_out` (cum) | **0** | **229** |
| `other_vases` / `other_us` | 3 / **24 349 µs** | 0 / **0 µs** |
| `DIRTY-GATE publish[fired=…]` | **912** | **228** — 4× fewer consultations |
| `target_us` (cum, the term the saving comes off) | **2 666 358** | **2 491 346** |

★★★ **And there is the whole argument in the last row.** The breadth the scope removes is
**24 349 µs**; the boot-to-boot spread in the *same* boot's `target_us` — the quantity it would
be subtracted from — is **175 012 µs, 7.2× larger**. ⇒ **the saving is not merely small, it is
below the noise floor of its own denominator**, and no number of boots makes it visible in the
outcome.

### 3.1 ⚠ ONE ARM-S BOOT TRUNCATED — reported, and NOT attributed to scoping

`w328s1` came back **`complete=false`, `pinned=12433/13313`** with `CUP3_VAL=43` and **zero
`Xid`**. That is `the_drain_budget_truncation.md`'s defect, and it is graded here on **state**
exactly as w319 asked: a green `CUP3_VAL` on a truncated drain is *"a publication miss waiting
for a different workload"*.

⊘ **It is not attributed to scoping**, and the control says why: arm A's own margins are
**1.01× twice** — `DRAIN_MS` 2960 and 2969 against a 3 000 ms budget, i.e. **98.7 % and 99.0 %
consumed**. On master the drain is one boot's noise from truncating, and scoping removes work
rather than adding it. **1 of 6 cup3 boots at ≤1.12× margin truncated** — a rate consistent
with w314's ~20 %.

★ **And this is the finding that matters more than the scoping result:** w326 recorded the
budget at 93 % consumed and called the remaining 7 % *"a silent truncation that every pass/fail
metric reports as green"*. **On this branch's control it is 98.7–99.0 % consumed.** The margin
did not shrink because anything regressed — `[measured w321]` per-row host-pin latency is
**boot-variable** — but the headroom w326 measured is not a property one can rely on.

---

## 3.2 ★★★★★ WHAT DOES MOVE IT — the coalescer, 30.6× / 16.9× / 14.0× of budget

`[measured w328, arm `w328c` = scope + `KAYFABE_DRAIN_BATCH=coalesce`, n=3]` — one lever added
to arm S, everything else identical:

| boot | `CUP3_VAL` | `worst_trap_us` | `DRAIN_MS` | residual | drain share | **margin** | `complete` | `pinned/asked` |
|---|---|---|---|---|---|---|---|---|
| `w328c1` | 43 | **188 776** | **98** | 90 776 µs | 51.9 % | **30.61×** | true | 13 313/13 313 |
| `w328c2` | 43 | **459 984** | **177** | 282 984 µs | 38.5 % | **16.95×** | true | 13 313/13 313 |
| `w328c3` | 43 | **314 033** | **215** | 99 033 µs | 68.5 % | **13.95×** | true | 13 313/13 313 |

- **Margin at 3 points: 13.95× / 16.95× / 30.61× of the 3 000 ms budget**, against **1.01× /
  1.01× / 1.12×** on the control. Every residual printed; **no point is budget-clamped**, so
  the fit uses all three, and dropping the largest residual (`w328c2`, 282 984 µs) leaves
  13.95×–30.61×.
- `worst_trap_us` **2 983 348 → 320 931 mean, 9.3×**; best boot **16.1×**.
- **`complete=true` and `pinned == asked` on 3/3**, where the same binary without this one word
  truncated 1 of 3.
- ⊘ Squarely inside w321's pre-registered **2.21×–34.9 %-of-budget** band, measured
  independently three rungs later on a different day.
- ★ `inline_exceptions` **19 213 → 1 362 / 3 237 / 2 150** — the clause-(b) residue falls with
  it, because the coalescer's whole mechanism is *fewer, larger* host calls.

### 3.2.1 ★★★ AND THE WORST TRAP CHANGES OWNER

On the control the drain is **95.4–97.1 %** of the worst trap. With the coalescer it is
**38.5–68.5 %**, and the residual — 91–283 ms — is now comparable to it. ⇒ **once the drain is
coalesced, the publication CENSUS pass becomes a co-equal term of the worst trap**, which is
the site §2.2 priced at ~6.4 ms per join attempt and §2.3 found running with its gate
disarmed. That is what sweep 2's arm `G` tests, and it is why the ladder is ordered this way.

### 3.3 THE OTHER TWO WORKLOADS — one is live, one is vacuous, and the brief predicted which

**`w328e` — cup8 (2048² fp32 matmul), scope + coalesce, n=3.** The oracle that fails
*quietly-wrong* rather than loudly-absent:

| boot | `CUP8_BAD` / `CUP8_MAXERR` | `worst_trap_us` | `DRAIN_MS` | **margin** | `complete` | `pinned/asked` |
|---|---|---|---|---|---|---|
| `w328e1` | **0 / 0** | 166 436 | **77** | **38.96×** | true | 13 313/13 313 |
| `w328e2` | **0 / 0** | 180 177 | **63** | **47.62×** | true | 13 313/13 313 |
| `w328e3` | **0 / 0** | 279 180 | **186** | **16.13×** | true | 13 313/13 313 |

⇒ **bit-exact at 3/3 with the coalescer armed**, and margins **16.13×–47.62×** — a second
workload, n=3, three more points, none clamped.

⊘ **`w319_attribute.sh` returns `VERDICT=2 UNMEASURED` on all three, and that is STRUCTURAL,
not a failure**: it grades `CUP3_VAL`, and a cup8 boot emits none. Saying so is the point —
`2` is *"not a pass"*, and a lane that read it as one would be reading an inapplicable
instrument as a green. The grading here is `complete=` / `pinned == asked` / `Xid=0` /
`CUP8_BAD=0`.

**`w328x` — R33 arm 1 (a raw CE client), scope + coalesce.** ⚠⚠ **VACUOUS FOR THE DRAIN FIX,
AND THE BRIEF PRE-REGISTERED EXACTLY THIS.** The workload runs correctly — the arm-1 line is
present and byte-correct (`4096 bytes moved … GP_GET 1 caught GP_PUT 1`) — but:

```
DRAIN[visited=true asked=0 pinned=0 refused=0 DRAIN_MS=0 … complete=true]
W328PIN[arm=doorbelled scoped=true scoped_out=1 other_vases=0 other_us=0 other_pinned=0 drain_ms=0]
```

- **`asked=0`** ⇒ there is nothing for the coalescer to coalesce. **The coalescer arm is
  VACUOUS here**, on the same `asked=0` grounds it was vacuous for w321.
- **`scoped_out=1` but `other_vases=0 other_us=0`** ⇒ the scope *executed* and removed
  **nothing measurable**. Call it *minimally live* and no more.

⇒ **R33 is evidence that neither change BREAKS a raw CE client, and NO evidence about either
fix.** ★ **Coverage for this rung's claims therefore rests on cup3 and cup8**, at n=3 each,
and that is stated here rather than rounded up to "three workloads".

⊘ And the census makes it quantitative rather than a judgement call: **R33 runs `passes=2`**
publication passes in the whole boot, against **229** (cup3) and **275** (cup8), with
`target_us = 2 271 µs` total. There is nearly nothing of this plane in that workload to
exercise.

### 3.4 ★★ CUP8 CORROBORATES THE MECHANISM ON A SECOND WORKLOAD — and the cost SCALES WITH IT

`[measured, `w328e1`]`, the same correlation as §2.2 on a completely different program:

| | passes | mean | largest table walked |
|---|---|---|---|
| `candidates=0` | **180** | **2 420 µs** | **18 277 rows** |
| `candidates>0` | **95** | **35 774 µs** | 18 277 rows |

**Identical shape, same table size in both rows** — and `target_us = 3 834 095 µs` cumulative,
**larger than cup3's 2 666 358**, because cup8 has **95** join-carrying passes where cup3 has
49. ⇒ the publication join cost **tracks the workload's allocation count**, and on cup8 it
exceeds the coalesced drain by more than an order of magnitude.

⊘ `other_us = 0` and `other_published = 0` here too — but arm `w328e` is a **scoped** arm
(`scoped_out=825`), so those zeros are the scope working, **not** an independent measurement of
the breadth on cup8. The breadth's own price was measured on the **unscoped control only**
(§2.1), and cup8 has no unscoped arm in this rung. **Stated, not glossed.**

---

## 3.5 ★★★★★ THE GATE — the pre-registered falsifier fires, and w326's reading is REFUTED

`[measured w328, arm `w328g` = scope + coalesce + `KAYFABE_DIRTY_GATE_PUBLISH=on`, n=3]`.

The falsifier written into `w328_all2.sh` before these boots ran: *"`skipped` MUST go non-zero;
if it stays 0 with the gate ARMED then the epoch really does move every doorbell and w326's
reading was right after all."* Measured, `w328g1`:

```
gate=on this_doorbell[fired=0 skipped=1]
DIRTY-GATE publish[fired=11 skipped=217  95.2% skipped]  witness[fired=229 skipped=0]
```

⇒ **95.2 % skipped. The epoch does NOT move every doorbell.** `the_publish_trigger_measured.md`
attributed `fired=4 skipped=0` to the workload; it was the **arm word**.

⊘⊘ **AND THE SKIP COUNT IS DETERMINISTIC TO THE DIGIT, WHERE THE WALL CLOCK IS NOT.** Across
n=3 per workload:

```
cup3, gate on  (w328g1/2/3):    DIRTY-GATE publish[fired=11 skipped=217  95.2% skipped]   ×3
cup8, gate on  (w328ge1/2/3):   DIRTY-GATE publish[fired=12 skipped=262  95.6% skipped]   ×3
cup3, gate off (w328c1/2/3):    DIRTY-GATE publish[fired=228 skipped=0    0.0% skipped]   ×3
```

**Byte-identical on all three boots of each arm.** ⇒ *what the gate skips* is a property of the
workload and is reproducible; ★ this is the strong form of the result, and it does not depend
on any timing.

### ⊘⊘⊘ AND MY OWN FIRST READING OF THE COST WAS A SINGLE-BOOT ERROR — CORRECTED HERE

I first wrote *"2.21× off the cumulative publication BQL"* from `w328g1` alone. At n=3 that
does not survive:

| arm | gate | cumulative `target_us`, three boots | mean | spread |
|---|---|---|---|---|
| `w328a` (cup3) | off | 2 666 358 / 2 489 089 / 2 544 533 | 2 566 660 | **1.11×** |
| `w328s` (cup3) | off | 2 491 346 / 2 401 922 / 2 609 015 | 2 500 761 | 1.09× |
| `w328c` (cup3) | off | 2 591 947 / **7 128 882** / 5 149 318 | 4 956 716 | **2.75×** |
| `w328g` (cup3) | **on** | 1 207 402 / 3 131 822 / **496 271** | 1 611 832 | **6.31×** |
| `w328e` (cup8) | off | 3 834 095 / 4 136 243 / 5 982 723 | 4 651 020 | 1.56× |
| `w328ge` (cup8) | **on** | 2 736 944 / 448 512 / 412 527 | 1 199 328 | **6.63×** |

⇒ **The cumulative wall time is 6× boot-variable on the armed arms and 2.75× even on an
unarmed one.** The honest statements are:

- **cup8**: mean **4 651 020 → 1 199 328 µs, 3.88×**, and the two cheapest gated boots
  (448 512 / 412 527) are ~10× below the cheapest ungated one. **The clearest signal.**
- **cup3**: median over all nine gate-off boots **2 591 947** vs median over the three gated
  **1 207 402** — **2.15×**, but with a 6.3× spread on three points, **this is suggestive and
  not established.**
- ⊘ Arm `c`'s cumulative is *higher* than arm `a`'s, which no mechanism in this rung predicts.
  It is boot noise in a metric this rung did not design its arms to resolve. **Said, not
  explained away.**

⚠ **The lesson is the brief's own, and I walked into it**: *a single-boot result has a ~20 %
false-negative rate*, and I quoted a 2.21× from one boot in a rung whose whole thesis is that
n=1 misattributes.

⚠ `target_published` = **63 / 66 / 66** on the gated arm against **66 / 63 / 66** on the
control — same set, so no publication was dropped. Flagged rather than dismissed, because
*"the gate dropped a publication"* is precisely the failure this gate could have.

### 3.5.1 ⊘⊘ AND THE APPARENT REGRESSION IN ARM G IS **NOT THE GATE** — IT IS `chains`

Arm G's drains are **slower** than arm C's, and the naive reading is *"the gate hurt the
drain"*:

| boot | arm | `DRAIN_MS` | **`chains`** | `worst_trap_us` | margin | `complete` |
|---|---|---|---|---|---|---|
| `w328e2` | C-family | 63 | **75** | 180 177 | 47.62× | true |
| `w328e1` | C-family | 77 | **136** | 166 436 | 38.96× | true |
| `w328c1` | C | 98 | **242** | 188 776 | 30.61× | true |
| `w328c2` | C | 177 | **511** | 459 984 | 16.95× | true |
| `w328e3` | C-family | 186 | **594** | 279 180 | 16.13× | true |
| `w328c3` | C | 215 | **690** | 314 033 | 13.95× | true |
| `w328ge2` | **G**(cup8) | 94 | **188** | 230 481 | **31.91×** | true |
| `w328ge3` | **G**(cup8) | 106 | **213** | 210 114 | **28.30×** | true |
| `w328g3` | **G** | 256 | **938** | 395 380 | **11.72×** | true |
| `w328g1` | **G** | 524 | **1 751** | 730 814 | **5.73×** | true |
| `w328ge1` | **G**(cup8) | 627 | **2 698** | 957 020 | **4.78×** | true |
| `w328g2` | **G** | 1 281 | **5 155** | 1 588 258 | **2.34×** | true |

★★★★★ **The rows sort by `chains`, and the arm word does not enter.** Feeding each boot's
`chains` into **w321's cost model, unmodified and not refitted** —
`drain_us ≈ chains × 232 µs + rows × 3.35 µs`, with `rows = 13 313` on every boot:

| boot | gate | chains | predicted | measured | error |
|---|---|---|---|---|---|
| `w328e2` | off | 75 | 61 999 | 63 000 | **+1.6 %** |
| `w328e1` | off | 136 | 76 151 | 77 000 | **+1.1 %** |
| `w328ge2` | **on** | 188 | 88 215 | 94 000 | +6.6 % |
| `w328ge3` | **on** | 213 | 94 015 | 106 000 | +12.7 % |
| `w328c1` | off | 242 | 100 743 | 98 000 | **−2.7 %** |
| `w328c2` | off | 511 | 163 151 | 177 000 | +8.5 % |
| `w328e3` | off | 594 | 182 407 | 186 000 | **+2.0 %** |
| `w328c3` | off | 690 | 204 679 | 215 000 | +5.0 % |
| `w328g3` | **on** | 938 | 262 215 | 256 000 | **−2.4 %** |
| `w328cc1` | off ★ | 1 031 | 283 791 | 297 000 | +4.7 % |
| `w328cc2` | off ★ | 1 373 | 363 135 | 388 000 | +6.8 % |
| `w328g1` | **on** | 1 751 | 450 831 | 524 000 | +16.2 % |
| `w328ge1` | **on** | 2 698 | 670 535 | 627 000 | −6.5 % |
| `w328g2` | **on** | 5 155 | 1 240 559 | 1 281 000 | **+3.3 %** |

**14/14 within 16.2 %, median absolute error 5.0 %, across a 69× range in `chains`, a 20×
range in `DRAIN_MS`, two workloads, both gate arms, and the same-hour control.** ⇒ w321's model, fitted on w321's boots, **predicts w328's boots on a different
day without a single parameter moved.** That is a stronger corroboration of the model than
w321 could give itself.

⚠ **And it is exactly this campaign's banked trap firing in my favour:** *a candidate whose
magnitude matches your measurement belongs to the INSTRUMENT until proven otherwise.* The
5.4× spread between arm C and arm G's drains looked like my arm and is the **host's free
lists** — the boot-variable contiguity `the_drain_cost_is_per_call_not_per_page.md` §3 already
warned about. ⊘ **w321 measured that variability at 3.31×–11.29× over two boots; nine boots
here span 75 → 5 155 chains, a 69× range.** The variability is far larger than recorded.

### 3.5.2 ⚠⚠ WHAT I CANNOT SEPARATE, AND THE DESIGN FLAW THAT CAUSED IT

Arm G's `chains` (938 / 1 751 / 5 155) are systematically higher than arm C's (242 / 511 /
690). **Two live explanations, and this sweep cannot choose between them:**

1. **the gate causes it** — skipping publications changes the host allocation order; or
2. **session drift** — host free-list fragmentation accumulates, and **arm G ran last.**

⊘ Evidence against (2) alone: `w328g3` ran *after* `w328g1`/`g2` and has the **lowest** chain
count of the three (938 < 1 751 < 5 155), so the drift is not monotone. Evidence against (1)
alone: the whole session's chain counts trend upward, and the arms were never interleaved.

★★★ **AND ARM `GE` LARGELY SETTLES IT — with the gate ON, two boots produced `chains = 188`
and `213`, the second- and third-LOWEST counts in the entire rung.** So *"the gate causes
fragmentation"* cannot be a mechanism that operates on every boot. Over all twelve coalesced
boots:

```
gate ON  chains: 188  213  938  1751  2698  5155      median 1344.5
gate OFF chains:  75  136  242   511   594   690      median  376.5
Mann-Whitney U (n1=n2=6): min U = 8, and p<0.05 needs min U <= 5  ⇒ NOT SIGNIFICANT
```

⇒ **Suggestive on the median, not significant at n=6 vs 6, and still confounded with arm
order.** Recorded exactly that way. ⊘ A rung that wanted to settle it would **interleave the
arms**, which costs nothing and is the standing recommendation from §3.5.2.

★ **The design flaw is mine and it is worth stating plainly: a SEQUENTIAL arm sweep on a box
whose free lists drift CONFOUNDS ARM WITH SESSION TIME.** Every arm in this rung — and in
w321, w326 and w327 before it — is ordered, not interleaved. The fix is one line (interleave
the tags) and it costs nothing.

### 3.6 ★★★★★ THE DISCRIMINATOR — the SAME ARM, run LAST, and the gate is EXONERATED

`[measured w328, arm `w328cc` = arm C's exact environment (`scope` + `coalesce`, **gate OFF**),
launched immediately after all 24 sweep boots, n=2]`:

| boot | when | gate | `chains` | `DRAIN_MS` | `worst_trap_us` | margin | `complete` | `CUP3_VAL` |
|---|---|---|---|---|---|---|---|---|
| `w328c1` | early | off | **242** | 98 | 188 776 | 30.61× | true | 43 |
| `w328c2` | early | off | **511** | 177 | 459 984 | 16.95× | true | 43 |
| `w328c3` | early | off | **690** | 215 | 314 033 | 13.95× | true | 43 |
| **`w328cc1`** | **last** | off | **1 031** | 297 | 449 933 | **10.10×** | true | 43 |
| **`w328cc2`** | **last** | off | **1 373** | 388 | 498 146 | **7.73×** | true | 43 |

⇒ **The SAME ARM, with the gate OFF, run at the END of the session, produces `chains` = 1 031
and 1 373 — above ALL THREE of its own earlier boots, and above three of the six gate-ON
boots** (188, 213, 938).

★★★★★ **THE CONFOUND RESOLVES IN FAVOUR OF SESSION DRIFT, AND THE GATE IS EXONERATED.**
`chains` is host free-list state that drifts with time-in-session; the arm word does not
predict it. ⊘ **And the exoneration was cheap — two boots, six minutes** — because the
question was posed as *"what would distinguish these two explanations"* rather than argued
from the nine boots already in hand.

⚠ **The standing recommendation is unchanged and is now paid for twice: INTERLEAVE THE ARMS.**
Nine boots could not separate what two boots in the right order settled.

⊘ **What is NOT in doubt:** `complete=true` and `pinned == asked` on **9/9** coalesced boots,
`CUP3_VAL=43` on 6/6 cup3 boots, `CUP8_BAD=0` on 4/4 cup8 boots, and **the worst margin
anywhere in the coalesced set is 2.34×** against master's **1.01×**.

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

## 5. Results, and how to re-derive every number here

`traces/w328_scope/` — the two launchers (with their pre-registrations in their own headers),
one arm log per arm, one **census** per boot, and the small per-boot artefacts.

| tool | what it answers | selftest |
|---|---|---|
| `scripts/bench/w328_census.sh <tag>` | distils one boot's publication plane to ~45 lines: arm words, the drain, the cumulative breadth, the per-pass distribution, and **the cost-vs-`candidates` correlation with `max_total` beside it** | — |
| `scripts/bench/w328_fit.py <qemu.log>…` | the worst-trap attribution table, margins, residuals; **refuses a fit below 3 unclamped points** and refits without the largest residual | `--selftest` ⇒ 10/10 |
| `scripts/bench/w319_attribute.sh <tag>` | is a red **yours** or the pre-existing truncation | `--selftest` ⇒ 6 fixtures + 1 matcher, run before every sweep |

⊘ **The `run_<tag>_qemu.log` files are not committed** — ~3.6 MB × 24 boots. Each census names
its source log **and its byte count**, so the omission is auditable rather than silent.

### 5.1 ⊘ THE INSTRUMENT WAS FROZEN AT SWEEP START, DELIBERATELY

`w328_arm.sh` in the repo carries three lines (`gate=`, `DIRTY-GATE`, `CUM over N passes`) that
were added **after** the first sweep began and were **not** synced to the bench. All eight arms
therefore ran the identical script, and those three lines are recovered per boot by
`w328_census.sh` from the same log. ⚠ The change only *added* output — which is exactly why it
would have looked harmless to sync, and why it was not.

### 5.2 Test state at this branch's HEAD

- `cargo test -p kayfabe-qemu-raw --features host-isolates --all-targets --no-fail-fast` ⇒
  **141 passed, 0 failed.** ★ This is the configuration in which **every line this rung
  changed is compiled**; the whole diff sits behind `#[cfg(feature = "host-isolates")]`.
- `cargo test --workspace --all-targets --no-fail-fast` ⇒ **2920 passed, 6 failed.**
  ⊘ **In that configuration none of this rung's code is compiled** — its own four tests do not
  even appear in the run — so the 6 cannot be attributable to it. They are
  `a_device_with_no_fb_source_refuses_the_vidmem_ring`,
  `a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote`,
  `the_logic_crates_carry_no_unnamed_guest_os_assumption`, and three
  `doorbell_reaches_the_completion_observer` cases.
  ⚠ **w323's stated stable red set is 7 and this is 6.** I did **not** re-derive w323's exact
  invocation, so the difference is **unattributed** and is **not** evidence about this branch
  in either direction. Recorded rather than rounded to "matches".

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
