# w321 — the coalescer, measured. **Outcome (A).**

> Branch `w321-batch-the-drain`, base master `5f30594e`, binary `e8ed422c`.
> Bench `vh`, real GA106, host driver open `580.159.04`, guest stock `580.159.04`.
> Pre-registration: `scripts/bench/w321_all.sh`'s header (written before the arms ran).
> Mechanism and decomposition: `docs/design/the_drain_cost_is_per_call_not_per_page.md`.
> **Every boot below is the SAME BINARY.** The only difference between the green arm and the
> red control is one word in the environment: `KAYFABE_DRAIN_BATCH=coalesce`.

---

## 0. The grade, stated the way w319 pre-registered it — on the DRAIN'S STATE, not the outcome

⚠ **A faster drain makes the w319 truncation rarer whether or not it was fixed**, because the
defect is cost-driven. A green run is therefore not evidence. The property is
**`complete=true` AND `pinned == asked`**, both in ROWS.

| | boots | `complete=true` | `pinned == asked` | outcome |
|---|---|---|---|---|
| **coalesce, cup3** | 4 | **4 / 4** | **4 / 4** | `^CUP3_VAL=43` 4/4, `Xid=0` 4/4 |
| **coalesce, cup8** | 3 | **3 / 3** | **3 / 3** | **`^CUP8_BAD=0 ^CUP8_MAXERR=0` 3/3**, `Xid=0` 3/3 |
| **coalesce, R33 arm 1** | 3 | 3 / 3 (`asked=0`) | 3 / 3 | the arm-1 COPY line 3/3 · ⊘ **the drain is UNEXERCISED there** |
| **off (control), cup3** | 2 | **0 / 2** | **0 / 2** | one `43`, one **RED with the defect's exact fingerprint** |
| **coalesce + `ROW_LIMIT=11800`** | 3 | 0 / 3 (by construction) | 3 / 3 | **3 / 3 RED — the reproducer still fires** |

---

## 1. ★★★★★ THE HEADLINE — the drain went from STRADDLING its budget to 2.9–45 % of it

`VAS_DRAIN_WALL_BUDGET` is **3 000 ms**. Margin is quoted as a multiple of it, at the WORST
observed contiguity, never at the mean — see §3 for why that distinction is not pedantry.

| boot | arm | ceiling | chains | rows/chain | `DRAIN_MS` | **margin** | `complete` | `pinned/asked` | `last_pinned_va` | result | `Xid` |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `w321i1` | **off** | 11.29× | 13 300 | 1.00 | **3000 ⚠HIT** | **1.00×** | **false** | 13300/13313 | `0x2047f2000` | `43` | 0 |
| `w321o1` | **off** | 10.39× | 11 668 | 1.00 | **3000 ⚠HIT** | **1.00×** | **false** | 11668/13313 | `0x203193000` | **RED** | **31 · CE3/CE1 · `0x2_0440f000` · FAULT_PDE** |
| `w321s1` | coalesce | 3.31× | 4 024 | 3.30 | **887** | **3.38×** | true | 13313/13313 | `0x2047ff000` | `43` | 0 |
| `w321c1` | coalesce | 4.07× | 3 271 | 4.07 | **892** | **3.36×** | true | 13313/13313 | `0x2047ff000` | `43` | 0 |
| `w321c2` | coalesce | 28.87× | 470 | 28.32 | **154** | **19.5×** | true | 13313/13313 | `0x2047ff000` | `43` | 0 |
| `w321c3` | coalesce | 82.68× | 177 | 75.21 | **86** | **34.9×** | true | 13313/13313 | `0x2047ff000` | `43` | 0 |
| `w321e21` | coalesce | 18.51× | 728 | 18.28 | **237** | **12.7×** | true | 13313/13313 | — | **`CUP8_BAD=0 CUP8_MAXERR=0`** | 0 |
| `w321e31` | coalesce | **2.00×** | **6 644** | 2.00 | **1360** | **2.21×** ⚠ | true | 13313/13313 | — | **`CUP8_BAD=0 CUP8_MAXERR=0`** | 0 |
| `w321e32` | coalesce | 22.83× | 591 | 22.52 | **180** | **16.7×** | true | 13313/13313 | — | **`CUP8_BAD=0 CUP8_MAXERR=0`** | 0 |

★★★ **`w321o1` IS THE CONTROL AND IT WENT RED, IN THE SAME SWEEP, ON THE SAME BINARY.**
`pinned=11668/13313`, `last_pinned_va=0x203193000` — **strictly below** the completion-semaphore
page `0x2_0440f000` — budget hit, and the host driver reported
`Xid 31 … FAULT_PDE ACCESS_TYPE_VIRT_WRITE @ 0x2_0440f000`. `w319_attribute.sh w321o1` returns
**`VERDICT=1 PRE-EXISTING`**. ⇒ the defect is alive on the off arm and absent on the on arm,
and nothing but the environment separates them.

⊘ **`w321i1` is the `w314br4` shape and must not be miscounted**: it hit the budget, came back
`43`, and stopped at `0x2047f2000` — **past** the fault page, 13 rows short of the top. Budget-hit
is not the failure; budget-hit *below the page* is.

---

## 2. ★★★★★ THE REPRODUCER STILL FIRES — AND ITS FINGERPRINT IS BYTE-IDENTICAL TO w319'S

`KAYFABE_DRAIN_BATCH=coalesce KAYFABE_VAS_DRAIN_ROW_LIMIT=11800`, n=3:

| boot | `DRAIN_MS` | chains | rows/chain | `pinned/asked` | `last_pinned_va` | host `Xid` |
|---|---|---|---|---|---|---|
| `w321x1` | **52** | 78 | 151.28 | 11800/11800 | **`0x203217000`** | 31 · **GRAPHICS/FE** · `0x2_0440f000` · FAULT_PDE · VIRT_WRITE |
| `w321x2` | **64** | 119 | 99.15 | 11800/11800 | **`0x203217000`** | 31 · **CE3/CE1** · `0x2_0440f000` · FAULT_PDE · VIRT_WRITE |
| `w321x3` | **50** | 67 | 176.11 | 11800/11800 | **`0x203217000`** | 31 · **GRAPHICS/FE** · `0x2_0440f000` · FAULT_PDE · VIRT_WRITE |

**3 / 3 RED. `last_pinned_va = 0x203217000` on all three — the SAME value w319 recorded on its
own three boots** (`traces/w319_intermittent/`), at the SAME limit, **without lowering it**.

★★★ **Why the reproducer survives the fix, and it is by construction rather than by luck:**
`vas_drain_row_limit()` caps `vas_guest_ram_rows`, so the truncation happens **on the ROW list,
before the coalescer is handed a single row.** ⇒ the instrument bounds *coverage*; the fix
changes *cost*. A fix that had broken the reproducer would have broken the instrument.

★★ **And the separation is now demonstrable rather than argued:** the same truncation that took
**2 508–2 775 ms** to produce in w319 now takes **50–64 ms** — a 45× speedup with the defect
**unchanged in every observable**. That is as direct a statement as this bench can make that
the fault is about *which rows*, not about *how long*.

★ **The engine varies and the page does not** (GRAPHICS/FE ×2, CE3/CE1 ×1), independently
reconfirming w319 §3b: *the engine is incidental and the page is the invariant*.

---

## 3. ★★★★★ THE CONTIGUITY IS BOOT-VARIABLE — **2.00× to 82.68×**, and nothing predicted it

| boot | order | `va_runs` | `pair_runs` | ceiling | `break_va_only` | `break_gpa_only` | `max_run` |
|---|---|---|---|---|---|---|---|
| `w321i1` | 1 | 3 | 1 179 | 11.29× | **0** | 1 176 | 16.8 MiB |
| `w321s1` | 2 | 3 | 4 022 | 3.31× | **0** | 4 019 | 3.6 MiB |
| `w321c1` | 3 | 3 | 3 266 | 4.07× | **0** | 3 263 | 12.1 MiB |
| `w321c2` | 4 | 3 | 461 | 28.87× | **0** | 458 | 19.1 MiB |
| `w321c3` | 5 | 3 | 161 | **82.68×** | **0** | 158 | 33.9 MiB |
| `w321e21` | 8 | 3 | 719 | 18.51× | **0** | 716 | 19.5 MiB |
| `w321o1` | 13 | 3 | 1 281 | 10.39× | **0** | 1 278 | 12.9 MiB |
| `w321e31` | 15 | 3 | **6 644** | **2.00×** ⚠ | **0** | 6 641 | 1.5 MiB |
| `w321e32` | 16 | 3 | 583 | 22.83× | **0** | 580 | 17.3 MiB |

Three facts:

1. **`va_runs = 3` on every single boot, and `break_va_only = 0` on every single boot.** The
   whole 54.5 MiB is three contiguous VA spans and **not one break is VA sparsity**. Every
   break — 1 176 to 4 019 of them — is *physical scatter*.
2. **`len_4k = 13 312` of 13 313 on every boot.** The table is single pages; the runs are
   between rows, never inside one.
3. ★★★★★ **The achievable ratio moved by 41× across nine boots of the same binary, same box,
   same workload.** Guest-RAM physical contiguity is a property of the **host's allocator state
   at that boot**, not of the workload — early boots trend upward (3.31 → 4.07 → 28.87 → 82.68),
   which is what free-memory compaction looks like from inside, and then `w321e31` fell back to
   **2.00×** after a long sweep had churned the host's free lists.
   ⇒ **the win is a DISTRIBUTION and the margin must be quoted at the WORST observed value.**
   That is `w321e31`: **2.00× ceiling, 6 644 chains, `DRAIN_MS = 1360`, margin 2.21×.**
   ⚠⚠ **AND THE TAIL IS NOT BOUNDED BY NINE BOOTS.** The worst observed moved from 3.31× to
   2.00× the moment more boots were run; nothing here excludes a 1.3× boot, which the model in
   §5 prices at ~2.4 s and 1.25× margin. **The coalescer removes the STRADDLE — master sits at
   1.00× on every boot — but it does not make the margin boot-invariant.** §5 says what does.

★★★★★ **AND THE TAIL HAS A SHAPE, WHICH IS THE WHOLE OF §5's ARGUMENT.** `w321e31` is not
uniformly scattered: `runsz_4k = 6 338` of its 6 644 runs are **single pages**, and the other
306 runs carry the remaining 6 975 rows at ~22.8 pages each. ⇒ **6 338 singleton chains ×
232 µs = 1 471 ms — essentially the entire 1 360 ms drain.** The bad case is *singletons*, and
a fix that merges pages **regardless of physical adjacency** deletes exactly that term.

⊘ **`w238`'s constraint is confirmed in kind and refuted in magnitude for this population.**
*"The GR ring is not physically contiguous, so 'one descriptor per run' is one per PAGE"* is
true of a ring. Over the drained table the mean run is 3.3–82.7 pages, buddy-allocator shaped:
on `w321i1`, **754 of the 1 179 runs are single pages** while the other 425 carry **12 559 of
the 13 313 rows**. ⇒ the COUNT is in short runs; the MASS is in long ones, and a merge is paid
by the mass.

---

## 4. The three workloads

- **cup3** (`^CUP3_VAL=43`, first compute) — **4 / 4** on the coalesce arm
  (`w321s1`, `w321c1..3`), `Xid=0` on all four.
- **cup8** (`^CUP8_BAD=0 ^CUP8_MAXERR=0`, bit-exact 2048² — the only oracle that fails
  *quietly-wrong* rather than loudly-absent) — **3 / 3** (`w321e21`, `w321e31`, `w321e32`),
  `complete=true` and `13313/13313` on all three, `Xid=0` on all three. ★ And `w321e31` is the
  **worst-contiguity boot in the whole rung** (2.00×, 6 644 chains, 1 360 ms) — so the bit-exact
  oracle passed *at* the tail, not only in the comfortable middle. See §4b for the two harness
  failures this arm cost.
- **R33 arm 1** (raw CE client, no libcuda) — **3 / 3** the verbatim COPY line,
  *"4096 bytes moved … GP_GET 1 caught GP_PUT 1 — read back through an INDEPENDENT mapping"*.
  ⊘⊘ **AND THE DRAIN IS UNEXERCISED THERE.** Every doorbell on that workload reports
  `asked=0 pinned=0 chunks=0`. ⇒ arm R is a **no-regression check, NOT a test of the fix.**
  Saying otherwise would be the single-workload-grade error `relaxation_inert_gate.sh` exists
  to prevent, arriving from the other side: the second workload cannot reach the changed code.
  ⚠ Its `Xid 31 CE0 @ 0x7_00100000 FAULT_PTE` is `w309_crit1.sh fresh`'s **deliberately
  provoked** fault, printed and not judged, exactly as `w317_r33_repeat.sh` documents.

### 4b. ⚠⚠ TWO HARNESS FAILURES ON THE cup8 ARM, BOTH CAUGHT BY THE `⊘UNMEASURED` DISCIPLINE

1. **`w308_cup8.sh` requires its arm** (`baseline|cup8`) and `exit 64`s without one. The first
   spelling of the launcher line omitted it ⇒ **3 boots in 13 seconds**, every field
   `⊘ABSENT-UNMEASURED`. ★ Had the arm script printed `0` for an absent `CUP8_BAD`, the run
   would have read as **3/3 bit-exact from zero boots**. Fixed in `w321_all.sh`.
2. ★★★ **`build_qom_shim.sh` REFUSES AN ARCHIVE MORE THAN 30 MINUTES OLD** when cargo has
   nothing to rebuild — *"cargo did not rebuild it and this script will not install an archive
   it did not just produce"*, `BUILD RC=1` ⇒ `W290P EXIT rc=92`. ⇒ **a sweep longer than 30
   minutes on an unchanged tree fails its later boots**, and the failure is a *build* refusal
   that looks nothing like a workload problem. Worked around by `touch`ing a crate source.
   ⚠ Worth knowing before any multi-arm night run: it is not a flake and it is not the tree.

---

## 5. ★★★★★ THE COST MODEL, AND WHAT IT SAYS THE NEXT RUNG IS WORTH

Least squares over the seven boots' parent-side per-chain IPC (`W321IPC`, bracketed to the
drain), against rows-per-chain spanning **1.00 → 75.21**:

> ### `drain_us  ≈  chains × 232 µs  +  rows × 3.35 µs`

| boot | chains | predicted | **measured `DRAIN_MS`** | in the fit? |
|---|---|---|---|---|
| `w321e31` | 6 644 | 1 585 | **1 360** | ⊘ **no — HELD OUT** |
| `w321s1` | 4 024 | 978 | **887** | yes |
| `w321c1` | 3 271 | 804 | **892** | yes |
| `w321e21` | 728 | 213 | **237** | yes |
| `w321e32` | 591 | 182 | **180** | ⊘ **no — HELD OUT** |
| `w321c2` | 470 | 154 | **154** | yes |
| `w321c3` | 177 | 86 | **86** | yes |

★ **Two parameters, seven boots in the fit, 75× in chains, every prediction within ~11 % and
two exact — and the two boots measured AFTERWARDS were predicted without being fitted**
(`w321e32` to **1 %**, `w321e31` to 17 %). ⊘ That out-of-sample check is the only reason this
is quoted as a model rather than as a curve through its own points.
⚠ **And a fit is not a mechanism** — this campaign has been burned three times by a magnitude
that matched. Two independent corroborations, neither used to determine the parameters:
the intercept **232 µs** is what the 1-row boots measured directly (**218 and 249 µs**), and
the slope's implied floor for 13 313 pages, **44.6 ms**, is what `w321c3`'s residual actually
is (86 ms measured − 41 ms of chain cost = **45 ms**).

**What it means:**

- **Per-chain fixed cost dominates.** 232 µs is paid once per host chain regardless of how many
  pages it covers; a page costs 3.35 µs at the margin. ⇒ *the axis that matters is the CHAIN
  COUNT*, and neither transport batching nor budget raising touches it.
- ⊘ **A transport-only batch is worth 1.37×, not the fix — and it is NOT what the brief and my
  own redirect predicted.** Of the 232 µs, ~86 µs is the three round trips and ~132–146 µs is
  the child's RM ioctls. Collapsing 3 → 1 round trip removes ~58 µs of 232 ⇒ at the worst
  observed boot **1 360 ms → ~1 000 ms**, margin 2.21× → 3.0×. Real, cheap-ish, and **it does
  not remove the boot-to-boot variance**, which is why it was measured and not built.
- ★★★★★ **THE NEXT RUNG IS `chains → va_runs = 3`, AND THE MODEL PRICES IT AT ~45 ms.**
  `alloc_os_descriptor` describes a **user-VA range** with `PHYSICALITY_NONCONTIGUOUS`, so the
  physical scatter is not the real constraint — the constraint is contiguity in the *isolate's
  own mapping*, and that is ours to choose. Reserve one user-VA window per VA run and
  `mmap(MAP_FIXED)` each contiguous GPA run into its slot; then **one** `OS_DESCRIPTOR` and
  **one** `map_gpu_va` per VA run. 3 chains × 232 µs + 44.6 ms ≈ **45 ms, and boot-invariant** —
  it deletes §3's 25× variance rather than living with it.
  ⊘ It is a genuine change to the guest-RAM grant's shape (`mode2_isolate_memory_boundary.md`'s
  boundary: the VMM authorises one run today), which is why it is a rung and not this rung.
  ★★★ **And §3 now says exactly why it is worth taking:** the worst boot's cost is 6 338
  SINGLETON chains, which physical adjacency can never merge and this can. It converts a
  2.21×–34.9× margin that depends on host memory fragmentation into a **fixed ~45 ms, 67×
  margin** that does not.

---

## 6. ★★★★★ A CORRECTION w321 OWES w319 — "the disposal consumes 73 % of the 4 s budget" IS THE DRAIN

`the_drain_budget_truncation.md` §5 argues against raising the budget partly on this:

> *"`[measured w314]` the surrounding **disposal** already consumes **2.65–2.92 s** of a 4 s
> `scrubberDestruct` budget (73 %)."*

⊘ **The disposal does not consume that, and never did.** Measured on these boots, in every arm,
the disposal's own emission is `DRAIN-TIMING max_drain_us = 40 402 … 49 246` against a budget of
**`40000 us (1% of scrubberDestruct's 4000000 us)`** — i.e. **~1.0–1.2 %**, by that budget's own
construction (w317).

★★★ **The 2.65–2.92 s is the PUBLICATION DRAIN'S OWN `DRAIN_MS`**, and w319's own §2 table prints
the two numbers it was rounded from: `w314br1 DRAIN_MS = 2672` and `w314bt1 DRAIN_MS = 2898`.
⇒ 2.672 s and 2.898 s ⇒ *"2.65–2.92 s"*. The figure was **the thing being fixed, attributed to
the thing beside it.**

⇒ **Consequence, and it is favourable in a way the argument did not allow for:** clause (b)'s
headroom was not 73 % consumed by an unrelated pass — it was 73 % consumed by **this drain**, and
w321 takes that to **2.2 % – 22 %** (86–892 ms of 4 000 ms). ⚠ Same class this tree keeps paying
for: *a correct-sounding sentence in a live doc is the last place a reader looks for a wrong
attribution*, and it survived because both numbers are real and adjacent.

---

## 7. ⚠⚠ `host_rows` DROPS BY TWO-THIRDS AND NOTHING IS UNMAPPED

| arm | `host_rows` | `already_host` | `already_pinned` | `MERGE-AGREES=false` |
|---|---|---|---|---|
| off (`w321i1`) | **18 295 of 18 309** | 18 295 | **0** | 0 |
| coalesce (`w321s1`) | **5 416 of 17 566** | 5 416 | **12 136** | 228 |

`5 416 + 12 136 = 17 552` of 17 566 — **the same coverage, in the other of the two records.**
`commit_pin_guest_ram`'s merge into `Binding::host` is bounded to a row whose extent matches the
grant **exactly** (`kayfabe-fwd/src/lib.rs:1930-1932`), because one host handle written into N
rows would be freed N times by `Spine::stage_dropped_vases` — a double free strictly worse than
the leak the merge closes. ⇒ run pins land in `Vas::guest_ram_pins` and report
`MERGE-AGREES=false`, which w296 already records as **the designed outcome of a legitimate run
pin**, not a defect.

⚠ **A reader who greps `host_rows` and nothing else will read this fix as having unmapped
two-thirds of the address space.** ⊘ It does not trip criterion (E): `regression_check_e.sh`
prints `host_rows` and **explicitly does not grade it**, for a reason w304 paid for.

---

## 8. Attribution of every boot (`w319_attribute.sh`, `--selftest` PASS 6/6 + matcher first)

| boot | verdict |
|---|---|
| `w321o1` | **1 PRE-EXISTING** — truncated, fault above `last_pinned_va` |
| `w321x1`, `x2`, `x3` | **1 PRE-EXISTING** ×3 — the reproducer, as pre-registered |
| `w321c1`, `c2`, `c3`, `s1` | **0 GREEN** — `complete=true`, `pinned == asked`, no Xid |
| `w321i1` | **0 GREEN** — `complete=false` 13300/13313 but past the page (the `br4` shape) |
| `w321r1` | **0 NOT-THIS-DEFECT** — drain complete; the fault is `w309_crit1 fresh`'s own |

---

## 9. The unit-test baseline — **identical, and +6**

`cargo test --workspace --no-fail-fast --features kayfabe-qemu-raw/host-isolates`, both revisions
in the same tree and the same target dir, back to back:

| | passing | failing targets | failing tests |
|---|---|---|---|
| **base `5f30594e`** | **2 883** | 3 | 6 |
| **branch `e8ed422c`** | **2 889** | 3 | 6 |

The failing set is **byte-identical** — `doorbell_reaches_the_completion_observer` (3),
`ring_out_of_our_own_framebuffer` (2), `guest_os_axis_gate` (1) — i.e. exactly the pre-existing
red the brief named. **+6 is `shim::w321_coalesce_tests`**, all passing, absent from base.
⇒ **zero test regression.**

⚠ **A harness note that cost a cycle:** `cargo test … | tail -200` produced **nothing at all**
and hung — `tail` buffers to EOF, and something in the suite left a child holding the pipe's
write end after cargo exited, so EOF never came and the job read as *still running* with a
zero-byte result. Redirect to a file; do not pipe the suite.

---

## 10. What is NOT established

- ⊘ **The coalescer is UNEXERCISED on R33 arm 1** (`asked=0` every doorbell). Coverage of the
  changed code rests on cup3 and cup8.
- ⊘⊘ **The contiguity tail is unbounded by nine boots, and this is the rung's weakest point.**
  Worst observed **2.00× ⇒ 1 360 ms ⇒ 2.21× margin** — and the worst observed got *worse* as
  soon as more boots were run. A 1.3× boot is not excluded, only unobserved, and the §5 model
  prices it at ~2.4 s (1.25× margin). ⇒ **the coalescer removes the straddle; it does not make
  the margin boot-invariant.**
- ⊘ **Per-page cost is measured to 75 rows/chain and one 2 MiB cap.** Longer runs are not
  established, which is what the 2 MiB split is for.
- ⊘ **No multi-process or long-run measurement.** Every boot here is one guest process reaching
  `cuCtxCreate` once.
- ⊘ **`ipc_share` is 96–99 % on every arm**, so our own core cost was never the question and
  this rung says nothing about improving it.
