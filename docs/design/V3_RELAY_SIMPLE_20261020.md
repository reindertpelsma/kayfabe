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
| `Wait::Marker` (SYNC_WAIT gate, batch/credit marker) | marker processed (`REFERENCE`) | engine non-stall fd (the marker's own `NON_STALL_INTERRUPT`, emitted AT the target position) | arm-before-evaluate holds in the loop; MEASURED miss found and fixed: a look that found the record busy dropped the consumed event (seed 20, `aad1e9ff4`); a second unclassified I6 hit remains (§5) | solved by construction, 1 open finding |
| `Wait::Popped` (no marker possible: open packet; Pointer mode not at an observed `GP_PUT`; markers latched off) | PBDMA pops | NONE | — | **spin**; with Change A it is only private-ring CREDIT (the write-back class is gone). Proposed source: a marker at the last packet boundary / observed `GP_PUT` the relay passes while credit < half the ring (a state rule, not a modulus). NOT built, cost not measured. |
| guest `GP_GET` write-back (Change B only, `GuestGet::Popped`) | pops | engine event when a marker follows, else NONE | — | **gone with Change A** |

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

## 5. Results (MEASURED, dev box, `cargo test --release -p kf-chan --features chaos-hooks`)

See the branch's final report for the per-test table. Key: the stress expected-reds (exhausted try_lock; seed 18) are GREEN
un-ignored with an infinite park; the stress harness found a real lost wake in this design (seed 20, 3/3 runs: a look that
consumed an engine event and found the record busy did not ring the waiting relay) — fixed; after the fix seed 20 passes 2/3,
the third trips the new I6 (a satisfied wait nobody rang while every worker blocks): **open, unclassified**.

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
