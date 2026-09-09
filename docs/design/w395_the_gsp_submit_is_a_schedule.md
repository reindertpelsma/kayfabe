# ★★★★★ w395 — THE GSP SUBMIT IS A SCHEDULE: `NV_PGSP_QUEUE_HEAD(0)` validates, kicks, returns

**STATUS — 2026-09-09 — LIVE. Built on branch `w395-gsp-submit-async`, default `on`, measured
on a live GA106 (§6). Not merged; the owner's call.** Parents: `w383_the_doorbell_is_a_schedule.md`
(the channel-doorbell lane this mirrors), `publication_off_the_bql.md` (§5.2's *"no guest-visible
MMIO read depends on completed work"*, which §4 here narrows once more), and the w394 commit
`742b9e88` that found the register.

## §0 The rulings (verbatim, 2026-09-09) — the specification

> *"no inline blocking executions in mmio traps. just general rule. real gpu also never holds any
> mmio write for milliseconds right. like rpc mmio starts the operation, the block is for example
> a semaphore"*
>
> *"yes but submit register is a schedule, you should return to vm immediately and run it off the
> vcpu threads"*
>
> *"do not execute emulated channels under a doorbell"* · *"doorbell must be simple, it just reads
> a table, does ring host + return for passthrough or queues and wake thread for emulated, all non
> blocking, and returns."*
>
> ★★★★★ **Sharpened hours later, and it inverts the default:** *"the thing is to avoid a blocking
> call on vcpu thread at all, unless its required like a memslot install, even mmaps in vmm va can
> often run largely off vcpu thread"*.

⇒ The rule is an **allowlist**: nothing blocking runs on a vCPU thread unless it is on a short,
named, justified list. This rung moves one entry off the vCPU — the largest one measured — and
§5 writes down what is **left**, entry by entry, with the reason each cannot move yet. *"It was
already there"* is not a reason and is not used as one.

## §1 What was measured, and why this register

`[measured w394h, rev f365d831, RTX 3060 / GA106 / 580.159.04, fully armed]`

```
TRAPWITNESS off_trap_claims=11491 inline_exceptions=51
            worst_trap=1791581us at=bar0+0x110c00  slow_traps(>1000us)=380
```

`worst_trap` is `TrapGuard`'s bracket around **one guest MMIO access** (`shim_unsafe.rs` wraps
both `regs_read` and `regs_write`). `bar0+0x110c00` = `NV_PGSP_QUEUE_HEAD(0)` (`ga10x.rs:94`).
Its write classified to `BootStep::CommandDoorbell` (`seq.rs`), and `GspFsm::apply` ran
`self.doorbell(ram, policy)` — which **drained and serviced the whole GSP command ring**, policy
chain and host RM verbs included, inside the guest's store, under the BQL, every vCPU stopped.
**1.79 s**, and **380 traps over a millisecond** in one boot: a population, not an outlier.

★ The channel doorbell had already been fixed three ways (w383/w384/w394) and `worst_trap` went
1 927 527 → 1 855 594 → 1 791 581 µs — because the real holder was a register nobody was looking
at. That is the reason §6 grades on the **site** (`at=`) and not on the number alone.

## §2 Why deferring is safe — the guest-observable contract was always asynchronous

Hardware: `kgspSetCmdQueueHead_TU102` writes the head pointer and returns
(`ogkm-580: kernel_gsp_tu102.c:354`); the driver then waits in `_kgspRpcRecvPoll` on the
**response queue in its own RAM** (`kernel_gsp.c`, `GspMsgQueueReceiveStatus`). Nothing the guest
observes depends on the service having happened before the store retires.

⊘ **And the "status IRQ" was never the notification.** `ServiceReport::raise_status_irq` reaches
the C shell as `KayfabeRegWrite.raise_status_irq`, and the shell **refuses it by name**
(`qemu/hw/misc/nvkvm/nvkvm.c:603`, `irq_requests_dropped`). The only vector this device sends is
the os-event/completion path through `raise_cpu_intr`, hung off *work completing* at the channel
doorbell (§16.77.1), not off GSP register writes. So the contract the guest sees is: *post the
command; poll the response queue; find the reply.* Only the **servicing** was inline; the
notification was always "the reply lands in the ring". The worker therefore raises **nothing** —
a worker that sent MSI-X 0 "for the status queue" would be a *new* notification the control
never sent. The request is counted (`GSPQUEUE status_irq_requested=`) so the rate is visible.

## §3 The mechanism

```
guest store to bar0+0x110c00                                (vCPU, BQL held)
  → TrapGuard::enter_at(site)                               unchanged
  → RegPlane::write: decode_reg == GspQueueHead ∧ armed     ★ BEFORE state.lock() — round 2
      → gsp_gate (AtomicU8 snapshot, §4.3):
            HALTED  ⇒ fault QueueNotBound, faults+1          E8, zero RAM, zero locks  (§3.2)
            PREBIND ⇒ transitions=1                          E12, zero RAM, zero locks
            BOUND   ⇒ lane.kick()                            two compares + notify_one, leaf mutex
  → WriteOutcome { gsp_service_scheduled: true, commands: 0 }
  → return to VM entry

  (The FSM's own `SubmitMode::Deferred` arm — the same gate, reached through
   `mmio_write_with` — remains for callers that drive the FSM directly, e.g. the crec replay,
   and for the bind's B4 drain, which arrives through MAILBOX1 and does take the lock.)

kayfabe-gsp-submit worker
  loop wait_kick():
    loop:
      RegPlane::service_gsp_queue_pass(1)     ← takes the plane mutex for ONE command:
        fsm.service_deferred(ram, policy, 1)     gate again (E8/E12), then drain ≤1 command,
                                                 publish readPtr ack, maybe Running
        same counters the inline path bumped (irq_requests, commands, faults, ram_refusals)
      until a pass answers 0 commands (ring drained, or a message still being written)
    note_completed
```

The bind (`PublishBootArgs`, the `MAILBOX1` store) is deferred the same way: `INIT_DONE` is still
posted **in the store** — `kgspWaitForRmInitDone` polls for it — and the B4 backlog drain
(`GSP_SET_SYSTEM_INFO`, `SET_REGISTRY`, a policy pass with host verbs in it) is owed to the worker.

### §3.1 The lane is a LEVEL, and coalescing is correct here (where w383 refuted it for channels)

`w383` measured that coalescing **channel** doorbells is wrong, because the ring reader's cursor
is not re-read at execution. The GSP command queue is the opposite shape, and the FSM's own code
says so: `drain_commands` reads the guest's `writePtr` **fresh on every pass** and services every
complete element from our `readPtr` up to it, in ring order, committing the cursor per message.
The `QUEUE_HEAD` *value* is not consulted at all (`GspReg::GspQueueHead(_) => CommandDoorbell`).
⇒ N stores before the worker wakes and one store after the last are **one act**: a pass that
answers everything in the ring. Ordering lives in the ring, not in the kicks — `GspSubmitLane`
carries "there is work" as a level, and a level cannot reorder or lose a command.

The one hazard of a level is the **lost wakeup**: a kick arriving mid-pass, folded into a pass
that has already read `writePtr`. `kick` coalesces **only into an untaken kick**; a kick during a
pass stays pending and the worker goes round again (`the_kick_during_a_pass_is_not_lost`).

### §3.2 Where the validation runs — in the STORE, deliberately

`boot.rs`'s E8 (`GspFault::QueueNotBound` for a doorbell while `Unbound` in `Halted`) is the
stale-binding defence: a guest that dropped its queues and is still ringing. It runs **in the
store**, on both arms, through one shared `doorbell_gate()`, because:

1. it is *validation*, not execution — two enum compares on state the write already holds the
   lock for, zero guest RAM;
2. its refusal is **the guest's**, attributable to *this write* through the same `Err` path and
   the same fault counter as the control. A refusal raised from a worker has no trap to attach
   to;
3. the worker **re-runs the same gate** before reading anything (`service_deferred`), so a
   binding dropped between the kick and the pass — a teardown on another vCPU — is refused there
   by the same name. `GSPQUEUE prebind=` counts the E12 form of that.

`kayfabe-qemu-raw/tests/gsp_submit_schedule.rs` pins both: E8 refuses in the store with **zero
kicks**; E12 kicks nothing.

### §3.3 Fail-closed — by DISARMING, not by declaring the boot unmeasurable

The channel lane's worst failure was *"armed with nothing draining it"*, and its answer was to
say so loudly. Here the guest is **synchronous under its GPU lock** — one RPC in flight — so a
stranded kick is not a backlog, it is a driver blocked in `rpcRecvPoll` until its timeout. So a
spawn failure **puts the FSM back in `Inline`** (the control, never worse than the status quo)
*and* runs one full inline pass right there so nothing already kicked is stranded, then says at
maximum volume that the boot is measuring the control from that point on.

## §4 ⊘⊘ THE RELOCATION HAZARD — and the two changes that exist only because of it

Moving the service to a worker that holds the plane mutex would simply **move** the 1.79 s if any
guest MMIO access during the RPC wait needed that mutex. Two do:

1. **`_kgspRpcRecvPoll` reads MMIO on every iteration.** `_kgspRpcDrainEvents` calls
   `kgspHealthCheck_HAL` (`kernel_gsp.c:1827`); CrashCat is configured for the GSP falcon with
   `bEnable = NV_TRUE` (`kernel_gsp_tu102.c:79`), so the health check reads the CrashCat
   wayfinder scratch register — an offset **no `GspReg` names**, which `read_inner` answered as
   `Unclaimed` *after taking the plane lock*. ⇒ Every poll iteration would have parked the vCPU
   behind the worker.
   **Fix:** `BootSequence::answers_unnamed_reads()` (false for the falcon regime, `true` for
   `Gh100FspBoot`, which does answer unnamed offsets), and `read_inner` classifies an unclaimed
   offset **lock-free** when the sequence cannot answer it — the exact `None` the locked arm
   would have returned, because `mmio_read_with` consults the model first and the sequence
   second, and both are consulted here. The diagnostic samples (`unclaimed`, `fb_window`) moved
   out of `PlaneState` onto a leaf mutex for the same reason (`note_unclaimed` took the plane
   lock too).
2. **Claimed GSP reads, the interrupt tree and the BAR0 window still take the mutex.** They are
   not on the RPC-wait path, but an ISR on another vCPU can read `CPU_INTR` while a pass runs.
   **Bound, not removed:** one command per lock hold (`GSP_PASS_MAX_COMMANDS = 1`), so the wait
   is one command's service — the unit the C serviced per doorbell anyway. `GSPQUEUE
   worst_hold_us=` measures exactly this, and §6 reads it.

3. **The store itself** — round 1's finding (§6.0): classifying the armed store under
   `state.lock()` queued it behind the worker's current command, and the 1.8 s did not move.
   **Fix:** `RegPlane::gsp_gate`, an `AtomicU8` snapshot of the FSM's gate
   (pre-bind / bound / halted) refreshed under the mutex at every site that can move `queue`
   or `phase` — the write path (both result arms), `device_reset`, the worker's pass, arm and
   disarm. The armed store reads the snapshot and never takes the mutex. The one race (a store
   reading `BOUND` an instant before a teardown on another vCPU) degrades to a kick the worker
   re-gates and counts (`prebind=` / `faults=`); the authority is the worker's gate, the
   snapshot only decides whether the *store* refuses loudly on the vCPU, which it does in every
   non-racing case exactly as the control did.

⚠ `publication_off_the_bql.md` §5.2's finding *"no guest-visible MMIO read depends on completed
work"* stays true; what this section adds is that a read can depend on a **lock** the completed
work holds, which is a different sentence and was invisible while every service ran on the vCPU.

## §5 ★★★★★ THE ALLOWLIST — what still blocks on a vCPU thread after this change

Enumerated from `Regs::write` (`shim.rs`) and `RegPlane::write` (`plane.rs`) as they stand on
this branch, on the shipping arms. **Each entry is either justified or named as work to move.**
The list has more than a couple of entries; that is a finding, stated plainly.

| # | what runs on the vCPU | why it is still there | status |
|---|---|---|---|
| 1 | **Memslot install / materialize** (`self.device.materialize_pending()`, the Q5 join) | KVM requires memslot changes on a vCPU-stopped, BQL-held context — the owner's canonical allowlist member | **ALLOWED** |
| 2 | **TLB-invalidate blockage point** (`KAYFABE_MMU_INVAL=on`: publish under the guest's own `TRIGGER` spin) | The guest is *already* spinning on the register by protocol; the owner's publish-trigger ordering ranks this exact boundary first (`publish_trigger_preference_ordering.md`). It is bounded by the guest's own timeout and measured (`MMUINVAL worst_hold_us`) | **ALLOWED, measured** |
| 3 | **Budgeted retired-drain + pin reclaim** (`drain_retired_budgeted`, `pin_reclaim_gone`, 40 ms budget) | Revocation (valid→invalid) may not be deferred — a guest omission holds a window open (`publication_off_the_bql.md` §4). Budgeted to 1 % of the shortest named guest timeout | **ALLOWED, bounded** — candidate to shrink |
| 4 | **Ring adoption** (`adopt_pending_channel_rings`: fb-leaf resolve + `join_one_fb_leaf`, a host verb) | Runs on the trap after *any* write. ⊘ No reason it must be on the vCPU except that the channel's host object is born on the next verb — it is a **candidate to move** to the doorbell worker's pass, which already runs before the forward | **TO MOVE** |
| 5 | **Notifier/birth grants + forward drains** (`pending_err_notifier_grants`, `pending_birth_notifier_grants`, `report_*_drain`) | Host verbs reached from the trap. Same status as 4 | **TO MOVE** |
| 6 | **The bind's own reads** (`PublishBootArgs`: LibOS region-array walk, region page table, `INIT_DONE` post) | Bounded guest-RAM reads, no host verb, once per driver life; `INIT_DONE` must be visible before the store retires because `kgspWaitForRmInitDone` polls for it immediately | **ALLOWED, small** |
| 7 | **Plane-mutex waits behind a worker pass** (claimed GSP reads, `CPU_INTR`, BAR0 window) | Bounded to one command's service by `GSP_PASS_MAX_COMMANDS = 1` (§4.2). The full fix is a split of the FSM's read shadow from its service state | **BOUNDED; measured as `worst_hold_us`** |
| 8 | **Channel doorbell `Offered::Full` fallback** (the w383 lane's cap) | Runs inline only when the lane is saturated at 4096 distinct tokens; counted as `PUBQUEUE refused=` | **ALLOWED at the cap, must read 0** |
| 9 | **`kftime`/`trapwitness` bookkeeping** | Atomics and a capped table; not blocking | n/a |

⇒ Entries **4 and 5** are the next things to move, and `worst_trap at=` after this change is
what says whether they bind (§6).

★★★★★ **This table is now the census, not a description of it** (round 3, after merging
`8a84ef9f`): every row marked ALLOWED/BOUNDED/TO MOVE above goes through
`kayfabe_util::lock::BlockingSection` — entry 1 through `enter_required_on_vcpu` (the
allowlist door, the only one), entries 2–5 and the control's inline GSP service through
`enter` — and `VCPU-BLOCKING [n × what (ALLOWLISTED | ⊘ NOT ALLOWLISTED)]` prints on the
per-doorbell `PT-DECODE` line beside `TRAPWITNESS`. A row here that the census does not print
is a row that stopped being true.

## §6 The measurement

Box `w393` (vast 50376491, RTX 3060, KVM template), binary built from this branch at
`748092c6` and **verified by content** (`strings … | grep -c GSP-SUBMIT-ASYNC` = 5; the rev stamp
is empty because the box checkout is an rsync without `.git`). Fully armed per the brief
(`KAYFABE_ISOLATES=real … KAYFABE_CE_EXECUTOR=host NVKVM_RAM_MB=16384`), **2 boots per arm,
interleaved on/off/on/off**, strictly serial, one QEMU at a time.

The arm that RAN is read back from the device's own `GSP-SUBMIT-ASYNC arm=` line, never from the
environment that asked.

### §6.0 ⊘⊘ ROUND 1 (`748092c6`, the first cut) — CORRECT, AND THE TRAP DID NOT MOVE

Four boots, interleaved, all four `W392D_OUTCOME=(P)` / `THREADS 4 of 4 verified` /
`MEAN_FALSIFIER=PASS`, `Xid=0`, `rpcRecvPoll=0`, `RmInitAdapter failed=0`. The guest's
`NVRM` line counts (26 in the boot dmesg, 58 after the workload — `GspRmFree … 0x56` and the
`rs_server.c` asserts that follow it) are **identical on both arms** and therefore pre-existing.

| boot | arm ran | `worst_trap` | `at=` | `slow_traps(>1000us)` | `off_trap_claims` / `inline_exceptions` |
|---|---|---|---|---|---|
| `w395c_on_1`  | on  | 1 802 444 µs | `bar0+0x110c00` | 87 | 135 / 31 |
| `w395c_off_1` | off | 1 884 842 µs | `bar0+0x110c00` | 79 | 134 / 31 |
| `w395c_on_2`  | on  | 1 880 027 µs | `bar0+0x110c00` | 84 | 133 / 31 |
| `w395c_off_2` | off | 1 872 005 µs | `bar0+0x110c00` | 65 | 134 / 31 |

⇒ **INCONCLUSIVE on timing (the ranges overlap) and CONCLUSIVE on mechanism**: the first cut
deferred the service but still classified the store **inside `state.lock()`** — `RegPlane::write`
took the plane mutex before reaching the FSM's gate — so the guest's next `QUEUE_HEAD` store
queued behind the worker's current command. The 1.8 s moved from *executing* the command to
*waiting for the lock the executor held*. Same site, same duration, one lock over. ★ This is the
owner's warning arriving in its purest form: a deferral whose entry point takes the lock the
worker holds is not a deferral.

Also learned: the teardown census never prints on this harness (the guest is powered off and
QEMU exits without `detach_ram`), so `GSPQUEUE` now rides the per-doorbell `PT-DECODE` line the
way `PUBQUEUE`'s depth does; and the hook's verdict lines land in `run_<tag>_probe.log`.

### §6.1 ROUND 2 — the lock-free gate (`RegPlane::gsp_gate`)

The armed store now consults an atomic snapshot of the gate (bound / pre-bind / halted),
refreshed under the mutex at every FSM mutation site (bind, teardown, reset, worker pass), and
**never takes the plane mutex**: E8 refuses, E12 classifies, bound kicks — all lock-free.

Four boots, interleaved, binary content-verified (`ARMED WITH NO LANE` string = 1), base
`742b9e88` + this branch at `e8e23283`. All four `W392D_OUTCOME=(P)` / `THREADS 4 of 4
verified` / `MEAN_FALSIFIER=PASS`, `Xid=0`, `rpcRecvPoll=0`, `RmInitAdapter failed=0`.

| boot | arm ran | `worst_trap` | `at=` | `slow_traps` | `GSPQUEUE` (armed arm) |
|---|---|---|---|---|---|
| `w395c_r2_on_1`  | on  | 1 743 823 µs | `bar0+0x110c00` | 50 | `queued=491 coalesced=0 taken=491 passes=984 commands=493 prebind=0 faults=0 status_irq_requested=984 completed=491 depth=0 high_water=1 worst_hold_us=1123 worst_drain_us=1127` |
| `w395c_r2_off_1` | off | 1 924 058 µs | `bar0+0x110c00` | 87 | (control — no lane) |
| `w395c_r2_on_2`  | on  | 1 841 164 µs | `bar0+0x110c00` | 83 | `queued=491 … commands=493 … worst_hold_us=852 worst_drain_us=856` |
| `w395c_r2_off_2` | off | 1 877 619 µs | `bar0+0x110c00` | 85 | (control — no lane) |

★★★★★ **Read the two halves separately, because they say different things.**

1. **The GSP plane is off the vCPU, completely and safely.** 491 kicks, 493 commands, zero
   coalesced (the guest is synchronous — one RPC in flight — so a level never folds), zero
   faults, zero pre-bind kicks; the longest the worker ever held the plane mutex for one
   command was **1.1 ms** (`worst_hold_us=1123`, then 852), and the longest drain of one wake
   was the same — i.e. every kick was one command. The store itself is lock-free.
   Correctness held on every boot.
2. ⊘⊘ **And `worst_trap` did not move — so it was never the GSP service.** With the ring
   serviced entirely on the worker, one `bar0+0x110c00` trap still lasted 1.7–1.8 s on both
   arms (ranges overlap: on 1.74–1.84 s, off 1.88–1.92 s — **INCONCLUSIVE** as a timing
   result). The store's own work is now two atomics and a `notify_one`. What remains inside
   `TrapGuard`'s bracket at that site is **`Regs::write`'s post-plane drains** — materialize
   (memslot install), ring adoption, notifier grants, the birth/forward drains, the
   reclaim/retired drain — which run after *every* register write and ride whichever write
   arrives while their work is pending. `QUEUE_HEAD` is the write that arrives then, because
   the guest's RPC reply is what makes that work pending. §1's attribution of the 1.79 s to
   *"the queue drained inline"* was an inference from the site; the site was right and the
   inference was wrong. §6.2 attributes it by segment.

### §6.2 Segment attribution (`KAYFABE_KFTIME=census`, one boot per arm)

`w395diag_on` (arm `on`, `KAYFABE_KFTIME=census KAYFABE_KFTIME_CENSUS_EVERY=100`, raw client
`(P)` / `4 of 4` / `MEAN_FALSIFIER=PASS`, `worst_trap=1877907us at=bar0+0x110c00`), the last
census per segment of the write path, sorted by `max_us`:

```
KFTIME-SEG materialize   shape=host  n=540600  total_ms=2041.918  mean_us=3   max_us=1877849  share=41.8%
KFTIME-SEG ring_adopt    shape=work  n=540600  total_ms=155.990   mean_us=0   max_us=42195    share=3.2%
KFTIME-SEG fwd_drain     shape=host  n=540600  total_ms=136.352   mean_us=0   max_us=36205    share=2.8%
KFTIME-SEG reap          shape=work  n=539969  total_ms=241.865   mean_us=0   max_us=22378    share=5.0%
KFTIME-SEG plane         shape=work  n=540600  total_ms=895.391   mean_us=1   max_us=13158    share=18.3%
KFTIME-SEG plane_read    shape=work  n=21500   total_ms=77.044    mean_us=3   max_us=13129    share=83.8%
KFTIME-SEG birth_drain   shape=work  n=540600  total_ms=80.064    mean_us=0   max_us=10778    share=1.6%
KFTIME-SEG err_grants    shape=work  n=540600  total_ms=4.064     mean_us=0   max_us=65       share=0.1%
```

★★★★★ **`worst_trap` IS `materialize_pending` — the memslot install** (`max_us=1877849` against
`worst_trap=1877907us`: the same event to 58 µs). That is the owner's own canonical allowlist
member — *"unless its required like a memslot install"* — arriving on the `QUEUE_HEAD` store
because the RPC reply the guest has just drained is what makes a memslot pending. It is
**allowed** and it is **1.9 s**, which is worth its own rung (which memslot, and whether the
16 GiB `memfd` guest RAM or the w393 BAR mirror is the one that costs it); it is not this one.

⊘⊘ **And the CONTROL says the same thing** (`w395diag_off`, arm `off`, `(P)` / `4 of 4` /
`PASS`, `worst_trap=1875321us at=bar0+0x110c00`): `materialize max_us=1875049`, and the `plane`
segment — which on the control INCLUDES the inline GSP command service — peaks at **13.3 ms**
(`n=540600 total_ms=806.238`, against `895.391` on the armed arm: the same population to within
noise). ⇒ **The inline service never cost more than ~13 ms per store on either arm.** w394's
sentence *"drains and services the whole GSP command queue inline … that is the 1.79 s"* was an
inference from the trap's site to the trap's cause, and the cause was a different frame in the
same bracket. This rung implements the ruling exactly as specified and measures it as such;
what it does **not** do is move `worst_trap`, because `worst_trap` was never the thing the
ruling named at this register. ★ Same class as the campaign's *"a site is not a cause"* — the
instrument that settles it is `KFTIME-SEG`, which existed all along and was not armed for w394.

⇒ On the armed arm the FSM's whole share of the store (`plane`, which includes the bind and
every other claimed register) peaks at **13.2 ms**, and the GSP command ring contributes
nothing to it (`GSPQUEUE worst_hold_us` 0.85–1.1 ms, on the worker). The remaining
not-allowlisted residue in the bracket is `ring_adopt` (42 ms max), `fwd_drain` (36 ms),
`reap` (22 ms, budgeted), `birth_drain` (11 ms) — §5's entries 3, 4 and 5, now with numbers.

### §6.3 Round 3 — merged master (`281ba010`…) + the `VCPU-BLOCKING` census

[TO FILL — ledger lines + `VCPU-BLOCKING [n × what (ALLOWLISTED | ⊘ NOT ALLOWLISTED)]`]

### §6.5 Perf (`gpu_bench`) — reported as RANGES, n=2 per arm

[TO FILL, or INCONCLUSIVE if the ranges overlap]

## §7 What is NOT measured

- The worker racing the guest: `cap1_deferred_submit.rs` proves the deferral changes nothing the
  guest can observe **in a total order**; only the live boots exercise the interleaving, and n=2.
- `QueueFull` back-pressure on the worker: the same retry the inline arm had (the next kick
  re-reads the refusing message), untested live because this guest is synchronous.
- Anything on the GH100 regime: `answers_unnamed_reads() == true` keeps its locked read path
  byte-identical, unmeasured.
- Entries 4/5 of §5 as their own trap sites — only `at=` after this change can rank them.

## §8 Files

- `crates/kayfabe-gsp/src/boot.rs` — `SubmitMode`, `ServiceReport::service_owed`,
  `doorbell_gate`, `service_deferred`, `service_command_queue_bounded`.
- `crates/kayfabe-device/src/gspsubmit.rs` — the lane. `plane.rs` — `arm_gsp_submit`,
  `service_gsp_queue_pass`, the lock-free unclaimed read, `Samples`.
- `crates/kayfabe-arch/src/gsp.rs` — `BootSequence::answers_unnamed_reads`; `gh100.rs` override.
- `crates/kayfabe-qemu-raw/src/shim.rs` — `KAYFABE_GSP_SUBMIT_ASYNC`, the worker, the census.
- `crates/kayfabe-crec/src/replay.rs` — `Replay::with_deferred_submit`;
  `tests/cap1_deferred_submit.rs`. `crates/kayfabe-qemu-raw/tests/gsp_submit_schedule.rs`.
