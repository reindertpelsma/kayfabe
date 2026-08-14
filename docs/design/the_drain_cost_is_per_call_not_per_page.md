# The drain's 225 µs/row, decomposed — and why the fix is a COALESCER and not a batched socket

> **STATUS: LIVE — 2026-08-14 (w321).** Branch `w321-batch-the-drain`, base master `5f30594e`.
> Bench `vh`, real GA106, host driver open `580.159.04`.
> **Answers** `the_drain_budget_truncation.md` §5 option 2 (*"make the pins cheap enough that
> completeness is affordable"*) and **corrects its stated mechanism**, which is folded in above
> that section rather than left beside it.
> Per-boot artefacts: `traces/w321_batch/`. Pre-registration: `scripts/bench/w321_all.sh`'s
> header, written before the arms ran.

---

## 1. The finding, in one paragraph

The doorbell-time drain costs `rows × ~225 µs`, and that 225 µs is **three synchronous
cross-process round trips** — `VerbPlan::PinGuestRam` is `map_guest_ram` →
`describe_guest_ram` → `map_gpu_va` (`kayfabe-isolate/src/lib.rs:2890-2930`) and, on the
`host-isolates` arm, **each one is its own `Request` over the isolate socket**
(`ProxyRmBackend::call`, `kayfabe-isolate-host/src/isolate.rs:380`, `&mut self` — the API
itself forbids overlap). Bracketing both sides of that socket at once says the cost **splits**:
**~86 µs (39 %) is transport and ~132 µs (61 %) is the child's own RM ioctls.** ⇒ a fix that
batched only the transport would have removed 39 % — 3.0 s → 1.8 s — and **not cleared the
budget with margin**. The other 61 % comes off only by asking the host **fewer, larger**
questions. And it does come off, because the second half of the measurement is that **the RM
ioctl cost is per CALL, not per page**: at 3.3 rows per chain the child's per-call means are
`10 / 42 / 74 µs` against `13 / 42 / 77 µs` at 1 row per chain.

---

## 2. ★★★★★ THE DECOMPOSITION — two brackets, on the two sides of one socket

Neither bracket alone can answer *"socket or ioctl?"*: the parent's counter **includes** the
child's service time, and the child's counter **excludes** the transport. Subtract.

`[measured w321, vh, real GA106, boot `w321i1`, `KAYFABE_DRAIN_BATCH` unset ⇒ master's
behaviour]`

```
W321IPC[ipc_calls=39900 (3 /row) ipc_us=2904395 (218 us/row)
        drain_us=3000000 ours_us=95605 (7 us/row) ipc_share=96%]

W321CHILD worker=0 served=110000 —
        map_guest_ram[n=18244 mean=13us]
        describe_guest_ram[n=18244 mean=42us]
        map_gpu_va[n=18228 mean=77us]
```

| term | per row | share of 218 µs | what it is |
|---|---|---|---|
| `map_guest_ram` | 13 µs | 6 % | the child `mmap`s the guest-RAM `memfd` slice |
| `describe_guest_ram` | 42 µs | 19 % | `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over those pages |
| `map_gpu_va` | 77 µs | 35 % | `RmMapMemoryDma`, fixed, at the guest's own VA |
| **child total** | **132 µs** | **61 %** | **the RM ioctls** |
| **transport** | **86 µs** | **39 %** | 3 round trips × ~29 µs |
| our own core | 7 µs | 3 % | route locks, `resolve_guest_ram`, commit |

★ **`ipc_calls / rows = 3` exactly.** The count was read from the source before it was
measured, and the measurement returned the integer. That is the check that the two brackets
are describing the same thing.

### ⊘ THE UC/WB HYPOTHESIS IS NOT THE MECHANISM ON THIS PATH — checked, and cheap to check

A plausible candidate for a ~100 µs per-mapping kernel cost is `set_memory_uc`'s CLFLUSH plus
a global TLB-shootdown IPI. It is **not in this chain**: `alloc_os_descriptor` requests
`NVOS02_FLAGS_COHERENCY_CACHED | NVOS02_FLAGS_MAPPING_NO_MAP`
(`kayfabe-isolate-host/src/rm.rs:2412-2415`), so the descriptor is **cached** and RM builds
**no CPU mapping** for it at all; and the guest-RAM view the descriptor is taken over is a
plain `mmap` of a `memfd`, which is write-back by default and never rewritten.
⇒ The 42 µs and 77 µs are `get_user_pages` and PTE fill. ⚠ Both of those are *per-page* work
in principle — which is why §4's flatness measurement is the load-bearing one and not a
detail.

---

## 3. ★★★★★ THE CONTIGUITY CENSUS — and it is BOOT-VARIABLE, which nothing predicted

`drain_contiguity` classifies the rows the drain is about to walk. A coalesced pin needs
**both** halves contiguous: the VAs must abut (or a fixed mapping would cover addresses the
guest never bound) and the GPAs must abut (or one `OS_DESCRIPTOR` over one `mmap` slice cannot
describe them).

`[measured w321, vh, real GA106]`

**Nine boots, every one 13 313 rows of the same workload on the same box and the same binary.**
Full table in `traces/w321_batch/RESULTS.md` §3; the range is what matters here:

| | `va_runs` | `pair_runs` | ceiling | `break_va_only` |
|---|---|---|---|---|
| best (`w321c3`) | **3** | 161 | **82.68×** | **0** |
| worst (`w321e31`) | **3** | 6 644 | **2.00×** | **0** |

Three facts, and the third is the one nobody had:

1. **The table is all single pages** — `len_4k = 13 312` of 13 313. There is nothing to
   coalesce *inside* a row; the runs are between rows.
2. **In VA the whole 54.5 MiB is THREE spans, on every boot, and `break_va_only = 0`.**
   ⇒ **every break is physical scatter and not one is VA sparsity.** A table broken by VA
   would admit no coalescing at all and no batched verb either; this one is broken by the
   guest allocator's physical placement, which is exactly the axis a merge can fold.
3. ★★★★★ **The achievable ratio MOVED BY 41× ACROSS NINE BOOTS OF THE SAME BINARY ON THE SAME
   BOX.** Guest-RAM physical contiguity is a property of the **host allocator's state at that
   boot**, not of the workload. ⇒ **the coalescing win is a DISTRIBUTION, the margin has to be
   quoted at the worst observed contiguity, and the tail is not bounded by nine boots** — the
   worst observed got *worse* (3.31× → **2.00×**) the moment more boots were run.
   ★ And the bad case has a shape: `w321e31`'s 6 644 runs are **6 338 single pages** plus 306
   long ones, so its cost is almost entirely singleton chains — precisely what a merge *by
   physical adjacency* can never help, and precisely what §6's next rung deletes.

⊘ **`w238`'s constraint is confirmed in kind and refuted in magnitude for this population.**
*"The GR ring is not physically contiguous, so 'one descriptor per run' is one per PAGE"* is
true of a ring. Over the whole drained table the mean run is 2.0–82.7 pages, and the shape is
the buddy allocator's: on `w321i1`, 754 of the 1 179 runs are single pages while the other 425
carry 12 559 of the 13 313 rows. ⇒ **the COUNT is in short runs; the MASS is in long ones**,
and a merge is paid by the mass — except on a fragmented boot, where the singletons ARE the
mass and the merge stops paying.

---

## 4. The fix, and the one property that makes it work

`KAYFABE_DRAIN_BATCH=coalesce` merges consecutive candidate rows that abut in **both** `va` and
`gpa` into one chain, split at **2 MiB** — the C's own boundary, and this repo's own record of
its sysmem chunker (*"splitting only at 2 MiB boundaries"*). **Default off ⇒ byte-identical to
master**, so the same binary carries both arms and the only variable between them is one word
in the environment.

★★★ **THE PROPERTY IT RESTS ON, MEASURED RATHER THAN ASSUMED: the RM ioctl cost is PER CALL,
not per page.** Comparing the child's own per-call means at 1.00 vs 3.30 rows per chain:

| child verb | 1.00 rows/chain (`w321i1`) | 3.30 rows/chain (`w321s1`) |
|---|---|---|
| `map_guest_ram` | 13 µs | **10 µs** |
| `describe_guest_ram` | 42 µs | **42 µs** |
| `map_gpu_va` | 77 µs | **74 µs** |

⇒ a chain covering 3.3× the pages costs **the same or less**. Had the cost been per-page, the
merge would have moved the count and not the clock, and this table is what would have said so.
⚠ It is measured at 3.3× and at one 2 MiB cap; it is **not** established for arbitrarily long
runs, and the 2 MiB split is what keeps the extrapolation short.

### ⊘ A refused chunk falls back to its own rows

`StraddlesRuns` is a property of the **merge** — the chunk left the hypervisor's stated run —
and not of any row in it. Dropping such a chunk would lose up to 512 rows for a boundary this
file invented, which is strictly worse than the truncation w321 exists to remove. So a chunk
that the VMM will not resolve, or that the host refuses, is re-walked **row by row through
exactly master's loop**. ⇒ the worst case of the coalescing arm is master's cost for that
chunk plus one wasted chain, and **never a missing mapping**.

### ★★★ `pinned` STAYS IN ROWS

w319's grading invariant is `pinned == asked` with **both terms row counts**. A coalescing fix
that reported *chains* there would make its own success read as a 3–11× regression in the one
line every other lane grades on. The chain count is `W321BATCH`'s `chains=`, beside it.

---

## 5. ⚠⚠ THE ONE NUMBER THAT LOOKS LIKE A CATASTROPHE AND IS NOT: `host_rows`

`[measured w321, boots `w321i1` (off) and `w321s1` (coalesce)]`

| arm | `host_rows` | `already_host` | `already_pinned` | `MERGE-AGREES=false` |
|---|---|---|---|---|
| off | **18 295 of 18 309** | 18 295 | **0** | 0 |
| coalesce | **5 416 of 17 566** | 5 416 | **12 136** | 228 |

**`5 416 + 12 136 = 17 552` of 17 566 — the same coverage, recorded in the OTHER of the two
records.** `commit_pin_guest_ram`'s merge into `Binding::host` is bounded to a row whose extent
matches the grant **exactly** (`kayfabe-fwd/src/lib.rs:1930-1932`), and that bound is
deliberate: one host handle written into N rows would be freed N times by
`Spine::stage_dropped_vases` — a double free strictly worse than the leak the merge closes.
⇒ **every run pin reports `MERGE-AGREES=false` and lands in `Vas::guest_ram_pins` instead**,
which is precisely what `vas_published_ranges`' 2026-08-13 correction and `PublishCensus`'
`already_pinned` exist to say.

⚠ **A reader who greps `host_rows` and nothing else will read this fix as having unmapped
two-thirds of the address space.** That is the `a_second_source_of_truth_beside_a_complete_value`
class arriving in the one row a story turns on, and it is why the two numbers must be read
joined. ⊘ It does **not** trip criterion (E): `regression_check_e.sh` prints `host_rows` and
**explicitly does not grade it**, for a reason w304 paid for.

---

## 6. ★★★★★ THE COST MODEL, AND THE NEXT RUNG IT PRICES

Least squares over seven boots' parent-side per-chain IPC, rows-per-chain spanning 1.00 → 75.21:

> ### `drain_us  ≈  chains × 232 µs  +  rows × 3.35 µs`

Every fitted point within ~11 %, two exact, and — the only reason it is quoted as a model — the
**two boots measured afterwards were predicted without being fitted** (`w321e32` to 1 %,
`w321e31` to 17 %). Corroborated twice more without using the fit: the intercept **232 µs** is
what the 1-row boots measured directly (218 / 249 µs), and the slope's implied 13 313-page
floor of **44.6 ms** is `w321c3`'s actual residual (86 − 41 = **45 ms**).

⇒ **The axis is the CHAIN COUNT.** Neither raising the budget nor batching the transport
touches it: of the 232 µs, ~86 µs is the three round trips, so collapsing 3 → 1 is worth
**1.37×** — real, and not what closes this.

### ★★★★★ THE NEXT RUNG: `chains → va_runs = 3`, priced at ~45 ms and BOOT-INVARIANT

`alloc_os_descriptor` describes a **user-VA range** under
`NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS` — so **the physical scatter is not the real
constraint**. The constraint is contiguity in the *isolate's own mapping*, and that is **ours to
choose**: reserve one user-VA window per VA run and `mmap(MAP_FIXED)` each contiguous GPA run
into its slot, then issue **one** `OS_DESCRIPTOR` and **one** `map_gpu_va` per VA run.

`va_runs = 3` on **every boot measured**, so this is `3 × 232 µs + 44.6 ms ≈ 45 ms` — and it
does not move with host fragmentation. ⇒ it converts w321's **2.21×–34.9 %-of-budget margin
that depends on the host's free lists** into a fixed **~67× margin** that does not.
⊘ It is a real change to the guest-RAM grant's shape — `mode2_isolate_memory_boundary.md`'s
boundary has the VMM authorising **one run**, and this needs it to authorise a scatter list —
which is why it is a rung of its own and not a widening of this one.

---

## 7. Results

See `traces/w321_batch/RESULTS.md`.
