**STATUS: LIVE in the code, 2026-10-14** (branch `claude/userring-eventloop-20261014`: tests-first commit `95a9ad422`, then the
fix; GPU-free results in §5-§6, Linux hardware validation in §8). Written DESIGN-ONLY first; §2 and §5 keep the
state of the code BEFORE the fix (77eaf7620) as the record of what the tests found.
Labels: **[measured]** = a run or a test result named here; **[read]** = public hardware text (`dev_pbdma.ref`,
`dev_ram.ref`, ogkm 580) or kayfabe code; **[inferred]** = reasoning, never evidence.

# V3 — the USER-ring relay as an event loop: no timer carries correctness

## 0. Short answer

**UPDATE (end of the work, read this first):** the event loop is in the code and validated on Linux (§8): fast suite 30/30 with and without the
debug switch, desktop smoke and 3 S3 cycles both ways, gates 9/9 + gate10, the stress tool 642/642 plans through the relay. Validating it
exposed that the SYNC_WAIT gate itself was unsound: **`GP_GET` runs ahead of processing** (bare-metal measurement, §3.3), so a gate that waits for
`GP_GET` lets the relay read bytes the guest has not written. The gate now proves the barrier by a tagged `USERD.REFERENCE` marker; a SYNC_WAIT behind
an open packet cannot be gated and is read at once (a departure from the owner's text, §3.3). Drain path before/after: ~4 ms -> ~60 us per three
SYNC_WAITs, 288 -> 14 000 plans/s (§8.1). The text below up to §5 is the state of the work at each step and is kept as the record.


The relay of Windows USER rings (`crates/kf-qemu/src/userpump.rs`, driven by the one worker of
`crates/kf-chan/src/worker.rs`) learns about everything that is not a doorbell by being re-rung on a
timer: a 1 ms `TICK_TAG` refresh for `tick`/`retry`, a 250 ms active window and a 20 ms idle poll
(`user_gate::Poll`), a 50 ms `Grace` for scan refusals. The owner's rule is that correctness and
performance never depend on timing; a timer is only a worst-case safety net, and every firing that
finds work is a loud counter. This document does three things:

1. states the contract the relay owes (§1, §2) and **corrects the `Poll` comment**, which overreads a
   measurement (§2.2);
2. specifies the event-driven worker loop that meets it (§3);
3. records the failing-first tests (§4, §5): an exhaustive interleaving model of the worker with the
   guest as a nondeterministic writer, a gate/PBDMA parse suite, and the production defects they exposed.

**Result of the tests-first step [measured, `cargo test -p kf-chan`, §5]:** the model finds that every
wait the relay has besides the doorbell itself (a draining SYNC_WAIT gate, a full private ring, a busy
record lock, the write-back of the guest's `GP_GET` that a full guest ring waits for) is noticed only
by the tick — with timers that never fire the relay deadlocks with covered work unrelayed in 4 of 10
scenarios (5 with a 4-entry private ring); and that two **production primitives are wrong independent of the loop**: `SyncGate`
forgets nothing (a SYNC_WAIT entry one ring lap later is waved through, §5.1) and `Grace` finality is a
function of the wall clock (§5.2).

## 1. The owner's principles (binding, this session)

- Correctness and performance must not depend on timing. No timer may fix a race or a perf issue; use
  epoll, events, locks and loops. Busy-polling exists only as a **bounded** latency optimisation before
  blocking; nothing depends on the spin or on a timeout expiring.
- A worst-case backstop wake (arbitrary value, reasoned bound; 50 ms here, §3.6) is only a safety net for a
  lost or never-armed interrupt. Every firing that finds work is a loud counter (a bug), never the normal path.
- One worker waits on ONE epoll for the doorbell eventfd, the host/NVIDIA event fds, and peeks not-yet-fd
  state (semaphores, `GP_GET`/`GP_PUT` words) with a `timeout=0` spin first. It never becomes deaf. The
  vCPU/doorbell path and the drainer never block.
- **The doorbell contract is narrow and weak**: a doorbell makes every `GP_PUT` write that precedes it
  visible, nothing more. `GP_PUT` ahead of the last doorbell is NORMAL (the next doorbell covers it).
- Construction rule: a doorbell turn clears the flag BEFORE reading `GP_PUT`; the worker never blocks while
  the flag, a gate completion or a pending resume is set; a doorbell during a draining gate leaves a level
  state and `GP_PUT` is re-read when the gate completes; each batch of private-ring work ends with a marker
  (an interrupt on the worker's epoll) where a packet boundary allows it; inside an open packet the length-0
  control entry carries no interrupt and that case is counted separately.
- Correctness comes from construction plus an exhaustive model, not from measuring doorbells.

## 2. What the code does today (read at 77eaf7620)

### 2.1 The loop

`kf_chan::worker::run` (`worker.rs`): `on_tick()` → `seen` → `worker_pass` → park by `try_park(seen)` →
`epoll_wait(1 ms if owed else 50 ms)` → handle the ready tags → `on_other(TICK_TAG)`. For the relay:

| piece | file:line | what it does | timer-carried? |
|---|---|---|---|
| doorbell → token RUNG → `worker_pass` claim → `user_ring_serve` | `userpump.rs:2227` | `Cell::enter()` sets `retry` then `try_lock`; `Pump::turn` | no (event) |
| `Pump::turn` reads `GP_PUT` only with no segment in flight | `userpump.rs:1262` | after the claim cleared RUNG | no |
| gate: `SyncGate::entry` → `Wait`/`PublishBarrier` | `userpump.rs:1305` | `tick = true; return` | **yes** (1 ms) |
| no private credit (`fetch.available()`) | `:1319, :1408` | `tick = true; return` | **yes** |
| `Busy` guest `GP_GET` write / `WouldBlock` record lock | `:1214`, `:2229` | `tick = true` / `retry` stays | **yes** |
| guest `GP_GET` write-back after the hardware passed an entry | `:1198-1219` | only inside a turn | **yes** (a full guest ring waits for it) |
| idle `GP_PUT` read without a doorbell | `Poll`, `user_ring_refresh` `:2323` | every tick for 250 ms, then every 20 ms | **yes** |
| scan refusal retry | `Grace` `user_gate.rs` | 50 ms of wall clock | **yes** |

### 2.2 Correction of the `Poll` comment (and of `V3_USER_RING_SCAN_RACE.md` §4)

The comment on `user_gate::Poll` (and `V3_USER_RING_SCAN_RACE.md` §4) says that "205 doorbells against ~70 000
`GP_PUT` reads on one channel, and pumps idle 1.6-3.3 s with the guest's `GP_PUT` ahead of the cursor, no turn, no
doorbell, right before the guest's per-channel recovery" show that the guest advances `GP_PUT` without a doorbell.
**It does not.**

1. [read, `userpump.rs:262 note_put`, `:1266`] `puts_with_doorbell` / `puts_without_doorbell` are bumped once per
   TURN that found `GP_PUT != cursor`, and a turn relays exactly ONE GP entry (`cursor` advances by one, `again`
   re-rings). So 69 971 "puts without a doorbell" are 69 971 relayed ENTRIES that followed the first entry of a
   doorbell's batch: 69 971 / 205 is about 340 entries per doorbell, the guest's batching — the weak contract the
   owner states (a doorbell covers every `GP_PUT` write before it) — and says nothing about a missing doorbell.
   The counter also counts deliveries (`DbFast` coalesces the stores of one eventfd read), not stores.
2. [owner's reading, consistent with the data; not re-measured here] the idle windows of 1.6-3.3 s match the guest's
   2 s timeout detection (TDR wait), and the Xid-32 hang that began them started about 2 s EARLIER; they are the guest waiting
   after the relay had already stopped serving, not proof of a doorbell-less submission.

`dev_pbdma.ref`'s statement that Host synchronises `GP_PUT` by itself while a channel is loaded describes hardware, not
something a driver may rely on without a doorbell, and a lost doorbell cannot be measured reliably: correctness comes
from construction and the model (§4). The comment is corrected in the fix commit and `Poll` is removed, not re-tuned.

### 2.3 Cost in the model's terms

Every one of the table's "yes" rows is a place where progress depends on time passing. The model (§4) makes
that checkable: time is a scheduler step the configuration may forbid.

## 3. The design

### 3.1 State and wake sources of the worker

Per relay record: `waiting: Option<Wait>` where `Wait::Fetched{target, carried}` (private `GP_GET` must reach
`target`; `carried` = the marker carries no interrupt), `Wait::Credit` (any private progress frees a slot).
The durable demand flags that exist already stay: the token's RUNG/BUSY_RUNG word (the doorbell flag) and
`Cell::retry` (a turn deferred by a busy lock). New wake sources on the ONE epoll:

| source | readiness | action |
|---|---|---|
| doorbell eventfd (`worker_efd`) | existing | claim, turn (flag cleared before `GP_PUT` is read) |
| host engine non-stall event (`OTHER_TAG_BASE+i`) | existing fd, new handler | clear, then peek every relay's `waiting` (`GP_GET`); ring the satisfied |
| lock release (the act thread's long holds: birth, the 5080 object) | the act's completion rings (`user_ring_wake`, existing) | durable demand `Cell::retry`, set BEFORE `try_lock` |
| a busy lock / admission nobody announces (`pt`, USERD admission) | none exists | `Wait::Contended`: re-rung by the spin / event looks at most `CONTENTION_RETRIES` times in a row (`user_loop::may_retry`), then only by the backstop, which counts it (`backstop_contended`) |
| walk done | existing (`wait_walk`) | unchanged |
| backstop | `epoll_wait` timeout | §3.6 |

### 3.2 A turn

Unchanged order, new ending (the `Need` it returns replaces `tick`/`poll`): claim (RUNG→BUSY) → observe
private `GP_GET` and write the guest's `GP_GET` back → read `GP_PUT` → read the entry → gate → read the
pushbuffer → publish → `Again`. It ends with one of:

- `Again` — progress was made: ring the token again (BUSY_RUNG; the bounded `REACT_ROUNDS` keep the worker
  fair). No timer, no park.
- `Idle` — `GP_PUT == cursor`, nothing in flight.
- `Private{target, carried}` — wait for the private `GP_GET` to pass `target` (a draining gate witness, or
  work in flight whose progress frees the guest's ring through the `GP_GET` write-back).
- `Credit` — the private ring is full: wait for any private progress.
- `Contended` — a lock or admission was busy (`AccessError::Busy`, `Failure::Retry`, a failed `record.enter()` /
  `pt.try_lock()`): the relay looks again, a bounded number of times, then only the backstop does.

The gate keeps the real `SyncGate`. A doorbell that arrives while the gate drains sets RUNG as always; its
turn finds `Gate::Wait` and ends in `Private` again, and when the witness is fetched the completion rings the
token so `GP_PUT` is re-read before reading on (the "pending" flag of the principles is the level state
`waiting` plus the fresh `GP_PUT` read at every turn start).

### 3.3 The marker: proof AND wake (CORRECTED 2026-10-14 by a hardware measurement)

> **[measured] `GP_GET` is not a gate proof.** `kf-syncwait-stress --families barrier` on bare metal (RTX 4070 /
> 595.91.07): an ACQUIRE stalls entry 0; entries `[acquire segment][LENGTH-0 SYNC_WAIT][segment]` are published.
> While the acquire is unsatisfied `GP_GET` is already 3 (all entries; with the other shape, 2 of 2), the segment
> behind the barrier has NOT executed and `USERD.REFERENCE` is 0; when the gate opens the segment runs and
> `REFERENCE` becomes the `SET_REFERENCE` value. `GP_GET` counts entries the PBDMA has POPPED from the GPFIFO and
> runs ahead of processing. The gate that the previous revision (and this document's first version) built on
> "`GP_GET` past the witness means the barrier's wait finished" was therefore unsound: through the relay with the
> debug switch the `late` stress family failed 49 of 60 plans, the scan meeting the POISON the guest had not yet
> replaced and refusing the channel. This is the failure `V3_USER_RING_SCAN_RACE.md` §3 set out to fix; the earlier
> "0 stale reads in ~100 000 segments" measured Windows traffic whose bytes arrived before the relay looked.

The proof is a processed method with a CPU-visible effect: the **marker**, a private 16-byte segment per ring slot,
`[SET_REFERENCE tag|n, NON_STALL_INTERRUPT, 0]` (public C56F host methods; no semaphore memory, which would need a
writable mapping the restricted space does not authorise, OWNER_RULINGS §AI). Processing it writes `USERD.REFERENCE`
(in order, after every earlier method, acquires included) and raises the engine's non-stall event the worker's epoll
already watches. `user_gate::Marks` keeps `issued`/`seen` counts; only a value carrying the tag `0x4B46` in the high
half counts (the guest's forwarded segments may themselves execute `SET_REFERENCE` on the private channel; untagged
values are ignored, and a guest that forges the tag only reads its own late bytes early). Where the marker goes:

- **gate**: barrier (LENGTH-0 SYNC_WAIT) then marker; the gated entry is read when `Marks::seen >= target`;
- **batch**: when a turn finds `GP_PUT == cursor` with unmarked work and the scan is at a packet boundary, a marker
  ends the batch; the relay waits for it (`Wait::Marker`): its processing implies the pops the guest's `GP_GET`
  write-back needs, and its interrupt is the wake. One private slot is kept in reserve so a full ring can still
  publish its marker before waiting on credit.
- **open packet**: a SYNC_WAIT entry reached with a method packet open across it **cannot be gated**: no witness may
  enter an open packet (the original Xid-32 PBENTRY) and a control entry proves nothing. The hardware does not delay such a
  segment either (`GP_ENTRY1_SYNC`: once the split packet's header is read the preceding segment counts as processed), so
  its bytes are there when the guest publishes; the relay reads it at once and counts it (`carried_ungated`). A guest that
  writes such an entry's bytes late is NOT protected (`design_a_sync_wait_behind_an_open_packet_cannot_protect_late_bytes`).
  **This departs from the owner's text** ("use the length-0 control entry ... the wake comes from the spin of the `GP_GET`
  word"): there is no honest wake for a proof that does not exist.

Pops carry no event: while a marker is outstanding the worker's looks (spin, any engine event, the safety net) also ring a
relay whose pops outran the guest's `GP_GET` so the write-back happens (`pop_writebacks`).

### 3.4 The spin

When no token is RUNG and some relay is `waiting`, the worker does up to `SPIN` iterations of
`epoll_wait(timeout=0)` plus a peek of the `waiting` relays' `GP_GET`, then parks. A satisfied peek rings the
token and the loop continues. `SPIN` is a latency knob: the model proves liveness with `SPIN = 0`
(`design_never_depends_on_the_spin`).

### 3.5 The park

`try_park(seen)` keeps its rule (sequence compared after the scan; refused if a ring bumped it), the lost-wakeup
window of which the model reaches and the mutant without the comparison (`mutant_lost_wakeup_is_found`) breaks.

### 3.6 The backstop

`BACKSTOP_MS = 50` (the existing `PARK_MS`; unchanged idle wake rate, 20/s, an order below the guest's 2 s TDR
detection, and the only timer in the loop). A backstop wake peeks every relay's `waiting` and `GP_PUT`. What it
finds is counted, not hidden: `backstop_found_work` (a satisfied non-carried wait: a lost or never-armed
interrupt, a bug), `backstop_carried` (a satisfied control-witness wait: expected, separate), `backstop_put_ahead`
(`GP_PUT` ahead of the cursor with no doorbell: legal in the narrow window between a guest's `GP_PUT` write and
its ring; information about guests that skip doorbells), `event_without_progress` (an interrupt before `GP_GET`
was visible). The model proves the backstop is NEVER what completes a gate when interrupts are delivered
(`design_b_no_turn_is_caused_by_a_timer_and_the_backstop_finds_nothing`) and that a lost interrupt is covered by it and counted
(`design_a_lost_interrupt_is_covered_by_the_backstop_and_counted`).

### 3.7 Counters (bounded in production; per-event detail only behind `KF3_USER_RING_TRACE`)

`kf_chan::user_loop::Stats` (fixed atomics, no per-event allocation): wake source (`wake_doorbell` /
`wake_event` / `wake_spin` / `wake_backstop`), the four backstop counters above plus `backstop_contended`,
`gates`, `carried_gates`, `credit_waits`, `contended` (turns), `markers`, and a SYNC_WAIT drain latency
histogram (16 log2 buckets of microseconds, gate published → witness fetched). One `USER_RING_LOOP SUMMARY`
line at device stop, the same text in the `PROBE-DUMP` device line (`relay[...]`), per-event detail only
behind `KF3_USER_RING_TRACE`. The per-channel `USER_RING_SYNC_CARRIED` lines stay.

## 4. The tests: files, scenarios, what each asserts

### 4.1 `crates/kf-chan/tests/user_loop_model.rs` — the exhaustive interleaving model

Not `loom`: `loom` explores memory orderings of real threads; this model needs a state machine with blocking,
events and absent timers, so it is a deterministic explicit-state explorer (breadth first, every interleaving of
four actors; no `Instant`, no sleep, a "timer" is a scheduler step a configuration may forbid). `loom` stays
where it belongs: `kf-trap` models the token/wake words with it (`REACT_ROUNDS` and `try_park` are checked
there); the model here reuses those rules as steps. Actors: the **guest** (script of `Fill` late pushbuffer
bytes, `Push`, `SetPut`, `Ring`, `Release` of a semaphore, `UseNew` ring, hostile `PutRaw`, `Retire`), the
**PBDMA** (private ring in order; `GP_GET` passes an entry when it begins; a segment's acquire blocks the next
entry; a marker raises the completion interrupt; optional visibility lag of `GP_GET`), the **environment** (act-thread lock
windows; elapsed time and park timeouts only if allowed), the **worker**, one of two policies: `Today` (a
transliteration of `worker.rs` + `Pump::turn` + `user_gate::Poll`, line references in the source) and `Design`
(§3). The turn is a transliteration, with the REAL `kf_chan::user_gate::SyncGate` and `witness()`; each micro-step
(claim, observe, write-back, read PUT, read entry, gate, read pushbuffer, publish) is atomic so any other actor
runs between any two. 10 scenarios (batched writes with one ring, PUT ahead of the ring, coalesced rings,
SYNC_WAIT with a late-written pushbuffer, `LENGTH == 0` entries first/middle/last/back-to-back, ring wrap with
the guest blocked on the relay's `GP_GET` write-back, migration with and without a gate, two channels one stuck)
x configurations (no timers / timers+backstop, interrupts delivered / lost, `GP_GET` lag, spin 0/1, private ring
4 or 8, a busy-lock window).

Checked (all derived from §1 and `dev_pbdma.ref`, not from the code): (1) liveness with timers that NEVER fire:
at every terminal state every entry covered by a doorbell was relayed and the guest finished; (2) no turn is
caused by a timer (`TIMER_TURN`); with delivered interrupts the backstop never finds work; (3) a pushbuffer is
never read before the guest wrote it / before the drain (`STALE_READ`), and a method byte is never published
inside an open packet (`BYTES_IN_OPEN_PACKET`); (4) the PBDMA executes the guest's entries in order, none dropped
or replayed (`ORDER`), the relay never publishes more than one ring lap beyond them (`UNBOUNDED`); (5) the worker
is never parked with a doorbell flag pending (`PARKED_WITH_WORK`). **Teeth:** five mutants of the design (no
sequence check in `try_park`, a ring while BUSY dropped, byte witness always, no gate, no marker, no lock wake)
are each FOUND; `the_design_exploration_reaches_every_race_it_claims_to_cover` asserts the races (park refused
because the sequence moved, completion wake, spin, gate wait, credit shortage, busy record, write-back) are
actually reached; state counts 125 .. 934 455.

### 4.2 `crates/kf-chan/tests/user_gate_rules.rs` — the gate and the PBDMA parse

The relay's gating loop (real `SyncGate`, `Scan`, `Fetch`) against a PBDMA parse model that parses ACROSS
entries: every one of the 1024 cut sets of a boundary-crossing stream x every SYNC_WAIT mask (118 098 cases)
and 4 000 random streams (random packet forms, counts, random data words that look like headers) with random
cuts, `LENGTH == 0` control entries (SYNC_WAIT or not) first/last/back-to-back, over rings of 4-12 slots so the
cursor wraps: the private ring, parsed like the PBDMA, generates exactly the guest's methods. Plus the gate's
own rules: a retried turn publishes one barrier; a length-0 SYNC_WAIT is gated; a new ring generation is gated
afresh; refusal finality needs no clock.

## 5. Results at 77eaf7620 (tests-first commit `95a9ad422`; measured: `cargo test -p kf-chan --test user_loop_model --test user_gate_rules --release`, ~25 s)

`user_loop_model`: 32 tests, **25 pass, 7 fail**. `user_gate_rules`: 9 tests, **6 pass, 3 fail**.

| test | result | what it revealed |
|---|---|---|
| `today_a_relay_is_live_without_any_timer` | FAIL | 4 of 10 scenarios deadlock with covered work unrelayed and no timer: the late-pushbuffer gate (executed `[1]` of 3), `LENGTH == 0` SYNC_WAIT entries (nothing executed), ring wrap (the guest blocked on a `GP_GET` write-back that only a tick performs), migration behind a gate |
| `today_credit_waits_are_event_driven` | FAIL | same plus plain migration with a 4-entry private ring |
| `today_a_turn_deferred_by_a_busy_lock_is_resumed_by_an_event` | FAIL | a doorbell deferred by a busy record waits for the tick (nothing rings on release) |
| `today_migration_is_live_without_timers` | FAIL | the gate behind a migration |
| `today_b_no_turn_is_caused_by_a_timer` | FAIL | all 10: the tick/Poll idle refresh rings a channel nobody rang (`TIMER_TURN`) |
| `today_a_channel_stuck_in_a_gate_costs_no_wakeups` | FAIL | a gate nobody releases is re-polled at 1 ms for as long as it is stuck |
| `design_a_sync_wait_in_the_same_slot_one_lap_later_is_not_read_early_fails_today` | FAIL | the PRODUCTION primitive: §5.1 |
| `a_sync_wait_entry_one_ring_lap_later_is_gated_again_fails_today` | FAIL | §5.1 |
| `the_same_slot_gated_twice_a_lap_apart_authors_two_barriers_fails_today` | FAIL | §5.1: 2 markers instead of 4 |
| `a_refusal_becomes_final_without_any_time_passing_fails_today` | FAIL | §5.2 |
| the 25 + 6 others | pass | the design satisfies the same properties (liveness with no timers, spin 0, lost interrupts via the backstop, lag, migration exactly-once, wrap, hostile PUT, storms, retire in a gate, two channels), Today passes the safety half and the scenarios without a wait, the five mutants are found, the exploration is non-vacuous, the PBDMA parse suite is green |

The Today failures are one class: **every wait other than the doorbell is a tick**. Scenarios without a wait
(`put_then_ring`, `batched`, `put_ahead`, `coalesced`, plain migration with a 8-entry private ring) pass without
timers, which also says the doorbell path itself (token word, `try_park`) is right.

### 5.00 The redesign after the hardware measurement (§3.3) -- model results

The model's hardware gained `GP_GET` prefetch (entries popped at once) and `USERD.REFERENCE`; the first design (proof
by `GP_GET`) then FAILED 21 tests with `STALE_READ` (commit `GP_GET runs ahead`, failing-first); after the redesign the 37
model tests pass, including: the `GP_GET` proof as a mutant is found (`STALE_READ`), a sequential `GP_GET` would have
made it correct, credit waits with a shallow pop window (depth 1) are event-driven. Known limitations pinned as tests:
a SYNC_WAIT behind an open packet cannot protect late bytes; a guest whose GPFIFO is FULL behind its own gated entry
deadlocks (the guest-visible `GP_GET` is the relay's write-back and a gated entry is relayed only after its gate, while on
the hardware `GP_GET` passes it at once) -- it takes a full ring, 32767 entries on a 32768-entry ring, but it is a
fidelity gap.

### 5.0 After the fix (this branch)

[measured, host, `cargo test -p kf-chan --release`] `user_loop_model`: 34 tests, 34 pass (the six `today_*` tests became
`the_retired_loop_*`: they now assert that the checker still REJECTS the retired loop, at exactly the scenarios in the
table above); `user_gate_rules`: 9/9; `kf_chan::user_loop`: 8 unit tests. The model's Design policy now calls the
production decision functions (`user_loop::{idle, wait_of, found, may_retry}`, `Wait::satisfied`) and the real
`SyncGate`; the turn remains a transliteration (§6). Fixed: `SyncGate` forgets a proven wait when the relay looks at
another entry (§5.1); `Grace` counts re-reads (`REFUSE_RETRIES`, §5.2); `Poll`, `tick`, the 1 ms refresh and
`user_ring_poll_due` are gone from production (the refresh survives behind `KF3_USER_RING_TRACE` for the trace's
deferred re-reads only); the private-ring batch marker, the engine-event / spin / backstop looks and the counters of §3
are in `userpump.rs`, `worker.rs` (`run_with_relay`) and `user_loop.rs`.

### 5.1 Production defect: `SyncGate::cleared` is never forgotten

[read + measured] `SyncGate::entry` returns `Proceed` for any entry equal to `cleared = Some((generation,
cursor))` and `cleared` changes only when ANOTHER gated entry clears. After one lap of non-SYNC_WAIT entries
the ring reaches the same `(generation, cursor)` with a new SYNC_WAIT entry and it is waved through: the relay
reads a pushbuffer the hardware has not fetched (the original `Legacy`/`MethodOverflow` failure of
`V3_USER_RING_SCAN_RACE.md` §3, one lap later). The model reaches the stale read in 75 steps. Rings are large on
Windows (32768-131072) so a lap without a SYNC_WAIT is rare there; the Linux guests with the
force-migratable switch (§6) have 512-entry rings. Fix: forget `cleared` when the entry at `cleared` is relayed
and the cursor moves on (the retried-turn guarantee, `a_retried_turn_at_the_same_gated_entry_publishes_one_barrier`, is pinned).

### 5.2 Production defect: `Grace` finality is a function of the clock

[read + measured] `Grace::refused` returns "retry" until 50 ms of wall clock have passed; with the clock frozen it
never becomes final. After the SYNC_WAIT gate no retry was ever observed (`V3_USER_RING_SCAN_RACE.md` §5, §8), and a
refusal of a segment that is not behind an unreached SYNC_WAIT is deterministic (the bytes are written before
the GP entry, the GP entry before `GP_PUT`, `GP_PUT` before the doorbell). The retry is kept as defence in depth
but its finality counts re-reads (a bounded number of wakes that re-read the same entry), never time. **Open
decision for the coordinator/owner:** remove `Grace` altogether (a refusal after the gate is final at once) or
keep the bounded re-read count; this document implements the re-read count.

## 6. What is inferred, unverified, and open

- **[inferred] The marker's interrupt reaches the worker's epoll.** A `NON_STALL_INTERRUPT` method on the private
  twin raises the engine's non-stall notifier whose host event the worker already watches (`engines[i].ev`); the
  per-engine mapping for a user-work twin is not measured. Falsifier (Linux validation): the marker count and
  the completion wakes agree, `backstop_found_work == 0`. The same interrupt reaches the PT-NSI relay and may raise a
  spurious guest interrupt per batch (owner ruling §X permits it; rate to be measured).
- **[inferred] `GP_GET` is visible when its interrupt arrives.** Not guaranteed (USERD is in video memory, the
  interrupt travels another path). The model shows liveness then needs the spin or the backstop
  (`design_get_visibility_lag_after_the_interrupt_needs_the_backstop`); `event_without_progress` counts it.
- **Control-witness gates (a packet open across a SYNC_WAIT entry) have no interrupt.** Their completion comes
  from the spin or the backstop and is counted apart; the model proves nothing else can do it
  (`design_carried_gate_needs_the_backstop_and_counts_it_separately`, spin-only deadlocks). Latency up to
  `BACKSTOP_MS` per such gate. Measured so far: a handful (`USER_RING_SYNC_CARRIED`).
- **The model's turn is a transliteration.** It uses the real gate, witness and token rules, but `Pump::turn`
  itself needs a ChanPlane and cannot run GPU-free. The mapping is by line; the Linux stress tool (§7) and the
  counters are the empirical check.
- **Doorbell-less submission** is not assumed and not excluded: the backstop reads `GP_PUT` and counts
  `backstop_put_ahead`, so a guest that does it shows up as data and still makes progress at backstop latency.
- **Open:** `RESET_CHANNEL` (G17) stays refused; the model's `Retire` covers a channel retired in a gate, not a
  served reset. Ring-size changes in the model are by `UseNew` (same size); the 32768→65536 growth only changes the
  bound the pump checks (`put >= r.source.1`).

## 7. Plan (steps 2 and 3 of the task)

2. DONE (§5.0). 3. Linux validation behind `KF3_DEBUG_FORCE_MIGRATABLE=1` (debug only, refused in the production profile)
   with a guest SYNC_WAIT stress tool, the fast suite 30/30 both ways, desktop smoke, S3 cycles, gates 9/9 + gate10 (§8).

## 8. Linux validation (measured; kf3 `1d845d50`, host RTX 4070 / 595.91.07, fast guest = the host's kernel and driver, every Linux-guest run under `flock /tmp/claude-linux-guest.lock`, beside the owner's VM 2990 which was not touched)

**The debug switch** `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1` (`kf_rm::chanlink`): every USER channel of the guest on a graphics, copy or video
engine (not guest-kernel, not UVM-owned, not RM-internal) takes the migratable USER-ring relay, with its framebuffer USERD read and
written through the same CPU view a Passthrough twin uses (`userrelay::StoreUserd`). Off by default; without `KF3_DEBUG=1` it is REFUSED by
name (one line) and ignored; never for a guest that declared Windows. A kernel or UVM channel keeps its route (its physical operands need
translation), so "all channels" means all USER channels.

**The stress tool** `kf-syncwait-stress` (`crates/kf-harness`, run in the fast guest through `KF_TOOL_BIN`/`KF_TOOL=1`): own channels, own
GPFIFO/USERD/pushbuffers (framebuffer), host-method streams only (release, reduction-add, acquire), exactly-once check per unit
(value and `count == 1`: a drop or a replay is visible). Families: `split` (every cut of a 3-unit stream x SYNC_WAIT, 82), `random` (cuts,
SYNC_WAIT masks, LENGTH-0 control entries first/middle/last/back to back, four doorbell modes, 150), `late` (pushbuffers POISONed until the
CPU writes them behind a semaphore-gated SYNC_WAIT, 1-3 gates, optional control entry at the boundary, 60), `wrap` (8-entry ring lapped,
200), `storm` (40 extra doorbells, 30), `conc` (three channels, 120), `rate`, `barrier` (the hardware measurement of §3.3).

| run | switch | result |
|---|---|---|
| bare metal (host, uid 65534), all families | - | 642/642 plans PASS; `barrier`: GP_GET already covers all entries while the acquire stalls, REFERENCE 0 -> 7 on the gate (§3.3) |
| fast guest, plain Passthrough (control), `final-off` | off | 642/642 PASS |
| fast guest, relay, rev `025a34fa` (the GP_GET proof) | on | `late` 11/60: **49 channels refused** `USER_RING_SCAN Header ... raw=0xc0001234` (the scan met the POISON) |
| fast guest, relay, rev `375b8fd7` and `1d845d50` (the REFERENCE proof) | on | **642/642 PASS**, 0 `USER_RING_REFUSED`, `backstop_found_work = 0` |
| fast suite, 30 arms (`fastoff`, `faston`) | off / on | **30/30 and 30/30**, 0 refusals over the 30 QEMU logs (`faston`: up to 3 841 doorbells, 779 markers per arm) |
| desktop smoke (Cinnamon/Xorg, glxinfo NVIDIA, glxgears 20 s) | off / on | both PASS: 0 guest "Flip event timeout", host Xid delta 0, guest Xid 0, 58.8-60.0 FPS; `on`: 11 user-ring births, 0 refusals (`nvassert=12` is the known startup count) |
| `desktop_s3.sh`, `S3_CYCLES=3` | off / on | **3/3 and 3/3** |
| `v3_gates.sh` | - | 9/9 + gate10 PASS, births 12/12 USER |
| unit (every `kf-*` crate, release) | - | 2 958 passed; `kf-mem` `the_aggregate_extent_of_the_batches_of_a_space_is_capped` failed once under parallel load and passes alone (kf-mem untouched) |

### 8.1 Before / after: the relay's drain path (`--families rate`, guest, relay forced, same hardware; the owner's Windows question)

400-800 plans of 4 entries / 4 units, SYNC_WAIT on 3 of the 4 entries per plan, spin-polled completion; ring -> last unit.

| | old tick loop (`77eaf7620` + only the debug switch, kf3 `4f45d551`) | event loop (`1d845d50`) | bare metal (no relay) |
|---|---|---|---|
| gateable SYNC_WAITs (boundary cuts) | p50 **4 000 us**, p99 4 100 us, **288 plans/s** | p50 **60 us**, p99 131 us, **13 800 - 14 500 plans/s** | p50 4 us, 110 000 plans/s |
| SYNC_WAIT behind an open packet (the Minecraft shape) | p50 **4 050 us**, 273 plans/s | p50 **39 - 44 us**, p99 69 - 142 us, **19 400 - 20 600 plans/s** | p50 4 us, 109 000 plans/s |

About 50x lower latency and 50-75x the batch rate. The old loop paid about one 1 ms tick per gate (3 gates = ~4 ms); the new one pays
no tick: an ungated open-packet entry costs nothing extra, a gateable one waits for its marker's interrupt or the spin.
Measured counters of the new run: SYNC_WAIT drain latency (gate authored -> marker processed) log2 buckets of microseconds
`[.., 4-8 us: 1 968, 8-16 us: 400, 16-32: 21, ..]` (of 2 400 gates); wake sources over the run `doorbell 5 402, event 7 048, spin 3 799,
backstop 28`; `backstop_found(work) = 0`; `event_without_progress 16`; `pop_writebacks 479`. In the 30-arm `faston` suite: `markers` up to 779,
`backstop` 15-441 wakes per arm with `backstop_found = 0`. **For the Windows agent**: the Minecraft trace's 25 153 SYNC_WAIT-with-open-packet
entries in 6 minutes were each a barrier + control witness waiting on the 1 ms tick; they are now read at once (`carried_ungated`); gateable
ones wait ~10 us. Whether this moves Heaven's 13-23 FPS is a Windows measurement (hypothesis of the coordinator: unverified here); this
table bounds the relay's share per SYNC_WAIT at <= 4 ms per 3 gates before and ~0.1 ms after.

### 8.2 What was found while validating (all fixed unless stated)

1. **The gate proved the wrong thing** (§3.3): `GP_GET` runs ahead of processing. Found by the stress tool through the relay (49/60 refusals), pinned by a
   bare-metal measurement, fixed by the tagged `USERD.REFERENCE` marker. The Windows runs had not shown it because their bytes arrived before the relay looked.
2. `SyncGate::cleared` (§5.1) and `Grace` (§5.2), as found by the tests-first step.
3. `USER_RING_PUT_READ: Gone` at a channel's free (a refusal log line and a poison count for a retiring relay) is now a retry.
4. **Open, by design**: a SYNC_WAIT entry behind an open packet is not gated (late bytes there are unprotected); a guest with a full GPFIFO behind its own gated entry
   deadlocks (§5.00); the guest-visible `GP_GET` lags a draining gate by the next look (spin / any engine event / the 50 ms safety net).
5. **Not measured**: Windows (the coordinator schedules it); the staged-trigger path (`segment_turn` rare packets, Windows migration) now waits on markers
   too but no Linux traffic reaches it; GP_GET/REFERENCE visibility lag after an interrupt is modelled, not observed (`event_without_progress`
   is 12-205 per run, spurious or early events, never a stalled wait: `backstop_found(work)` stayed 0).

## 9. Batching: one turn per segment was structural (second task, 2026-10-14; STATUS: LIVE in the code, results in 9.5)

Trigger: the Windows agent's Heaven D3D11 trace (`traces/win11_userring_20261014/README.md` §4c, run `fc8ba3b8`): 2 314 629 GP segments relayed
(about 26 000/s, about 1 900 per frame), 2 486 644 relay turns (1.07 per segment), 5 592 guest doorbells (63/s), two `kf3-worker` threads at about
50 % CPU, 17-20 fps. The gate drains are NOT the cap (about 4.6 per frame at 0.28 ms). The coordinator's questions, in order: what is
already batched, what is per segment and why, is the INLINE path there, what do the numbers say.

### 9.1 What is already batched, what is per segment [measured: `kf-syncwait-stress --families flood`, guest, relay forced, kf3 `639acf57`]

`flood` = plans of 48 plain one-segment units (14 words each), the guest ringing after every entry / after PUT-ahead entries / once. The relay's
own flow counters (always on, `USER_RING_LOOP SUMMARY`, line `flow(...)`), summed over the three variants (28 800 segments):

| counter | value | per segment |
|---|---|---|
| relay turns | 31 223 | **1.08** (Windows: 1.07) |
| entries relayed per turn | 28 800 turns of exactly 1, 2 423 of 0 | **1 every time** |
| private-ring publications | 30 448 | 1.06 |
| host doorbells (`Pump::publish` -> `rm.doorbell`) | 30 448 | **1.06: one per publication** |
| guest `GP_PUT` reads | 31 222 | 1.08 |
| guest `GP_GET` write-backs | 4 183 | 0.15 (once per turn that saw pops) |
| batch markers | 1 648 | 0.057 (one per backlog drain) |
| `GP_PUT` delta at a turn's PUT read (entries ahead of the cursor, log2 buckets 0..) | `[5931, 4154, 2251, 3232, 6417, 6815, 0, ..]` | **a backlog of 16-63 entries was present in 13 232 of 28 800 turns** |

So: the doorbell is already coalesced where it is cheap (the token absorbs K guest rings into one claim, `DbFast` coalesces the stores of one eventfd
read; Heaven rings for 0.24 % of its PUT advances), markers are per batch, the write-back is per turn. **Not batched, per segment:** the turn
(token claim, `record.enter`, pt lock, `retry.swap`, `ring_internal` + `eventfd` write for the `Again`), the `GP_GET`/`REFERENCE` observation, the
`GP_PUT` read, the GP-entry snapshot, the pushbuffer snapshot, the scan, the publication (guard nest + PUT store), the host doorbell. The natural
backlog is there (the delta column) and one turn never takes more than one entry.

Per-stage cost [measured, `KF3_USER_RING_STAGES=1` (measurement only, off by default, no clock read otherwise); ns per call; guest on a host that
also ran two other VMs, so absolute times are noisy (the same binary ran the three variants at 97-111 k segments/s in one run and 50 k in another)]:

| stage | ns/call | calls | share of a turn |
|---|---|---|---|
| `serve_total` (whole claim) | 5 764 | 31 223 | |
| `turn_total` | 5 620 | 31 223 | 100 % |
| observe (`GP_GET` + `REFERENCE` of the private USERD) | 1 155 | 31 223 | 21 % |
| `GP_PUT` read (guest USERD) | 579 | 31 222 | 10 % |
| GP-entry snapshot (8 bytes) | 1 270 | 28 800 | 22 % |
| gate (`SyncGate::entry`) | 21 | 28 800 | 0.4 % |
| pushbuffer snapshot (56 bytes) | 1 658 | 28 800 | 29 % |
| scan | 94 | 28 800 | 2 % |
| publication, total (guards, entry write, PUT store, host doorbell) | 630 | 30 448 | 11 % |
| of which the host doorbell | **27** | 30 448 | 0.5 % |
| guest `GP_GET` write-back | 178 | 4 183 | 0.4 % |
| marker publication | 629 | 1 648 | 0.6 % |

Reading: a segment turn costs about 5.6 us here, none of it in the doorbell (27 ns) or the scan (94 ns). The two snapshots (2.9 us for 64 bytes in
two reads) are fixed cost, not byte cost [read: `userread::snapshot` builds a `Vec<Span>` with room for `PIECES` spans, resolves the rows twice
and scans `ram.blocks` per piece; `Snapshot::admit` resolves again at publication] -- the exact source guards of the per-entry TOCTOU defence, which
stay. The observation (two loads from the private USERD) and the PUT read are the per-turn costs batching removes. [inferred] The Windows 30-38 us
per segment is worker CPU over the traced build (`KF3_USER_RING_TRACE` on) divided by segments; the production per-stage numbers on Windows are the
Windows agent's to take with `KF3_USER_RING_STAGES=1` on this build (it prints the same line).

### 9.2 Is the INLINE path there? [read: grep of `crates/`]

No. There is no `CopyRing`, no inline claimant on the doorbell trap and no K constant for the Windows USER relay: every USER-ring turn runs in
`ChanPlane::user_ring_serve` on the worker (`userpump.rs`), rung by the token. (`kf_trap::Action::RingHostInline` is the Passthrough route and
unrelated.) `docs/design/V3_WINDOWS_USER_RING_RELAY.md` §4.3 calls the inline path an optimisation behind an owner ruling (§6.2) and the worker path
the correctness baseline; the architecture review says CopyRing "rarely applies on Windows" (Windows channels are scanned from birth).
[inferred, from the Heaven numbers] an inline path would not help this workload: the guest rings 63 times a second, and the worker relays about 410
entries per doorbell from PUT advances nobody rang for. The lever is the worker pass, which is what this section changes.

### 9.3 The change (minimal; no timer, no queue, no change to a guard)

Owner rule (doorbell contract; `OWNER_RULINGS.md`, "no deliberate coalescing delay"): only the LAST doorbell must be drained; batching comes from the
natural backlog. The relay already computes every step from "cursor to the current `GP_PUT`"; what it lacks is going on to the next covered entry
inside the same claim:

1. `Pump::turn` runs `turn_body` in a loop for as long as the previous iteration published and asked for another turn
   (`user_loop::chain_more(relayed, again, split)`, bounded by `CHAIN_MAX` -- a COUNT; a gate, a staged segment, a migration, a credit shortage and a
   refusal retry end the chain). The loop sits inside ONE token claim: no release, no re-claim, no `ring_internal`/eventfd write per entry.
2. Per chain, once: the `GP_GET`/`REFERENCE` observation and the guest `GP_GET` write-back (the first iteration), the `GP_PUT` read (its value is
   cached for the chain: the flag was cleared before the first read, so a doorbell that lands later makes the token re-run the claim, and a stale PUT
   only means fewer entries now -- the existing construction rule), and the **host doorbell**: `publish` marks it owed, the chain rings it once at
   its end, after the batch marker (so one doorbell covers the entries and the marker), on every exit including errors.
3. Per entry, unchanged: GP snapshot, pushbuffer snapshot, scan, gate, the publication transaction with its exact source guards (`Snapshot::admit`
   per source), the private-ring write. The per-entry guards are a hostile-guest TOCTOU defence and stay exact.
4. Hostile guests: the per-claim work is bounded by `CHAIN_MAX` entries and (as before) by `REACT_ROUNDS` re-claims; a `GP_PUT` out of range or
   backwards is refused/bounded by the same checks at the one PUT read; a neighbour channel is served between claims.

Tests (written first, `crates/kf-chan/tests/user_loop_model.rs` section (e), `user_loop.rs`): the model gets `Cfg::{metrics, defer_bell, chain}`,
host-doorbell and turn counters, `V_CHAIN_BOUND`, and the mutant `no_final_bell`. At the tests-first commit these FAIL (the model and `chain_more` are
still today's): `a_batch_of_covered_entries_is_one_turn_and_one_host_doorbell` (3 covered entries cost **4 host doorbells and 5 turns**, expected 1
and one turn relaying 3), `the_per_turn_bound_is_a_count_and_the_rest_follows_in_the_next_turn` (most entries one turn relayed: 1, expected 2),
`the_chained_next_entry_step_is_reachable` (no chained step), `a_chain_that_ends_without_its_doorbell_strands_the_entries` (the mutant is not
modelled), and `user_loop::tests::a_claim_continues_through_covered_entries_up_to_the_bound`. These pass and pin the rest: the last doorbell is
drained in every contract scenario, a gate splits a batch, a migration inside a batch drops and replays nothing, a deep backlog cannot starve the
neighbour, a hostile or backwards `GP_PUT` is bounded, K rings do not cost K turns.

### 9.4 Large segments: scan, CE copy and the exact-source rule (options only; nothing implemented)

What the code does with a segment [read]: the private ring's entry is the GUEST's own GP entry (`write_user_gp(slot, s.entry)`): the pushbuffer is
never copied, the GPU fetches it from guest memory. The snapshot (a CPU copy of the pushbuffer, up to `MAX_SOURCE_BYTES`) exists only to be scanned
for packet boundaries (the open-packet state across entries, where a marker may enter), for SYNC_WAIT/gate decisions and for the migration trigger
and software methods. Only "rare" segments (a trigger or software method) are staged into the small private cell (`write_authored`). So there is
nothing for a CE to copy on the common path, and a CE staging copy would not remove the CPU read the scan needs.

| option | what it saves | cost / safety |
|---|---|---|
| A. do nothing for bulk; batch (9.3) | the per-turn costs | none; large segments cost one memcpy (`pb_snapshot` is fixed cost for 56 bytes, byte cost only for the rare multi-KB segment: the first Heaven segment is 8 966 words) |
| B. sparse header walk: read only the packet HEADER words and skip `count` data words of a long INC/NON_INC/INC_ONCE packet (the scan already skips by count) | the memcpy and the page-piece loop of a big inline-data segment (O(headers) instead of O(words)) | the snapshot is today's immutable copy; a sparse read must still be one guarded, consistent read of the words the scan uses, and a changed header after the scan is the same exposure as any unscanned byte (hardware containment: a USER channel in its own space, measured Xid 32 on the guest's own channel only). Needs the snapshot API to read `Vec<(offset,len)>` pieces under one guard. Worth doing only if the segment-size histogram (counters below) shows bytes, not segments, dominate |
| C. CE copy of the segment into the private staging area, the private GP entry pointing at the copy | closes the TOCTOU for that segment (the GPU executes the bytes the scan saw) | a host copy channel in the SAME VA space as the guest's (the CE must read guest VAs), a semaphore release + SYNC_WAIT barrier before the copied entry (the staged path already has barrier+marker), a completion path for the copy channel, staging space for up to 1 MiB per in-flight segment (the private cell is `CELL_BYTES`), and the scan still needs the header words on the CPU, so the CPU read is not removed. Compatible with the gate and markers (it IS the gate pattern) and with the exact-source guard (the guard is checked at the CE launch), but it adds a GPU-side dependency and latency per large segment (about the CE launch + 10s of us) and a new failure mode (a stalled copy channel blocks the guest's ring). No: not for the common path |
| D. skip the scan for a segment that is one long data packet | all of it | unsafe as stated: the relay must know the packet ends exactly at the segment end (else a marker lands inside an open packet, the original Xid 32); that needs the header = option B |

[inferred] B is the only one that removes a measured cost, and only for large segments; C trades CPU for GPU latency and complexity and does not help the
dominant (small-segment) case. The histogram that decides (always-on counters in `USER_RING_LOOP SUMMARY`): `segment_words_log2` (relayed segments by
pushbuffer length, bucket i = 2^i..2^(i+1)-1 words), `words_total`, `put_delta_entries_log2` (entries ahead of the cursor at each PUT read),
`entries_per_turn_log2`. On `flood` all 28 800 segments are 14 words (bucket 3). The Heaven distribution is for the Windows agent to take on this build.

### 9.5 Results (edcaaca6: tests-first `7476ef69f`, fix `edcaaca67`; Linux guest, relay forced, host RTX 4070 / 595.91.07, GPU shared with two other VMs)

**GPU-free [measured]:** `cargo test -p kf-chan` 143 lib + the integration tests, the model file 47 tests (the five batching tests that failed at
24e7a297 pass; the chained, deferred-doorbell model also passes every pre-existing contract, gate, lap, migration, retire, neighbour and
mutant test; two mutant/limitation tests needed `chain = 0` or a different scenario because batching removes the interleaving that exposed them,
see the test comments), `cargo test -p kf-qemu --lib` 259.

**What changed in the code (fewest edits; no guard, timer or queue touched):** `user_loop::{CHAIN_MAX = 16, chain_more}`; `Pump::turn` loops
`turn_body` inside one claim while the turn published and asked for another (`again`) and needs no turn of its own (`segment`, `shadow`); the
first iteration observes `GP_GET`/`REFERENCE` and writes the guest's `GP_GET` back, the `GP_PUT` read is cached for the claim (keyed by source
generation), `publish` only marks the host doorbell owed and `Pump::ring_owed` rings it once on every way out of the chain (errors included).
Counters and the stage timers are the measurement of 9.1.

**`kf-syncwait-stress --families flood` back to back, old (`639acf57`, counters only) vs new (`edcaaca6`), same guest, alternating rounds**
(segments/s; noisy: one new-build 'each' round fell to 57 k while another VM was busy):

| variant | old r1 / r2 | new r1 / r2 |
|---|---|---|
| each (PUT + doorbell per entry) | 79 k / 108 k | 103 k / 57 k |
| put-ahead (PUT per entry, one doorbell) | 111 k / 111 k | 154 k / 147 k |
| end (one PUT, one doorbell) | 113 k / 111 k | 163 k / 153 k |

The relay's own counters, one pair of runs (28 800 segments each): turns 32 262 -> **11 390** (1.12 -> **0.40** per segment), host doorbells 30 821 ->
**9 361** (1.07 -> **0.33**), `GP_PUT` reads 32 262 -> 11 390, `GP_GET` write-backs 26 375 -> **7 070**, entries per turn `[1: 28 800]` ->
`[1: 6 413, 2-3: 1 268, 4-7: 52, 8-15: 26, 16: 1 202]` (the guest's PUT was one entry ahead in 8 159 of the turns: no backlog to take; where
a backlog existed the chain reached its bound). Markers rose 2 021 -> 8 159 (a faster relay catches up with the guest more often, and each catch-up
ends in the batch marker the owner's rule asks for) at 0.42 us each.

Per entry now [measured]: about 4.5 us in a turn (12.5 us per 2.8 entries), down from 5.8: the observation (1.2 us), PUT read (0.7), write-back and
doorbell are amortised; what is left is the two snapshots (3.0 us), the publication (0.42) and the scan (0.09) -- the per-entry exact source
guards the owner asked to keep. [inferred] the next lever is the fixed cost inside `userread::snapshot`/`Snapshot::admit` (two row resolutions, a
`PIECES`-sized span vector per resolution, a block scan per piece), not a guard: a profile on the Windows build will say.

**Validation of the batching (Linux guest, relay forced by `KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1`, kf3 `edcaaca6` unless noted; the card was shared with
the owner VM 2990 and another agent\x27s VM, so the framebuffer store was 1024 MiB instead of 4096 and absolute rates are noisy):**

| check | result [measured] |
|---|---|
| `kf-syncwait-stress`, all families incl. flood, relay forced / plain | `d5be61ed`: 1 570 plans pass, 0 fail, 0 `USER_RING_REFUSED`; `edcaaca6` the same on and off |
| fast suite 30/30 | on: 30/30, off: 30/30 (`edcaaca6`) |
| desktop smoke (Cinnamon, gears) | 0 flip timeouts, 0 Xid, 59.5-60 FPS, relay on / off / old build alike; the `display: STALL` lines (9-18) appear on the old build and with the relay off too: the 1 GiB store beside two other VMs, not the relay |
| S3 suspend/resume x3 | on 3/3, off 3/3 (`edcaaca6`) |
| unit | kf-chan, kf-qemu, kf-rm, kf-trap, kf-harness: 1 440 passed, 0 failed (release) |
| `v3_gates.sh` | **7/9, gate10 PASS, births 12/12 USER.** gate5 and gate6 fail (`semaphore never reached`) on bare metal with no kayfabe code in the path, and the same two fail with the previous build\x27s binaries run now (they passed 9/9 at 22:19 on an idle card): the card is busy with two other VMs. Inconclusive, not attributable to this change; to be re-run on an idle card |

**Where the rest of the per-entry cost is [measured, partly misread]:** a `perf record -g` of the relay worker threads (45 k samples) was dominated by
`turn_body` (everything inlined) and attributed 18 % to `RawVec::finish_grow`/`malloc` under a `hash_one::<&(u32,u32)>` frame; the build has no frame
pointers, so the call chain is a guess. The one allocation that provably scaled with the budget -- `userread::resolve` reserving all `PIECES` spans on
every resolution -- was fixed (`d5be61ed`/`6c2cb437`) and gained about 100 ns per snapshot, not 18 %. The stage timers are the evidence; the remaining
per-entry floor is the two snapshots (about 2.8 us) -- finer timers inside `snapshot` (resolve / prepare / blocks / copy) are the next measurement.

**Open / for the Windows agent:** (1) run this build with `KF3_USER_RING_STAGES=1` on Heaven D3D11 and read the `USER_RING_LOOP SUMMARY` flow line (turns and
host doorbells per segment, `segment_words_log2`, `put_delta_entries_log2`, stage ns); the Linux suite traffic is 16-word segments only (1 118 segments in
19 arms, nothing like Heaven). (2) The chain bound `CHAIN_MAX = 16` is a guess from fairness, not from Heaven: the flood run hit it in 1 202 of 7 700
chains. (3) Markers rose from 0.07 to 0.28 per segment because the faster relay catches up with the guest more often; each is a publication and an interrupt
event. A rule that skips the marker when the chain ended exactly at the bound would need a way to know the next claim is coming without a timer -- not done.
