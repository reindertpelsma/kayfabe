# w468/w469 — slow traps 6776 → 134, and the invalidate register left the table

**STATUS: LIVE, 2026-09-11.** Measured on box 50585481 (GA106, driver 580.159.04),
one binary per comparison, both arms `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`
and `MEAN_FALSIFIER=PASS`.

## The numbers

`KAYFABE_MIRROR_REVAL_INLINE=1` is the control (the pre-w468 inline walk); absent is
the deferred walk. Both arms in ONE binary, at `7be0cc9b`.

| | defer | inline (control) |
|---|---|---|
| `slow_traps(>1000us)` | **134** | 179 |
| `by_decade` 1-10ms / 10-100ms / >1s | 116 / 17 / 1 | 155 / 23 / 1 |
| `bar0+0xb830b0` (`NV_PFB_PRI_MMU_INVALIDATE`) | **absent from the table** | **53, worst 42 518 us** |
| `worst_trap` | 1 620 938 us @ `bar0+0x110c00` | 1 916 348 us @ `bar0+0x110c00` |

The same A/B at `aa7165ce` — identical code except w469 — reads **6055 / 6889**. So
the two changes are separable and both are real:

| change | slow_traps |
|---|---|
| before both (w467 atomic arm) | 6776 |
| w468 alone (defer, pre-w469) | 6055 |
| w469 alone (inline control) | 179 |
| both | **134** |

## w469 — the 40x was a stale mirror, not a throttle

w466 mirrored the FSM's deferred-doorbell count into a lock-free `AtomicU32` so the
MMIO hot path need not take the plane lock, and stored it on the **write path only**.
`service_one_deferred_command` decrements the real count and left the mirror stale,
so `RegPlane::pending_command_doorbells` read nonzero until the guest's next register
write. `Regs::write` gates its GSP-submit enqueue on exactly that value ⇒ **every
MMIO write enqueued a spurious pubqueue token.**

⊘ The first reading of the w467 A/B — *"the big lock was throttling the guest, so
169 was a throttled number and 6776 is the honest one"* — was a theory built on the
A/B alone and is **refuted**. `an_armed_queue_head_write_records_and_returns` had
been red since w466 and named the defect exactly (`left: 2, right: 0`). The unit
suite was not run when w466 landed.

## w468 — the invalidate trap walked the BAR mirror on the vCPU

`BarMirror::after_write` runs inside the guest's MMIO exit. On every
`MMU_INVALIDATE` trigger **and** every `bar-pde-update` it called `revalidate()`:
one page walk per live slot plus a memslot ioctl per drop. The vCPU now only
**requests** the walk (one atomic bump); the publication worker runs it, ordered
**before** the invalidate's completion is written, so the guest's own spin is what
blocks — the same contract the VA publish already uses.

- ⊘ The walk covers **62** live slots (`kept=62 removed=1`), not tens of thousands.
  It is expensive **per slot**.
- ★ The `bar-pde-update` walk attaches to **whichever trap next observes the changed
  count**, which is why the worst trap sat on `bar0+0x110c00`, a GSP register with
  nothing to do with BAR page tables.
- ★ Safety: a stale slot is observable only by a guest thread racing its own
  invalidate, and it names a translation that same guest had legitimately mapped a
  moment earlier ⇒ guest self-corruption, never cross-process leakage. A translation
  to memory the guest does not own cannot appear this way: a new key is installed
  only on a fresh fault, which takes the ownership check.
- ⊘ `BarMirror::arm` takes `DoorbellAsyncArm::defers()`. On the `off` control there
  is no worker to drain the deferral, so it keeps the inline walk.

## What is still open

Defer arm, 134 slow traps, top sites:

| site | hits | worst |
|---|---|---|
| `bar0+0x110c00` | 9 | **1 620 938 us** |
| `bar0+0xbb0090` | 24 | 6 611 us |
| `bar0+0xb81208` | 20 | 17 745 us |
| `bar0+0xb81408` | 17 | 6 036 us |
| `bar0+0x110094` | 14 | 3 581 us |

⊘ **The 1.6 s is NOT the plane lock.** `GSP-DRAIN worst_single_command` over a whole
run is **3 840 us**, so the worker's longest hold is under 4 ms. The 1.6 s is
unattributed; do not theorise again without measuring.

`inline_exceptions=24` is unchanged (21 × `SharedDevice::verb_op` execute phase,
3 × `fwd::dispose_on`).
