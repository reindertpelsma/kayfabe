# `traces/w317_drain/` — the bench measurement of the budgeted BQL disposal

> **STATUS: LIVE — 2026-08-14 (w317).** Branch `w317-budgeted-drain`, rev `8339f739`, on master
> `73dc2246`. Bench **`vh`**, real GA106, host driver open `580.159.04`, guest kernel 6.8.0-117,
> stock guest driver. Stamp gate PASSED on every boot
> (`STAMP=8339f739… HEAD=8339f739…`).
> Design: `docs/design/budgeted_bql_disposal.md`.
>
> **Verdict: pre-registered outcome (A) — with one named finding that belongs to (B).**

---

## 1. ★★★★★ THE HEADLINE — the worst BQL hold falls from 92.6 % of budget to 3.2 %

`[measured 2026-08-14, vh, n=4 boots, tags `w317br1..4`; the w314 rows are quoted from
`traces/w314_confirm/`, same box, same instrument, same workload]`

| arm | rev | worst single BQL hold, µs | vs `scrubberDestruct`'s **4 000 000 µs** |
|---|---|---|---|
| clean master (w314) | `eb3d99ad` | **2 918 210** | 73.0 % |
| + pin release (w314) | `28882ec2` | **3 702 806** | **92.6 %** |
| **+ budgeted drain (this rung)** | `8339f739` | **127 330** | **3.2 %** |

⇒ **29× on the worst observed hold**, and the guest is running between the pieces.

### The two distributions, n=4 each, min / median / max

| instrument | boot 1 | boot 2 | boot 3 | boot 4 | min | median | max |
|---|---|---|---|---|---|---|---|
| `max_reap_us` — **w314 branch** | 3 336 519 | 3 702 806 | 3 263 826 | 3 250 535 | 3 250 535 | 3 300 173 | 3 702 806 |
| `max_reap_us` — **w314 clean master** | 2 648 366 | 2 918 210 | 2 666 893 | 2 772 771 | 2 648 366 | 2 719 832 | 2 918 210 |
| **`max_reap_us` — w317** | **69 295** | **55 586** | **52 948** | **52 035** | **52 035** | **54 267** | **69 295** |
| **`max_drain_us` — w317** (new) | 91 833 | 127 330 | 91 470 | 92 566 | 91 470 | 92 200 | 127 330 |

★ **The ranges do not overlap, and not narrowly**: w317's largest `max_reap_us` (69 295) is
**38× below** clean master's smallest (2 648 366). This is the claim; a single pair of numbers
would not have been.

⚠ **Read the two w317 rows together, not separately.** The disposal *moved*: `max_reap_us` no
longer contains it. What is left in the reap is the isolate child's `waitpid` + namespace
teardown — a floor the budget does not and cannot touch, and worth naming as *the thing that is
now the reap*. The honest post-fix figure is `max(max_reap_us, max_drain_us)`, and the two
occur on **different traps**, so they do not add.

---

## 2. Correctness — both workloads, and the termination argument, on hardware

| # | criterion | verdict | the number |
|---|---|---|---|
| **4** | `^CUP3_VAL=43` at n ≥ 4 | ✔ **PASS 4/4** | `CUP3_VAL=43  CUP3_RC=0` on `w317br1..4`, `Xid=0` on all four |
| **5** | `R33 arm 1` (raw CE, no libcuda) fires | see §4 | |
| **6** | `regression_check_e.sh` | ✔ **PASS 4/4** | `(E) VERDICT = 0`; E1 `Xid = 0` · E2 `★DRAINED rows=229 Σpinned=18228` · E3 all-zero |
| **7** | `PIN-RELEASE released=N>0`, `refused_no_host_vas=0`, `REAP` present | ✔ **PASS 4/4** | `released=18228 refused_no_host_vas=0 rows_deduped=18228`, identical on all four; `REAP` ×2 |
| **8** | guest `dmesg` non-empty, has `NVRM`, no stall | ✔ **PASS 4/4** | 5 379 bytes, `NVRM` ×31, stall lines **0** — ⚠ and see §5 for why that green means little |
| **3** | `DRAIN-DEFER` returns to **0** | ✔ **PASS 4/4** | `deferred_for_drain=1 still_retired=1` → `deferred_for_drain=0 still_retired=0`, both lines on every boot |
| **2** | `max_drain_us` ≤ budget + one chunk | ✔ PASS, ⚠ **and the chunk dominates** — §3 | 91 470 … 127 330 |

★★★ **Criterion 3 is the one that is not a repeat of w314.** §5.1 of the design argues that a
retired proc's queue is *closed and strictly decreasing*, so the defer cannot be permanent.
The `1 → 0` transition, on all four boots, is that argument observed rather than asserted —
and `still_retired` reaching 0 with `reaped=1` says the proc really did complete its teardown,
not merely stop being counted.

⊘ `refused_no_host_vas=0` and `released=18228` being **byte-identical to w314's** is the
non-regression: the budget changed *when* the release runs and nothing about *what* it releases.

---

## 3. ⊘⊘ THE FINDING THAT BELONGS TO OUTCOME (B) — **the overshoot is the CHUNK, not the budget**

The budget is 40 000 µs. Three of four boots measured `max_drain_us ≈ 91 500–92 600` with
**`turns=1 disposed=64`** — i.e. **a single chunk of 64 disposals, alone, took ~92 ms.**

```
w317br1  DRAIN-TIMING max_drain_us=91833  disposed=64  residue=0 turns=1 budget_hit=true
w317br2  DRAIN-TIMING max_drain_us=127330 disposed=128 residue=0 turns=2 budget_hit=true
w317br3  DRAIN-TIMING max_drain_us=91470  disposed=64  residue=0 turns=1 budget_hit=true
w317br4  DRAIN-TIMING max_drain_us=92566  disposed=64  residue=0 turns=1 budget_hit=true
```

⇒ **the deadline never gets a chance to bind on those traps**: the first turn already exceeds
it. The delivered bound is `40 ms + one chunk`, exactly as `RETIRED_DRAIN_CHUNK`'s docs state —
but in practice it is **the chunk alone**.

### ★ The per-disposal cost is bimodal, and the estimate was low AGAIN

Earlier, non-maximal turns on the same boot:

```
w317br1  max_drain_us=46475 disposed=384 turns=6  ⇒ 121 µs / disposal
w317br1  max_drain_us=74278 disposed=512 turns=8  ⇒ 145 µs / disposal
w317br1  max_drain_us=91833 disposed=64  turns=1  ⇒ 1 435 µs / disposal
```

⇒ a typical disposal is **~120–145 µs**, and there is a class costing **~1.4 ms** — repeatably,
at ~1 429–1 435 µs on two different boots. ⊘ **This is the third time this quantity has been
estimated and the third time the estimate was low**: w310 §5 said 1–2 µs (measured 35 µs, ×20);
this rung's constant docs said ~70 µs (measured 120–145 µs typical, ×2; and 1.4 ms tail, ×20).
★ The *design* survived it because the budget is a **time**, not a count — the only thing the
bad estimate cost was the granularity, and even there the constant's own docs pre-computed
*"even if the per-disposal cost is wrong by the same 20× factor, one chunk is ~90 ms — still
2 % of the 4 s bound"*, which is what happened, to the millisecond.

### ⊘ What is NOT known, and why the chunk was NOT retuned here

Whether the 92 ms turn is **64 uniformly-slow disposals** or **one very slow disposal among 63
fast ones** is unmeasured, and the two have opposite fixes: the first is cured by a smaller
chunk (16 would give ~23 ms), the second is not curable by any chunk — one disposal is
indivisible. `disposed=64` counts disposals, not their individual costs.

⇒ **Retuning on this data would be fitting to a number whose shape is unknown.** The next
instrument is a **per-disposal cost histogram** inside `Worker::execute`'s `Release` arm (or
simply the max single-disposal time), which distinguishes them in one boot. Named, not guessed.

⚠ Even so: **92 ms is 2.3 % of the 4 000 ms bound.** The exposure this rung was written against
is closed either way; this is a refinement, not a residual failure.

---

## 4. ★★★★★ THE SECOND WORKLOAD — `R33 arm 1` FIRED 3/3

⊘ **`43` alone is not sufficient evidence** — `scripts/bench/relaxation_inert_gate.sh` exists on
master because a single-workload grade let a regression in (w304 → w313).

`[measured 2026-08-14, vh, tags `w317r331..3`, `w309_crit1.sh fresh`]` **All three boots print
the client's own four-fact line, verbatim and identical:**

```
★     R33 arm 1 COPY      = 4096 bytes moved: dst[0] 0x3f0011cc -> 0xc0ffee33,
      dst[last] 0xc0fff232 (want 0xc0fff232), engine semaphore 0x00000001
      (declared 0x00000001), GP_GET 1 caught GP_PUT 1 — read back through an
      INDEPENDENT mapping (its own device node, its own mmap, a kernel-chosen address)
```

⇒ **the raw-CE plane — no libcuda in the process, its own `FERMI_VASPACE_A`, its own operands —
is green with the budgeted drain active.** `R33_RC=1` on all three, identical to w314's.

| boot | `max_reap_us` | `max_drain_us` | `DRAIN-TIMING` |
|---|---|---|---|
| `w317r331` | 3 675 | 21 108 | `disposed=15 residue=0 turns=1 budget_hit=false` |
| `w317r332` | 3 205 | 20 869 | `disposed=15 residue=0 turns=1 budget_hit=false` |
| `w317r333` | 3 341 | 19 362 | `disposed=15 residue=0 turns=1 budget_hit=false` |

⊘ `budget_hit=false` and `DRAIN-DEFER` **⊘UNMEASURED** are the **correct** readings here, not
failures: this workload's whole queue is **15** disposals, so it fits inside one turn, the
deadline is never reached, and no proc is ever held back. ⚠ An absent `DRAIN-DEFER` on this arm
is *"the mechanism had nothing to do"*, which is a different fact from *"it did nothing"* — the
cup3 arm is where it is exercised.

★ **And this arm is the cleanest per-disposal cost measurement in the whole run**: 15 disposals,
one turn, no deadline, no batching — **~1 291–1 407 µs per disposal**, three times. See §3.

⚠ **A, B and F of w310's list are cup3-only and must not be graded over this arm**:
`w309_crit1.sh`'s `fresh` arm **provokes a fault on purpose**, so its boot legitimately carries
`Xid 31 CE0 … @ 0x7_00100000` — arm 4's own control operand — and its `hostdmesg` is 227 bytes
rather than the cup3 arm's 0. The harness prints the client's own line verbatim and never a
pass/fail word of its own.

---

## 5. ⚠ What these boots do NOT establish

- **The stall detector is still not the witness.** `guest_stall_lines=0` on all four, exactly as
  it was on w314's 3.70 s boot — Linux's soft-lockup watchdog fires at ~20 s, five times the
  budget. It is necessary and nowhere near sufficient. **The numbers are the instrument.**
- **One workload shape.** cup3 tears down **one** guest process with ~18 228 pins. A workload
  with several dying processes, or one with 10× the pins, is unmeasured — though §5.1's
  termination argument is about the queue's *closure*, not its length.
- **Nothing about `MAX_RETIRED_PROCS = 1024`.** Deferring holds procs on the retired list longer;
  the cap is nearer than before and no boot here approaches it.
- **`DRAIN-DEFER` reaching 0 is per-boot, not per-instant.** These boots' guest kept issuing MMIO
  writes after the CUDA process exited. A guest that goes silent right after teardown would drain
  more slowly; the trajectory line is what would show it.

## 6. Files

| file | what |
|---|---|
| `w317_repeat_w317br.log` | the drive log — every cup3 boot's anchored values, criterion E inline |
| `w317_repeat_w317r33.log` | the raw-CE arm, n=3 |
| `run_w317br[1-4]_qemu.log.gz` | the device's own emissions, including every `DRAIN-TIMING` / `DRAIN-DEFER` / `REAP` line |
| `run_w317br[1-4]_probe.log` | the guest-side workload output (`^CUP3_VAL=`, `^CUP3_RC=`) |
| `run_w317br[1-4]_dmesg.log` | guest `dmesg` — 5 379 bytes, `NVRM` ×31, **asserted non-empty** |
| `run_w317br[1-4]_hostdmesg.log` | host `dmesg` **delta**. ⊘ **Zero bytes is the normal green** — no host driver message during the boot, not a truncated capture |
