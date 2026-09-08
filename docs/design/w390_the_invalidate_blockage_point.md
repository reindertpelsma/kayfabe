# w390 — THE TLB-INVALIDATE BLOCKAGE POINT: WIRED, MEASURED, AND BLOCKED BY THE DIRTY GATE

**STATUS: LIVE, 2026-09-08.** All claims measured on a real GA106 (RTX 3060, open 580.159.04,
box `kayfabe-w390-mode2-bench`), nine boots across four revisions. Supersedes nothing; extends
`REQUIREMENTS_TARGET.md` R1.1 with the first hardware reading of its third blockage point.

---

## §0 — THE OWNER'S QUESTION, ANSWERED IN BOTH DIRECTIONS

> Owner, 2026-09-07: *"Is it still needed for correctness to do `vas_publish` at doorbell? If
> we disable it, would the raw clients we have written before including the concurrent one
> still pass?"*

**Measured. Two boots, one variable, with a negative control that FIRES.**

| rung | doorbell arm | invalidate | `CUP3_VAL` | host Xid | rows published by the invalidate |
|---|---|---|---|---|---|
| `w390b3/b4/c1` | `drain` (publishes) | off | **43** | 0 | — |
| `w390c2/c3/c4` | `drain` (publishes) | **on** | **43** | 0 | 1 of 377 passes |
| **`w390e1`** | **`assert` (publishes NOTHING)** | off | **`NO_KERNEL_LINE`** | **16 × Xid 31** | 0 (no passes) |
| **`w390e2`** | **`assert` (publishes NOTHING)** | **on** | **`NO_KERNEL_LINE`** | **16 × Xid 31** | **1** of 377 passes |

1. ★★★★★ **YES — publication is REQUIRED for correctness.** `e1` removes it and compute dies:
   no kernel line, and **16 host `Xid 31`** (MMU fault). ⊘ This is the negative control the
   whole rung needed; without it a `43` anywhere else would be unreadable. It independently
   re-confirms `publication_is_the_wall` on a fifth machine.
2. ★★★★★ **NO — the TLB-invalidate blockage point does NOT substitute for it.** `e2` arms the
   halt, runs **377 publication passes** at `arm=publish`, and produces the **identical**
   failure: same `NO_KERNEL_LINE`, same 16 × `Xid 31`. Moving the trigger, alone, does not work.

---

## §1 — THE MECHANISM IS SOUND. THIS IS NOT A BLOCKAGE-POINT FAILURE.

`[measured, w390c2 vs w390c1 — one variable]`

- `worst_hold_us = 25242` **vs `0`** on the control ⇒ the guest really was held, up to 25.2 ms.
- `over_budget = 0`, `reentrant = 0`, `pending = false` at teardown ⇒ inside
  `INVALIDATE_HOLD_BUDGET_US`, never re-entered, never left the guest spinning.
- `CUP3_VAL = 43` on every armed boot with the doorbell still publishing ⇒ **zero regression.**
- `triggers_at_first_doorbell = 66` of 377 ⇒ **311 (82.5 %) fire AFTER the first doorbell**, on
  the compute path. ⊘ *"The invalidates are front-loaded into bring-up"* is **REFUTED**.

⇒ The halt is real, bounded, correctly completed, and lands where it would be useful. It is
**not** the thing that is broken.

---

## §2 — ★★★★★ THE BLOCKER IS THE w318 DIRTY GATE, AND IT IS BLIND BY CONSTRUCTION

All 377 invalidate passes attributed, on the boot where **the doorbell published nothing**
(`w390e2` — so "the doorbell got there first" cannot be the explanation):

```
  293  ⊘SKIPPED(w318 dirty gate …) REPLAY-OF-LAST-CENSUS
   82  ⊘ THE LOOP BODY NEVER RAN — live_pids=1 vas_keys=0
    2  (other)   ⇒ 1 row published, total, across the whole boot
```

★★★ **293 of 377 passes were told "nothing has changed" while the guest's mappings were
changing and NOTHING had been published.** The gate's key is
`(Vas::publish_epoch, joined_now)` — **our** publication epoch and **our** join count. Neither
moves merely because the guest created a mapping. So the gate answers *"unchanged since the
last completed pass"* truthfully and uselessly: it tracks OUR output, not the GUEST'S input.

⊘ **This is not a bug in the gate as designed.** w318 built it to skip *re-publishing what we
already published*, and against the doorbell — which runs constantly — that is exactly right
and bought an 85.2 → 4.08 ms trap. It becomes wrong the moment publication is driven by a
trigger the gate's epoch does not track.

⇒ **The next fix is named, and it is not in the blockage point.** Either the dirty gate learns
a guest-side dirty signal (the PTE writes the trap surface already sees), or the invalidate
path bypasses the gate for the PDB the invalidate names. ⚠ The second is cheap and is the one
this rung's measurement supports; the first is the general answer.

⚠ The remaining 82 (`live_pids=1 vas_keys=0`) are a *different* fact: a proc exists holding no
VAS. Publication there is vacuous, not refused. **Do not sum the two.**

---

## §3 — SCOPE, STATED SO IT IS NOT OVER-READ

- **`hubtlb_only = 232` of 377.** Those are **BAR** VA-space invalidates.
  `kayfabe_device::mmuinval` says out loud that a publish plane must not treat them as
  compute-path. The compute denominator is **`gpu_vas = 145`**, not 377.
- **The drain half is structurally unavailable here.** `publish_vas_rows` on arm `drain` also
  runs the guest-RAM pin drain, which is scoped by `CeChannelFacts`. An invalidate names a PDB
  (or `ALL_PDB`), **never a channel**, so it printed
  `⚠⚠ TARGET NEVER VISITED — the drain did NOT run` on all 377 lines. That is why the
  invalidate publishes as `Publish`, not `Drain` — a refusal, not a preference.
- **`n = 1` per arm on `e1`/`e2`.** ⚠ `a_single_boot_43_has_a_20pc_false_negative_rate` cuts
  the other way here — a **failure** is not a false negative in the same sense, and both arms
  failed identically with the same 16-Xid signature, but neither has been repeated.

---

## §4 — THREE INSTRUMENT DEFECTS THIS RUNG FOUND, ALL IN MY OWN CODE

1. **The halt was armed TWICE per trigger** (`armed=754` vs `triggers=377`). A nested
   `BlockageGuard` under the guard already gated on `out.invalidate`. ⇒ `publications / armed`
   — the ratio C1 is graded on — was wrong by a factor **nobody would question, because 754 is
   a plausible number.** Caught by its own census.
2. **The publish line was HALF-UNTAGGED.** `publish_vas_rows` splices `"\nkayfabe: "` between
   its two clauses; tagging the returned string tags only the first. The half carrying
   `published=` printed anonymously and was **indistinguishable from the doorbell's own
   publishes** — `grep MMUINVAL-PUBLISH | grep published=` returned **0** over a boot with 377
   passes, and the row histogram mixed two producers into one pile of 1212. I read a number
   off that pile and reported it. Same class as `a_count_cannot_see_a_substitution`.
3. **`over 0 VAS row(s)` could not distinguish "found nothing" from "never looked".** Now
   prints `live_pids` / `vas_keys` at the zero.

★★★ **And a fourth, which was not code:** `w390d1/d2` were run with
`KAYFABE_VAS_PUBLISH=assert` **in the environment**, which `w290p_run.sh:105` overrides
unconditionally from the positional `$ARM`. Both rungs actually ran `arm=drain` with the
doorbell publishing normally, and `d1` returned `43` — which I came within one message of
reporting as *"publication is not needed for correctness"*, **the exact opposite of the truth**.
⊘ It was caught by grepping **the arm actually in force**, not the env I passed. The script
documents this at line 103: *"VAS_PUBLISH stays POSITIONAL … overriding it by env as well
would give one arm two sources of truth."* The harness was right; the invocation was wrong.

---

## §5 — WHAT THIS LEAVES FOR R1/R2

- R1.1's third blockage point: **BUILT, ARMED, BOUNDED, MEASURED.** No longer `◐`.
- R2's P1/P2 (publication off the doorbell): **NOT YET ACHIEVABLE** — and the obstacle is now
  named and is one component away, in `DirtyGate`, not in the blockage model.
- ⊘ The blockage-point model itself is **untouched and unrefuted**: the guest genuinely is
  stopped at the invalidate, for 25 ms, 311 times on the compute path.
