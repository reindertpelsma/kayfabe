# `traces/w319_intermittent/` — the ~20 % `FAULT_PDE` intermittent, ROOT-CAUSED and REPRODUCED

> **STATUS: LIVE — 2026-08-14 (w319).** Branch `w319-intermittent-fault-pde`, base master
> `ef05f9b3`, bench `vh`, real GA106, host driver open `580.159.04`.
> Predictions: `PREREGISTERED.md`, committed before each arm ran.
> Mechanism: `docs/design/the_drain_budget_truncation.md`.
> **Outcome: (A)** — reproduced on demand *and* mechanism attributed by modulation.

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

### ★★★ THE PIN IS ON THE CRITICAL PATH — AND IT IS NOT SUFFICIENT ALONE

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
