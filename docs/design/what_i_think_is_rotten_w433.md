# What I think is rotten, and what I would delete

**STATUS: LIVE, 2026-09-11.** Written because the owner asked for pushback and offered to
delete code. Opinion, grounded in what this session measured.

## 1. The rot is real, it has ONE shape, and it is countable

**Mechanism without consumers.** Built, wired, unit-tested, never reached. From one night:

| inert thing | how it was found |
|---|---|
| `measure_guest_ram_pin_rate` | the worker forced an arm whose `measures_pin_rate()` is false — `pins=0` all boot |
| `VAS_PINRATE_ROWS = 256` | a vCPU-era bound that outlived the thread it was written for |
| `holds_for_refresh` | **0** fires against **304** row-binding RPCs — keyed on an event that never happens |
| `out.invalidates` | decoded from the guest, stored, read by **no production code** — only 4 tests |
| `LANE_FULL_INVALIDATES`, `LANE_FULL_DOORBELLS`, +2 | incremented, never read, so "is the lane big enough" was unanswerable |
| `dbtable.rs` + `promotion.rs` | **648 lines, zero production referrers** |
| 3 of 5 red CI gates | firing on PROSE — a comment saying a file avoids `unsafe` |

⊘ **I wrote two of those.** `dbtable` and `promotion` are mine and unwired. This is not a
complaint about someone else's code.

## 2. The cause is structural, not carelessness

Every mechanism ships with unit tests that **construct it directly** and assert its behaviour
in isolation. Nothing asserts it is **reachable from a boot**. So CI goes green over inert
code, and the only instrument that has ever caught this is a boot-log census — which I added
three of tonight, ad hoc, each time after being misled.

★ The sharpest single symptom: **the production path that backs guest RAM is a 634-line
function called `measure_guest_ram_pin_rate`.** A measurement became the data plane, and its
name still says what it was for.

## 3. The biggest simplification available is ALSO the parity fix

The sweep/publication machinery exists to make **lazy discovery converge**, because we believed
the guest never announces its page-table changes. **It does** — the UVM `MEM_OP`
`MMU_TLB_INVALIDATE`, which we decode correctly and throw away.

And `[measured w315]` **91.5 % of an 86.7 ms doorbell trap is page-table + publication; the
real host forward is 4 ms.**

⇒ The machinery that exists to work around a message we are ignoring **is** the launch
overhead. Consuming the barrier is not a nicety on top of the current design; it removes the
reason the current design exists.

## 4. What I would delete

- **The pin-rate sampling path and its arms.** With an authoritative barrier you back what the
  barrier names, when it names it. No sample, no cap, no convergence, no race.
- **Most of `VasPublishArm`** and the watermark/rescan convergence machinery it serves.
- **`dbtable.rs` + `promotion.rs`** — wire them this week or delete them. 648 lines that no
  boot has ever executed are worse than absent: they read as capability.
- **A large share of the 46 environment arms.** Each was an experiment; the winners should
  become the behaviour and the losers should go. Two separate bugs this session were *"the arm
  was set to the wrong value"* and *"the default was false"* — that is the arm surface
  charging rent.

## 5. What I would NOT touch

- **The instrument discipline.** Refuse-by-name, the `⊘`/`★` markers, measured citations with
  dates and revisions. It is why four inert mechanisms surfaced in a single night, and it is
  the asset this repo has that most do not.
- **The ABI / chips layering** and the isolate boundary. Both are fine.
- **`shim.rs` at 19 598 lines** is a real problem and splitting it is **cosmetic** next to the
  above. I would not spend the week on it.

## 6. One process change, cheap, that makes the rot self-reporting

Every mechanism prints a *"did I fire"* counter into ONE census line, and a gate asserts the
expected set is non-zero on a real boot. That converts *"green CI over dead code"* — the
actual failure mode — into something a single grep answers.

## 7. ⊘ Where I withdraw my own earlier recommendation

**GPGA does not fix the LLM race, and I said it did.** I proposed it when I believed no trigger
existed. The LLM's operands are **guest RAM at CUDA-chosen VAs**; GPGA reserves guest **VRAM**.
Backing guest RAM eagerly at arbitrary guest-chosen GPU VAs is not what that design does.

GPGA may still be right for the framebuffer and for out-of-memory recoverability. It is **not**
the answer to the thing I invoked it for, and the w426 commit that concluded otherwise was
reasoning from *"there is no trigger"* — which the owner then corrected.
