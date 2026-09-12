# The stall was ours: a census, a lock, and three blind instruments (w505–w525)

**STATUS: LIVE, 2026-09-12. ★★★★★ THE FIRST GOAL IS MET — see "Where it ended" at the bottom.** Supersedes the starvation reading in `every_slow_trap_BLOCKED_w499.md`
and retires hypotheses 1–9 listed there and in `RESUME_HERE_w494.md`.

## The answer, in one line each

| what | measured | where |
|---|---|---|
| the worst MMIO trap of the boot | `worst_trap=15982us at=bar0+0xb81208` | the **CPU interrupt tree**, `0x00B8_1000`–`0x00B8_1643` |
| what it waited for | `rank0 worst_wait=8279us` beside `worst_hold=8209us` | `plane.rs:2802`, `ce_session_with_root` |
| what the holder was doing | `ring_inline → vas_coverage → reachable_ranges → quicksort` | `SharedDoorbell::ring_inline`, **our own census** |

A guest write to an interrupt-enable register waited **16 ms** because the plane's single
mutex also covers `fb`, `ram`, `mmu` and the page-table witness, and the doorbell worker holds
that mutex for **8.2 ms** across a whole CE submission — inside which it sorts range lists to
build a diagnostic string.

⇒ **None of it is the guest's work.** The owner said this before any of it was measured:
*"locks taking over data structures that protect 90% of the data completely irrelevant for the
vcpu mmio handler"*, and *"a doorbell is only a token + channel … why is vcpu thread holding
submission locks?"*

## Why it took nine hypotheses

★★★★★ **Three instruments printed an unrecorded quantity as `0`, and a healthy lock prints `0`
too.** Nothing in the output distinguished *"measured zero"* from *"not measured"*.

1. **w505** — `RankedMutex::lock` never recorded a **wait**. `RankedRwLock::{read,write}` both
   did. `RegPlane::state` is a `RankedMutex`, so rank 0's waits were never measured; the census
   skips a rank whose wait and hold are both zero, so **rank 0 was absent from every census**,
   and six hypotheses read that absence as "never contended".
2. **w507c** — `RankedMutexGuard::drop` never recorded a **hold** either. Once w505 made rank 0
   visible it printed `worst_wait=3825us worst_hold=0us`, which I read as *"no holder is long ⇒
   the waiter is starved by a crowd of short ones"* and built two commits on. ⊘ That zero was
   also not a measurement. Starvation and one-long-holder want **opposite fixes**.
3. The same night, `lockcost` printed only from a teardown path no boot reaches, and the stall
   alarm armed 61 072 times while a blocked signal (SIGALRM, which QEMU masks on vCPU threads)
   swallowed every firing.

⇒ The defect was never in the logic. **One guard type was missing a line the other two had**,
and nothing quantified over the three. `a_hold_through_each_guard_type_is_recorded_with_its_site`
now does.

## The three instruments that ended it

- **`worst_wait_blocked_by=`** — who held the rank when the worst waiter **arrived**, sampled
  before blocking. ⊘ `worst_wait` and `worst_hold` accuse *different threads*, and nothing said
  they were the same acquisition: `[measured w509]` rank 1 printed `worst_wait=2425us` beside
  `worst_hold=5967us` at a worker-thread site, and the line did not establish any relation
  between them. **Eight hypotheses died in that gap.**
- **`lockcost::hammer_census`** — acquisitions **counted per site**, lock-free. `worst_hold`
  names only the single *longest* holder, so it structurally cannot name a *crowd of short*
  ones. It reports its own `overflow=`, because a ranking over a truncated table names a
  runner-up and reads exactly like a correct answer.
- **The stall alarm, aimed at the worker.** Same tool, other thread. It printed the five
  quicksort frames directly; no inference was involved.

## What was fixed, and what was not

Fixed:
- `cpu_intr` moved out of `PlaneState` to its own Leaf-rank lock. All four uses took the plane
  lock for that field **alone**. ★ The test hung on its first run — the write arm still had a
  vestigial `state.lock()`/`drop(s)` pair around the changed line, and every existing test
  passed anyway.
- The per-doorbell `vas_census` is off by default. It is a **pure report**; unlike `pt_witness`,
  `pt_decode` and `pt_sweep` beside it, nothing downstream depends on it having run.
- The sweep defers to an in-flight MMIO trap (w507b). ⊘ Kept because it is correct and cheap
  (`deferrals=77–145 giveups=0`), **not** because it was the stall. It was not.

⊘ Not fixed, and both are measured:
- `rank1`/`rank2 worst_hold=28491us slow_holds≈1100` at `device.rs:4220` — the PLAN phase of
  `sweep_pt_tables_revoking` holds the Device read lock **and** the Proc cell for 28 ms, and
  vCPUs waited on both (`slow_waits=8` and `50`).
- The rest of the `PlaneState` split. The 13 fields divide as **registers/FSM** (`fsm`,
  `policy`, `unclaimed`, `fb_window`, `bar0_window`) versus **memory/VA** (`ram`, `fb`, `mmu`,
  `pt_witness*`). Only PRAMIN and the BAR0 framebuffer window need both — and PRAMIN is the
  owner's single enumerated stall exception.

## w515–w516: the second stall, and two more one-armed instruments

★★★★★ **The vCPU's other blocking site, measured:**

```
nvkvm_bar1_write -> Regs::write -> SharedDevice::take_table_changes
                 -> RankedMutex<Proc>::lock -> futex_wait
```

`take_table_changes` takes **every `Proc` cell's lock** — rank 2 — purely to read each address
space's `AddressTable::generation`, a counter. `[measured w514]` rank 2 is held for **5878 µs**
by the page-table sweep's COMMIT phase, and w511's `worst_wait_blocked_by` names the waiter
and the holder as the **same line**, `device.rs:4232`.

⇒ The vCPU never needed the per-VAS detail. It needs one bit — *"is there anything for the
publication worker to do?"* — so it now reads `any_table_change_epoch()`, one atomic bumped
beside every generation bump. ⚠ The safety argument is an **asymmetry**, not a count: the
epoch over-reports and never under-reports, because nothing clears it. A spare publication job
costs a pass that finds nothing; a missed one leaves a guest mapping unpublished. Never invert
that.

⊘ **Two more instruments were one-armed, and both cost a boot:**
- The stall alarm fired on the **worker**, where `Worker::execute` calls `assert_lock_free`
  before any verb — so the worker blocking in `recv()` on the isolate socket is the design
  *working*, not a stall. But the handler `_exit(42)`s on its first firing, so one boot yields
  one backtrace, and this **healthy** path won the race against the sick one every time.
  **An instrument that fires on a healthy path silences the instrument aimed at the fault.**
- The alarm was armed only in `kayfabe_shim_regs_write`. A slow **read** trap could never fire
  it, and that silence read as *"no trap was over budget"* — with a 5747 µs trap in the same
  log and `armed=62631 refused=0`. Same shape as the ranked-lock guards: one arm instrumented,
  the other not, and an unmeasured path renders exactly like a healthy one.

## w522–w525: the split, and the two failures it caused

The plane's single mutex covered the GSP state machine, the command policy, the BAR0 window
latch, guest RAM, the framebuffer and the page-table format. The copy-engine session needs
**two** of those, so `fb` and `mmu` moved to `PlaneMem` under a rank between `Plane` and
`Device` — a placement that is forced, not preferred, because a register path may take the
state machine and then reach the framebuffer, while the session calls into the core inside
its closure.

**Measured on complete, passing runs** (`(P)`, `MEAN_FALSIFIER=PASS`, `THREADS 8/8`):

| | w514 | w517 | w521 | w524 |
|---|---|---|---|---|
| slow traps (>1 ms) | 41 | 37 | 10 | **2** |
| rank-0 worst wait (µs) | 8834 | — | 8834 | **687** |
| slow waits, **all** ranks | — | — | — | **0** |

⚠ **Two failures, both mine.**

1. **A rank inversion killed w523's boot.** `read_published_va` and `read_va_from_root` walk
   under `PlaneMem` and then read guest RAM — which stayed with the state machine — so they
   took rank 1 and reached for rank 0. The doorbell worker panicked by name, poisoned both
   locks and ended QEMU mid-run.
   ⊘ **The truncated run's census looked BETTER** (`slow_traps 10 → 1`) and was worthless:
   60 875 traps instead of 89 310. A shorter run has fewer slow traps for free. Read
   `W392D_GUEST_RC` and the "NO CENSUS" line before reading any number from a log.
   ★ Those two functions had **no test at all** — which is why an inversion reached a boot
   rather than a `cargo test`. `published_va_lock_order.rs` covers them now, verified by
   re-introducing the bug.
2. **A diagnostic aborted the VM at teardown.** The deferred-notes sink and the rank witness
   touch a thread-local on every lock release; on a departing thread that panics inside an
   `extern "C"` frame, which cannot unwind and becomes a process abort. ⇒ **A diagnostic may
   never be the thing that kills the process it is describing.**

## The last bad number, and what it was

`worst_trap` sat at ~16 ms on `bar0+0xb81208` for seven consecutive boots while every lock
rank read `slow_waits=0`. The alarm, aimed at that one register, named it:

```
drain_retired_budgeted -> dispose_on -> ProxyRmBackend::free -> read_frame -> recv
```

A blocking socket round-trip to the isolate, inside an MMIO trap. The reap moved to the
doorbell worker at w525. ⊘ That is a **contract change**, not a bug fix — two reap tests
assert the trap does it, and they now pin the arm with no worker while the shipping arm gets
its own test.

## Open, and needing the owner

**Must a page-table sweep commit land atomically?** Chunking `commit_pt_sweep_revoking` is
mechanically trivial — it loops over independent per-VAS results — but today the proc lock
makes it atomic. Chunked, a doorbell landing between batches could resolve a VA as unbound and
take a spurious fault. See `RESUME_HERE_w494.md`.


## Where it ended — w525, on a complete passing run

```
W392D_GUEST_OUTCOME=(P)   MEAN_FALSIFIER=PASS   THREADS 8 of 8   panics=0
TRAPWITNESS  inline_exceptions=0  worst_trap=3622us at=bar0+0x110094  slow_traps(>1ms)=1
TRAP-CPU     n=89655  worst_wall=688us  slow_blocked=0  slow_starved=0  slow_preempted=0
VCPU-BLOCKING none — no blocking door was reached on a vCPU thread
```

⇒ The owner's first invariant holds **by measurement**. `inline_exceptions` is **0**, its
stated target, from **166** at w394.

| | w394 | w514 | w517 | w521 | w524 | w525 |
|---|---|---|---|---|---|---|
| worst trap | 1.81 s | 16.0 ms | 16.1 ms | 16.9 ms | 16.6 ms | **3.6 ms** |
| slow traps (>1 ms) | 6776 | 41 | 37 | 10 | 2 | **1** |
| slow waits, all ranks | — | — | — | — | 0 | **0** |

★★ **CONFIRMED n=2 at w530**, same tree, both boots complete and passing: `(P) 8/8`,
`panics=0`, `inline_exceptions=0`, `VCPU-BLOCKING none`, worst trap 2758 µs vs 3622 µs,
slow traps 3 vs 1. ⊘ That spread is the rented VM's ~10 ms steal noise floor, not a change —
both worst traps are at the **same register** and both report `slow_blocked=0`.

⚠ **One instrument disagreement, stated rather than resolved.** `TRAPWITNESS worst_trap=3622us`
against `TRAP-CPU worst_wall=688us`: they cover different sets, because `TRAP-CPU` is armed
only in the **write** entry point while `TRAPWITNESS` covers reads too. So the 3.6 ms trap was
a **read** of `bar0+0x110094`, the GSP queue register the guest polls. Not chased — one trap
in 89 655, with `slow_blocked=0` saying it waited on nothing of ours.
