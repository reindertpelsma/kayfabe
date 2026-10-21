**STATUS: RESEARCH, 2026-10-21.** Race / correctness audit of kayfabe v3 at `origin/master` e64b8e5f6, branch `claude/audit-race-20261021`. Read-only on product code, GPU-free,
nothing run on hardware. Product code is unchanged; the only code added is three `EXPECTED RED` tests (section 9). Evidence labels: **CODE READ** (file:line at e64b8e5f6, nothing run),
**MEASURED** (a test in this branch was run; its name is given), **INFERRED** (reasoning, never evidence). Where a finding came from a delegated lane and the auditor did not re-read the
lines himself it is tagged `lane` and keeps the lane's label; `verified` means the auditor re-read the cited lines.

# V3 race / correctness audit (2026-10-21)

## 0. How this was done (and what was not)

* The auditor (Sonnet 5.5) read the worker core himself: `kf-chan/src/worker.rs`, `handoff.rs`, `user_loop.rs`, `kf-core/src/plane.rs`, `kf-trap/src/{wake,token,bitmap,trap,ring}.rs`,
  `kf-qemu/src/{ptlock,userrelay,userquota}.rs`, `userpump.rs` 565-640 and 2685-3260, `device.rs` worker/drainer/stop/bar0 paths, `chan.rs` `serve`/`alive`/`retire` sites,
  `ffi_unsafe.rs`, `dbfast.rs` `service_ready`, the event-loop design doc and the two prior audits (failure catalogue 81 rows, blocking audit W1-W20).
* Six read-only Sonnet 5.5 lanes swept the rest in parallel (lock graph; VA manager; atomics/orderings/seqlocks; reset/shutdown/unload/S3; guest-memory TOCTOU + cursor arithmetic;
  process-level + C device + dbfast + broker). Their findings are included with their own evidence labels. The auditor re-read the lines of every finding ranked HIGH, of the top five, and of
  the ones used to build the red tests (marked `verified`). **No lane result was ever run**; only the three tests in section 9 were.
* Not re-audited at all: lifecycle machine logic (`user_lifecycle.rs`, LIFE-D1..D6), display plane internals (`display.rs`, `kf-disp`, 25 % read), `kf-broker` (grep + the socket code only),
  `kf-gsp` beyond queue parsing and held replies, `kf-rm` policies, CUDA walker `.cu`, the stress harness (`relay_stress.rs` is **not on master**; it lives on the relay-simple lineage).
* Existing evidence re-run on master: `cargo test -p kf-chan --test handoff_protocol` 6/6 and `--test relay_fairness` 35/35 green (debug, 221 s). They model an idealised loop (see 4.3 for what
  they do not model).

## 1. Result in one screen

| severity | count | ids (merged duplicates counted once; full index in 2.5) |
|---|---|---|
| CRITICAL | 0 | none confirmed (VA-2 becomes CRITICAL if the guest issues ALL_PDB invalidates on a system with more than 62 GPU address spaces: UNCERTAIN Q2) |
| HIGH | 5 | VA-4, RC-03, VA-2, VA-1, AT-1 |
| MEDIUM | 27 | RC-01, RC-04, RC-05, RC-10, RS-1, RS-3, RS-5, RS-6, AT-2, AT-3, VA-6..VA-11, PR-1, PR-2, PR-3, PR-5, CU-5, LK-3..LK-8 |
| LOW | 42 | RC-02, RC-06..RC-09, RC-11, AT-4/6/7, VA-12..17, RS-4/7/8/9/10, PR-4/8/9/10/11, GM-2/3/5/6/7/8, CU-2/3/4/6/7/8/9, LK-9..LK-12 |
| INFO | 18 | AT-5/8/9/10/11, VA-18/19, GM-1, CU-10/11/12, RS-11/12, PR-13/14, RC-12, LK-13/14 |

Rule-3 violations (owner rule "nothing blocks on a vCPU or under a lock a vCPU takes; the drainer never blocks") are marked **RULE** below: PR-1, VA-10, LK-3, LK-4, LK-5, LK-7, LK-8, LK-9, PR-2.
No lock-order CYCLE exists (section 3).

**Top five (risk to the owner's freezes and to the hostile-guest model, not count):**

1. **VA-4 (HIGH, lane + auditor-read amplifier)** an invalidate trigger that is left armed (root moving during a walk, and every other one-shot "unreconciled" path) is never retried, and while ANY
   trigger is armed `walk_busy` stays true, which stalls every Windows USER relay publication and every Translated waiter of every process (`device.rs:2391`, `userpump.rs:577,712`): one stuck
   trigger is a guest-wide freeze (the 2026-10-09 mechanism, catalogue VA-ARMED-FOREVER, still open).
2. **RC-03 (HIGH, SECURITY, MEASURED)** the rotating scan start is a no-op for the 12-bit token index: 64 hot low tokens starve every higher token for ever (`bitmap.rs:94`, red test).
3. **VA-2 / VA-1 (HIGH, SECURITY, CODE READ)** more than 64 rooted spaces in one walk, or one over-large diff, makes `refuse_batch` / a failed report leave EVERY invalidate in the batch armed
   and kill every split; no retry changes the next identical batch.
4. **AT-1 (HIGH, verified)** `PrivRing::push` loads `tail` then `head`; a stale `tail` makes `tail - head` underflow, the ring poisons stickily and every later privileged write is dropped
   (`ring.rs:110`).
5. **RC-01 (MEDIUM, verified)** a relay's doorbell turn that loses `record.enter()` to a PEER worker's look is dropped with only `Cell::retry` set; no look or holder re-rings it
   (`userpump.rs:2794`, `2980`); the 50 ms net does not rescue it either (`look_needed` ignores demand). Same family as the baton (F1); zero probability with one worker.

**UNCERTAIN headline (section 7):** whether the NVIDIA event fds are consumed by the first `epoll` poll (so the second worker never sees the event the sleep-safety table says reaches
every worker), whether the guest issues ALL_PDB with more than 62 spaces, and whether `Peek::Event` setting `unannounced` for UNRELATED engine events (RC-04) keeps both workers spinning on a
loaded host.

**Red tests (section 9):** `audit_scan_limit_lost_wake` (RC-02), `audit_scan_starvation_same_summary_word` (RC-03), `audit_dbfast_service_ready_is_bounded` (PR-2): all three RED as run.

## 2. Findings, ranked

Format: **id (severity)** `file:line` - trigger - consequence - guest-triggerable - evidence - fix - test. `SECURITY` = a guest can trigger it deliberately and it harms other processes of
the guest, the host, or the management plane (self-only harm is noted as such).

### 2.1 HIGH (in the order of the top-five list)

**VA-4 (HIGH, lane CODE READ; catalogue VA-ARMED-FOREVER)** `kf-mem/src/vasmgr.rs:1479-1488,1756-1766`; amplifier `kf-qemu/src/device.rs:2391-2394`, `userpump.rs:577,712`.
*Trigger:* the guest moves a space's root while a walk is in flight: `Want::Root` is queued and `rewalk` set, the invalidate branch requeues only on `partial`, `absent_ok` excludes
`rewalk`, so the trigger lands in the `unreconciled` branch and stays armed; the follow-up `Want::Root` carries no trigger. All other unreconciled paths (refused unmap, slot overflow,
submit/poll error, settle failure: `vasmgr.rs:1118,1361,1410,1489,1761,1787`) are one-shot (`device.rs:2180-2185` observes each seq once). *Consequence:* the guest spins on the trigger;
and because `walk_busy` is true while any trigger is armed (`device.rs:2391`), Windows USER relays never publish (`userpump.rs:577,712`) and Translated-channel waiters are never rung
(`device.rs:2398`): ONE stuck trigger stalls every process. *Guest-triggerable:* the root move needs the guest kernel (SET_PAGE_DIRECTORY via RM) - guest root, or a userspace action that makes
RM do it (INFERRED). *Fix:* treat `rewalk` like `partial` for invalidates (requeue); give every unreconciled path a bounded retry or a loud give-up that clears over absence; do not count a
stuck trigger as "walk busy" for other spaces. *Test:* invalidate in flight, `set_root` Moved, `on_walk_ready`: busy then the second walk clears; retry-to-clear bound test.

**RC-03 (HIGH, SECURITY, MEASURED, verified)** `kf-trap/src/bitmap.rs:94-130` (`scan`, `next_start`), `kf-qemu/src/device.rs:753` (`Plane::for_device` -> `TokenIndex::RunlistVector`, token = `runlist << 11 | chid`, `kf-trap/tokenindex.rs`).
*Trigger:* a guest process owning >= 64 channels on the routes that publish bits (Translated / Emulated / USER-ring relay) with tokens in one bitmap word (e.g. GR runlist chids 0..63) rings them
all before every worker scan. `scan()` takes the lowest `limit` (= `SCAN_LIMIT` 64) set bits in ascending order. The fable-review fix rotates only the first SUMMARY word, and a summary
word is 4096 tokens = TWO whole runlists (e.g. every GR channel of the guest, chid 0..2047, plus the next runlist), so inside it the order is still strictly ascending: a victim in the next bitmap word
(here token 100) is never scanned. *Consequence:* starvation of every higher-numbered channel in the same two runlists for as long as the attacker keeps ringing - other users' processes,
and kernel channels whose chid is above the attacker's (chid recycling puts new kernel channels into low free slots). Two workers dilute this (the second scanner can run between the attacker's
re-rings) but do not remove it; with ONE worker (section 6) there is no dilution. *Guest-triggerable:* yes, unprivileged guest userspace (needs the channel budget, not privilege).
*Evidence:* MEASURED: `kf-trap/tests/audit_scan_starvation_same_summary_word.rs` is RED (victim never scanned in 1000 passes, single scanner); the existing
`an_unprivileged_process_cannot_starve_the_kernels_channels` passes only because its victim (9000) sits in a different summary word. *Fix:* resume the scan at the bitmap WORD after the last
returned token (keep `next_start` at word granularity) so the lowest word cannot monopolise a scan. *Test:* the red test above plus the RC-02 test.

**VA-2 (HIGH, SECURITY, lane CODE READ, spot-verified)** `kf-mem/src/vasmgr.rs:68,1258,1323-1332,1386-1397`; `max_slots = 128` (`kf-cuda walk.rs:195`).
*Trigger:* an ALL_PDB invalidate (or `Split{pdb:None}`) resolves to `table.rooted()`; with more than 64 rooted spaces (more than 62 guest spaces plus the BAR1/BAR2 apertures) `batch.walked.len() >
MAX_SPACES_PER_WALK` and `refuse_batch` runs. *Consequence:* every invalidate in the batch stays armed (the guest polls the trigger under its GPU locks: the run-223 hang), every split gets
`Err` (kills its channel), queued `Want::Root` is dropped; it repeats on every ALL_PDB. *Guest-triggerable:* yes - an unprivileged process opening ~64 GPU contexts; the issuing of ALL_PDB
by the guest driver is INFERRED (UNCERTAIN Q2). *Fix:* submit in chunks of <= 64 and clear the trigger when every chunk landed. *Test:* `vasmgr.rs` test with `walker.nslots = 100`, 70
rooted keys, one ALL_PDB: all submits <= 64 entries and the trigger clears.

**VA-1 (HIGH, SECURITY, lane CODE READ)** `cuda/walk/kf_walk.cu:1217-1238`, `kf-cuda/src/walk.rs:1438`, `kf-mem/src/vasmgr.rs:432-470,1404-1433`.
*Trigger:* the total diff runs of one walk exceed `run_capacity` (1 << 18) - ~1 GiB of scattered 4 KiB sysmem in one process is enough - or a space exceeds its entry/pool budget: the kernel
reports TRUNCATED with `run_count = 0`, `capacity_retry` does not fix a report-level truncation and `poll()` returns `Err` ("nothing applied, nothing committed"). *Consequence:* every
invalidate and split of the batch fails (`vasmgr.rs:1411-1429`), the next diff is identical and fails again: one process's mapping size denies every other space's invalidates (guest hang)
until it unmaps. *Fix:* chunk the diff per walk (PARTIAL) or fail only the offending entry. *Test:* a `report_cap` knob in `ModelWalker` that fails `poll()` above a total; two spaces in
one batch, A over the cap, B small; B's trigger must clear.

**AT-1 / CU-1 (HIGH, verified, CODE READ)** `kf-trap/src/ring.rs:110-116`.
*Trigger:* vCPU A loads `tail = T`; before A loads `head`, another vCPU pushes T and the drainer commits it (`head = T+1`); A's `tail.wrapping_sub(head)` = `u64::MAX >= CAPACITY` and A
poisons the ring (`poisoned.store(true)`). *Consequence:* sticky: every later privileged register write returns `Dropped` (`ring.rs:104`) and `trap.rs:216` turns it into `Action::None` -
including GSP queue-head writes, so RPCs hang until a device reset. *Guest-triggerable:* by chance for any SMP guest (the window is two adjacent loads; nested KVM preemption widens it);
guest root can raise the odds with multi-vCPU BAR0 write bursts; harm is to the guest's own device. *Evidence:* CODE READ only (the race window cannot be hit deterministically from the public
API; a stress test needs an injected delay). *Fix:* load `head` before `tail` (head <= tail always holds in that order) or re-read `tail` before poisoning; extract a pure
`is_full(tail, head)`. *Test:* `is_full(T, T+1)` is false for T in {0, 1, 4095, 4096, 2^32, u64::MAX-5}. Note `occupancy()` (`ring.rs:184`) has the same shape but its only caller is the
drainer, which is the only head mover (safe today). Release builds have `overflow-checks = true`, so an underflow there would abort the VM.

### 2.2 MEDIUM

**RC-01 (MEDIUM, verified, CODE READ; model gap)** `userpump.rs:2794` (`record.enter()` loss returns `Some(false)`), `2980-2983` (`peek_one`: `record.try_lock()` loss and the holder path return without
`pending()`), `userrelay.rs:81-103` (`enter`), `userpump.rs:2970` (`look_needed(why, false, ..)` ignores demand).
*Trigger:* two workers; relay R has a recorded wait (`waiting` flag, i.e. a Marker wait - the steady state of a pipelined client). A doorbell makes worker A claim R; `enter()` sets
`Cell::retry` then `try_lock`s; worker B is inside `peek_one(R)` (it holds `record.try_lock()` across 2 UC loads, or inside `alive()`/`wake_if`), so A returns `Some(false)`, the token is
released, the doorbell consumed. The comment says "the act thread's completion rings"; here the holder is a peer WORKER, which does not hand off and does not re-check `pending()`.
*Consequence:* the new GP_PUT is not read until R's recorded wait is satisfied (the earlier batch's marker); bounded latency (one GPU batch) in the normal case, a stall in the unusual case
that the earlier batch cannot finish without the dropped work (cross-channel semaphore chains). The 50 ms net does not rescue it: `Peek::Backstop` rings only satisfied waits, demand is
ignored by design. Probability rises with RC-04 (a spinning peer peeks R continuously) and is zero with ONE worker. *Guest-triggerable:* indirectly (any busy client); a deliberate
variant needs two clients. *Fix:* after `peek_one` releases the record, `if record.pending() { ring }` (the holder hands off), or in `serve` register with the handoff on a failed `enter()`;
or make `Cell::request()` ring the token. *Test:* structural (`kf-qemu/tests`) `user_ring_peek_one` must contain a `pending()` re-check after the try_lock section; the model
(`relay_fairness`) has no record-lock holder that is a peer worker - add one.

**RC-04 (MEDIUM, CODE READ + INFERRED cost)** `userpump.rs:3078-3083`, `worker.rs:174-196,228-240`, `userpump.rs:2932` (`user_ring_unannounced`).
*Trigger:* `dispatch` reports `event = true` for ANY engine/completion/RC fd; `peek(Peek::Event)` then sets `Cell::unannounced` for EVERY waiting relay whose wait is not yet satisfied,
including events caused by other clients, the guest's own Passthrough work, or the relay's own older marker in a pipeline deeper than one. `unannounced` is cleared only when that relay's
wait is satisfied or its next turn re-arms it. *Consequence:* `idle(unannounced) == Spin`: every worker stops parking and busy-polls (`epoll_wait(0)`, `user_rings.lock()`, 2 UC loads per
waiting relay per iteration) for as long as ANY relay has an outstanding marker and events keep arriving - i.e. most of a loaded multi-client run; the doc says marker waits "cost
nothing (they block)". With a GPU-blocked relay (unsatisfied acquire) both workers spin until it resolves. *Guest-triggerable:* yes (a process can leave a marker wait outstanding by
submitting work that blocks on a semaphore it never releases, then any desktop activity feeds events): host CPU exhaustion of two cores, UC read traffic towards VRAM; SECURITY (DoS of the
host share, not of other VMs). *Evidence:* the code path is CODE READ; the CPU cost is INFERRED - no hardware counter exists on master for `unannounced_spins` vs `wake_event`.
*Fix:* mark `Lagging` only when the event can be attributed to this relay (its marker count moved in the event word, or a per-relay event); otherwise leave the relay announced. *Test:*
two relays, relay B's marker events arrive while relay A's wait is unsatisfied: the sleep-safety model must allow the worker to block (add to `relay_fairness`).

**RC-05 (MEDIUM, CODE READ)** `userpump.rs:2826-2830` (`serve` returns on `t.stopped || t.disabled` without touching `waiting`/`unannounced`), `2945-2955`, `worker.rs:237`.
*Trigger:* a relay with a `Popped` wait (or RC-04's `Lagging`) whose channel is then disabled / stopped / preempted by the host (TDR flow, DISABLE_CHANNELS) or whose GPU work hangs: the
pops never come. *Consequence:* `unannounced` stays set and the workers spin at 100 % until the channel is enabled or freed (seconds during a TDR, unbounded for a hung channel);
additionally the spinning worker re-takes the blocking `user_rings.lock()` back-to-back (`userpump.rs:2948`) so the drainer/act/other worker that block on it
(`user_ring_record`) can be starved by lock barging (INFERRED). *Guest-triggerable:* yes (hang one channel on purpose). *Fix:* clear/ignore `unannounced` while the channel is disabled or
stopped and re-set it at enable; bound the spin with a sleep-free yield (`sched_yield`) so a spinning worker never owns a core outright. *Test:* model: a disabled channel with a Popped wait
must let the worker block.

**RC-10 / GM-4 / PR-12 (MEDIUM, CODE READ, UAF only with RAM unplug)** `kf-qemu/src/mem.rs:176-204` (`block_for`, `at_file_offset` return a `Copy` `RawRegion` after dropping the read
guard), users `chan.rs:1496,1983,2038,2105,6444,6618,7478,7556`, `device.rs:3922,3935`, `display.rs:383,2597-2712`. *Trigger:* `kf3_ram_del` (QEMU `region_del`, main loop / a vCPU in a PCI
config write) runs while a worker/drainer copies through the escaped pointer. *Consequence:* stale view after a PAM/SMRAM/overlay flip (memory-safe); use-after-unmap only if QEMU
frees the RAM block (DIMM / virtio-mem unplug, balloon discard). `userread.rs` keeps the guard across the copy (the safe pattern). *Guest-triggerable:* not for host memory safety unless
hotplug is enabled. *Fix:* copy under the guard, or refuse DIMM/virtio-mem at realize and assert "RAM blocks outlive the device". *Test:* none GPU-free beyond a sanitizer run.

**RS-3 (MEDIUM, SECURITY, lane CODE READ, spot-verified)** `chan.rs:5853-5890`, `userrelay.rs:979-993`, `userquota.rs:102-118`. *Trigger:* free of a USER-ring channel while a worker holds its
token Busy at BOTH retire attempts (the drainer's statement-time `retire_user_ring_token` uses `record.try_lock()`, result ignored at `chan.rs:5657`; the act's `prepare_native_cleanup`).
`free_attempted` is set first and never cleared, `prepare_native_cleanup` returns `TOKEN_STILL_OWNED`, `release_twin` returns `USER_RING_FREE_QUARANTINED` and no later free can retry.
*Consequence:* the native host channel, the VA lease, one cap and a birth-hold slot leak until QEMU exits; the token stays non-Dead so the chid cannot be reborn. *Guest-triggerable:* yes
(free during a long worker turn on that channel - a hostile process can lengthen its own turn with a large source copy, ~66 ms); bounded by the channel budget, self-DoS of the VM's GPU
channels. *Fix:* on `TOKEN_STILL_OWNED` clear `free_attempted` and re-queue the cleanup with `delay_act`; never quarantine on a transient Busy. *Test:* extend
`userquota::a_busy_worker_blocks_native_cleanup_and_quota_release`: after the worker releases, the re-queued cleanup must succeed and release the quota exactly once.

**RS-1 (MEDIUM, lane CODE READ)** `chan.rs:5655-5660`, `kf-core/plane.rs:387-392`, `token.rs:retire`. Relayed (non-USER-ring) Windows twins have route Translated but the free path
`let _ = self.plane.free_channel(..)` ignores a Busy token ("a Passthrough token is never BUSY"): the per-runlist cap is never released and the chid's next birth is refused. Fix: retire on
the act thread with bounded retries; release the cap only after confirmed retire + free. RS-2 (INFERRED, impact high / likelihood low): `drop_relay` releases the CPU view while a worker
that already cloned the `Arc<Relay>` can still `step()` through the old mapping (no `released` flag).

**RS-5 (MEDIUM, lane CODE READ)** `chan.rs:7124,7143,7229`: a Translated birth that arms a framebuffer-USERD `Store` view and then fails leaks one RM CPU-view record (`UserdView` has no
`Drop`; `release_cpu_view` only in `retire`). Fix: owning release guard; test: fake-RM arm/release counts across injected birth failures.

**RS-6 (MEDIUM, lane CODE READ)** `reset.rs:247,307-322,364-465`, `device.rs:3690-3710`, `kf3.c kf3_reset_exit`. After `wait_reset` times out (2 s) the vCPUs resume with the gate closed: BAR0
writes are dropped and only counted, shadow reads still serve the old life; no later re-sync. Waits with the gate closed and one stall log per stage: display `scan.inflight`, act-thread
barrier, Memory stage (an in-flight walk; a refused scrub retries for ever), `finish()` (every Busy token). A wedged host call leaves the VM permanently gated with no escalation.

**AT-2 (MEDIUM, lane CODE READ; catalogue RELAY-INTR-SHADOW still reachable)** `device.rs:1634-1636` (`intr.write` then `intr.shadow`), `1838-1841` (`latch_and_deliver`: `latch` then
`shadow_all`), `kf-trap/cpuintr.rs:264,285`: a vCPU W1C and a worker/drainer/display latch each load the atomics and then store the shadow word, unordered; the shadow can miss a pending
bit the atomics hold, the guest ISR (reading the shadow, no exit) sees nothing pending and the already-sent message is not repeated: a completion stays invisible until a later republish.
Window ~20-50 ns. The display plane fixed the same class with a stable-snapshot loop (`display.rs:2490-2536`). Fix: bounded re-publish loop with a `fence(SeqCst)`; test: two threads on a
real `CpuIntr` + atomic shadow array, assert shadow == atomics at quiescence (probabilistic).

**AT-3 = VA-3 (MEDIUM, verified)** `mem.rs:2885-2891` (`invalidate_write` signals the VA wake BEFORE returning `v`) and `device.rs:1618-1623` (the vCPU stores `v` busy into the shadow AFTER
the return; the comment at `device.rs:1614-1617` says "arm, shadow, then one wake"). A vCPU preempted between the signal and the store lets the VA thread complete a vacuous clear and
`publish_trigger` store idle; the vCPU then stores busy over it. The port is idle, the guest sees busy and spins under its locks until the VA loop's next `publish_trigger` (up to the
50 ms poll). A candidate contributor to VA-LATENCY (INFERRED). Fix: store the shadow before the signal, or re-check the port after the store. Test: needs the arm/store/wake sequence
pulled behind a fake shadow.

**VA-6..VA-11 (MEDIUM, lane CODE READ)**: VA-6 `Want::Root` lost on walk error / batch refusal (`vasmgr.rs:1393,1419-1429`); VA-7 walker slots leak when > 64 resets are pending
(`vasmgr.rs:1139-1150`, `KF_MAX_RESET`), after which "no walker slot left" for new spaces (guest-triggerable by freeing many spaces); VA-8 host VA space leaked when a mirror retires with
live channels (`mem.rs:3433-3458`); VA-9 steer path `release_micro` drops the reservation before the host free returns - a VA-thread map in that window gets `HeldByHost` and a persistent
hole (`batch.rs:2064-2077,2697-2748`); VA-10 = LK-2 (RULE; the lock lane rated it HIGH) the USER snapshot holds `rows.read()` + `ram.blocks.read()` across a UC copy of up to 0xE0000 bytes (BLOCK-UCCOPY, `userread.rs:117-120,
252-272`): the VA thread's `rows.write()` and the BQL thread's `RamMap::add/del` queue behind it, and a writer-waiting std `RwLock` also refuses new `try_read` (-> Contended); VA-11 the
VA thread has unbounded waits (`batch.rs:1387-1420` `claim_map` with a 100 ms re-check and no deadline; unload scrub; BAR1 `Settle::Pending` waits for the QEMU main loop) and
`Trigger::is_overdue` is never called in production.

**PR-1 = LK-1 (MEDIUM, RULE, lane CODE READ; the lock lane rated it HIGH)** `kf3.c:811-858`, `dbfast.rs:438-455,510-592,643-685`: with `doorbell-ioeventfd=on` (default off) the C memory listener (main loop with the BQL, or a
vCPU in a PCI config write) calls `kf3_doorbell_site` -> `DbFast::site_add/site_del`, which hold `reg` across one `KVM_IOEVENTFD` per live token (up to 256, each an SRCU grace period,
10-40x on nested hosts) while the act thread holds the same mutex across its own ioctls: the BQL (and thus every vCPU on a BQL path, QMP, timers) stalls and the BAR1 overlay completion
the VA thread waits for is delayed. Contradicts the `ffi_unsafe.rs:915` doc "never on a vCPU's MMIO path". Fix: queue place/unplace to the act thread; delete bounded, add async.
**PR-2 (MEDIUM, RULE, verified, MEASURED)** `dbfast.rs:826-851`: `service_ready` loops while the poll returns a full batch and is called before every privileged write; >= 64 distinct tokens
re-signalled by exit-less ioeventfd stores keep it in the loop for ever (the drainer starves: GSP ring, heartbeat, reset, VA clears). Red test `audit_dbfast_service_ready_is_bounded` does
not return in 3 s. Guest-triggerable, own VM only; default-off feature. Fix: cap polls/time per call.
**PR-3 (MEDIUM, lane)** `dbfast.rs:484-505,599-609,664-685`: a failed deassign is only counted; the fd is closed after the ack but KVM keeps its registration, a re-born token with the same
value gets EEXIST and its doorbells never exit: silent permanent loss. Fix: quarantine the notifier, mark the value poisoned.
**PR-5 (MEDIUM, lane)** `kf3.c:1996-2011`: no `vmsd`, no `migrate_add_blocker`, no run-state handler: `migrate`/`savevm` appears to succeed without device state and GPU-written pages bypass
dirty tracking (operator action only). Also PR-6/RS-9 (LOW): hot-unplug and a failed hot-realize leave Rust threads and BH/hook pointers on freed C state; `kf3_unrealize` only sets stop flags.

**CU-5 (MEDIUM, SECURITY, lane CODE READ, reachability INFERRED)** `kf-gsp/boot.rs:906,2506-2528,2764-2783`: `GspFsm.held` is unbounded (a held reply keeps up to ~64 KiB, `remove(0)` O(n),
a log line each) because held replies never call `post()` so `QueueFull` gives no back pressure: a guest that never reads the status ring and keeps issuing commands whose policy holds /
defers (`UPDATE_BAR_PDE`, `SET_PAGE_DIRECTORY`, RmControl) grows host heap without bound. Fix: `VecDeque` + `MAX_HELD`, stop draining at the cap.

**LK-3 (MEDIUM, RULE, verified, CODE READ)** `kf-mem/src/cpuwin.rs:905` (`CpuWindow::repoint`: `self.state.lock()` blocking), `kf-qemu/src/mem.rs:2855-2866` (`pramin_write`, vCPU).
The doc on `pramin_write` says "No lock: two racing writers each re-check the latest word"; but `repoint` takes the pool's blocking `state` mutex and holds it across `arm_store_in_trap`
(`NV_ESC_RM_MAP_MEMORY` + `mmap`, or an inline `openat` when the node pool is empty) and nested `blocks.read`. *Trigger:* two vCPUs write the PRAMIN window base (a guest loop), or the VA
thread's `reset_pramin` (`mem.rs:2828`) holds `state`. *Consequence:* a vCPU waits for another thread's RM ioctl, which itself waits on the host RM API lock held by the act thread's RM
calls (bounded by RM latency, tens of ms under contention; chains into VA-10/LK-2). The owner ruling of 2026-09-25 sanctions ONE vCPU syscall, not a vCPU-vCPU lock wait. *Guest-triggerable:*
yes (guest root loops the write). *Fix:* `try_lock`; the loser leaves `pramin_want` set and the holder's existing re-check loop applies the latest value.

**LK-4 (MEDIUM, RULE, lane CODE READ; prior W6 still true)** `chan.rs:3577,3749,4727,4741` (+ `bar0trace_dump` 7453): the drainer takes `Slot` blocking (under the `gsp` lock) while a worker holds
it across a whole Translated pump (RM view arm, UC copies and stores, 60-200 ms by the prior audit's arithmetic). Held GSP replies stall. `KF3_BAR0_TRACE` is in the Windows lane's default
`FLAGS`. *Fix:* keep `guest_idx`/`tsg`/`scheduled` in an `Arc` of atomics, or `try_lock` + defer.

**LK-5 (MEDIUM, RULE, lane CODE READ; prior W9 still true)** `userpump.rs:2945-2955`: `user_ring_peek` holds the blocking `user_rings` lock across every waiting relay's `try_lock`s and two UC loads
(the in-code comment "a handful of instructions" is wrong), run on every loop by both workers; `alive`, `user_ring_serve`, the act thread and the drainer (`retire_user_ring_token`) take it
blocking; a failed `try_lock` in `waiting()` / `unannounced()` returns `true`, so the other worker cannot park. With RC-04/05 spinning, lock barging can starve the drainer (INFERRED).
*Fix:* clone the `Arc<Cell>`s out under the lock and iterate unlocked.

**LK-6 (MEDIUM, lane CODE READ)** the act thread takes `record` blocking (`userpump.rs:2741`, `userrelay.rs:851,929,946`, `release_twin`) while a worker turn holds it across RM view arm/release and UC
copies, and `finish_user_ring_free` / the 5080 act hold it across `release_cpu_view` / `alloc_deferred_api`: an act-queue stall (every held guest RPC reply) of RM-ioctl latency + copy time.
*Fix:* act `try_lock` + re-queue, or the worker arms views off-record.

**LK-7 (MEDIUM, RULE, lane CODE READ)** `chan.rs:7365` `rc_scan` takes `pt` BLOCKING across 4 UC notifier loads per unreported twin (W8); workers hold `pt` (try) across storage writes and the
ring publication (`userpump.rs:584`), up to ~224K UC stores on the "rare" path; the drainer and act take `pt` blocking. *Fix:* `rc_scan` `try_lock` + rescan flag; notifier words in an `Arc`.

**LK-8 (MEDIUM, RULE, flag-gated, lane CODE READ)** `chan.rs:3392` `note_inline`: with `KF3_MAPLOG` (in the Windows lane's default `FLAGS`) every inline Passthrough doorbell does `eprintln!` ON THE vCPU
thread: a blocking `write(2)` per guest doorbell if the log filesystem stalls. *Fix:* counter + first-N.

Catalogue rows that are NOT new but are still reachable on master (HELDREPLY, EVENTDROP, PREEMPT139, TRANSLATED-DEAD, DISABLE-STICKY, SLOT-TRYLOCK, INTR-SHADOW = AT-2): section 5. RS-7 (reset gives up after ~2 s and leaves `pt`, `relays`, `user_rings`, `slots`, caps populated for the next life) is LOW.

### 2.3 LOW (auditor's own)

**RC-02 (LOW, MEASURED, verified)** `kf-core/src/plane.rs:545-556` + `kf-trap/src/bitmap.rs:94-130` + `worker.rs:206,241`. A pass whose first `limit` (64) returned tokens are all stale
(`claim()` -> `NotOurs`: Dead/Idle words with a bit left over, e.g. channels rung then retired) serves 0; `scan` re-published the remainder WITHOUT a bump; `seen` was loaded before the pass
and nothing moved it, so `try_park(seen)` is accepted while a live RUNG token with its bit set waits. Only the 50 ms park timeout rescans it (the net is load-bearing here; with
`KF3_WORKER_PARK_INFINITE=1` it is stuck). *Red:* `kf-core/tests/audit_scan_limit_lost_wake.rs`. *Guest-triggerable:* guest root (allocate / ring / free >= 64 channels between worker passes).
*Fix:* `scan` bumps the wake word when it re-publishes a remainder, or the worker does not park when `scan` returned `limit` entries.

**RC-06 (LOW, CODE READ)** `worker.rs:202,252`: `owed = on_tick()` is computed at the top of the iteration; the Immediate poll's `dispatch` (-> `on_completion` -> `nsi_fifo_edge`, or an engine
edge) can make a paced raise owed AFTER that, and the park then uses the stale `owed == false` (50 ms, or infinite with the knob) instead of 1 ms. Pacing is off by default
(`KF3_PT_NSI_MIN_INTERVAL_US`); with it on, a trailing raise is delayed up to 50 ms (infinite with the knob: dropped). Fix: recompute `on_tick()` immediately before the park decision.

**RC-07 (LOW, CODE READ)** `device.rs:3551-3553` `stop()` signals `worker_efd` ONCE for two workers; the first to wake drains it and the second stays in `epoll_wait`; it exits only at the 50 ms net
(never with `KF3_WORKER_PARK_INFINITE=1`). `kf3_unrealize` is probably never called at normal exit (lane PR-7), so impact is hot-unplug only.

**RC-08 (LOW, CODE READ)** `userpump.rs:2917` / `userrelay.rs:66-103,168`: `user_ring_wake` = `request()` (retry = true) then `wake_if` (`try_lock`); a holder that is IN a turn re-checks
`pending()` at the end of `user_ring_arm` WHILE STILL HOLDING the record; a waker that stores `retry` after that check and loses `try_lock` to the still-held guard leaves the demand with
no ring (window: the instructions between `arm` and the guard drop). Also the software-object act (`userpump.rs:2735-2790`) returns through `?` without `user_ring_wake`. Fix: check
`pending()` after dropping the guard, or ring unconditionally.

**RC-09 (LOW, CODE READ)** `token.rs:retire/reset_fresh` return `false` on a failed `compare_exchange` (e.g. a concurrent `ring()` Idle->Rung), the same "CAS failure is not evidence of another
owner" rule `allocate`/`claim` document; callers read `false` as "Busy" (`free_channel`, `reset_tokens`). A guest ringing a token in a loop while it is retired can make single attempts
fail; callers that do not retry (RS-1, RS-3) turn it into a leak. Fix: loop on CAS failure, return false only for Busy/BusyRung.

**RC-11 (LOW, CODE READ)** `ffi_unsafe.rs:221-229`: workers are `for i in 0..2` (hard-coded); a failed spawn after the drainer started returns `-1` with the drainer, earlier workers and the
leaked `Device` still running (no join handles are kept anywhere).

Other lanes' LOW: AT-4 (TickShare seqlock lacks fences; x86-safe), AT-6 (`RawRegion` plain stores, INFERRED codegen risk), AT-7 (re-validation loops lack StoreLoad), VA-12..VA-17, RS-4
(act-thread 200 x 1 ms sleeps in `retire` under the slot lock: stalls all acts), RS-7, RS-8, RS-10 (failclosed does not cover `kf3-scanline`/`kf3-probe`/`kf3-strand-diag`; a poisoned lock
makes `RamMap::add/del` silently no-op), PR-4, PR-8..PR-11 (PR-9/PR-10 are broker symlink / `nb_listen` TOCTOU: local-attacker SECURITY, small), GM-2 (forged marker tag also gates the
staged-trigger actions), GM-3 (registry looked up twice), GM-5..GM-8, CU-2..CU-4, CU-6..CU-9, LK-9 (drainer `free()` blocks on `Book.pending` held by an act across a blocking `pt.lock()`: transitive RULE), LK-10 (act `retire` holds the slot across up to 200 x `sleep(1ms)` + RM frees; a worker `try_lock` miss then drops the ring), LK-11 (`kf3_device_reset_wait` sleep-polls up to 2 s on the main loop with the BQL; no deadlock because the Memory stage does not wait for overlay acks), LK-12 (worker 'brief' blocking locks: `kernel_tokens`, `relays` + `Relay` mutex, `hw_words` `storage.lock()`, `Inbox`, `Registry.inner`, `sw_objs`, `slots.read()`).

### 2.4 INFO

AT-5 (`ring_internal` stamp regress, unused), AT-8 (`EngineIrq` race, tests only), AT-9 (**wake-word protocol re-verified correct**, incl. weak memory), AT-10/AT-11, VA-18/19, GM-1 (pushbuffer
is scanned from a private copy but the GPU re-fetches it: accepted by the 2026-10-13 ruling, which is only "proposed ruling text" in `V3_WINDOWS_USER_RING_RELAY.md:190` and absent from
`OWNER_RULINGS.md`), CU-10/11/12, RS-11/12, PR-13/14, LK-13/14. **RC-12 (INFO, CODE READ)**: no v3 lock is ranked - `grep` finds no `lockwitness::note_acquired` / `LockRank` / `trapwitness::mark` in any
`kf-*` crate outside tests - so every `assert_lock_free` in `kf-linux-raw` (RM ioctls, epoll, eventfd create) passes vacuously and the catalogue's "lock-rank witness exists" pin
(BLOCK-DRAINER-SLOT / BLOCK-RCSCAN) does not guard v3 locks.

### 2.5 ID index (counts are computed from this table; merged duplicates share a row)

| severity | ids |
|---|---|
| HIGH (5) | VA-4 (=VA-5), RC-03, VA-2, VA-1, AT-1 (=CU-1) |
| MEDIUM (27) | RC-01, RC-04, RC-05, RC-10 (=GM-4=PR-12), RS-1 (+RS-2), RS-3, RS-5, RS-6, AT-2, AT-3 (=VA-3), VA-6, VA-7, VA-8, VA-9, VA-10 (=LK-2), VA-11, PR-1 (=LK-1), PR-2, PR-3, PR-5, CU-5, LK-3, LK-4, LK-5, LK-6, LK-7, LK-8 |
| LOW (42) | RC-02, RC-06, RC-07, RC-08, RC-09, RC-11, AT-4, AT-6, AT-7, VA-12, VA-13, VA-14, VA-15, VA-16, VA-17, RS-4, RS-7, RS-8, RS-9 (=PR-6), RS-10, PR-4, PR-8, PR-9, PR-10, PR-11, GM-2, GM-3, GM-5, GM-6, GM-7, GM-8, CU-2, CU-3, CU-4, CU-6, CU-7, CU-8, CU-9, LK-9, LK-10, LK-11, LK-12 |
| INFO (18) | AT-5, AT-8, AT-9, AT-10, AT-11, VA-18, VA-19, GM-1, CU-10, CU-11, CU-12, RS-11, RS-12, PR-13, PR-14, RC-12, LK-13, LK-14 |

SECURITY-flagged (a guest can trigger it on purpose and it harms other guest processes, the host or the management plane): RC-03, VA-1, VA-2, RC-04, RC-05, RS-3, CU-5, PR-2 (own VM only),
PR-9/PR-10 (local attacker), LK-3 (vCPU-vCPU), LK-8. Findings the guest can only trigger against itself: AT-1, RC-02, GM-2, GM-3, RS-7.


## 3. Lock graph (lock lane, CODE READ)

No lock-order CYCLE was found. No lock in `kf-chan`, `kf-qemu`, `kf-mem` or `kf-core` is ranked (RC-12): the only live witness is `leafwitness` around `HostRm.objects` (a leaf, never held
across an ioctl), so there are no ranks to compare the real order with. Poison: a panic on a service thread aborts the VM (`failclosed`), so poison is reachable only from the non-service
threads (probe, scanline, strand-diag); several sites still skip silently on poison (`Handoff::register`, `Inbox::push`, `RamMap::add/del`, `walk_busy()`: LK-13).
Thread classes: **V** vCPU (BAR0 trap), **M** QEMU main loop / a vCPU in a PCI config write with the BQL, **W** worker (x2), **D** register drainer, **A** act thread, **VA** `kf3-vamgr`;
`b` = blocking acquire, `t` = try.

### 3.1 Locks

| lock (decl) | protects | taken by | held across |
|---|---|---|---|
| `PraminPool.state` (`kf-mem cpuwin.rs:818`) | PRAMIN views | **V b** (`cpuwin.rs:905` via `mem.rs:2855`), VA b (`mem.rs:2830`) | `NV_ESC_RM_MAP_MEMORY` + mmap, inline `openat`, nested `blocks.read` (LK-3) |
| `RamMap.blocks` RwLock (`mem.rs:98`) | guest RAM blocks + generation | **M b write** (`mem.rs:152,162`); V b read (PRAMIN `file_range`); D b read (`device.rs:3922`); W b read (Translated), W t read (USER); VA b | W: `try_read` across a UC memcpy of up to 896 KiB, 3-4 nested `try_read` in `publish_guarded` (VA-10) |
| `dbfast.reg` (`dbfast.rs:349`) | doorbell registry | **M b** (`site_add/site_del` 643/664), A b (`register/deregister`) | one `KVM_IOEVENTFD` per placement (budget 256), `epoll_ctl` (PR-1) |
| `dbfast.tx`, `dbfast.side` | command queue; drainer half | A (brief); D | `side`: across `deliver` |
| `Handoff.list` (`handoff.rs:20`) | waiting relays | every `PtGuard` drop; W, A `register` | leaf; the ring runs after unlock |
| `ChanPlane.pt` PtLock (`chan.rs:2279`) | passthrough twins | D b (many), A b, W t (`userpump.rs:471,584,2493,2602,2802`), **W b `rc_scan` (`chan.rs:7365`)** | W: `publish_guarded` across storage writes + ring publication; `rc_scan` across UC notifier loads; guard drop runs `Handoff::released` (LK-7) |
| `Cell.inner` "record" (`userrelay.rs:58`) | USER-ring state | W t (`enter`, `peek`, `alive`, `wake_if`), A b (`userpump.rs:2741`, `userrelay.rs:851,929,946`, `release_twin`) | W: the whole turn incl. RM view arm/release and UC copies; A: `alloc_deferred_api`, `Pump::release` (LK-6, RC-01) |
| `Lease.storage` (`userva.rs:139`) | private ring storage | VA t, W t, **W b `hw_words` (`userpump.rs:685`)**, A b | VA: RM ops + UC clears |
| `user_rings` (`chan.rs:2284`) | token -> record | **W b `user_ring_peek` (`userpump.rs:2948`)**, `alive`, `serve`; A b; **D b** (`retire_user_ring_token`) | peek: record t, storage t, 2 UC loads per waiting relay, `eprintln` (LK-5) |
| `relays` + `Relay` mutex (`chan.rs:2295`) | USERD relays | W b (`relay_serve`, `alive`), A b (`drop_relay`) | `drop_relay`: `eprintln` + UC loads |
| `slots` RwLock + `Slot` mutex (`chan.rs:2250`) | Translated channels | W t (`serve`), **D b** (`chan.rs:3577,3749,4727,4741`), A b (`retire`) | W: the whole pump (RM arm 1322, UC copy/stores); A `retire`: <= 200 x `sleep(1 ms)` + RM frees (LK-4, LK-10) |
| `Book.pending` (`userbirth.rs:63`) | pending USER births | A; **D b** (`free` -> `cancel`) | A `commit`: record t, pt b, `quota.state`, caps (LK-9) |
| `NativeQuota.state` (`userquota.rs:23`), `ChanPlane.caps` (`chan.rs:2248`) | quota / VM caps | A, D | caps |
| `PlacedRows` RwLock (`mem.rs:845`), `RowsLog.log` (`mem.rs:920`) | placement rows + change log | VA b, A write (steer, `chan.rs:274`), W b (Translated), W t (USER) | VA `build`: rows.read -> sked -> regions; W: `try_read` across a UC copy |
| `GpuMirror.sked`, `Regions.state` (`userva.rs:25`), `Queue.pending/held` (`userva.rs:237,239`) | SKED rows, VA exclusions, birth queue | VA, A | `Queue.held` across `Storage::prepare` RM ioctls (`userva.rs:536`) |
| `Inbox.{q,splits,gates,split_tokens,split_results,walk_waiters}` (`mem.rs:2210-2239`) | VA inbox | D, W b (brief), VA | Vec/HashMap ops only |
| `Mirrors`, `spares`, `RingSlots`, `Bar1Overlay.done` (`mem.rs:1900`), `TrapNodes` | misc VA | D b, VA, W t, **M b** (`Bar1Overlay.done`), V b (`TrapNodes`) | brief |
| `gsp` (`device.rs:268`), `Reset.run` (`reset.rs:205`) | GSP FSM, reset sequencer | D only (+ M at realize) | the whole statement chain, UC / guest-RAM I/O |
| small `ChanPlane` maps (`by_obj`, `sw_objs`, `scopes`, `pt_objs`, `dbg`, `groups`, `cuda_limit`, `guest_sched`, `enc_sessions`, `acts`, `rc_queue`, `preempt_done`, `usermode_off`, `user_birth_holds`) | per-subsystem | D, A, W (`sw_objs`, `acts`) | brief |
| `kernel_tokens` (`plane.rs:219`), `Registry.inner` (`kf-rm defapi.rs:228`), `HostRm.objects` (leaf, witnessed), `broker.relay`, `display.model` | misc | W b per served token; D | brief; `Registry.inner` calls a closure under it |

No `Condvar`, `Barrier` or blocking channel exists in production; `recv`/`recv_timeout` only on the act queue, the PRAMIN reaper and the gpucopy thread; `dbfast.deregister` waits up to 2 s for the
drainer's ack on A. `OnceLock`s with real init closures are VA-only or env flags.

### 3.2 Order edges (A held, then B taken)

E1 `gsp` -> `display.model`, `preempt_done` (D). E2 `pt` -> `pt_objs` (documented "pt, then pt_objs"). E3 `pt` -> `by_obj` (D). E4 `sw_objs` -> `scopes` -> `by_obj`, no reverse nesting.
E5 `record` -> `pt` (A b at `userpump.rs:2741/2768`; W t) and `record` -> `storage` (W t). E6 `user_rings` -> `record`(t) -> `storage`(t) (peek). E7 `Book.pending` -> `Queue.pending` / `Reply`;
`Book.pending` -> `record`(t) -> `pt`(b) -> `quota.state` -> `caps`. E8 `slot` -> `caps` (A `retire`); `slot`(t, W) -> `sw_objs`, `Registry.inner`, `acts`, `Inbox.*`, `rows.read`, `blocks.read`.
E9 `Queue.held` -> `rows.read` -> `sked` -> `{Regions.state, own}` (VA); `rows.write` -> `Regions.state` (A steer), `rows.write` -> `RowsLog.log`. E10 `PraminPool.state` -> `blocks.read` (x2),
`TrapNodes.nodes` (V). E11 `dbfast.reg` -> `tx` -> KVM ioctl. E12 `Registry.inner` -> kf-rm Graph (via `live_handle`; reverse not verified: UNCERTAIN L1). E13 any `PtGuard` drop -> `Handoff.list`
(leaf) -> ring closure (atomics + `eventfd` write; no lock).
**Cycle verdict: none.** The only same-thread nesting of one lock is `try_read` on `rows`/`blocks` (3-4 deep in `publish_guarded`); a pending writer makes the nested attempt fail (`Busy`/`Retry`,
safe) - converting any of them to `read()` would deadlock against a queued `ram_add` writer (std `RwLock` is writer-preferring). No std-`Mutex` re-acquire through a closure/callback was found
(`Cell::wake_if`, `Book::commit`, `HandoffHub`, `SteerClaim::drop`, `Drop for Birth`, `NeverBorn::drop` checked).

### 3.3 Prior blocking audit (W1-W20 and V1/V2) at this commit

Still true: W1/W2 (worker RM view arm/release: `userstore.rs prepare`, `chan.rs:1322`), W3 (UC copy under `rows`/`blocks`), W4/W5, W6 (drainer `slot.lock()`), W7 (slot `try_lock` miss: unchanged code),
W8 (`rc_scan`), W9 (`user_ring_peek`), W10, W11, W12, W13 (`KF3_RELAY_GET_REFRESH`), W14 (`eprintln`; `KF3_MAPLOG` still in the Windows `FLAGS`), W15, W16, W19, V1, V2.
Fixed / mitigated: W17 (contended `record` costing up to 50 ms: handoff + `user_ring_wake`), W18 (spin <= 64: replaced by the single loop; the net stays as a counted bug detector).
New: `PraminPool.state` on the vCPU (LK-3), `Book.pending` chain (LK-9), `try_lock().map_or(true, ..)` spuriously reporting "unannounced" when a peer holds `user_rings` (LK-5), the drainer's
`Ram::read` queued behind a `RamMap` writer (VA-10), `bar0trace_dump` (LK-4), `note_inline` stderr on a vCPU (LK-8), `kf3_device_reset_wait` sleep on the BQL (LK-11).

## 4. The big worker loop

### 4.1 Loop as implemented (`kf-chan/src/worker.rs:200-282`, per iteration)

`owed = on_tick()` -> `seen = worker_wake().seen()` (Acquire, BEFORE the scan) -> `worker_pass` (`RungBitmap::scan(limit 64)`, claim each, <= `REACT_ROUNDS + 1` rounds each) ->
`poller.wait(Immediate)` + `dispatch` (doorbell efd drained; completion fd -> ring in-flight tokens + `on_completion`; engine/RC fds -> `on_other`) + `relay.peek(Event)` -> if
`relay.waiting()` then `peek(Spin)` -> `n > 0`: repeat -> `idle(relay.unannounced())`: `Spin` repeat -> `try_park(seen)` (refused if the sequence moved) -> `epoll_wait`
(50 ms net, 1 ms if `owed`, infinite with the knob) -> `unpark` -> dispatch -> backstop peek on a timeout -> `on_other(TICK)`.
The wake word protocol is sound (AT-9 and the auditor's own reading): publisher = `bits.publish` (Release) then `bump` (AcqRel RMW); consumer = `seen` load before the scan, `try_park` CAS;
both RMWs on one word.

### 4.2 Wake-source table (every obligation the loop owes; "net" = the 50 ms park timeout)

| # | obligation | change that needs handling | wake source | reaches | non-missable? | gap / evidence |
|---|---|---|---|---|---|---|
| 1 | flagged token (vCPU doorbell, `ring_internal`) | token Idle->Rung, bit, `bump` | `worker_efd` write iff a poller is registered (`SignalOne`) + `seen` compare in `try_park` | any worker that scans | yes, except **RC-02** (scan limit + stale bits: rescued only by the net) and **RC-03** (starved, never scanned) | `trap.rs:150`, `plane.rs:308-316`, tests `audit_scan_*` |
| 2 | token rung by a holder after release | `PtGuard::drop` / `handoff_released` -> `ring_internal` + efd | as 1 | any | yes | `ptlock.rs:116-120`; ring result honoured (`chan.rs:2932`) |
| 3 | in-flight Translated fence | host FIFO edge | completion fd in every worker's poller; `for_each_inflight` rings tokens | first poller to observe it (UNCERTAIN Q1) | yes if at least one worker sees it | `worker.rs:174-196`, `completions.rs:70-80` |
| 4 | guest completion interrupt (Passthrough NSI) | per-engine non-stall event | engine fds (tag >= 2<<32) -> `nsi_engine_edge` -> `latch_and_deliver` | first poller (Q1) | yes; if `poller.watch` of an engine fd is refused the source is gone (eprintln only: `device.rs:3592`) | net-dependent in that case |
| 5 | host RC (channel error) | RC fd | tag `RC_TAG_OFFSET` -> `rc_scan` (blocking `pt.lock()`, W8) | first poller | yes | blocking lock on a worker |
| 6 | relay Marker wait | marker's NON_STALL interrupt -> REFERENCE visible | engine fds + `peek(Event)`; a spent hint marks `Lagging` -> spin | first poller | designed non-missable; **RC-04** over-triggers the spin on unrelated events | `userpump.rs:3078` |
| 7 | relay Popped / credit wait, `put_full` with partial pops | private GP_GET moves | none by design: `unannounced` spin | n/a | spin (CPU) | **RC-05** spin on a disabled/hung channel; 2 UC loads per iteration |
| 8 | relay Contended (lost `pt` / rows / ram / USERD-admission / storage lock) | lock released | lock HOLDER: `Handoff::released` (PtGuard drop; drainer after a pass with n > 0; VA loop end; act end; `ram_add/del`) | any | yes for holders that call it | gaps: peer-worker holders of the record lock (**RC-01**), drainer holds outside `drainer_pass`, VA hold across a long iteration (latency only) |
| 9 | Translated slot `try_lock` miss in `serve` | holder (drainer/act/probe) releases | **none** (`chan.rs:7736-7740` returns false, token released, no re-ring) | n/a | **missable** (catalogue RELAY-SLOT-TRYLOCK, still reachable) | not rescued by the net either |
| 10 | Translated stash waiting for a VA walk | walk lands | `wait_walk` registers + signals the VA wake; VA thread idle -> `ring_internal` + efd | any | yes (Dekker through the VA eventfd) | `mem.rs:2329-2346`, `device.rs:2398` |
| 11 | Translated ring not scheduled / disabled / stopped | enable act | `ring_internal` + signal at every enable site | any | yes | `chan.rs:3628,4415-4430,4729-4739,4755-4764` |
| 12 | USER-ring demand left by an act (birth, enable, software object) | act completes | `user_ring_wake` = `request()` + `wake_if` + unconditional efd signal | any | yes except the **RC-08** window and error returns that skip the wake | `userpump.rs:3180-3195` |
| 13 | paced Passthrough raise (default off) | pacing interval elapses | the TIMER (1 ms park when `owed`) | n/a | by design a timer; **RC-06** stale `owed` | `worker.rs:100-109,252` |
| 14 | `stop` | `Device::stop` | one efd signal | first waker only | **RC-07** second worker exits at the net | `device.rs:3551` |
| 15 | relayed twin GP_GET refresh (`KF3_RELAY_GET_REFRESH`, default off) | tick | after every park wake (`TICK_TAG`) - a timer | n/a | by design | `worker.rs:280` |
| 16 | trace re-reads (`KF3_USER_RING_TRACE`) | tick | `user_ring_poll_due` -> 1 ms park | n/a | measurement only | `userpump.rs:3090` |

**Paths that only work because of the 50 ms net on master** (each is a counted bug when it fires, `timeouts_with_work` / `backstop_found_*`): RC-02 (the next loop iteration after a net
timeout rescans the bitmap); row 4 with a refused engine watch; RC-06 (stale `owed`); RC-07 (second worker stop); RELAY-F5 on the narrow path where the record is held by an ACT that does not
call `user_ring_wake` (below). **Not rescued even by the net** (stronger than load-bearing): row 9 (slot try_lock miss), RC-01 (dropped relay demand), RC-03 (starved token).

### 4.3 Handoff / epoch (ABA)

`Handoff` (`handoff.rs`): the Dekker pair is correct as written (loser: `prepare()` -> attempt -> `register` (waiters++ under the list lock) -> `changed_since` ; holder: lock released ->
`bump()` -> `waiters()` -> `take()` -> ring; all `SeqCst`); `epoch` is a `u64` (no wrap), `register` is idempotent, `take` swaps under the list lock, `ring` runs outside every lock. A
stale registration for a retired / reallocated token yields a spurious ring only. Findings are about COVERAGE, not the protocol: (a) the model (`relay_fairness`, `handoff_protocol`)
assumes every holder of every lock a relay can lose calls `released()`; holders that do not: the peer worker's `peek_one`/`alive()` (RC-01), `wake_if` (RC-08), the diagnostic thread
(atomics only); holders that call it late: the drainer only after a pass with `n > 0` (`device.rs:2632`), the VA thread once per loop iteration, the act thread after the whole act
(RS-4: up to 200 ms of sleeps); (b) `take()` ignores a poisoned list (`unwrap_or_default`), leaving registrations unrung - needs a panic under a Vec push, negligible.

### 4.4 RELAY-F5 on master

Catalogue RELAY-F5 ("a look that found the record busy consumed the engine event and rang nothing") was FIXED on the relay-simple lineage (`aad1e9ff4`, not an ancestor of master). Master's
`user_ring_peek_one` still returns false on a failed `record.try_lock()` (`userpump.rs:2980`). Master's one-big-loop covers the case where the holder is a worker mid-turn (its own
post-turn `peek(Spin)` re-evaluates) and where the holder is a peer's look; it does NOT cover a holder that is the act thread in an act that never calls `user_ring_wake`
(`finish_user_ring_free`, `release_never_born_record`: the relay is being retired, so benign) - F5 is therefore **reachable only on retiring relays: LOW (INFERRED from the holder enumeration, not demonstrated)**. The stress harness that found it
(`relay_stress.rs`, seed 20) is not on master; RELAY-F4, RELAY-SEED1SLOW and RELAY-I6FP cannot be re-run here.

## 5. Catalogue status on master (81 rows; the 29 open and 7 unknown-cause first)

Legend: **R** still reachable on master (CODE READ at the cited lines), **F** fixed / superseded on master, **?** cannot tell GPU-free, **NA** not re-audited.

| id | status | note |
|---|---|---|
| RELAY-F1 baton strand | F (model) | handoff M on master; hardware A/B still pending; residual = RC-01 (record holder that does not hand off) |
| RELAY-F2 pop write-back | F | `peek_one` rings `Marker|Popped` with partial pops (`userpump.rs:3010`) |
| RELAY-F3 busy worker never looks | F | one big loop |
| RELAY-F4 seed 18 / SEED1SLOW (UNKNOWN) / I6FP | ? | stress harness not on master |
| RELAY-F5 | R (narrow, see 4.4) | |
| RELAY-LOSTREG | F | epoch re-read in `lost()` |
| RELAY-WATCHED | F | `Watched` wrapper is not on master's `worker.rs` |
| RELAY-IRQVIS (open) | R | answered by the `Lagging` spin; RC-04 shows the spin over-triggers |
| RELAY-GUESTFULL-RED (UNKNOWN) | ? | test not on master |
| RELAY-NSI-UNARMED (open) | NA | `ptnsi.rs` not re-audited |
| RELAY-INTR-SHADOW (open) | **R** | AT-2 |
| RELAY-TRANSLATED-DEAD (open) | **R** | `chan.rs:7964` `g.dead`, `serve` returns false at `7744`; `host.rs:1778` |
| RELAY-SLOT-TRYLOCK (open) | **R** | `chan.rs:7736-7740` |
| RELAY-PREEMPT139 (open) | **R** | `device.rs:3467` drops entries (no display model / no registration / non-QueueFull refusal); queue cap 64 (`chan.rs:565`) |
| RELAY-HELDREPLY (open) | **R** | `boot.rs:2764` FIFO + no deadline; `2726` decrements the doorbell count before the post |
| RELAY-EVENTDROP (open) | **R** | `boot.rs:2882-2890`, `device.rs:3373,3408,3521`, `boot.rs:1524-1541` |
| RELAY-DISABLE-STICKY (open) | **R** | `chan.rs:4395` sets `g.disabled` before `disable_channels(..)?` at 4403 |
| BLOCK-RMIOCTL (open) | **R** | `userstore.rs:392` (`prepare` on the worker), `chan.rs:1325` |
| BLOCK-UCCOPY (open) | **R** | VA-10 |
| BLOCK-DRAINER-SLOT (open) | lane | blocking `slot.lock()` sites listed in section 3 |
| BLOCK-RCSCAN (open) | lane | section 3 |
| BLOCK-LOG (open) | **R** | `device.rs:3633` `eprintln!` per engine wake when `maplog::on()` |
| BLOCK-VCPU-RAMMAP (open) | **R** | `mem.rs:150-166` writers on the BQL/vCPU thread behind VA-10 readers; PR-1 for `dbfast` |
| BLOCK-BACKSTOP | F | `CONTENTION_RETRIES` and the Backstop exemption deleted; but see RC-01 (demand ignored) |
| VA-HANG-E4 | F | clear over absence |
| VA-ARMED-FOREVER (open) | **R** | VA-4 / VA-5 |
| VA-STALECLEAR (open) | R, narrower | seq CAS stops a stale clear by sequence; the scope is still rebuilt from live latches (`mmuinval.rs:236`, GM-6) |
| VA-LOSTSPECIFIC (open) | **R** | only the newest arm observed (`device.rs:2180-2185`) |
| VA-BACKLOG (open) | R, bounded | one want per VA-loop iteration, no coalescing |
| VA-PTLEAVES (open) | partly F | residual: user-ring private RO storage is a host-owned VA in a Passthrough space (`userva.rs:16-18,545-568`) |
| VA-MISALIGNED-LEAF (UNKNOWN) | ? | |
| VA-UNCHANGED / HELDHOLE / STRADDLE | F (pinned) | VA-13 lists the residual transients (CPU windows re-express; PARTIAL path) and VA-9 a new hole |
| HOST-DEVRESET | F design / residual RS-1..RS-7 | |
| HOST-S3 | F GPU-free / hardware ? | `wake_reset`, `resume` present |
| LIFE-*, DISP-*, HOST-* (other), PROC-* | NA | not re-audited |

## 6. The 1-worker case (owner wants `min(2, vcpus)`)

Hard-coded: `ffi_unsafe.rs:221` `for i in 0..2`; no property, `Config` has no worker count and Rust never learns the vCPU count. The wake word supports <= 255; `relay_fairness` models 1, 2, 3 workers
(`the_safety_net_is_never_needed_with_one_and_two_workers` green); `handoff_protocol` is thread-count independent.

* **What disappears with one worker:** RC-01 (needs a peer's look), `user_rings` map contention between workers, the first-poller-consumes-the-event question (Q1), the dispatch race on `efd.drain()`.
* **What gets worse:** (a) every blocking call is now a full outage of relay service, engine-event delivery and completions - the longest blocking call IS the latency of everything (W1/W2 RM
  ioctls, W3/W4 UC copies of 60-200 ms, `rc_scan`, `eprintln!`); with two workers the second keeps serving. (b) The `unannounced` spin (RC-04/05) pins the only worker; on a host with 1-2
  cores it competes directly with the vCPU thread and the VA thread (a hot-spinning worker owns up to a core). It still serves tokens every iteration, so no starvation of service, but
  guest-visible slowdown. (c) The `NotYet` branch of `worker_pass` spins without a wake source until the mirror catches up (production `operands_translatable` never returns `NotYet`, so this is
  test-only today). (d) RC-02 / RC-03 are unchanged (single scanner: RC-03 is worse, there is no second scanner to run between the attacker's re-rings).
* **What does not depend on the count:** token claim exclusion (`claim` CAS), handoff, `try_park`, the act thread's `retire` loop.
* **What to do when making it configurable:** Config field + loop; keep the join handles (RC-11); stop must signal once per worker or loop-signal until all exited (RC-07); the PROF/heartbeat
  lines assume two; run `relay_fairness` workers = 1 as the default arm; add the RC-04/05 spin guard before dropping to one worker on a small host.

## 7. UNCERTAIN - precise questions for a stronger model (Opus)

1. **NVIDIA event fd semantics.** Does `nvidia_poll` clear the dataless-event flag on the first `file->poll` that observes it (so with two epoll instances only the first worker to poll sees
   a completion / engine event), or is it cleared only by `read()`? Nothing in the repo reads these fds (`worker.rs` dispatch drains only `worker_efd`). If the first poller consumes it, the
   sleep-safety table's claim "level-triggered in every worker's own poller, reaches every worker" is false and every obligation whose only source is an engine fd must be re-derived for the
   case where the consuming worker is mid-turn (row 3/4/6).
2. **Does the guest issue ALL_PDB (or `Split{pdb:None}`) invalidates, and how many rooted spaces does a typical Windows 11 / Linux desktop hold?** Decides whether VA-2 is CRITICAL.
3. **Can a record-lock hold by a peer worker's look strand a relay forever?** (RC-01) Construct, or refute, a schedule where the dropped demand is the ONLY thing that can complete the
   relay's recorded wait (cross-channel semaphore chain / ring-full guest). If none exists, downgrade to a latency finding.
4. **Is the `Peek::Event` -> `unannounced` rule (RC-04) intended to fire on unrelated events?** If the design wants "spin only for MY lagging marker", what per-relay signal exists in the
   event (the non-stall event carries no channel identity: `completions.rs` header)? Is a bounded spin (N iterations then block with the net as the only wake) acceptable under "no heuristics"?
5. **Hardware semantic of a MEM_OP PDB write racing a walk** (VA-4): does RM serialize "latch PDB, then trigger" across vCPUs (decides VA-STALECLEAR / LOSTSPECIFIC reach)?
6. **`PrivRing::push` (AT-1):** is the window reachable in practice (how long can a vCPU thread be descheduled between two adjacent atomic loads on the nested boxes)? Is a sticky poison the
   right response to ANY apparent overflow?
7. **RS-1/RS-2:** is the relayed-twin route (Translated token for a non-USER-ring Windows twin, `chan.rs:6948`) active in the production Windows configuration?
8. **Completion-interrupt vs GP_GET write-back order (event notification ordering):** `dispatch` injects the guest interrupt (`on_other` -> `nsi_engine_edge`) BEFORE `peek(Event)` rings the
   relay whose turn performs `write_guest_get`. Is there any guest path (ring-full waiter, Windows scheduler) that sleeps for GP_GET progress and relies on that interrupt as its only wake?
9. **PAM / SMRAM region flips (GM Q2):** do they produce `kf3_ram_del` + `kf3_ram_add` for RAM sections, bumping `RamMap::generation` (terminal `Topology` for every USER relay)? Self-DoS of
   every Windows relay by guest root if so.
10. **Held replies bound (CU-5):** which production policies hold / defer a hostile repeat?
11. **L1.** Is the kf-rm Graph / osevent / `memory_access` lock order compatible with `Registry.inner` -> Graph (E12)? Not read.
12. **L2.** Is kf-mem `BatchedVas` (`book/own/micro/claims`) truly leaf with no host call under it? Taken from its documentation and the prior audit.

## 8. Coverage

| area | files / lines | depth | by |
|---|---|---|---|
| worker core | `kf-chan/worker.rs` (305), `handoff.rs` (118), `user_loop.rs` (500), `kf-core/plane.rs` (672), `kf-trap/{wake,token,bitmap,trap,ring}.rs` | read fully | auditor |
| relay glue | `kf-qemu/ptlock.rs`, `userrelay.rs` 1-330 + 830-1010, `userquota.rs`, `userpump.rs` 10-45, 2685-3260, 1865-2040, `userstrand.rs` 55-110 | read | auditor |
| device threads | `device.rs` `worker_loop`, `stop`, `bar0_write`, `ram_add/ram_del`, drainer handoff sites, `operands_translatable`; `ffi_unsafe.rs` 1-480, 880-945, 1095-1141; `chan.rs` `serve`, `alive`, 3070-3110, 7725-7760, 5640-5665, 5850-5935 | read by range | auditor |
| design | `V3_USER_RING_EVENT_LOOP_20261014.md` (§0-§10), catalogue (all rows, status column), `V3_BLOCKING_AUDIT_20261020.md` | read | auditor |
| tests run | `handoff_protocol` 6/6, `relay_fairness` 35/35 (debug); the three new red tests | run | auditor |
| lock graph | `kf-chan/src` + `kf-qemu/src` (chan, device, userpump, userrelay, userva, userstore, userread, mem, display, actq, reset, defapi, dbfast, ptnsi) | lane | lock lane |
| VA manager | `kf-mem/src/{vasmgr,apply,batch,ledger,cpuwin,store,maplog}.rs`, `kf-trap/mmuinval.rs`, `kf-qemu/{mem.rs (VA parts),userva.rs,memreset.rs}`, `device.rs` va_loop | lane (vasmgr 1-1810, apply ~60 %, batch 326-3350, mem.rs ~45 %) | VA lane |
| atomics | every `Relaxed` in kf-trap/core/chan/mem/rm/host/gsp/qemu/linux-raw/util/broker/disp (counts: qemu 417, mem 164, util 121, chan 116, rm 71, linux-raw 36, trap 34, broker 19, gsp 6, disp 5, host 5, core 0) | classified; `ring`, `cpuintr`, `shadow`, `relay_dispatch`, `ticks`, `ports` read | atomics lane |
| reset / unload / S3 | `reset.rs` 1-465, `memreset.rs` 1-130, `device.rs` ~2/3, `chan.rs` ~2500 lines, `boot.rs` ~900 lines, `kf3.c` 780-870 + 1560-2026, `actq.rs`, `failclosed.rs` | lane | reset lane |
| guest memory / arithmetic | `user_turn.rs`, `user_gate.rs`, `user_scan.rs`, `user_stream.rs`, `ring.rs`, `userread.rs`, `userstore.rs`, `userpump.rs` ~2300/4164, `tmode.rs` ~1250/2071, `translated.rs` 700+, GSP queues (sub-lane), display (sub-lane ~25 %) | lane | memory lane |
| process / C / dbfast / broker | `kf3.c` (all), `kf3.h`, `dbfast.rs` (all), `broker.rs`, `kf-broker` `conn.rs` 895-1155, `link.rs`, C broker 3241-3412 | lane | process lane |
| **skipped** | lifecycle logic (`user_lifecycle.rs`), `display.rs` ~75 %, `kf-disp` ~60 %, `kf-gsp` beyond queues/held replies, `kf-rm` policies, `kf-mem/sim`, `.cu` kernels, `third_party`, `scripts/`, all `*_tests.rs` bodies, hardware | not read | - |

## 9. Tests added (all `EXPECTED RED`, committed alone)

| test | finding | state as run |
|---|---|---|
| `crates/kf-core/tests/audit_scan_limit_lost_wake.rs` | RC-02 | RED: pass served 0, `try_park` accepted, live token 70 still RUNG with its bit |
| `crates/kf-trap/tests/audit_scan_starvation_same_summary_word.rs` | RC-03 | RED: victim token 100 never scanned in 1000 passes |
| `crates/kf-chan/tests/audit_dbfast_service_ready_is_bounded.rs` | PR-2 | RED: `service_ready` did not return in 3 s |

Not written (and why): AT-1 (needs an injected delay between two loads), AT-2/AT-3/RC-01/RC-04 (the structures live behind a live `ChanPlane`; they need the proposed `look()` refactor from
the event-loop doc §9 or a source pin, and a red source pin proves a text, not a behaviour), VA-1/2/4 (need `ModelWalker` knobs the lane specified).
