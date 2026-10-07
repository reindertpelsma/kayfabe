# w754 — the GSP command-queue doorbell, and what is ACTUALLY on the vCPU behind it

**STATUS: LIVE, 2026-09-16 — MEASURED. See §MEASURED at the end; the pre-registration below
is kept verbatim and its six predictions are resolved there, two of them REFUTED.**

Pre-registration, written and committed **before the first
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


---

# ★★★★★ MEASURED — 2026-09-16

`[vast 51217315, GA106, 580.159.04 OPEN, three arms, ONE binary per run, control first;
traces/w754_gsp_queue_off_vcpu/]`

Two runs, because run 1 found a **third** cost the pre-registration did not know about and cut
C is the answer to it.

| | baseline **w752** (`fcce2a11`) | run 1 **A+B** (`8286ee65`) | run 2 **A+B+C** (`018c41d4`) |
|---|---|---|---|
| **control** (`arena`) | `25 077 µs @0x110c00` | `45 580 µs @0x110118` | ★ **`920 µs @0xb81608`**, `slow_traps=0` |
| **device** (graded) | `24 999 µs @0x110c00`, cpu `23 979` | `53 345 µs @0x110118` | ★ **`9 649 µs @0xbb0090`**, cpu **`4 621`** |
| third arm | `split` `30 474 µs @0x110c00` | `inline` `46 136 µs @0x110118` | `inline` `17 976 µs @0xb81208`, cpu `711` |

**Device arm: 24 999 µs → 9 649 µs (2.59×), and the 10–100 ms decade is EMPTY
(`by_decade[1-10ms=5 10-100ms=0]`, was `[5, 1]`).**
**At the register the brief named: `bar0+0x110c00` 24 999 µs → 1 579 µs, 15.8×.**
**Control arm: 25 077 µs → 920 µs, 27×, and `slow_traps(>1000us)=0` — not one trap over a
millisecond in the whole boot.**

## The three cuts, each with its own A/B

### Cut A — the queue-head write takes no rank-0 lock

`GSP-HEAD posted=545 folded=545 in_flight=0` on the device arm (`563/563` on the control), on
both runs. **Every doorbell the trap posted was folded in; nothing was parked.**
⊘ **Prediction P5 HELD**: cut A alone moved `worst_trap` by less than 20 % — in fact it moved
it not at all, because the plane lock was never what the 25 ms was. `LOCKCOST rank0
worst_wait` fell 8 790 µs → 911 µs anyway, which is cut A being worth something other than the
headline.

### Cut B — the ring adopt's page-table settlement, off the vCPU

★ **One env var, same binary, same box** (`KAYFABE_MATERIALIZE_INLINE=1` forces the body back
onto the vCPU). Run 2, arms 2 vs 3:

| | settlement OFF the vCPU | settlement ON the vCPU |
|---|---|---|
| `RING-ADOPT` | `ran=21 off_vcpu=21 on_vcpu=0` | `ran=28 off_vcpu=7 on_vcpu=21` |
| `bar0+0x110c00` | **1 trap, worst 1 579 µs** | **3 traps, worst 15 336 µs** |
| `slow_traps(>1000us)` | **5** | **10** |
| `VCPU-BLOCKING` | **`total=102 doors=6`** | `total=111 doors=9` |
| `inline_exceptions` | **0** | 5 |

⊘ Run 1 showed the same A/B independently: `0x110c00` 1×2 183 µs vs 6×17 616 µs.
⇒ **P4 HELD** (`ran>0`, `on_vcpu=0`, and the counter is shown to move on the other arm);
**P3b HELD** — `0xb81408 / 0xb81410 / 0xb81608 / 0xb81610` left `SLOW-SITES` with it. They
carry no per-register work that could cost milliseconds; they were the same code at a
different address.
⊘ **P2 REFUTED as written**: `0x110c00` did not leave `SLOW-SITES` — it is still listed once,
at 1 579 µs. The 25 ms is gone; the register is not.

### Cut C — the 46 ms the pre-registration did not predict at all

Run 1's worst trap moved to `bar0+0x110118` (`NV_PGSP_FALCON_DMATRFCMD`) at **45–53 ms on all
three arms**, `cpu_of_that_trap` ≈ wall (**100 % CPU**), **zero context switches**. One boot
with `KAYFABE_STALL_ALARM_US=15000 KAYFABE_STALL_ALARM_AT=110118` named the frame
(`w754_stall_backtrace.txt`): a `OnceLock<Vec<u64>>` initialising
`(0..regs_aperture_len).step_by(4).filter(decode_reg)` — **4 194 304 evaluations over a 16 MiB
aperture** — inside `publish_gsp_registers`, inside `RegPlane::write`, inside
`kvm_vcpu_thread_fn`.

⊘⊘⊘ **And the fix was already in the same function, for the twin sweep.** w573 ends
`with_objects` with `plane.dead_pages = plane.build_dead_page_bitmap();` and the comment
*"deferring it put 4.2 million predicate evaluations under a halted guest."* Same count, same
aperture, same file, three functions apart.
★ `publish_gsp_registers`' own doc bounded the cost — *"by the chip's register map, not by
traffic"* — which is true of the **loop body** and false of the **sweep that builds the list it
iterates**. ⚠ A bound stated about the wrong quantity reads exactly like a bound.

⇒ P1 is **moot**: the KFTIME segment census was never run, because the stall alarm answered
the attribution question outright and the A/B answered the causal one. Recorded as not-done
rather than quietly dropped.

## ⊘ P3 REFUTED — `worst_trap` is NOT sub-millisecond on the device arm. The new worst, named.

    device: worst_trap=9649us at=bar0+0xbb0090   cpu_of_that_trap=4621us   (48 % CPU)
            LOCKCOST rank0 worst_wait=9625us worst_hold=701us slow_waits=3
                     worst_wait_blocked_by=crates/kayfabe-device/src/plane.rs:3286
            SLOW-SITES bar0+0xbb0090=3(worst 9649us) bar0+0x1700=2(worst 1179us)
                       bar0+0x110c00=1(worst 1579us)

★★★ **And it is a DIFFERENT KIND of violation, which is exactly what `cpu_of_that_trap` is
for.** The three this rung fixed were 96–100 % CPU; this one is **48 % CPU / 52 % wait**, and
rank 0's own census names the blocker: `plane.rs:3286` is `RegPlane::window_page_backing`,
which takes **both** `state` and `mem` to resolve a framebuffer window page. `bar0+0xbb0090` is
the **usermode doorbell** — the register a CUDA process rings.

⇒ The next cut is **lock scope, not thread placement**. Moving more work off the vCPU cannot
help a trap that is already waiting for a lock a worker holds; the settlement's reader
(`PlanePtBytes::read_in`, `plane.rs:2267`, the worst *holder* at 701 µs) and the window
resolver need to stop sharing rank 0 with the doorbell's path. ⚠ Do not read this rung's
success as licence to repeat its method here.

## P6 — no regression, on the graded arm, run 2

`TRAP_FILLS=0` and `misses=0` on **both** BAR1 and BAR2 · `named=312390` (w752: 311 180) ·
`RmInitAdapter failed!`=0 **asserted against a non-empty dmesg** (5 597 bytes, **33 `NVRM`
lines**) · `SMI_RC=0` · `MODPROBE_RC=0` · `HOST_DMESG_XID=0` · control `(P)` `THREADS 8 of 8`
`MEAN_FALSIFIER=PASS` · `VCPU-BLOCKING total=102 doors=6` — **identical to w752, not worsened**.

⊘ **The raw client is `(R)` `THREADS 0 of 8` on the device arm, as it is at HEAD.** Another
lane owns it. Reported, not graded, and it is `(R)` on the known-positive arm too — so it
cannot be hiding a difference between them.
