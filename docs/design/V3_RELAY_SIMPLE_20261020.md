**STATUS: RESEARCH (code on branch `claude/relay-simple-20261020`, GPU-free results only; NOTHING here ran on hardware), 2026-10-20.**
Labels: **MEASURED** = a test / run named here; **CODE READ** = file:line; **INFERRED** = reasoning. Base `82830cd5e` + the breaker,
stress, found-in-production and progress-watchdog branches (tests/diagnostics merged unchanged).

# V3 — the simple USER-ring relay: one level loop, a retry set, the guest's GP_GET = our read index

## 1. What it is (owner's design, as built)

* **P1 hints, state is truth.** Doorbells, engine non-stall interrupts and lock releases only end a blocking `epoll_wait`. Every
  iteration re-reads STATE (the demand flag, the recorded wait against the private `GP_GET` / tagged `REFERENCE`); no event is
  counted or read as an id. **P2 doorbell contract unchanged**: no look reads `GP_PUT` (`user_loop::look_needed`).
* **Change B (structural, commit `e49f0c225`).** `worker::run_with_relay` is ONE loop with ONE `epoll_wait` per iteration:
  `seen` -> `worker_pass` (per-visit bound: the token claim RUNG->BUSY, `REACT_ROUNDS`, `SCAN_LIMIT`; a hot relay is republished
  and re-rung, so it cannot pin a worker) -> `relay.peek` = the level evaluation of ALL relays (every iteration, busy or idle, on
  every worker) -> `epoll_wait(timeout 0)` if `n > 0 || rang || relay.unsourced()`, else `try_park(seen)` + a BLOCKING wait ->
  `dispatch` drains the fds (before the next evaluation: arm-before-evaluate). The park keeps a 50 ms LAST-RESORT net that no
  logic depends on; `worker::set_park_net(false)` makes it infinite and every model / the stress harness run that way; a timeout
  after which the loop finds work is the bug counter `timeouts_with_work` / `backstop_found_work`.
* **Claim + again.** The relay's existing durable demand flag (`Cell::retry`, set by `request()` BEFORE every `try_lock`) IS the
  "again" flag: whoever loses the record `try_lock` leaves it set; a turn that loses `pt` / storage / USERD admission
  (`Need::Contended`) sets it again in `user_ring_arm` (`record.request()`). A relay with the flag set and no token rung is in the
  **retry set**: every look re-rings it, and while the set is non-empty the worker does not block (`RelayLoop::unsourced`). No budget.
* **One decision.** `user_loop::look(LookIn) -> Look` (pure; production `user_ring_peek_one` and every model call it).
* **Change A (commit `41d42ee98`, one line `user_turn::GUEST_GET`).** The guest's USERD `GP_GET` := the relay's reading pointer
  (`Relay::guest_get_value`): the slot's 8-byte entry is loaded, validated and COPIED into host-owned private storage before the
  cursor passes it (`copy_before_get_...` test). Pointer markers stay for SYNC_WAIT gates and private-ring credit only; no pop
  write-back exists any more (`LookIn::get_follows_pops == false`). Flip back: `GuestGet::Popped`.
* **NOT built: stage 2 (vCPU inline k entries).** CODE READ: today every USER-ring doorbell takes the worker path
  (`device.rs` 3951-3980: `RingHostInline` only for Passthrough tokens). v3 permits it only as a bounded optimisation after the
  worker path (`V3_WINDOWS_USER_RING_RELAY.md:178-179`, `V3_USER_RING_LIFECYCLE_20261016.md:16-17, 394-395, 507-509`).

## 2. Deleted / kept (format-neutral line counts, both sides rustfmt'd; production files only)

| deleted | where | why it can go |
|---|---|---|
| `CONTENTION_RETRIES`, `may_retry`, the `retry_ok` arms | user_loop.rs, userpump.rs peek | demand is re-rung by every look; the retry set keeps the loop from blocking |
| `Wait::Contended`, `Found`/`found`, `Found::Contended` | user_loop.rs | a lost lock is demand, not a wait |
| `LOOK_EVERY = 4`, the `busy_passes` counter | worker.rs | every iteration evaluates |
| the bounded pre-park `SPIN` loop, `user_loop::idle`/`Idle` | worker.rs, user_loop.rs | the big loop is the one spin loop |
| `Peek::Backstop` and the backstop look/classification, `loud_rate`, Pointer backstop line | worker.rs, user_loop.rs, userpump.rs | no timer path; a timeout runs the same next iteration |
| the Marker-only pop write-back arm | userpump.rs | `look()` covers every wait; Change A removes the write-back |

Production diff vs the merged base: user_loop.rs -124/+164 (incl. `look` + its tests), worker.rs -159/+116, userpump.rs -97/+73,
userrelay.rs -1/+13, user_turn.rs -2/+35, progress.rs -31/+14 => **-414 / +415**. Not fewer lines; **fewer mechanisms**: three
reason-dependent look paths, a budget, a period and a spin became one pure decision and one loop. (`userpump.rs` was not
rustfmt-clean; the commit carries ~400 lines of rustfmt reflow besides the semantic change.)

## 3. Obligations and their wake sources (owner's formalisation: block only if covered change events ⊇ outstanding ones)

| obligation | change event that makes it handleable | wake source | can it be missed? | status |
|---|---|---|---|---|
| unserved token (doorbell, `Again`, re-ring) | RUNG set | worker eventfd + `seen`/`try_park` sequence | no (sequence compared after the pass, §5.3) | solved |
| record lock lost to the act thread | act ends | act completion `user_ring_wake` -> efd | no | solved |
| record / `pt` / storage / admission lost to anyone else (the retry set) | holder releases | NONE (the holder rings nobody) | — | **spin (timeout 0) on exactly it**; owner decision: a release-ring on `pt` = ~60 holders in chan.rs (CODE READ) |
| `Wait::Marker` (SYNC_WAIT gate; credit: the NEXT marker in ring order, `c777f03d9`) | marker processed (`REFERENCE`) | engine non-stall fd (the marker's own `NON_STALL_INTERRUPT`, emitted AT the target position) | no: the fd is drained before the evaluation (arm-before-evaluate). MEASURED miss found and fixed: a look that found the record busy dropped the consumed event (seed 20, `aad1e9ff4`). The later I6 hits were classified as monitor false positives (wake in flight; §5) | solved |
| `Wait::Popped` = private CREDIT with no marker outstanding (Pointer mode mid-batch: no observed `GP_PUT` boundary; Scan: packet open; markers latched off) | PBDMA pops | NONE | — | **OWNER DECISION: spin on exactly it.** No defined marker position exists: the relay does not parse Pointer-mode pushbuffers, so it does not know a packet boundary mid-batch, and no byte may enter an open packet. "Marker per forwarded batch" already exists (`end_batch`) and covers every credit wait that has a batch end inside the ring. Cost (INFERRED, not measured): Heaven publishes 923 entries/frame into a 510-entry ring (`V3_USER_RING_LIFECYCLE_20261016.md:483`), so about one such wait per frame; it lasts until the PBDMA pops the next entry (GP fetch runs ahead of execution, measured EL:150-158), so microseconds when the GPU fetches, up to the rest of the frame when the fetch window is full: between ~0 and one core x (fraction of the frame the private ring is full). A defined non-heuristic alternative: a private ring larger than the largest batch (capacity, not a threshold; e.g. 4096 entries = 64 KiB). |
| guest `GP_GET` write-back (`GuestGet::Popped` only) | pops | engine event when a marker follows, else NONE | — | **gone with Change A** (`end_batch` no longer waits for work in flight under ReadIndex, `c777f03d9`) |

SYNC_WAIT (CODE READ + v3 text): the gate exists because `GP_GET` runs ahead of processing (measured, `V3_USER_RING_EVENT_LOOP_20261014.md:150-158`);
in Pointer mode SYNC_WAIT entries are forwarded as they are and the PBDMA honours `GP_ENTRY1_SYNC_WAIT` natively
(`V3_USER_RING_LIFECYCLE_20261016.md:400`); only Scan/Window reads pushbuffers and needs the gate. A wait on a write by the guest
CPU / another channel has no ring we own; forwarding the acquire lets the hardware wait (Pointer mode does exactly that).

## 4. Deviation of the current code from intended v3 (v3 text, quoted by file:line in the report of the doc sweep)

* v3 has THREE `GP_GET` answers: Translated = our fetch index (`THE_TRANSLATED_PLANE.md:240-241`, `the_three_channel_kinds.md:245-246`);
  USERD relay = engine `GP_GET` written back (`V3_USERD_RELAY.md:57-58`); USER-ring relay = private pops written back
  (`V3_USER_RING_EVENT_LOOP_20261014.md:131`). **Change A moves the USER ring to the Translated rule: an explicit CONFLICT with
  the relay docs, consistent with the owner's 2026-10-20 ruling (GP_GET guards slot reuse only).**
* Removed by this branch: the reason-dependent looks, the retry budget, the busy-look period, the pre-park spin, the backstop
  as a logic path, pop write-back. Kept: lazy observe (now only for credit/gates), Pointer markers, the token claim, the per-visit
  bounds. v3 conflicts left open: "worker forwards any number of entries" is NOT built (one entry per turn, `REACT_ROUNDS`
  re-rings: `V3_USER_RING_EVENT_LOOP_20261014.md:91`); vCPU inline on migratable channels conflicts with
  `docs/OWNER_RULINGS.md:1215-1216` ("always-worker dispatch for ... migration-capable channels").

## 5. Results (MEASURED on the dev box, `cargo test --release -p kf-chan --features chaos-hooks`; nothing on hardware)

| finding | old code (merged base) | simple relay |
|---|---|---|
| F1 convoy past 64 lost try_locks (breaker :1109/:1119, fairness P13, stress exhausted-try_lock) | RED (5 breaker reds, P13 red, stress expected-red) | GREEN, bodies unchanged, infinite park |
| F2 Popped partial pops never written back (breaker :1172/:1182) | RED | GREEN under Change B (look rings pops for every wait); MOOT under Change A (no write-back; restated: the guest sees space once the relay READ the entries) |
| F3 busy worker never looks (P11-P13) | green with LOOK_EVERY | green, a look every iteration |
| F4 stress seed 18 | expected-red (ignored) | GREEN un-ignored (6 runs, infinite park) |
| F5 VA-refresh / lost-wake classes (relay_fairness, v2) | green | green (v2: not every test re-run after the last edit) |
| NEW: busy-record look dropped a consumed engine event (stress seed 20, 3/3; v1 model independently) | latent (the 50 ms net hid it) | FIXED `aad1e9ff4` |
| NEW: I6 hits (6 of 60 seeds) | not checked | **CLASSIFIED** (`fb88d5999`): monitor false positives -- an interrupt raised (eventfd signalled) but not yet dispatched on a loaded box, and the obligation compared without its identity; none of C1 (event consumed without a ring), C2 (interrupt before REFERENCE), C3 (claim/again lost), C4 (arm after evaluate) matched the forensics. Then (`c777f03d9`) the harness's holder lacked `user_ring_wake`'s unconditional worker wake (mirror infidelity, not production). Seeds 0, 1, 3, 18, 20 are a test; the seed-20 mutant is still found 2/2 |
| NEW: GP_GET/REFERENCE visible only after the interrupt (v1 model) | covered by the 50 ms net | **uncovered** with an infinite park; production's net covers it |

Full suite with Change A on (`41d42ee98`+), MEASURED: lib 261/261, relay_fairness 40/40, user_loop_model 46/46, wake_registry
19/19, user_lazy_turn 27/27, user_turn_unit 23/24 (`the_guest_ring_is_full_when_gp_put_leaves_one_slot...` is RED on the merged
base too, untouched), relay_stress 3/5 (smoke and seed 18 trip the OPEN I6), user_lifecycle_props 37/38 (`lazy_d6_...`: moot by
design under Change A like user_lazy_turn's D6, not yet restated), user_lifecycle_model 59/63: `l1_latched_markers_off...` and
`m1_no_pointer_markers...` expect a full guest ring to DEADLOCK without looks/markers -- under Change A it no longer needs pops, so
the expected deadlock is gone (moot by design, to be restated); `i1_pointer_mode_...` exceeds its 6 M state cap (the per-turn GET
write-back adds states; not a property failure); `lazy_every_other_scenario...` (same l1 scenario). v2: see the final report.

Changed / replaced tests: listed per file in the commit messages of `31297a6e4` (v3 Pointer, fairness) and the v1/v2/registry
commit; every replaced test pinned a deleted mechanism (budget, period, backstop, a source line) and was replaced by the property
or a pin of the new line. Stress: `I6` sleep safety, `I7` no hidden spin, the two expected-reds un-ignored. New invariant mutants:
`unsourced_ignores_carried` found by 3 (Pointer) + 4 (fairness, W=1/2/3) + 3 (v1) tests; `unsourced_ignores_demand` is EQUIVALENT
with the real look (it rings every demand before the block decision), found only together with a weakened look.

### 5.1 Second session (2026-10-20, +2 h): classification, restatements, runs (MEASURED unless marked)

* **I6 classified as harness artifacts, not design gaps.** Forensics at fire time (wait, words at the last look, REFERENCE store
  time, interrupt raise / dispatch times, token state; then the KERNEL: the monitor's own level-triggered epoll on the workers'
  eventfds and every worker's `/proc` state). Each hit was a wake in flight (an eventfd already signalled, a worker not yet
  scheduled on a box at load 8-11) or the harness holder lacking production's unconditional worker wake. With
  `RS_I6_KERNEL_ONLY=1` (only the kernel's ready set and thread states decide) seeds 0 (3 runs), 1 and 3 show NO I6. The real
  lost wake (seed 20, busy record) is still found by I6 when re-introduced (2/2).
* **Anomaly, unexplained:** seed 1 (1 worker, 7 clients, a holder taking locks for up to 20 ms) twice took ~570 s instead of
  ~10 s, without a violation. INFERRED candidate: R3 -- the worker spinning on the retry set while the lock holder competes
  for CPU on a loaded box. Not measured (no `retry_looks` captured for that run). This is the main open item.
* **Restated moot tests:** `user_lifecycle_model` l1 / m1 (and the lazy re-run): deadlock iff the guest's GP_GET waits for
  pops (both policies tested by the one switch); `lifecycle_props lazy_d6` and the `user_lazy_turn` wait-latch tests restated
  for ReadIndex; `i1` explores to a documented 6 M-state bound under ReadIndex.
* **Credit wake:** a credit wait now waits for the NEXT outstanding marker (the first point whose processing frees credit).
  The rest is the owner decision of §3.
* **Mutation, fairness script:** 5 of 6 mutants killed by >= 3 tests. `busy_pass_never_polls` survived every model; a new
  wake-registry pin kills it (1 test).

### 5.2 Third pass (owner adjustments: decouple Change A; MEASURED)

* **Change A is OFF by default** (`GUEST_GET = Popped`; cargo feature `guest-get-read-index` turns it on). Switching it off
  exposed one more unannounced obligation (v3 model break2 control): with GP_GET following pops, a wait while the guest's ring
  looked FULL and pops are unaccounted has no interrupt. Defined rule `Relay::wait_unannounced` (put_full && fetched < issued):
  the worker spins on exactly it. With ReadIndex the obligation does not exist -- the strongest argument for Change A.
* **Both policies, fast suites (lib, fairness 1/2/3 workers, v3, registry, v1, lazy_turn, lifecycle_props, turn_unit, stress,
  progress_model, gate_rules, lazy_model): all green under Popped and under ReadIndex** (head `aaceb73cb`).
  `user_lifecycle_model` restated tests green under ReadIndex (§5.1); not re-run under Popped in this pass. v2: 49 of 52 ok,
  0 failed, 3 slow tests unfinished at the 90 min timeout (p7 oracle + 2 new mutant tests).
* **Stress:** 300-seed run (ReadIndex binary, strict) stopped at about seed 125 for time: 2 failures, both the old HARD I7 (now
  SOFT), no I6, no stall. Baton preset 4 seeds: 0 failed, resp max 10.9 ms; negative control (`KF3_NEGCTL_NO_BUSY_LOOK=1`):
  3 of 4 failed, resp max 3.2 s -- the harness still sees the baton.
* **Mutation, stress script (6 seeds x 0.5 s):** killed control_look_removed 6/6, wake_not_bumped 2/6, marker_not_repeeked 2/6,
  unsourced_ignores_carried 2/6 (I6); survived idle_only_evaluation (needs the baton preset), unsourced_ignores_demand
  (equivalent, shown by the models), busy_look_ignores_wait (needs seed 20; found 2/2 there). Unmutated: 1 of 6 seeds tripped
  the old HARD I7.
* **Credit waits:** the marker per forwarded batch exists (`end_batch`); a credit wait waits for the NEXT outstanding marker.
  The remaining no-source case (one batch larger than the private ring in Pointer mode, packet open in Scan, markers latched
  off) is an OWNER DECISION (§3): spin on exactly it, or a private ring larger than the largest batch.

### 5.3 Hardware-ready head and build

Head `aaceb73cb` (Change A OFF) builds GPU-free (`cargo check -p kf-qemu`). On the trusted host, from a checkout of that
revision: `bash scripts/bench/build_kf3.sh <qemu-source-tree>` (installs `kf3-bins/<rev>/qemu-system-x86_64`); the Change A arm
is the same revision built with `cargo build ... -p kf-qemu` after enabling `kf-chan/guest-get-read-index` in `crates/kf-qemu/Cargo.toml`
(a one-line feature edit, not done on this branch). Diagnostics: `KF3_DIAG_PROGRESS=1000` prints `PROGRESS_STRAND` (a relay with
an obligation untouched >= 1 s while a worker kept iterating); endpoint: no `PROGRESS_STRAND` line, `backstop_found_work == 0`.

### 5.4 Grader round (2026-10-20, branch claude/relay-grade-20261020 merged; MEASURED unless marked)

* **G9 Watched unsourced (`fc8e97315`, real bug).** With `KF3_DIAG_PROGRESS` on, the diagnostic's wrapper answered the trait
  default `false` for `unsourced()`, so a worker could park beside the retry set -- in real operation the B arm of the hardware
  plan runs with that flag. Fixed: forwarded, the trait method has no default (a wrapper that forgets it does not compile), the
  block decision is one pure fn `worker::must_spin`. Behavioural tests: the wrapper answers like the wrapped relay, and wrapper
  and bare relay make the same block decision in all 24 obligation/pass states; mutant `Watched::unsourced -> false` killed by both.
* **G5 act-hold spin (`985e52776`).** Real operation: the 5080 / context act holds the record for a host RM call (ms); a serve that
  lost the record left demand and the worker spun at timeout 0 for the whole call. Now the act holds the record through
  `Cell::act_lock`, whose release re-rings a relay with demand and wakes the worker on every path (errors included), so the
  demand is a COVERED obligation and the worker blocks. SeqCst on `retry` / `act_hold` (one side always sees the other). Other
  holders (a look, the diagnostic, birth/free paths not converted) still make the worker spin. Model: hidden spins 0.
* **G5 phantom demand (`297de1e99`).** Real operation, 2 workers: `enter()` set the demand flag before its `try_lock`, so a peer's
  look in the turn's window saw "demand + record busy" and rang a finished relay again, possibly for ever under an adversarial
  schedule. Now demand is set only when the first `try_lock` loses (then one more try); the `pt`-lost arm makes demand durable
  itself. Model phantom cycles: W=2 25 -> 0, W=3 1949 -> 0; **W=2 with an act holder: 2 cycles remain (OPEN)**. An extra rule
  (no ring when a worker owns the token) was tried and reverted: it did not remove them.
* **Grade model W=3 + `pt` holder exceeds the explorer's 6 M-state budget** (`g1_models_exhaustive_three_workers`,
  `g2_sleep_safety_...`): MEASURED already at the merge `983fcca15` (before this round's fixes) and at 12 M; not raised (budget).
* **Real-thread G5 idle test under load** (10 runs beside 8 busy loops): 8 x I7 (1-5 serves in the quiet window) + 2 x "never
  settled": redundant doorbell rings (legal by the doorbell contract) whose wakes land after the guests are done, and a wake
  that ended before the window re-parking inside it; timeout-0 looks stayed 0 (no hidden spin). The harnesses now wait until
  nothing is rung, owned or demanded and count WAKEUPS: 10/10 under load.
* **Default stress runtime**: `relay_stress` + `grade_stress` 95 s (no features) / 97 s (`chaos-hooks`); seed 1 of the I6 seeds
  moved to an ignored test (sometimes ~570 s on a loaded box, open).

**Tests weakened / loosened in this branch (owner rule 2):** relay_stress I7 HARD -> SOFT (`0e0e054d1`); I6 monitor "wake in
flight" and kernel-view exclusions (classified false positives, §5.1); the quiet-window precondition (no token rung / owned, no
demand) and wakeups instead of parks (this round); `i6_classified` seed 1 moved to `#[ignore]`; `i1` a documented 6 M bound
under ReadIndex; the moot lifecycle / lazy tests restated per policy (§5.1); the stress expected-reds' assertion
`backstop_contended == 0` kept and two stronger ones added. None of these change production code.

## 6. Risks

* **R1** (Change A vs pushbuffer reuse / idle): open kernel RM 595.91.07 reads USERD `GPGet` only for its own CE channel's slot
  reuse (`mem_utils_gm107.c:1830-1862`) and diagnostics; UVM never reads it (`uvm_channel.c:1268-1273`: "UVM never reads GPGET.
  However, RM does read it when freeing a channel ... Mismatch ... makes RM wait"). The free-time comparison is in closed GSP-RM;
  Change A makes GET reach PUT EARLIER, never later (INFERRED). Closed UMDs: unreadable. Downgraded by owner ruling.
* **R2** credit wake: §3 row `Wait::Popped` — a spin until a pop, per credit wait mid-batch (Heaven publishes 923 entries/frame
  into a 512-slot ring, `V3_USER_RING_LIFECYCLE_20261016.md:483`: INFERRED one credit wait per frame). Not measured.
* **R3** spinning on the retry set while the holder is a descheduled vCPU (nested KVM): the worker burns a core for the hold;
  counted (`retry_looks`). **R4** `pt` is one map lock for every channel of the VM (~60 holders, CODE READ): every `pt` loss is a
  spin. **R5** fairness: unchanged per-visit bound (1 entry/turn, `REACT_ROUNDS`); a busy pass now always evaluates every relay
  (cost: one `epoll_wait(0)` + two atomics per idle relay + two private-word reads per waiting relay, per pass; not measured).
* **R6 (new, from removing the net in the tests):** with the 50 ms net in production a missed wake costs 50 ms, not a hang; with
  the net designed away, any lost-wake class is a hang in the tests — which is how seed 20 was found.

## 7. Hardware plan (not run)

Build on the trusted host per `scripts/bench/build_kf3.sh` (it installs `kf3-bins/<rev>/`); A = `82830cd5e`-merge base binary,
B = this branch's head. Linux, `KF3_DEBUG_FORCE_MIGRATABLE=1 KF3_DIAG_PROGRESS=1000`, the baton reproducer
(`scripts/bench/baton/`), n >= 10 per arm, ABBA order, one box, strictly serial; record `USER_RING_LOOP SUMMARY`
(`retry_looks`, `backstop_found_work`, `timeouts_with_work`), `PROGRESS_STARVED*`, client iteration rates. Falsifier for the
design: a starvation >= 1 s on B, or `backstop_found_work > 0` on B. Then gates 9/9, fast suite 30/30 on B. Change A canary
(verification, not a gate): a guest that overwrites every GPFIFO slot and its pushbuffer as soon as `GP_GET` passes it, with
a checksum the GPU writes; plus channel free / driver unload with in-flight work; then Windows (Heaven + Minecraft).
