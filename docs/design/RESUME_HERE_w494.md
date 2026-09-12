# RESUME HERE — w494. The trap cause is measured; one decision blocks the next step.

**2026-09-12, ~03:00.** Session stopped here deliberately, per *"stop the loop if you get
stuck or need a decision"*.

## The number that matters

| | session start | now |
|---|---|---|
| `worst_trap` | 1.81 s | **23.2 ms** |
| `slow_traps(>1ms)` | 6776 | **197** |
| `>1s` traps | 1 | **0** |
| `inline_exceptions` | 166 | **2** |
| vCPU blocking doors | 1239 reaches | **5** |

Client `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS` at **every** step,
on both arms of every A/B.

## ⚠ THE DECISION

`worst_hold_at` names `crates/kayfabe-rt/src/device.rs:4232` — the **COMMIT phase of the
page-table sweep**, holding rank 1 (`LockRank::Device`) for **17–20 ms**:

```rust
let mut out = self.with_proc_mut(pid, |p| {
    kayfabe_fwd::commit_pt_sweep_revoking(fmt, p, &results, revoke)
})?;
```

`commit_pt_sweep_inner` is `for r in results { … }` over independent per-VAS results, so
**chunking it is mechanically trivial** — commit in batches, releasing the lock between them.

⊘ **It is not trivial semantically.** Today the proc lock makes the commit atomic; nothing can
observe a half-swept VAS. Chunked, a doorbell forward landing between batches could resolve a
VA as unbound and take a **spurious fault**. The function's own comment justifies the locked
phase as *"re-resolving every target (R5)"* — a validity rule, silent on atomicity.

**⇒ Must a sweep commit land atomically, or may it be observed partially between batches?**
Answer that and the change is small.

## How the cause was found — the transferable part

`lockcost` recorded every ranked acquisition **all along, ungated**, and printed only from a
teardown path no boot reaches. Every boot tonight collected the answer and threw it away.
Surfacing it took one line and settled in a single boot what **six hypotheses** could not.

★ **An instrument that ENUMERATES beats an audit that REASONS.** Both changes that moved
numbers tonight came from instruments; none came from reasoning, and six careful arguments
from source were all wrong.

## Six refuted hypotheses — do not retry

| # | claim | how it died |
|---|---|---|
| 1 | the big lock throttled the guest | a stale atomic mirror (w469) |
| 2 | the mirror walk is huge | 62 slots |
| 3 | PRAMIN is unused in the open driver | it bootstraps BAR2 |
| 4 | inline host verbs are what `0x110094` waits on | pre-registered falsifier fired |
| 5 | memslot churn | removing all 4188 fills made it **7× worse** (w488) |
| 6 | `RegPlane::state`, rank 0 | rank 0 is **never contended** (w491) |

## Landed tonight

`w468` mirror walk off the vCPU · `w469` the stale pending mirror (the real 40×) · `w472a`
prefetch fills queued and lossy · `w472b` a guest reset silently disarmed GSP deferral ·
`w479` the isolate spawn off the vCPU (killed the 2.24 s trap) · `w480` both tail drains off ·
`w481` the trap census says WHEN · `w482/3` the over-budget watchdog (**TEMPORARY — delete
before shipping**) · `w489` the silent short write-back now counted and tested · `w491/2/4`
the contention census surfaced and attributed · `w493` the CE submission no longer runs under
the device lock.

Plus: the orphan deletions (739 tests green), the implementation-surface reference, and the
clean-slate architecture document.

## Open, in priority order

1. **The decision above.**
2. `docs/design/ORPHANS_wire_or_discard.md` — wire the `DoorbellTable` (correct, tested,
   unreachable) and the fn-71 reassembler (real hardware sends 2 per boot).
3. The **vacuous working-set gate** — an untracked VA reaches a real copy engine and the gate
   meant to stop it has never executed. Census added (w477); it read zero **with a zero
   denominator**, so the classifier never ran — that is not evidence of safety.
4. Delete the `KAYFABE_TRAP_FATAL_US` watchdog.
5. Six `e2_doorbell` tests red since w467.


---

## w495 — the hypothesis under test now, and a correction to it

**Hypothesis 8: the traps are mostly a DESCHEDULED vCPU, not work.** Every trap figure in this
campaign is wall clock. The w472 box ran 8 vCPUs plus a coordinator plus isolate processes on
**11 cores**. `bar0+0x110094` has no decode arm anywhere and still costs milliseconds — code
that does not exist cannot be slow, so the thread may simply not be running.

⊘ **CORRECTION, owner 2026-09-12:** I argued this partly from *"the Mode-2 C had no isolates
and no coordinator"*. **That is wrong** — the C's `nvkvm_isolate.c` carries an
`isolates[NVKVM_ISOLATE_MAX]` array and Mode 2 used them. How many were live during its
llama.cpp run is unknown. ⇒ the thread-count argument is weaker than I stated, and the
hypothesis rests on the direct experiment rather than on that comparison.

★ **The owner's ceiling, which is the useful part:** the C reached **49.9 tok/s against 47.5
host-native** on comparable vast boxes. An LLM at that rate cannot have had 20 ms traps. So
whatever we are seeing is **not inherent to this architecture** — it is ours, by construction
or by measurement.

**The experiment, one variable:** box `50685423`, RTX 3090 (GA102), **24 cores** against the
previous 11, same revision. Plus two instruments (w495, TEMPORARY):
- `TRAP-CPU` — wall and CPU for the SAME trap. `slow_starved` counts slow traps whose CPU was
  under a tenth of their wall ⇒ not running.
- the over-budget watchdog now `tgkill`s the **stuck thread** so its dump names the site.

⇒ If `slow_starved` dominates, or the tail collapses on 24 cores, it is scheduling and no
amount of lock work will help. If `cpu` tracks `wall`, it is real work and the dump names it.
