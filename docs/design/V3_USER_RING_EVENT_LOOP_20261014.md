**STATUS: LIVE in the code, 2026-10-14** (branch `claude/userring-eventloop-20261014`: tests-first commit `95a9ad422`, then the
fix; GPU-free results in §5-§6, Linux hardware validation in §8). Written DESIGN-ONLY first; §2 and §5 keep the
state of the code BEFORE the fix (77eaf7620) as the record of what the tests found.
**ADDENDUM 2026-10-20 (§9):** the default-off stall diagnostic `KF3_DIAG_USER_STALL=<ms>` (measurement only; branch
`claude/user-stall-diag-20261020`) is described in §9. It changes nothing above.
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

**★ CORRECTION, 2026-10-17 (owner ruling; supersedes §3.6's `GP_PUT` read and the "Doorbell-less submission" bullet in §6):** the backstop does
NOT read `GP_PUT`. The doorbell contract is: the last doorbell covers every `GP_PUT` write before it; nothing else announces `GP_PUT`; the relay loops
until its cursor equals the `GP_PUT` it loaded; a ring during a pass forces one more pass, repeated until a pass sees no ring; rings coalesce; a ring with
no work is safe. §3.6 and the bullet below kept an idle `GP_PUT` read on the backstop ("information only") that also rang the relay: that was a `GP_PUT` poll
in a different place, found by the lazy-observe review (`traces/lazy_observe_20261017/REPORT.md`, stop finding 2). Removed: the read, the
`backstop_put_ahead` counter, and the model's allowance for it (it is now the violation `PUT_POLL`, with a mutant that must be found). The pure decision is
`user_loop::look_needed` (a look needs demand or a recorded wait; the kind of look is never a reason). What the backstop may still do: re-look at relays that
WAIT (`Wait::Popped`, `Wait::Contended`, and a lost marker interrupt, each counted). Pinned by `kf-trap/tests/doorbell_contract.rs` (real token word),
`kf-qemu/tests/doorbell_no_put_poll.rs`, `kf-chan/tests/user_loop_model.rs` (`put_ahead_of_the_last_doorbell_is_owed_nothing_and_no_timer_reads_it`,
`mutant_the_backstop_polls_gp_put_is_found`).


`BACKSTOP_MS = 50` (the existing `PARK_MS`; unchanged idle wake rate, 20/s, an order below the guest's 2 s TDR
detection, and the only timer in the loop). A backstop wake peeks every relay's `waiting` (⊘ SUPERSEDED 2026-10-17: and its `GP_PUT`; see the correction above, the backstop reads no `GP_PUT`). What it
finds is counted, not hidden: `backstop_found_work` (a satisfied non-carried wait: a lost or never-armed
interrupt, a bug), `backstop_carried` (a satisfied control-witness wait: expected, separate), ~~`backstop_put_ahead`
(`GP_PUT` ahead of the cursor with no doorbell: legal in the narrow window between a guest's `GP_PUT` write and
its ring; information about guests that skip doorbells)~~ (⊘ REMOVED 2026-10-17, the doorbell contract), `event_without_progress` (an interrupt before `GP_GET`
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

## 9. `KF3_DIAG_USER_STALL=<ms>` -- why is the private ring not draining? (2026-10-20, DIAGNOSTIC, default off)

**STATUS: LIVE in the code, 2026-10-20, measurement only.** Never required to boot, never a decision input, never a fix.
Labels as above: **[measured]**, **[read]** (code or public headers), **[inferred]**.

### 9.1 The question

**[measured, owner's Windows guest, run 3094, outside reader of the private ring]** In 2 of 9 app freezes (Minecraft + Unigine
Heaven, windowed) the private ring stayed at `PUT - GET = 2` for 7-27 s with the PBDMA *inside* an earlier entry
(`USERD.PB_GET = 0x304720040`, a 500 byte guest pushbuffer at GPU VA `0x304720000`, `LEVEL = SUBROUTINE`; the next entries, a
40 byte guest pushbuffer and the relay marker, not begun; `REFERENCE` one marker behind). Kicks did nothing; in 5 of 9 freezes
the GPU had consumed everything. The outside reader could not resolve guest GPU VAs, so *what the stuck segment waits on* was unread.

**H-ACQ [hypothesis]:** the PBDMA is blocked in a semaphore `ACQUIRE` inside that segment, waiting for a release that never comes
(a release by another context whose own work is held or not yet published by the relay, or written somewhere the acquirer
does not read). **Falsifiers, stated before the run:** (a) the report's decode of the segment holds *no* acquire at or before
`PB_GET`; (b) it holds one, but the semaphore's `current=` already passes by the operation's name (`name_says_pass=yes`) -- then
the PBDMA is not waiting on that value; (c) `current=UNREAD(...)` for every acquire is *no* evidence either way; (d) the
segment is decoded as copy-engine or graphics work with no acquire and `SEM_EXECUTE RELEASE` with `release_wfi` -- then the stall
is a wait-for-idle on the engine, not on a semaphore.

### 9.2 What it does

`KF3_DIAG_USER_STALL=2000` (milliseconds; unset, empty or `0` = off; a non-number is ignored with one loud line). Read **once**,
when the first Windows USER-ring relay is created (`Pump::new`). On the relay **worker's tick** (`ChanPlane::user_ring_refresh`,
once per wait of the worker loop, at most every `BACKSTOP_MS` = 50 ms) it looks at each relay; for a relay whose private
`GP_GET != GP_PUT` for longer than the threshold it prints **one block, once per condition**; the condition re-arms when `GP_GET`
moves or the ring empties.

* **Where it may run:** the relay worker only. Not a vCPU, not the drainer, not the act thread. Two plain field writes
  (`Pump::diag_wake` in the look, `Pump::diag_turn` in the serve) record the last look that rang a relay (Spin / Event /
  Backstop) and the last turn. Pinned by source tests in `userpump.rs` (`the_stall_diagnostic_*`).
* **What it takes:** `try_lock` only -- the relay map, each relay's record (a record the act thread holds is skipped and counted
  `skipped_busy`) and its private storage. No wait, no sleep, no new thread, no timer of its own, no `unsafe` (the only new
  memory accessors are `Storage::gpu_cursors` / `read_gp`, safe wrappers over the existing CPU view).
* **What it reads:** the private `USERD` (36 words) and the private GP ring rows around `GP_GET` (at most 8); the guest's `GP_PUT` /
  `GP_GET` through the relay's own `UserdSource::read` (try-lock, `UNREAD` when busy); for the entry whose segment holds
  `PB_GET` (nearest behind `GP_GET` first), the guest pushbuffer (at most `MAX_PB_BYTES` = 2048 bytes from the segment start)
  and each acquire's semaphore (at most 8, 4 or 8 bytes) through **the relay's own reader** (`userread::snapshot` over the
  channel's placement rows, the same path that reads guest pushbuffers to parse `SYNC_WAIT` gates). The store reader is
  **passive** (`Cache::passive_prepare`): a video-memory window that is not already armed is not armed for the diagnostic
  (that could evict the relay's working set), so such a semaphore prints `current=UNREAD(vidmem)`.
* **What it prints:** the lines of `kf_chan::user_stall::render`: at most 112 lines of at most 320 printable-ASCII characters,
  **every integer in hex**, nothing from guest memory but integers (a method stream is decoded, never echoed).
* **Cost off:** one relaxed load per worker tick; a `None` test per look and per turn; nothing at boot.
* **Counters:** `diag_user_stall_{conditions,acquires,unread,would_pass,no_segment,skipped_busy}` are appended to the existing
  `USER_RING_LIFECYCLE SUMMARY` line (only when counted) and printed as `USER_RING_LIFECYCLE PERIOD <line>` at most every
  10 s, only when they moved.

### 9.3 The decode (`kf_chan::user_stall::decode`, pure)

Methods per ogkm `clc56f.h` / `clc86f.h` / `clc6b5.h` (595.84 and 615.71): `SEM_ADDR_LO 0x5c` (`31:2`), `SEM_ADDR_HI 0x60`
(`24:0`), `SEM_PAYLOAD_LO 0x64`, `SEM_PAYLOAD_HI 0x68`, `SEM_EXECUTE 0x6c` (`OPERATION 2:0` = `ACQUIRE 0`, `RELEASE 1`,
`ACQ_STRICT_GEQ 2`, `ACQ_CIRC_GEQ 3`, `ACQ_AND 4`, `ACQ_NOR 5`, `REDUCTION 6`; `ACQUIRE_SWITCH_TSG 12`, `RELEASE_WFI 20`,
`PAYLOAD_SIZE 24` = 64 bit, `RELEASE_TIMESTAMP 25`, `REDUCTION 30:27`, `REDUCTION_FORMAT 31`), the legacy
`SEMAPHOREA..D 0x10..0x1c`, `NON_STALL_INTERRUPT 0x20`, `SET_REFERENCE 0x50`, `WFI 0x78`, `SET_OBJECT 0x0` (class `15:0`), and --
only on a subchannel bound to an `xxB5` copy class -- `SET_SEMAPHORE_A/B/PAYLOAD 0x240/0x244/0x248` and `LAUNCH_DMA 0x300`
(`SEMAPHORE_TYPE 4:3`). All header forms (incrementing, non-incrementing, one-increment, immediate) are walked; the legacy
`GRP0/GRP2` forms are sized and skipped; a reserved header stops the decode (`malformed_at`), `END_PB_SEGMENT` stops it
(`ended_at`), a cut packet is `truncated`, the 64 method cap is `capped`.

Limits, said plainly: **[inferred]** the decode starts at the segment's first word and assumes it is a packet header; a segment that
begins inside a packet opened by an earlier entry (the lifecycle already reports `PUT_BOUNDARY_OPEN` for such guests) decodes
as garbage or `malformed_at=0x0`. `name_says_pass` is **inferred from the operation's name** (`ACQUIRE` equality, `GEQ` unsigned,
`CIRC_GEQ` signed difference, `AND` non-zero; `NOR` and the rest print `n/a`), never measured. The semaphore's value is a CPU
read through the relay's placement view at report time, not the value the PBDMA last saw. `PB_GET` is the *fetch* position: the
method the PBDMA executes may be a little before it, so the `<== PB_GET` marker sits on the first decoded method at or after it
and an acquire *before* it is the candidate.

### 9.4 Output format

One block per channel per condition (every line starts `kf3: USER_RING_STALL_DIAG`; the channel key `client:object` repeats on
each line after BEGIN). Lines: `BEGIN` (key, host id, `stalled_ms`, `threshold_ms`), `TOTALS`, `GUEST` (guest `GP_PUT`, `GP_GET`,
the relay's cursor), `PRIVATE` (`gp_get gp_put reference pb_get pb_put top_level_get`), `RELAY` (the parked `wait=`, `last_doorbell_ago_ms`
-- resolved to the look interval, `unknown` before the count first changes --, `doorbells`, `last_wake` + `last_wake_ago_ms`,
`last_turn_ago_ms`), `COUNTERS` (`issued fetched marks_issued marks_seen turns entries_*`), `RING` rows (`<== GP_GET`,
`<== SEGMENT_ROW`), `SEGMENT` (`slot va len_bytes read_bytes pb_get_off methods capped truncated malformed_at ended_at`), one `M +<byte offset>`
line per interesting method (`<== PB_GET` on one), `OTHER` (count and the first four method numbers of the uninteresting rest),
`NOTE`, `END`.

```
# Example: the shape of the 2026-10-20 incident, built from synthetic words by the pure renderer (GPU-free; NOT a hardware capture).
# The acquire, its address and the current value 0x29 are invented to show the format; a real block is what the owner's run prints.
kf3: USER_RING_STALL_DIAG BEGIN 0xc1d00001:0x5c000010 host=0x1c0 stalled_ms=0x7d3 threshold_ms=0x7d0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 TOTALS diag_user_stall_conditions=0x1 diag_user_stall_acquires=0x1 diag_user_stall_unread=0x0 diag_user_stall_would_pass=0x0 diag_user_stall_no_segment=0x0 diag_user_stall_skipped_busy=0x0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 GUEST guest_put=0x1ab guest_get=0x1a7 relay_cursor=0x1ab
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 PRIVATE gp_get=0x1a8 gp_put=0x1aa reference=0x4b466dd4 pb_get=0x304720040 pb_put=0x120361d40 top_level_get=0x304720000
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 RELAY wait=Marker(target=0x1234) last_doorbell_ago_ms=0x1f40 doorbells=0x77 last_wake=Backstop last_wake_ago_ms=0x30 last_turn_ago_ms=0x32
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 COUNTERS issued=0x1aa fetched=0x1a8 marks_issued=0x20 marks_seen=0x1f turns=0x400 entries_scan=0x0 entries_pointer=0x123 entries_window=0x0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 RING slot=0x1a6 entry=0x400120300000 va=0x120300000 len=0x40 subroutine=0x0 sync_wait=0x0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 RING slot=0x1a7 entry=0x1f40304720000 va=0x304720000 len=0x1f4 subroutine=0x0 sync_wait=0x0 <== SEGMENT_ROW
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 RING slot=0x1a8 entry=0x280120361d18 va=0x120361d18 len=0x28 subroutine=0x0 sync_wait=0x0 <== GP_GET
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 RING slot=0x1a9 entry=0x100304730000 va=0x304730000 len=0x10 subroutine=0x0 sync_wait=0x0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 SEGMENT slot=0x1a7 va=0x304720000 len_bytes=0x1f4 read_bytes=0x1f4 pb_get_off=0x40 methods=0x8 capped=0x0 truncated=0x0 malformed_at=none ended_at=none
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x24 SET_OBJECT sub=0x4 class=0xc6b5
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x2c SEM_ADDR_LO value=0x20361000
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x30 SEM_ADDR_HI value=0x1
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x34 SEM_PAYLOAD_LO value=0x2a
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x38 SEM_PAYLOAD_HI value=0x0
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x3c SEM_EXECUTE ACQ_CIRC_GEQ raw=0x1003 addr=0x120361000 awaited=0x2a wfi=0x0 payload64=0x0 timestamp=0x0 acquire_switch=0x1 current=0x29 name_says_pass=no
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x44 SET_REFERENCE value=0x4b466dd5 <== PB_GET
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 M +0x4c NON_STALL_INTERRUPT
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 NOTE name_says_pass is inferred from the operation name, never measured; current is read from memory through the relay's placement view, not by the GPU
kf3: USER_RING_STALL_DIAG 0xc1d00001:0x5c000010 END
```

### 9.5 Tests (GPU-free)

`kf-chan` `user_stall` (table tests built from the ogkm method numbers, hostile-stream fuzz, bounded-output and hex-only
checks, a fake placement view for `build_report`), `user_storage` (the two new reads), `user_lifecycle` (the counters line),
`kf-qemu` `userstore` (`passive_prepare` never arms or evicts), `userdiag` (`refusal_name`) and `userpump` (source tests
pinning where it runs, `try_lock` only, one relaxed load off, switch read once). Mutation results and counts are in the commit
message of the branch head and the hand-off report.

### 9.6 What only hardware can confirm

That the tick runs while the freeze lasts (a worker stuck inside a long turn prints nothing); that the stuck segment's start is a
packet boundary (else `malformed_at`); that the guest semaphores sit in guest system memory covered by the channel's placement
rows (else `UNREAD(unmapped)`) and not in an unarmed video-memory window (`UNREAD(vidmem)`); that `PB_GET` in the private
`USERD` is as fresh as the outside reader saw it; and the rate of reports under a real freeze (one block per channel per
condition by construction).

### 9.7 EXPERIMENT `KF3_DIAG_USER_STALL_RERING=1` -- ring the idle channel's doorbell once (2026-10-20, opt-in measurement, MUTATES scheduling state)

**Never a fix.** A timer-based re-ring would violate the owner rule; this is a one-shot probe of a hypothesis, default off, only effective
together with `KF3_DIAG_USER_STALL` (read once in `init`; a value other than `1` is off).

**Why [measured, run 3418, INDEPENDENT_FREEZE]:** a stall block with `gp_get=0x130 gp_put=0x133` (three published entries not fetched),
`pb_get == pb_put` (the previous 16-byte relay segment fully consumed: the PBDMA is **idle**), `wait=Marker(target=0xe8)` with
`marks_seen=0xe7`, `last_wake=Spin` ~2 s ago. An idle PBDMA with pending entries is a lost doorbell or a channel that is not
scheduled. The release fence before the doorbell is present (`release_fence` is an `sfence`), so a missing barrier is not the explanation.

**What it does.** When a stall block was printed for a channel whose private `GP_GET != GP_PUT` and the PBDMA looks idle
(`kf_chan::user_stall::looks_idle`: `PB_GET == PB_PUT`, or no private row holds `PB_GET`; a segment that holds it but could not be read is
*not* idle), and this condition has not been rung (`ChannelDiag::rering_ready`: once per condition; the gate reopens when `GP_GET` moves or the
ring empties), the diagnostic rings that channel's doorbell **once** with the same verb the publication uses (`rm.doorbell(token)`), from the
relay worker, `try_lock` rules unchanged (the pure decision is `should_rering(enabled, outstanding, idle, ready)`). It prints
`RERING rang doorbell_token=<hex> gp_get=<hex>` and, at each following relay tick, `RERING_RESULT +<ms> gp_get=<hex> moved=<yes|no>` until `GP_GET`
moves (`moved=yes`) or 3 s pass (`RERING_RESULT none`). **Reading it:** `moved=yes` shortly after the ring means the doorbell (or the
scheduling it implies) had been lost, not the GPU's work; `RERING_RESULT none` means a second doorbell does not wake it, so look at the
channel's scheduling state (next paragraph). **Falsifier of "lost doorbell":** `moved=no` for 3 s.

**New block line `TWIN`:** `doorbell_token`, the guest channel twin's `disabled` / `stopped` flags (`pt` map, `try_lock`), and the ms since the last
host `DISABLE_CHANNELS` disable act and enable act for this host channel (`ActTimes`, noted where the three host verbs return: STOP, the
`DISABLE_CHANNELS` act, the re-enable after STOP; nothing recorded with the diagnostic off). It separates the competing explanation: a channel left
disabled by an async preempt act (`disabled=0x1`, a disable act age with no later enable age).

Tests: `kf-chan` `user_stall` (decision truth table, once per condition, 3 s window boundaries, result lines, act-time table and its cap, the TWIN line),
`kf-qemu` `userpump` (switch read once, the doorbell is rung in one place after the block and behind the decision, the act notes after their verbs).

