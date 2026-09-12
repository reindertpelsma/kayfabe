# The stall was ours: a census, a lock, and three blind instruments (w505–w513)

**STATUS: LIVE, 2026-09-12.** Supersedes the starvation reading in `every_slow_trap_BLOCKED_w499.md`
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

## Open, and needing the owner

**Must a page-table sweep commit land atomically?** Chunking `commit_pt_sweep_revoking` is
mechanically trivial — it loops over independent per-VAS results — but today the proc lock
makes it atomic. Chunked, a doorbell landing between batches could resolve a VA as unbound and
take a spurious fault. See `RESUME_HERE_w494.md`.
