# The budgeted BQL disposal — bounding what one guest MMIO trap may do

> **STATUS: LIVE — 2026-08-14 (w317).** Branch `w317-budgeted-drain`, off master `73dc2246`.
> Parent findings, both folded in place: `guest_ram_pin_release.md` §5 (the exposure and its
> measurement) and `blocking_and_completion_model.md` §1 (the `INLINE-SAFE` predicate whose
> clause (b) this is about).
> ⚠ This doc states a **mechanism and its termination argument**. The bench numbers it is
> graded on are in §7; a claim here that is not in §7 has not been measured on hardware.

---

## 1. The number, and it is already measured

`[measured 2026-08-14 (w314), bench `vh`, real GA106 `580.159.04`, `max_reap_us` — the longest
single `reap_retired()` inside `Regs::write` — n=4 per arm, identical instrument on both trees]`

| arm | the four boots | worst vs `scrubberDestruct`'s **4 000 ms** |
|---|---|---|
| clean master `eb3d99ad` | 2 648 366 · 2 918 210 · 2 666 893 · 2 772 771 | **73.0 %** |
| + w310's guest-RAM pin release | 3 336 519 · **3 702 806** · 3 263 826 · 3 250 535 | **92.6 %** |

The ranges **do not overlap**. Two things follow, and the second is the one that sets this
rung's scope:

- ★★★ **The violation is master's, not w310's.** `Regs::write` already halted every vCPU for
  **2.65–2.92 s** before the pin release existed. w310 adds ~637 ms (+23 %).
- ⇒ **a fix must budget the WHOLE disposal**, not the pin release's share. Removing w310
  entirely would leave 73 % of the budget still being spent in one trap.

★★★★★ **And "every vCPU" is literal.** Every guest MMIO write arrives with the QEMU **BQL**
held (`blocking_and_completion_model.md` §0), so this is not a slow vCPU — it is **the whole VM
frozen**, main loop, timers and monitor included.

### ⊘⊘ The stall detector cannot witness this, and it was chosen to

w310's criterion D — *"guest `dmesg` carries no soft-lockup and no RCU stall"* — **passed on the
3.70 s boot.** It had to: Linux's soft-lockup watchdog fires at `2 × watchdog_thresh` ≈ **20 s**,
five times the budget that matters. ⇒ **a green stall check is not evidence of boundedness.**
The number is the instrument. This rung is graded on `max_reap_us` and `max_drain_us` directly.

---

## 2. What the disposal actually is

`Spine::vacate` / `Proc::retire` move a dead guest process's host objects into
`Proc::pending_release` as [`Orphans`] — `(host VAS, GPU VA)` unmaps, RM handles to free, and
(since w310) isolate-side guest-RAM `mmap` windows to `munmap`. Nothing is issued at that point;
staging is pure bookkeeping and legal under the device write lock.

The **disposal** — the part that blocks — is `Proc`'s `Drop`, reached from the reap that w303
armed in `Regs::write`. It walks the whole queue and issues every verb, on the calling thread,
with the BQL held. At w314's numbers that is ~54 700 disposals at ~70 µs each.

⊘ **Note what is NOT the problem.** The verbs are correct, the ordering is correct, and the
totals are correct. The only defect is *how much of it happens between two guest instructions*.

---

## 3. The mechanism — deferral was already the shape; only the bound was missing

★ **This is why the fix is small.** w303 armed the reap at the **GSP re-handshake edge**, and
that edge **recurs** — its own source says so: *"A proc with a worker still checked out is not
quiesced; `Spine::reap_retired` puts it straight back on the list and it is reaped at a later
quiesce point. Because this edge is every register write, 'a later quiesce point' is the guest's
next MMIO write — there is no deadline to arm and no thread to own it."*

So a retired proc that is not ready is already carried forward. w317 adds one more reason to
carry it forward, and a bounded amount of work per carry.

```text
  Regs::write  (BQL held, every vCPU halted)
    │
    ├── device.drain_retired_budgeted(CHUNK, deadline)      ← w317
    │     loop {
    │       PLAN    Spine::plan_retired_drain(CHUNK)        (device write lock; NO verb)
    │                 └─ Proc::checkout_retired_release_budgeted
    │                      ├─ idle test + worker checkout, indivisibly
    │                      └─ Orphans::split_off_budget(CHUNK)
    │       EXECUTE kayfabe_fwd::dispose_on(worker, batch)  (ZERO ranked locks — R1)
    │       RETURN  SharedDevice::return_worker             (→ Spine::checkin_retired)
    │       if deadline reached { stop; the rest rides to the next trap }
    │     }
    │
    └── device.reap_retired_held()                          ← w317
          Spine::reap_retired_with(ReapPolicy::HoldUndrained)
            └─ a quiesced proc with a NON-EMPTY drainable queue is DEFERRED, not reaped
```

### The three pieces, and why each is where it is

| piece | where | why there |
|---|---|---|
| `Orphans::split_off_budget(n)` | `kayfabe-isolate` | the cutting edge; it is a property of the value, not of any caller |
| `Proc::checkout_retired_release_budgeted` | `kayfabe-core` | the idle test and the checkout must be **one act** — splitting them is how the ordering gets got wrong (`checkout_with_pending_release`'s own argument, reused verbatim) |
| `Spine::plan_retired_drain` | `kayfabe-core` | pure state under the write lock; **one turn, not a loop**, because this crate has no clock (§8.3) |
| the loop + the deadline | `kayfabe-rt` / `kayfabe-qemu-raw` | the shell owns the wall clock |
| `ReapPolicy::HoldUndrained` | `kayfabe-core` | the gate that makes the bound bind |

### ★ The order is preserved ACROSS batches, not merely inside one

`Worker::execute`'s `Release` arm runs `unmap` → `free` → `guest_ram`, and both orderings are
load-bearing: unmap-before-free protects **our mirror** of the mapping, and free-before-`munmap`
is `Orphans::guest_ram`'s stated invariant that *"the GPU's access outlives our view of the
pages"* is a state no reader ever has to reason about.

`split_off_budget` fills the batch from `unmap` **to exhaustion** before touching `free`, and
from `free` to exhaustion before touching `guest_ram`. ⇒ every unmap in the whole queue is
issued before any free, no matter where the boundaries fall.

⊘ **A `budget/3`-per-kind split would break both, silently**: every object would still be
disposed of exactly once, so both the per-handle counts and the totals stay green.
`the_release_order_is_preserved_across_budget_boundaries` is the assertion that a passing
`EXACTLY ONCE` cannot make, and it was **watched failing** under exactly that mutation.

---

## 4. ★★★ The budget, and what it is a fraction OF

**`RETIRED_DRAIN_BUDGET_US = 40 000` — 40 ms, 1 % of `scrubberDestruct`'s 4 000 ms**
(`ce_utils.c:349`), the shortest *named* guest-side timeout in this tree.

⊘ **Deliberately not "4 s minus epsilon".** The 4 s is one guest operation on one workload; a
budget sized to just fit it fails on the next workload with a tighter timeout — and this
campaign has already been bitten once by grading a single workload
(`scripts/bench/relaxation_inert_gate.sh` exists because of it).

**1 % buys two independent margins:**

1. **100× on the named bound.** A guest operation whose timeout is 100× tighter than the
   scrubber's still survives a full drain, and one at the scrubber's own scale survives ~100.
2. **Below perceptibility.** 40 ms is under one 24 Hz frame. A whole-VM freeze at this size is
   indistinguishable from ordinary scheduling jitter.

### ⚠ And what the number is deliberately NOT derived from

It is **not** `N disposals × the measured per-disposal cost`. w310 §5's estimate of exactly that
quantity was wrong by **~20×** (`munmap` of a `MAP_SHARED` memfd window RM has
`pin_user_pages`-pinned: 35 µs measured, 1–2 µs assumed). A count-based budget degrades silently
by whatever factor the estimate is off; **a time budget re-measures the cost every turn, for
free, by construction.**

### ★★★ `RETIRED_DRAIN_CHUNK = 4` — the granularity, and it is MEASURED

The deadline is read *between* turns, so the delivered bound is `budget + one chunk`, and the
chunk is the only part a wrong cost estimate can inflate. It did:

- **The first value was 64**, sized against an estimated ~70 µs per disposal. `[measured
  2026-08-14, vh, n=4]` three of four boots came back `disposed=64 turns=1
  max_drain_us ≈ 92 000` — **one chunk, alone, took ~92 ms**, and the 40 ms deadline never got
  to bind. ⇒ the delivered bound was **3× the stated budget**.
- ⊘ `disposed=64 turns=1` fits **two models with opposite fixes**: 64 uniformly-slow disposals
  (a smaller chunk cures it) or one very slow disposal among 63 fast ones (no chunk cures it —
  one disposal is indivisible). Retuning on that data would have been **fitting, not
  measuring**.
- ★★★★★ **The discriminator was one throwaway boot at `chunk = 1`** (`traces/w317_drain/`,
  `w317c1diag`), pre-registered before it ran: with the deadline re-read after **every**
  disposal the worst trap measured **43 260 µs**, `CUP3_VAL=43`, `DRAIN-DEFER 1 → 0`. ⇒ **the
  worst single disposal is ≤ ~3.3 ms** — the expensive phase is uniformly expensive, there is
  no monstrous indivisible disposal, and a smaller chunk cures the overshoot proportionally.

**The rule, stated so the next person can re-derive it rather than inherit a number:**

> `chunk × worst_single_disposal` may contribute **at most a third of the budget**.

`4 × 3.3 ms ≈ 13 ms` = 33 % of 40 ms ⇒ **delivered bound ≤ 53 ms = 1.3 % of the 4 000 ms.**

⊘ **Not 1**, though 1 measured fine: each turn costs a device write lock, a `return_worker`
round and a `Worker::execute` call, and a backend where `execute` is one IPC per *plan* would
pay all of it per disposal. ⚠ That overhead was **not measured in isolation** — chunk=1's
per-disposal cost (111–173 µs) merely sits in the same range as chunk=64's (121–145 µs), which
bounds it as *small*, not *zero*.

⊘ A zero chunk would make the drain a no-op and `HoldUndrained` a **permanent** defer — strictly
worse than the stall. Refused at compile time (`const _: () = assert!(RETIRED_DRAIN_CHUNK > 0)`),
not reasoned about.

---

## 5. ★★★ Correctness: what guarantees the remainder is eventually done

*A drain that defers indefinitely is a leak with extra steps.* Three arguments, each checkable.

### 5.1 The queue is CLOSED and strictly decreasing — so it empties

A **retired** proc is out of every routing map and refuses every new op, so nothing the guest
does can add to its queue. The only other writer is §7.5's residue staging for verbs that were
*already in flight*, which is finite. Each drain turn removes `min(chunk, len)` and **never puts
anything back** — a failed disposal's residue is counted and dropped, exactly as `Proc::drop`
and `SharedDevice::drain_pending_releases` already drop theirs, precisely so that a
permanently-refusing object cannot be re-split forever.

⇒ the queue empties in at most `ceil(len / chunk)` turns. This defer **cannot** be permanent.

⊘ Re-staging the residue would be more "correct"-looking and would break this argument. It is
the one shape rejected on purpose.

### 5.2 The edge recurs, and it is the guest's own MMIO traffic

"A later turn" is the guest's next register write. This is not new machinery and not a timer;
it is w303's existing quiesce edge. ⚠ It is also the argument's weakest link: a guest that stops
touching the device stops draining. That is **outcome (B)** in §7 and it is instrumented
(`DRAIN-DEFER`), not assumed.

### 5.3 The unconditional backstop is unchanged

`ReapPolicy::Unbudgeted` — `Spine::reap_retired()`, every caller outside `Regs::write` — is
**byte-identically what master did**, and device teardown still drops every retired `Proc`,
issuing whatever is left. So the *totals* are not at the mercy of the budget at all; only the
*timing* is.

### 5.4 ⊘ `has_drainable_releases` is the NARROW predicate, on purpose

The reap gate asks *"is there staged work a drain could actually issue"*, **not** *"is
`pending_release` non-empty"*. An entry whose target GPU has no isolate is unreachable for any
drain — `Proc::drop` skips it by the same test, and its disposition of record is §7.0 namespace
death. Gating on the wider predicate would defer such a proc at **every** quiesce point forever:
a bounded drain turned into a permanent leak. Both sides use the one predicate.

---

## 6. ⊘ What this rung does NOT do

- **It does not make the disposal asynchronous.** The same objects are freed by the same verbs
  on the same thread. It converts **one 3.7 s stall** into **many bounded ones with the guest
  running in between**. That is a clause-(b) fix and nothing else.
- **It does not reduce total work**, and it slightly increases it: one extra device write-lock
  acquisition per register write, plus one `return_worker` round per turn.
- **It does not provide an off-BQL execution site.** `w306` established that
  `IsolateSlot::call` is write-then-read with every production caller inside the vCPU trap, and
  that *"the missing mechanism is not a completion transport, it is an off-BQL execution site
  for host verbs."* That is still missing. This rung is the answer to *"can clause (b) be met
  **without** it?"* — and see §7's outcome (D).
- **It does not touch `MAX_RETIRED_PROCS = 1024`.** Deferring holds procs on the retired list
  longer, so the cap is nearer than before. Named, not changed; 1024 simultaneous dying guest
  processes is not a shape any current workload reaches.
- **It changes nothing for any caller that is not `Regs::write`.** `reap_retired()` keeps its
  exact semantics, which is why no existing test needed editing.

### ⊘⊘ AND ONE DEFECT FOUND WHILE READING THIS PATH, NAMED AND NOT FIXED HERE

`Proc::stage_release` (`kayfabe-core/src/gpu.rs`) takes an `Orphans` **by value** and stages
only two of its three kinds:

```rust
q.unmap.extend(orphans.unmap);
q.free.extend(orphans.free);          // ← and `orphans.guest_ram` is DROPPED, silently
```

⇒ **any `guest_ram` window in a failed verb's residue is lost.** Its three callers are
`SharedDevice::stage_orphans` (§7.5's abandoned chain — the fallback that exists *precisely* so
outstanding host objects stay nameable), the retired-proc twin beside it, and
`kayfabe_fwd`'s residue path. This is the **exact class** `Orphans::len`'s own doc was written
against — *"exists so a caller counting cannot silently omit a kind"* — surviving one function
over, where the omission is not a miscount but a **leak**.

⚠ The fix is one line. It is **deliberately not made here**: staging `guest_ram` makes
`munmap`s happen that do not happen today, on the live verb path, and folding an unmeasured
behaviour change into the rung whose whole output is a **timing measurement** would make that
measurement unattributable. It needs its own rung, its own known-positive (a `VerbFailure`
carrying `guest_ram` residue, watched surviving the round trip) and its own boot.

---

## 7. ★★★ Pre-registered outcomes and the bench criteria

**Every outcome, written before the boots, so none reads as the favourable one.**

- **(A)** `max_reap_us` drops well below budget with both workloads green ⇒ report the new
  distribution and the budget's justification.
- **(B)** it drops but something else grows — deferred work piling up, more `Regs::write` work,
  a later stall ⇒ **name the thing that grew.** A bound that moves cost rather than removing it
  is worth knowing about. The instrument is `DRAIN-DEFER`: it must return to 0.
- **(C)** correctness breaks (`^CUP3_VAL=43` or `R33 arm 1` red at n ≥ 3) ⇒ the budget is too
  aggressive, or the remainder is not being finished; say which.
- **(D)** it cannot be bounded without an off-BQL execution site ⇒ ★ a **full result**: knowing
  the bound *requires* that site is a design fact worth more than a partial fix.

### The criteria

| # | criterion | why |
|---|---|---|
| **1** | `max_reap_us` — **distribution over n ≥ 4**, min/median/max, compared as **ranges** against w314's | the number that moved and the number that must move back. ⚠ A single pair of numbers is not the claim; non-overlapping ranges are |
| **2** | `max_drain_us` ≤ `RETIRED_DRAIN_BUDGET_US` + one chunk, on every boot | the budget is a claim; this is its falsifier. ⊘ ABSENT = **UNMEASURED**, never 0 |
| **3** | `DRAIN-DEFER` returns to **0** | §5.1's termination argument, made checkable. A value that rises and never falls is outcome (B) |
| **4** | `^CUP3_VAL=43` at **n ≥ 4** | ⚠ w314 measured a **~20 % false-negative rate** for a single cup3 boot on this box, on *master as well as on the branch*. n=1 is not a grade |
| **5** | `R33 arm 1` (raw CE client, no libcuda) fires | the two-workload gate. `43` alone is not sufficient — `relaxation_inert_gate.sh` exists because a single-workload grade let a regression in |
| **6** | `regression_check_e.sh` passes (criterion E as w304 rewrote it) | ⊘ do **not** grade `host_rows` as an exact value; the old criterion did and it fails on correct results |
| **7** | `PIN-RELEASE released=N`, `N > 0`, `refused_no_host_vas=0`; `REAP` present | w310's C/E/G, unchanged — the budget must not have quietly stopped the release running |
| **8** | guest `dmesg` non-empty, contains `NVRM`, no soft-lockup / RCU stall | necessary, ⚠ and **nowhere near sufficient** — see §1 |

⚠ **Any bench claim must carry the SOURCE REVISION it was measured at**, and the stamp gate
(`kayfabe-rev:` in the QEMU binary vs the tree's `HEAD`) must agree or the run is void.

---

## 8. Where the code is

| thing | file |
|---|---|
| the split | `crates/kayfabe-isolate/src/lib.rs` — `Orphans::split_off_budget` |
| the indivisible checkout | `crates/kayfabe-core/src/gpu.rs` — `Proc::checkout_retired_release_budgeted` |
| the narrow predicate | `crates/kayfabe-core/src/gpu.rs` — `Proc::has_drainable_releases` |
| one turn, planned | `crates/kayfabe-core/src/gpu.rs` — `Spine::plan_retired_drain`, `RetiredDrain` |
| the gate | `crates/kayfabe-core/src/gpu.rs` — `ReapPolicy`, `Spine::reap_retired_with`, `Reclaimed::deferred_for_drain` |
| the loop + the deadline seam | `crates/kayfabe-rt/src/device.rs` — `SharedDevice::drain_retired_budgeted`, `reap_retired_held`, `RetiredDrainStats` |
| the budget and the boot lines | `crates/kayfabe-qemu-raw/src/shim.rs` — `RETIRED_DRAIN_BUDGET_US`, `RETIRED_DRAIN_CHUNK`, `DRAIN-TIMING`, `DRAIN-DEFER` |
| the gate suite | `tests/tests/budgeted_drain.rs` — four tests, three severs watched failing |
