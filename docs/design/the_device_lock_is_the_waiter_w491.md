# w491 — the millisecond traps wait on the **Device** lock (rank 1). Measured.

**STATUS: LIVE, 2026-09-12.** First cause in this hunt that is measured rather than inferred.

## The measurement

`lockcost::census()` records `note_wait`/`note_hold` on **every** ranked acquisition, ungated,
and has all along — it printed only from a teardown path no boot reaches, so the data was
collected and discarded every time. w491 puts it on the boot line.

| | passthrough on | passthrough off |
|---|---|---|
| `rank1 worst_hold` | **23 881 us** | 22 881 us |
| `rank1 worst_wait` | 11 061 us | 5 156 us |
| `rank1 slow_holds` | **3 453** | 3 185 |
| `worst_trap` | 29 094 us | 32 378 us |
| client | (P) | (P) |

★ **`rank0` does not appear at all.** The census skips a rank only when both its worst wait
and worst hold are zero ⇒ the register plane's mutex was never meaningfully contended.

**Rank 1 is `LockRank::Device`** (`kayfabe-util/src/lock.rs:83`) — `SharedDevice`'s state.

⇒ a lock is **held for ~24 ms**, over three thousand times past the slow threshold, while the
worst trap is ~29 ms. Same magnitude, same boot, both arms.

## Why this took six hypotheses

Every refuted one was refuted correctly, and each narrowed the target:

| # | hypothesis | how it died |
|---|---|---|
| 1 | the big lock was throttling the guest | it was a stale atomic mirror (w469) |
| 2 | the mirror walk is O(tens of thousands) | it is 62 slots |
| 3 | PRAMIN is unused in the open driver | it bootstraps BAR2 |
| 4 | inline host verbs are what `0x110094` waits on | pre-registered falsifier fired; that register got *worse* |
| 5 | memslot churn | removing all 4188 fills made slow traps **7x worse** (w488) |
| 6 | `RegPlane::state`, rank 0 | **this census: rank 0 is not contended at all** |

⊘ Hypothesis 6 was right about the *mechanism* — a long hold on a lock the vCPU also takes —
and wrong about *which lock*. The source audit that produced it looked hardest at the plane
because that is what the vCPU touches first; the instrument looked at all of them.

★ **The lesson is the one this campaign keeps paying for.** An instrument that enumerates
beats an audit that reasons, every time. Six hypotheses cost a night; the census cost one line
and was already written.

## What to fix, and what NOT to fix

The hold is **not** across a host verb, an mmap or any syscall — `assert_lock_free` panics on
those with a rank held, and `kayfabe-rmrpc/src/policy.rs:3262` records the boot panic that
proves the check works. ⇒ **it is CPU work plus acquisition volume under the Device lock.**

The whole-VAS page-table sweep is the prime suspect for the volume: `refresh_ms` measures
**58-68 ms** while the worst single GSP command hold is 1.4-2.0 ms, so the refresh is many
acquisitions rather than one.

⚠ **Do not "fix" this by holding the lock longer** (one guard for the whole sweep). That
converts starvation into a guaranteed 24 ms stall for any waiter. The fix is the opposite:
**take what the sweep needs under one short hold, then walk the copy with the lock released**,
so the hold is bounded by a copy rather than by a walk.

⊘ **Next step is attribution, not a fix.** `note_hold` records a rank, not a site, so nothing
yet says *which* acquisition is the 24 ms one. Label the acquisitions before changing any of
them — five of the six dead hypotheses died of acting on a plausible mechanism without
knowing the site.
