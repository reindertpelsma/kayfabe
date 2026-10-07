> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# The partial rebuild — what is KEPT, what is REBUILT, and the evidence for the line

**STATUS: PARKED, 2026-09-17 (w755j). Owner: *"rebuild if chosen is partly then"* … *"but maybe
fix + delete gets it already done later. if so I am also happy"* … *"for later"*.**

⇒ **The ruling is: FIX + DELETE first, and revisit this only if that stalls.** The partition
below stays as the data a later decision starts from, so it is not re-derived from feel. ⊘ Do
not read `PARKED` as `REJECTED`: the deletion list under *"What the rebuild gets to DELETE"* is
**live work** under §7 of `SINGLE_STORE_PLAN.md`, licensed by a green suite. The only thing
parked is the *rebuild* of the four rotted crates.

★ And the one item that is live **regardless of which path is taken** is the rule in the defect-
class section: *every instrument must name which gate row it moves, and a boot report must lead
with the gate.* It found three defects within an hour of existing and costs nothing.

## The measurement that sets the line

`274k` lines under `crates/*/src` — of which **122k (45%) are comments**, so ~`141k` of code.
Where the defects actually were, counted as files touched by 2026-09-16/17's fix commits:

| crate | file-touches | src lines | verdict |
|---|---|---|---|
| `kayfabe-isolate-host` | **74** | 44k | REBUILD (carry route K, the RM verb layer) |
| `kayfabe-qemu-raw` | **34** | 35k | REBUILD |
| `kayfabe-device` | **28** | 27k | REBUILD |
| `kayfabe-mmu` | **10** | 10.7k | REBUILD (the relocation goes) |
| `kayfabe-abi` | 14 | 44k | **KEEP** — every touch was churn I introduced today, not rot |
| `kayfabe-linux-raw` | 9 | 16k | **KEEP** |
| `kayfabe-chips` / `-arch` | 5 / 0 | 7.5k | **KEEP** |
| `kayfabe-util` | 0 | 5.4k | **KEEP** — the lock-rank witness is what MEASURED rank 0 today |
| `kayfabe-gsp` / `-rmrpc` | 0 / 0 | 12.3k | **KEEP** |
| `kayfabe-core` | 0 | 15k | **KEEP** — `channel_kind()` is a total function of one field |
| `kayfabe-fwd` / `-rt` / `-isolate` | 4 / 4 / 4 | 29.6k | KEEP WITH AUDIT — the fault vocabulary is good, some NAMES are false |

⇒ **keep ≈ 110k, rebuild ≈ 117k (≈60k of actual code once comments are removed).**

★ The keep column is **derived from hardware**: ABI transcriptions pinned at compile time
against rustc layout and cross-checked with ogkm, per-die tables, the syscall layer. Re-deriving
it is exactly how this campaign lost weeks before, and nothing in it failed today.

## ⊘⊘⊘ THE DEFECT CLASS — and why volume is the wrong disease to treat

Every failure found on 2026-09-16/17 is **one shape**: *a mechanism built correctly and never
connected to the thing it was for.*

| defect | the mechanism | what it was never connected to |
|---|---|---|
| `placed` ledger was write-only | the ledger | the mapper |
| `StoreMapPort::unmap` | a complete unmap verb | any caller at all |
| `placement_census` | a census | any output |
| `ChannelKindCensus` | a verdict | any report |
| the client's five ledger rows | the gate itself | every boot report for the whole campaign |

⚠ **A rebuild does not fix this.** The same hands would rebuild the same habit. What fixes it is
a rule the current constraint doc does not have, and which found three of the five above within
an hour of existing:

> **Every instrument must name which gate row it moves, and a boot report must LEAD with the
> gate.** An instrument that cannot name its row is not built.

⇒ that rule belongs in the new constraint doc whether or not anything is rebuilt.

## What the rebuild gets to DELETE, earned by measurement rather than taste

- **fake fb / two worlds / the arena FB path** — superseded by the single store (§w721).
- **`relocate_entry` + `walkshadow::build_image`'s rewriting** — the live window replaces it,
  and removes the foreclosure that made the second walker unable to disagree in the one
  direction it exists for.
- **`CpuCeFb` and the CPU-memcpy FB plane** — emulated does not mean CPU. `[measured]` the
  control arm's `(P)` is partly bought by `FbPage::Heap => copy_from_slice`, which is
  residence-wrong and unshippable.
- **possibly the whole raw CE channel**, if CUDA streams serve the emulated CE/scrub planes
  (owner, 2026-09-17). Pending the `cuda-store-probe` answer.

## The architecture the last two days actually earned

1. **One store**, CUDA-owned if `--cuda-store-probe` says RM can name a CUDA allocation —
   then it is addressable by the walk kernel live, by `cuMemcpyAsync`, and by
   `map_store_slice` into guest VA spaces, from ONE object.
2. **The scratchpad's own two channels share the scratchpad's VAS** (owner). Guest VA spaces
   are a separate matter and always were — that is what `map_store_slice` is for.
3. **Route K** for channel birth: the isolate mints the client, the scratchpad maps.
   `[measured w755k]` `adopt_refused 4700 → 0`.
4. **Emulated work executes on the GPU**, async, completion written only on real completion.
5. **Every instrument names its gate row.**

## ⚠ The sequencing argument, stated so it can be overruled

We have, for the first time, a **measured causal chain to the gate**:
`ADOPT-WHY(6) → no birth → doorbell refused → copy never runs → NEVER RETIRED, no Xid`.
One boot tests it. ⇒ finish that first: a rebuild started now discards the first actionable
chain the campaign has produced, and the new architecture doc would be written from hope rather
than from a proven path.

⊘ **What would flip this:** if fixing conjunct 6 does not move the client, the model is wrong
somewhere deeper than the code, and rebuilding on a corrected model becomes the better buy.
That is one boot from being decided rather than argued.
