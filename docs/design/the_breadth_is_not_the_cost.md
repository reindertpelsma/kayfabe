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

<!-- W328-RESULTS-2 -->

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
