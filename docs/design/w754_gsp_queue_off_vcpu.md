# w754 — the GSP command-queue doorbell, and what is ACTUALLY on the vCPU behind it

**STATUS: LIVE, 2026-09-16.** Pre-registration, written and committed **before the first
boot of this rung** (the box was rented first because provisioning is the long pole; no
measurement of this branch existed when this file was written). Successor to
`fable_off_vcpu_design.md` row 3 and to `THE_CONSTRAINTS.md` §25's closing ⊘.

## The baseline, quoted rather than restated

`[measured w752, vast 51210329, GA106, 580.159.04 OPEN, TREE_REV fcce2a11;
traces/w752_inplace_repoint/]`

```
device:  worst_trap=24999us at=bar0+0x110c00   cpu_of_that_trap=23979us   (96 % CPU)
control: worst_trap=25077us at=bar0+0x110c00   cpu_of_that_trap=25007us
split:   worst_trap=30474us at=bar0+0x110c00
SLOW-SITES bar0+0xbb0090=5(worst 8802us) bar0+0xb81608=3(worst 6903us)
          bar0+0x110c00=2(worst 24999us) bar0+0xb81408=2(worst 5150us)
          bar0+0xb81410=1(worst 3898us)  bar0+0xb81610=1(worst 3922us)
          bar0+0x1700=1(worst 1125us)                             AT=teardown
LOCKCOST rank0 worst_wait=8790us worst_hold=748us slow_waits=4 slow_holds=0
```

## ⊘⊘⊘ THE BRIEF'S MECHANISM IS ALREADY BUILT, AND IT IS NOT WHAT COSTS THE 25 ms

The brief (and `boot.rs:945`'s own comment) reads the 25 ms as *"the queue-head write drains
and services the entire GSP command queue inline"*. **Measured false from this tree's own
logs, three independent ways:**

1. **Deferral is armed, early, on every arm.** `run_w752dev_qemu.log:43` carries
   `GSP-ASYNC ARMED` at device realize (12:29:33), long before the guest driver loads, and
   `GspFsm::device_reset` has preserved `defer_commands` across a guest reset since **w472b**
   (`boot.rs:828-846`, which is itself the correction of exactly the failure the brief
   describes). ⇒ `BootStep::CommandDoorbell` increments a counter and returns.
2. **The plane lock cannot account for it.** `rank0 worst_wait=8790us worst_hold=748us`. A
   24 999 us trap cannot be 24 ms of waiting on a lock whose worst wait all boot was 8.8 ms
   and whose worst hold was 0.75 ms.
3. **It is CPU, and the clock is per-thread.** `cpu_of_that_trap` is
   `CLOCK_THREAD_CPUTIME_ID` on the trapping thread
   (`kayfabe-linux-raw/src/signal_unsafe.rs:647-655`), sampled around the whole of
   `kayfabe_shim_regs_write`. 96 % CPU ⇒ the vCPU thread **ran** for 24 ms. A futex wait
   accrues none of it.

⇒ **The register is a bystander.** `0x110c00` is simply the register the guest writes most
often during driver init, so it is the trap that most often *notices* a latch change.

## ★★★★★ THE HYPOTHESIS — the guest page-table SETTLEMENT, run inline from the ring adopt

`Regs::write` (`shim.rs`) still calls `self.adopt_pending_channel_rings()` **on the vCPU, on
every arm**, gated only on `kayfabe_rt::device::pending_latch_epoch()` moving. When anything
is latched, that function runs three passes before it joins anything:

```
let w = self.doorbell_port.witness_executor_fb_pages();
let d = self.doorbell_port.decode_cpu_pt_writes();
let s = self.doorbell_port.sweep_cpu_pt_tables();
```

— *"the doorbell's own page-table settlement"*. That sweep reads guest page tables through
`PlanePtBytes::read_in`, the hottest rank-0 acquisition site in the boot
(`HAMMER(rank0) plane.rs:3203=306066`), and rank 1/2 are held 4658 us / 2288 us by its
commit. It ran **61 times** on the w752 device arm (`GR-RING-JOIN` block heads), and the
seven `SLOW-SITES` entries above total fifteen slow traps — the right order of magnitude for
"the settlement lands on whichever register write noticed the latch".

⊘ **And on the deferring arm its own ordering argument is already broken**: the function's
doc says it must run *before* `report_channel_birth_drain` and `report_engine_forward_drain`,
and on `DoorbellAsyncArm::On` **those two run on the worker while the adopt stays on the
vCPU**. Moving it to the worker does not merely relocate cost — it restores the ordering the
comment asks for.

## The two cuts

### Cut A — constraint 6, literally: the queue-head write takes NO plane lock

Today the write reaches `RegPlane::write`'s generic tail, takes `state.lock()` (rank 0) and
runs the FSM only to increment a counter. Classify it **before the lock**, beside the three
arms that already say *"⚠ No plane lock may be held"* (the usermode doorbell, the MMU
invalidate, the interrupt tree): shadow the value, bump a lock-free counter, offer the
pubqueue token, return. The worker folds the posted count into the FSM under the lock it
takes anyway and drains as before.

⊘ This does **not** change who services the queue — w432 already moved that. It removes the
vCPU's dependence on a lock a worker holds, which is the half `THE_CONSTRAINTS.md` §25 names
as *"LOCK-FREE IS NOT OFF-vCPU"* read in the other direction.

### Cut B — the settlement + ring adopt onto the doorbell worker

`adopt_pending_channel_rings` moves onto `SharedDoorbell`, and the worker calls it
immediately **before** `report_channel_birth_drain`. The vCPU keeps it only on the arm with
no worker (`!doorbell_async.defers()`), exactly as `materialize_pending` and the two drains
already do — and for the same stated reason: without a worker, a drain that never runs is a
guest parked forever.

⊘ `pending_err_notifier_grants()` is folded under the same gate: its only consumer is
`report_engine_forward_drain`, which does not run on the vCPU on the deferring arm, so on
that arm the vCPU was computing a `Vec` nobody reads — under the device read lock.

## PRE-REGISTERED PREDICTIONS — each with the value that refutes it

| # | prediction | refuted by |
|---|---|---|
| **P1** | `KAYFABE_KFTIME=census` on an unmodified device arm attributes the worst `mmio_other` event to the **`ring_adopt`** segment, worst ≥ 15 000 us, with `plane` worst < 10 000 us | `ring_adopt` worst < 5 000 us while some other segment (or the **named residual**) carries ≥ 15 000 us ⇒ the mechanism above is wrong and that segment is the answer |
| **P2** | After cut B, `bar0+0x110c00` **leaves `SLOW-SITES` entirely** on the device arm | it is still listed with worst ≥ 1 000 us |
| **P3** | `worst_trap` on the device arm goes **sub-millisecond** | any `worst_trap` ≥ 1 000 us — and then the SITE is the result, reported by name |
| **P3b** | ★ `SLOW-SITES` loses **more than 0x110c00**: `0xb81408`, `0xb81410`, `0xb81608`, `0xb81610` go too, because those offsets carry no per-register work that could cost milliseconds — they are the same settlement at a different address | they stay, with the same worsts ⇒ they are their own cost and cut B explains only one site |
| **P4** | The moved body still runs: `RING-ADOPT ran=` on the device arm is **> 0** and `on_vcpu=0`, and `GR-RING-JOIN` block heads stay within ±30 % of w752's **61** | `ran=0` (the arm never fired — starvation, the failure mode that looks like a hang) **or** `on_vcpu>0` on a deferring arm (the move did not take) |
| **P5** | Cut A alone moves `worst_trap` by **less than 20 %** | a large move from cut A alone ⇒ the plane lock WAS the cost and `LOCKCOST rank0` was lying |
| **P6** | No regression: `TRAP_FILLS=0` and `misses=0` on BAR1/BAR2, `named` ≈ 312 k, `RmInitAdapter failed!`=0 against a non-empty dmesg, `SMI_RC=0`, control `(P)` `THREADS 8 of 8`, `VCPU-BLOCKING total=102 doors=6` not worsened, failing-test set identical to the 30-name baseline | any of them moving |

⊘ **The raw client is `(R)` at HEAD on the device arm and another lane owns that.** It is
reported, never graded, and a red client may not be read as evidence for or against any row
above.

## The asserts, and constraint 29

Nothing is deleted. Two questions are **restated in the new shape**:

1. `an_armed_queue_head_write_records_and_returns` (w466/w469) asked *"does an armed
   queue-head write record instead of servicing?"*. Cut A moves where the record lives, so
   the successor asks the same question of the new home **and one more**: that the write
   takes **no rank-0 acquisition at all**. Fail-closed: the count must be visible to the
   worker, and the acquisition count must be zero.
2. `adopt_pending_channel_rings`' ordering obligation (*"before the birth drain"*) was
   asserted only by call-site order in one function. Its successor asserts it at the site it
   now lives in, on the arm that actually runs it.

## The known-positives this rung owes

- **The settlement really moved.** `RING-ADOPT ran= off_vcpu= on_vcpu= nothing_pending=`
  distinguishes *"nothing to adopt"* from *"the arm never ran"* — the distinction
  `a_census_zero_needs_a_known_positive` exists for. Broken on purpose by forcing the
  vCPU arm on a deferring boot and watching `on_vcpu` climb.
- **The guest's poll loop is not starved.** The guest polls its message queue for every RPC
  reply; the semaphore stays the source of truth and no eventfd replaces it
  (`await_semaphore`'s own warning). The evidence is `RmInitAdapter failed!`=0 on a
  non-empty dmesg — a starved poll loop cannot reach that.
- **The drain stays bounded.** `service_deferred_commands` already terminates on its own
  counter, never on guest data; cut A's fold-in preserves that (a `u32` posted count,
  saturating, drained to zero) and adds no loop that a guest can lengthen.
