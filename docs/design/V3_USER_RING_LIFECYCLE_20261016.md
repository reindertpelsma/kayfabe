**STATUS: LIVE (lazy observe, production port, GPU-free; NOT run on hardware), 2026-10-17.** Branch `claude/lazy-prod-20261017` (from PHASE 1c `736c097d4`) ports the scratch "lazy observe" (a cache of the guest `GP_PUT`, rule A, and a lazy look at the
private `USERD`, rule B; scratch D3D11 196 -> 299 fps) onto the production Windows USER-ring relay, with the doorbell contract (no idle `GP_PUT` poll) brought over from `codex/doorbell-contract-20261017`. **Read section 0c first; its hardware checklist is the check to run.**
Corrected in place below: invariant I8 ("`GP_PUT` is loaded once per turn" is now "at most once"; Pointer mode may use an earlier read), and the section 10 / 9 totals.

**STATUS: LIVE (PHASE 1c, worker-only, GPU-free), 2026-10-16.** PHASE 1c (branch `claude/win-lifecycle-p1c-20261016`, from `claude/win-lifecycle-p1b-20261016` `4ab092324`) answers the SECOND independent review
(Opus 5.5, same day; section 0b): the P2' assumption became a steady-state one, so it gets switches, log lines and a VM-wide guard; the three mutants that survived are killed; a stale boundary across a ring switch is fixed.
Still NOT validated on hardware. **Read sections 0b and 0a first; the hardware report checklist is section 10.**

**STATUS: LIVE (PHASE 1b, worker-only, GPU-free), 2026-10-16.** PHASE 1b (branch `claude/win-lifecycle-p1b-20261016`, from
`claude/win-lifecycle-p1-20261016` `6f9163848`) fixes the defects an independent review (Opus 5.5, same day) found in PHASE 1: D1-D6 and T1-T3 of
section 0a, which CORRECTS sections 1.1, 1.2, 2, 5 (P2), 6, 7 (questions 1-3) and 8-9 below; each corrected passage says so in place. Still NOT validated
on hardware. **Read section 0a first.**

**STATUS of the original PHASE 1 text: LIVE where not marked "CORRECTED 1b" / "SUPERSEDED 1b", 2026-10-16** (branch `claude/win-lifecycle-20261016`, from
`claude/perf-fix-20261015` `0a7461173`). Written DESIGN-ONLY first; the implementation, the tests and the results below
landed on that branch. NOT validated on hardware: no Windows, no Linux guest and no host run has been made, PHASE 2 (the inline
vCPU path) and PHASE 3 (batched copy for window reads) are not started. Labels: **[measured]** = a number from a named run or
test; **[read]** = code or public hardware text; **[inferred]** = reasoning, never evidence.

# V3 — the Windows USER-ring lifecycle: Scan, Pointer, Window

## 0. What this is and why

The USER-ring relay (`crates/kf-qemu/src/userpump.rs`) reads and scans EVERY GP entry's pushbuffer of every Windows user-work
channel for the channel's whole life. **[measured, S1, `traces/perf_lifecycle_20261016/s1/` on branch
`claude/perf-lifecycle-measure-20261016`, Heaven OpenGL, 250 s window]** that is 923 GP entries/frame and 865 pushbuffer reads/frame
(1.14 MB/frame, 3.2 BAR1 copies/frame), 100 % of the BAR1 pushbuffer copies of channels that had already seen their `5080` SET_OBJECT and
had no registration pending; the whole 12-minute run saw ONE trigger segment (`rare_segments=1`, `ring_migrated=1`). The scan exists to
find that rare thing: the software-method trigger of a registered deferred API (`FIFO_UPDATE_CHANNEL_INFO` ring growth, `DMA_INVALIDATE_TLB`,
context bundles) on the `5080` software object's subchannel. A channel that has no registration cannot execute a valid trigger.

Settled design (owner/coordinator, 2026-10-13/16; `V3_WINDOWS_USER_RING_RELAY.md` §10): scan from birth only until the `5080` binding is known
(**Scan**); then, while no registration is live, copy GP entries as they are and read no pushbuffer byte (**Pointer**, "migratable
passthrough"); while a registration is live scan again (**Window**) until the trigger was read and acted on and the registration is gone.

Scope. Only Windows USER-work channels the relay carries today (`userrelay::enabled`, sysmem USERD, private GPFIFO/USERD). Linux guests, Translated and
Emulated channels and vidmem-USERD channels never get a `Lifecycle` (G3; `userpump::tests::the_lifecycle_is_reachable_only_from_the_windows_user_ring_pump`
pins that no other source file names it and that the pump has exactly one constructor call, in the Windows birth).

## 0c. Lazy observe (production), 2026-10-17

Branch `claude/lazy-prod-20261017` (from `claude/win-lifecycle-p1c-20261016` `736c097d4`). Labels: **[measured]** = a GPU-free test run named here, or a scratch hardware number
quoted from its report; **[read]** = code; **[inferred]** = reasoning. Nothing here has run on hardware.

### What and why

At ~1,700 GP entries per Heaven D3D11 frame the relay's per-entry turn spent most of its CPU on four accessors that almost never change anything: the guest `GP_PUT` read, the private
`USERD` observation (`GP_GET`, `REFERENCE`: two loads through the store view), the guest `GP_GET` write-back and `Fetch::observe`. **[measured, scratch `claude/d3d11-gap-20261016`,
`traces/d3d11_gap_20261016/REPORT.txt`, Heaven D3D11, run totals]**: private-`USERD` observations 189.8 M -> 12.9 M, guest `GP_PUT` reads 189.3 M -> 0.29 M, guest `GP_GET` write-backs 77.6 M -> 6.3 M,
and fps 196.2 -> 299.1 (median of 3, +52.4 %, no Xid, no 153 / 4101). That run had A and B together; their separate contributions were never measured (the split arms
`KF3_PUT_CACHE` / `KF3_LAZY_OBSERVE` exist for exactly that, below).

* **A, the `GP_PUT` cache.** Remember the `GP_PUT` a turn read, keyed by the ring generation, and use it for later turns until the cursor reaches it; then read fresh.
* **B, the lazy private-`USERD` look.** Read `GP_GET` / `REFERENCE` (and write the guest's `GP_GET` back) only every `OBSERVE_INTERVAL = 16`-th turn, or on the turn after the relay ended
  waiting, or when private credits are below 64, or while a split segment is open, or (D6, new here) on the turn after a skipped turn saw the guest ring full.

### Where it lives (design decisions D1 - D6, settled by the owner / coordinator)

* **D1.** In `kf_chan`: the pure predicates and the per-relay state are `kf_chan::user_lazy` (`must_observe`, `observe_reason`, `PutCache`, `Lazy`); the relay consults them INSIDE
  `kf_chan::user_turn::Relay` (`Phase::Obs`, `put_step`), so the model, property and stress suites that drive the real turn drive them. `userpump.rs` supplies `Io::walk_busy` (moved out of
  `gp_put`, because the cached path must keep the mapping-walk gate) and the two switches; it adds no rule of its own. `Relay::new` leaves both rules OFF (so that every pre-existing test keeps its meaning);
  `Relay::with_lazy` opts in and the pump calls it exactly once (pinned).
* **D2.** Active ONLY in **Pointer** mode of the Windows USER-ring relay. Scan and Window turns always read fresh: `Lazy::a_active` / `b_active` test the mode. The cache is dropped (and counted by reason) at
  EVERY mode change (checked at the start of every turn against the mode the cache was filled in), at every ring migration (`Relay::switch_source`, also for a new ring with a LOWER `GP_PUT`), when a refusal
  ends the relay (channel reset) and at retire (`Relay::retire`, called by `Pump::release`). Linux, Translated, Emulated and the USERD relay never name the module (pinned, `lazy_observe_is_reachable_only_from_the_windows_user_ring_pump`).
* **D3.** Default ON. `OBSERVE_INTERVAL = 16` is a named constant. Measurement switches, read ONCE when the first relay is created (`OnceLock`, never per turn), logged as `USER_RING_LAZY_OBSERVE_MODE`:
  `KF3_PUT_CACHE=0` turns A off, `KF3_LAZY_OBSERVE=0` turns B off; anything else (including unset) leaves the rule on; no value changes the interval. **Class M (measurement); required to boot: no.**
  Fate: hardwire ON and delete both once the split arms are measured. Both are in `docs/design/V3_FLAG_INVENTORY.md`.
* **D4.** No `GP_PUT` polling and no timer carries correctness (below).
* **D5.** Counters: section "Counters".
* **D6.** The guest-visible `GP_GET` write-back may lag; the guard and its argument are below.

### Rule A, precisely

`PutCache::select(generation, cursor)` returns the remembered `GP_PUT` only if it was recorded in the same ring generation **and the cursor has not reached it**; `Relay::put_step` records every fresh read in Pointer mode and
asks `Io::walk_busy` BEFORE it consults the cache. Consequences, each tested:

1. *I8 holds with the value in use.* An entry strictly before a `GP_PUT` the relay loaded was published by the guest before that load, a `GP_PUT` only advances within a ring generation (a cooperative guest; a hostile one can
   rewrite it, and then the relay is no worse off than eager: it still reads only the slot at the cursor, validates the 8-byte snapshot it took, and refuses a value outside the ring by name), and the guest's `GP_GET`
   is ours (we write it back), so a slot the relay has not read cannot be overwritten by a cooperative guest. No slot at or beyond the loaded `GP_PUT` is ever read. *(Correction of I8 below: "loaded once per turn" is now "at most once".)*
2. *"Nothing to do" is decided only on a fresh read.* A cache hit has `put != cursor` by construction, so `end_batch`, the idle decision and the batch marker are reached only through a fresh read.
3. *Nothing waits on the cache.* The doorbell contract is unchanged: the last doorbell covers every `GP_PUT` write before it; the relay loops until its cursor equals the `GP_PUT` it LOADED; a doorbell that arrives during a
   pass makes another pass. The cache never delays the discovery of work, because the only moment it could hide work (the cursor at the cached value) is the moment it is not used.
4. *The Window anchor.* The anchor of a Window is the `GP_PUT` of the newest turn whose registry observation showed no live registration (section 0a). A cached value was read EARLIER than that turn's epoch load, so
   "read before the registration's reply" still holds, and a lower anchor can only start the Window earlier (it scans more). After the flip the cache is dropped on the first Window turn and every Window turn reads fresh. **[measured]**
   `lazy_a_registration_at_every_position_keeps_i4_with_the_cache_on` (400 streams x 2 configurations, I4 and identical staging / switch positions), `a_every_mode_change_drops_the_cache_and_the_window_reads_fresh`, the model's flip scenarios with lazy ON.

### Rule B, precisely, and why a stale view is harmless

A skipped turn does not call `Io::hw_words` and therefore neither observes the private ring nor writes the guest's `GP_GET` back (a write-back that is already OWED, because an earlier write failed on a busy lock, is still retried by `Phase::Wb`).

* *A stale `GP_GET` only lowers credit.* `Fetch::available` counts issued entries not yet seen popped; fewer observed pops means fewer credits, never more (tests: `b_a_skipped_observation_never_over_counts_credit_*`, `user_lazy_model.rs` `stale_get_never_yields_more_credits_than_the_real_ring_has`).
  A relay with fewer than `CREDIT_FLOOR = 64` credits observes on EVERY turn, so every credit-stall decision (`credit_need`, the `available() < 2` checks) is taken on a fresh view: if the real credit is below 2 the stale one is below 64.
* *A stale `REFERENCE` only delays a gate release.* `SyncGate` releases when the processed marker count reaches the witness; `Marks::peek` never implies progress that has not happened. While a gate is draining
  (`SyncGate::waiting`) the relay is in `Need::Marker` and observes on every turn (user_lazy_model `stale_reference_never_releases_a_gate_early`, `sync_wait_stream_is_safe_and_live_*`; the model's `a1_*` / `t2_*` scenarios with lazy ON).
* *Waits.* **D4: a stale read can never be the only thing standing between a wait and its release.** Every wait the relay creates passes through `Relay::set_need`, which latches `Lazy::wait_pending`; the next turn observes
  whatever the interval says, and the latch is cleared only by an observation. The wake of a wait is never the relay's own view: it is the marker interrupt, or the worker's LOOK (`ChanPlane::user_ring_peek_one`), which
  reads the private `GP_GET` and `REFERENCE` itself (fresh) and rings the relay when the wait is satisfied or when the PBDMA has popped more than the guest's `GP_GET` shows (`pop_writebacks`). A wait that is already satisfied
  by the time a stale turn creates it (the pops happened between the last look and this turn) is found by the first bounded spin look (`wake_spin`; the satisfied check reads the private words fresh), not by the 50 ms backstop and not by a timer; that costs one extra turn, never a lost wake. (If the spin budget is spent before the pops happen, the wait is found by the marker interrupt exactly as under eager observation.)
  An idle decision (`Need::Idle`) needs `fetched == issued`: with a stale view that can only be true when it is true.
* *Open segment.* While the pump holds a split segment (`segment_pending`) every turn observes.
* *Scan and Window.* B does not apply: unconditional observation (`obs_unconditional`).

### D6: the guest-visible `GP_GET` write-back may lag; the guard

The guest ring can look full to the guest only because the guest reads the `GP_GET` the relay last wrote (`written_get`); with B that value can be older than the PBDMA's progress. Definition: with `E` the guest ring's slots and `N = 16`,
the **effective interval** is `K = min(N, max(1, E / 2))` (`user_lazy::effective_interval`); the relay relays at most ONE guest entry per turn, so between two observations it consumes at most `K` slots, never more than half a ring.

1. The guest's view is never older than `K` relay turns: observation (and with it the write-back) happens on every turn of Scan / Window, on the turn after every wait, when credits are below 64, while a segment is open, and at the latest every `K`-th turn.
2. The guard: a turn that SKIPPED its look and read a `GP_PUT` showing the guest ring full (`put + 1 == written_get`, the guest may be blocked on exactly the write-back this turn did not make) sets a flag; the NEXT turn observes (`obs_forced_ring_full`).
   From the moment the relay itself sees the ring full the guest's wait is as long as under eager observation (one turn). With A the relay sees "full" only at a fresh read (the cursor reaching the cached value), so the unconditional bound is the interval.
3. The relay cannot go idle with the guest still waiting: a turn that finds `GP_PUT == cursor` takes the end-of-batch decision on a fresh `GP_PUT`; with anything in flight it ends in `Need::Marker` / `Need::Popped`, which forces the next look (D4), and the worker's look rings it as soon as the pops are visible. So there is no state in which the
   relay has no backlog, has not written back what it has seen, and nothing will make it look.
4. *A guest blocked on a full ring behind its OWN gate still makes progress.* The relay is in `Need::Marker` (the gate waits for the marker, which waits for the PBDMA, which waits for the guest's acquire), every turn observes and writes
   back, and the worker's look rings the relay for pops the guest's `GP_GET` does not show yet (`pop_writebacks`); the pops counted by that look use `Fetch::fetched`, which can only be staler than the PBDMA, so it rings at least as often as before and
   one forced observation brings the view up to date.
5. *Small rings.* `K <= E / 2`, and `K = 1` (eager) for `E <= 3`; no ring size makes the argument fail. **[measured]** `b_the_interval_never_exceeds_half_a_small_guest_ring` (rings of 4, 8, 16, 32),
   `lazy_d6_a_guest_with_a_full_ring_sees_a_get_no_staler_than_the_base_plus_the_interval` (rings 8 - 128 against the OFF relay on the same streams: the worst extra staleness was 15 entries = `N - 1`, the bound),
   `lazy_d6_a_turn_that_skipped_its_look_and_saw_a_full_guest_ring_is_followed_by_a_look`, `lazy_every_consumed_entry_reaches_the_guests_gp_get_even_with_a_lagging_pbdma`, model `lazy_a_guest_ring_full_of_work_is_written_back_in_every_interleaving`.

What B costs, stated: under steady Pointer-mode streaming the guest sees its `GP_GET` up to `K` turns late (about 16 x 3 us at the scratch per-entry cost of 3.05 us); a guest that fills its ring in that time waits for the next write-back.
The scratch measurement ran with a guest ring of 32,768+ entries and never saw it; the guest driver chooses the ring size (`K` is `N` for any ring of 32 entries or more), so the hardware check looks at `credit_waits`, the fps and `obs_forced_ring_full`.

### Counters (bounded atomics in `relay_stats.lazy`; plain per-relay integers folded in every 256th turn and at retire; ONE line per relay at retire, `USER_RING_LAZY_OBSERVE <key> ...`, and one VM-wide `USER_RING_LAZY_OBSERVE SUMMARY`)

| counter | meaning |
|---|---|
| `turns` | turns begun (every `Phase::Obs`) |
| `put_cache_hits` | turns that used the cached `GP_PUT` (A) |
| `put_fresh_reads` | turns that read `GP_PUT` from the guest `USERD` (any mode) |
| `obs_skipped` | turns that skipped the private-`USERD` look (B) |
| `obs_unconditional` | turns that looked because B does not apply (Scan / Window, or B off) |
| `obs_forced_wait` / `_low_credit` / `_open_segment` / `_ring_full` / `_interval` | B looked, by the FIRST reason that applied in this order: wait, open segment, low credit, ring full (D6), interval |
| `inval_mode_change` / `_migration` / `_reset` / `_retire` | the cache was dropped (counted only when something was cached) |

Relationships that must hold (tested): `turns == obs_skipped + obs_unconditional + obs_forced_*`; `put_cache_hits + put_fresh_reads <= turns` (a turn that ends at the mapping-walk gate or on a `GP_PUT` outside the ring uses neither; `==` otherwise).

### Hardware checklist (nothing below has been run)

**Linux gates (must be unchanged; the code is unreachable from a Linux guest).** `scripts/bench/v3_gates.sh` 9/9, `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180` 30/30. Expectation: identical to the p1c baseline; there is no `USER_RING_LAZY_OBSERVE_MODE` line (the line is printed when the first Windows
USER-ring relay is created) and no `USER_RING_LAZY_OBSERVE` line. **Falsifier: any Linux result lower than the p1c baseline, or any lazy line in a Linux-only run.**

**Windows Heaven (D3D11 first, then OpenGL; default profile, then `KF3_PUT_CACHE=0` and `KF3_LAZY_OBSERVE=0` arms from the same binary).** Read, in order, from the `USER_RING_LAZY_OBSERVE` lines and the existing lifecycle lines:
1. `USER_RING_LAZY_OBSERVE_MODE put_cache=true observe_interval=16`.
2. The identity `turns == obs_skipped + obs_unconditional + obs_forced_*`.
3. Pointer-mode streaming (D3D11 Heaven is almost all Pointer): `put_cache_hits / turns` above 0.95 (scratch: GP_PUT reads fell to 0.15 % of turns); `obs_skipped / turns` above 0.85 and `obs_forced_interval` about `turns / 16`;
   `obs_forced_wait` of the order of the marker count (`pointer_markers`); `obs_forced_low_credit` and `obs_forced_ring_full` near 0 (a large `obs_forced_ring_full` means the guest ring is small enough for D6 to matter: report it with the fps).
4. `inval_mode_change` about `2 x to_window_fresh` (Pointer -> Window and back; Heaven has one ring growth), `inval_migration` equal to the number of `USER_RING_MIGRATED` lines at most, `inval_reset == 0`, `inval_retire` at most 1 per channel.
5. Unchanged from the p1c checklist (section 10), all still 0: `put_boundary_open_*`, `window_fresh_start_parse_error`, `anchor_stale_start`, `switch_cursor_mismatch`, `pointer_pb_read_violation`; relay `backstop_found_work` 0 (a rise would mean a wake the stale view hid);
   `credit_waits` and `pointer_backstop` not above the p1c baseline.
6. fps: **D3D11 about 297 - 299 (scratch A+B 299.1, n = 3); a result of about 220 or less means the port lost the gain** (the p1c lifecycle alone was 196.2). The split arms say which rule carries it.
   OpenGL must not drop (scratch OpenGL scan-off 338 - 359 without lazy). No Xid, no guest `nvlddmkm` 153, no `Display` 4101.

### Tests (GPU-free; `cargo test --release -p kf-chan -p kf-rm -p kf-trap -p kf-qemu`)

* `kf-chan/tests/user_lazy_turn.rs` (27): the real `Relay` over a recording `Io`: A (hits, cursor == cached, fresh-only idle, Scan / Window fresh, every mode change, stale cache after a Window, migration to a lower `GP_PUT`, mapping walk, out-of-range `GP_PUT`),
  B (interval, off = every turn, after a wait, while waiting, credit floor, open segment, over-count, write-back lag, small rings, D6 guard), retire / reset, counter identities, no environment access.
* `user_lifecycle_props.rs` (+11): **`lazy_off_is_bit_identical_to_the_base_relay_over_a_long_randomised_sequence`** (600 streams, 167,887 turns, 158,636 entries, every `Io` call hashed; the golden was computed on the UNCHANGED relay and is checked for
  `Relay::new`, `LazyConfig::OFF`, both switches `0`, and interval 1 / 0 with the cache off; the ON configurations must differ), ON-versus-OFF forwarding identity over 4 configurations x 300 streams x all four lifecycle modes x lagging PBDMA, write-back liveness,
  D6 bounds, I8 with poison above `GP_PUT`, hostile `GP_PUT`, I4 at every flip, 8 post-switch scanned entries per ring switch, refusal / retire, the credit floor under a slow PBDMA.
* `user_lifecycle_model.rs` (+6): the registration cycle with and without a ring switch, each rule alone, the SYNC_WAIT scenario, a full guest ring, all in every interleaving with lazy ON; plus a child run of 33 existing scenarios with `KF_TEST_LAZY=1` (every `Cfg::new` opts in).
* `user_lifecycle_stress.rs` (+2): real threads, well-behaved guest with concurrent registrations (default interval: 9.5 M entries bit-exact, 8.3 M cache hits, 8.1 M skipped looks, 29 k D6 forced looks in one run) and the hostile writer.
* `user_lazy_model.rs` (17, from the doorbell-contract lineage): the abstract predicates; `user_lazy.rs` unit tests (+9 over the lineage's 0 in that module; the lib total is 198); `kf-qemu` pins (+7): reachability, the switches read once, the counters / retire hook / walk gate wiring, no switch read in `turn()`.

### Mutation table (`scripts/lazy_mutation_check.py`; full names in `traces/lazy_prod_20261017/mutation_table.md`)

25 exact-text mutants of production source; the unmutated suites are green first; EVERY mutant is killed by at least two tests (3 first had one kill and `pump_walk_gate_dropped` first survived; each got a second test and was re-run). Suites per mutant: kf-chan lib,
`user_lazy_turn`, `user_lazy_model`, `user_lifecycle_props`, `user_lifecycle_stress`, `user_turn_unit`, `user_gate_rules`, the `lazy_` model tests (the 33-scenario child run is unmutated-only); the kf-qemu mutants add the kf-qemu lib. `idle_on_a_cached_read` also HANGS a stress test (killed by the 900 s limit, counted as a kill).

| mutant | what is broken | tests failing | some of them |
|---|---|---|---|
| `no_invalidate_on_mode_change` | the cache is not dropped when the lifecycle mode changes | 8 | `a_a_stale_cache_from_before_a_window_is_never_used_after_it`, `a_every_mode_change_drops_the_cache_and_the_window_reads_fresh`, `lazy_a_guest_ring_full_of_work_is_written_back_in_every_interleaving` ... |
| `cache_survives_migration` | the cache survives a ring migration (no invalidation, and the generation key is ignored) | 4 | `a_a_ring_migration_drops_the_cache_even_when_the_new_ring_has_a_lower_put`, `a_ring_generation_switch_invalidates_the_cache_even_for_a_lower_put`, `put_cache_every_reachable_state_keeps_i8_order_and_the_doorbell_contract` ... |
| `idle_on_a_cached_read` | a cached PUT is used even when the cursor has reached it (idle / end_batch on a cached value) | 23 | `TIMEOUT:lazy_a_well_behaved_guest_with_concurrent_registrations_loses_and_duplicates_nothing`, `a_a_cached_put_is_never_used_when_the_cursor_has_reached_it_and_idle_happens_only_on_a_fresh_read`, `a_a_stale_cache_from_before_a_window_is_never_used_after_it` ... |
| `stale_credits_over_counted` | Fetch::available counts one credit too many | 8 | `a_full_private_ring_recovers_without_any_periodic_observation`, `n_one_matches_an_independent_eager_fifo_and_reference_oracle`, `stale_get_is_conservative_across_many_wraps_and_receipts_are_exact` ... |
| `low_credit_never_forces_a_look` | the credit floor never forces an observation (the stale count is trusted at any level) | 3 | `b_fewer_than_the_credit_floor_forces_a_look_every_turn`, `lazy_a_slow_pbdma_and_a_small_private_ring_run_on_the_credit_floor_without_stalling`, `user_lazy::tests::low_credit_is_its_own_reason_and_looks_on_every_turn` |
| `stale_reference_trusted` | a SYNC_WAIT gate is released one marker early (a stale REFERENCE would be trusted as progress) | 8 | `a_retried_turn_at_the_same_gated_entry_publishes_one_barrier`, `a_zero_length_sync_wait_entry_is_gated`, `lazy_a_registration_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer` ... |
| `unissued_reference_trusted` | Marks::peek believes a REFERENCE that names markers not issued yet | 2 | `stale_reference_never_releases_a_gate_early`, `user_gate::tests::marks_count_only_tagged_references_and_extend_the_sixteen_bit_count` |
| `b_skips_while_a_wait_is_pending` | a pending wait / draining gate no longer forces the look | 4 | `b_a_turn_after_a_wait_always_observes`, `b_a_wait_forces_a_look_on_every_turn_until_it_is_over`, `lazy_every_consumed_entry_reaches_the_guests_gp_get_even_with_a_lagging_pbdma` ... |
| `wait_never_latched` | the relay does not remember that it ended a turn waiting | 4 | `b_a_turn_after_a_wait_always_observes`, `b_a_wait_forces_a_look_on_every_turn_until_it_is_over`, `lazy_every_consumed_entry_reaches_the_guests_gp_get_even_with_a_lagging_pbdma` ... |
| `open_segment_does_not_force` | an open segment no longer forces the look | 3 | `b_an_open_segment_forces_a_look`, `b_an_open_segment_is_counted_as_its_own_reason_on_every_turn_and_never_skipped`, `user_lazy::tests::an_open_segment_is_its_own_reason_and_beats_credit_and_interval` |
| `interval_never_fires` | the periodic look never fires | 7 | `b_a_skipped_observation_never_over_counts_credit_and_the_next_look_catches_up`, `b_the_guests_gp_get_write_back_lags_by_at_most_the_interval`, `b_the_interval_never_exceeds_half_a_small_guest_ring` ... |
| `interval_not_capped_by_the_guest_ring` | the interval is not capped at half the guest ring | 2 | `b_the_interval_never_exceeds_half_a_small_guest_ring`, `user_lazy::tests::the_interval_never_exceeds_half_the_guest_ring` |
| `no_ring_full_guard` | a skipped turn that saw the guest ring full does not force the next look (D6) | 2 | `b_d6_a_guest_ring_that_looks_full_is_written_back_by_the_very_next_turn`, `lazy_d6_a_turn_that_skipped_its_look_and_saw_a_full_guest_ring_is_followed_by_a_look` |
| `window_uses_the_cache` | rule A applies in every mode (a Window / Scan turn may use or fill the cache) | 3 | `a_every_mode_change_drops_the_cache_and_the_window_reads_fresh`, `a_scan_and_window_turns_always_read_gp_put_fresh`, `user_lazy::tests::rule_a_applies_in_pointer_mode_only` |
| `b_applies_in_every_mode` | rule B applies in every mode (a Scan / Window turn may skip its look) | 2 | `a_every_mode_change_drops_the_cache_and_the_window_reads_fresh`, `a_scan_and_window_turns_always_read_gp_put_fresh` |
| `observed_keeps_the_wait_flag` | an observation does not clear the wait latch (every later turn looks) | 2 | `lazy_a_well_behaved_guest_with_concurrent_registrations_loses_and_duplicates_nothing`, `user_lazy::tests::a_look_clears_the_wait_latch_and_the_relay_goes_back_to_skipping` |
| `walk_gate_after_the_cache_is_dropped` | the relay no longer asks Io::walk_busy before reading | 2 | `a_walk_in_progress_ends_the_turn_before_any_read_even_with_a_valid_cache`, `x_the_walk_check_runs_before_the_cache_is_consulted_and_is_not_counted_as_a_put_use` |
| `refusal_keeps_the_cache` | a refusal (channel reset) does not drop the cache | 2 | `lazy_a_refused_entry_ends_the_relay_and_a_retire_drops_what_was_cached`, `x_retire_and_a_refusal_drop_the_cache_and_count_their_reason` |
| `retire_keeps_the_cache` | retire does not drop the cache | 2 | `lazy_a_refused_entry_ends_the_relay_and_a_retire_drops_what_was_cached`, `x_retire_and_a_refusal_drop_the_cache_and_count_their_reason` |
| `switches_read_per_turn` | the environment is read inside the turn | 2 | `chan::userpump::tests::the_lazy_observe_switches_are_read_once_at_relay_creation_and_never_per_turn`, `x_the_lazy_state_does_not_touch_the_environment` |
| `pump_never_opts_in` | the pump never calls with_lazy (production would run eager) | 2 | `chan::userpump::tests::lazy_observe_is_reachable_only_from_the_windows_user_ring_pump`, `chan::userpump::tests::the_lazy_observe_switches_are_read_once_at_relay_creation_and_never_per_turn` |
| `pump_switch_read_in_the_turn` | the pump reads KF3_PUT_CACHE inside turn() | 2 | `chan::userpump::tests::the_lazy_observe_switches_are_read_once_at_relay_creation_and_never_per_turn`, `chan::userpump::tests::turn_reads_no_lazy_switch` |
| `pump_walk_gate_dropped` | the pump's Io::walk_busy never reports a walk | 2 | `chan::userpump::tests::the_pump_wires_the_lazy_counters_the_retire_hook_and_the_walk_gate`, `chan::userpump::tests::the_walk_gate_registers_the_wait_and_tells_the_relay_to_stop` |
| `release_does_not_retire` | Pump::release no longer calls Relay::retire | 2 | `chan::userpump::tests::release_retires_the_relay_through_relay_retire`, `chan::userpump::tests::the_pump_wires_the_lazy_counters_the_retire_hook_and_the_walk_gate` |
| `pump_counters_never_flushed` | the per-turn flush of the lazy counters is gone | 2 | `chan::userpump::tests::the_pump_wires_the_lazy_counters_the_retire_hook_and_the_walk_gate`, `chan::userpump::tests::turn_folds_the_lazy_counters_into_the_vm_wide_ones` |

The owner's list maps to: cache never invalidated on a mode flip = `no_invalidate_on_mode_change`; cache survives migration = `cache_survives_migration` (the generation key is ignored too: it would also miss by generation); idle on a cached read = `idle_on_a_cached_read`; stale credits over-counted = `stale_credits_over_counted` and `low_credit_never_forces_a_look`; stale REFERENCE trusted = `stale_reference_trusted`, `unissued_reference_trusted`; B skip while a wait is pending = `b_skips_while_a_wait_is_pending`, `wait_never_latched`; Window flip uses the cache = `window_uses_the_cache`; switches read per turn = `switches_read_per_turn`, `pump_switch_read_in_the_turn`.

### Not covered / honest limits

* The pump's `PumpIo` accessors and `segment_turn` are still not driven GPU-free (only pinned); `Io::walk_busy` moved from `gp_put` into its own method and is pinned, not run.
* The lazy tests drive `Relay`, not `Pump::turn`; the counter flush and the retire line are pinned by source text.
* The simulators consume the private ring at a fixed rate; a PBDMA that stalls for a long time is covered by the lagging-PBDMA tests, not by a real GPU.
* Rule A relies on a cooperative guest advancing `GP_PUT` monotonically within a ring generation; for a hostile guest the argument is "no worse than eager", which the hostile-writer stress and `lazy_a_hostile_gp_put_*` exercise but cannot prove.
* The performance claim (+52 %) is scratch, with A and B together, and has NOT been reproduced on the production relay.

## 0b. PHASE 1c: what the second review found and what changed (2026-10-16)

Labels as in 0a. **[measured]** = reproduced in this branch (the 1b base `4ab092324` for the "reproduced" column) by a named test or run; **[inferred]** = reasoning. The review's own
labels are kept where they matter. **Corrections to 0a below, in place of the text they correct:** P2' is not a rare window-start assumption but a STEADY-STATE one (every Pointer
batch-end marker rests on it, several hundred a second in Heaven); the named falsifier (`put_boundary_open_scan`) only saw the short Scan stretch before the first `SET_OBJECT`
(S1: about 75 entries per channel); and there was no switch to turn the new behaviour off. Both are changed below.

| id | finding (review severity) | reproduced on `4ab092324`? | fixed by | tests that guard it |
|---|---|---|---|---|
| N1 | P2' (a `GP_PUT` the guest wrote is a packet boundary) is a steady-state assumption: one violation corrupts the guest channel (HIGH, design) | [inferred] from the code: `end_batch` runs at every caught-up `GP_PUT` in Pointer mode | the VM-wide latch (below): the first packet seen open at a `GP_PUT` boundary switches Pointer markers off for every channel; modes and log lines make it measurable | unit `the_first_packet_open_at_a_gp_put_boundary_switches_pointer_markers_off`, `the_latch_is_vm_wide_and_counted_once`; model `l1_*`; props `latch_*`; Relay unit `a_packet_open_at_gp_put_in_scan_mode_is_reported_*` |
| N2 | the falsifier cannot falsify P2' where it matters and there is no switch (HIGH for the run plan) | [read] the pump hardwired `Lifecycle::new()` | `KF3_LIFECYCLE_MODE` (below); `always_scan` measures the whole run | unit `always_scan_never_leaves_scan_*`, model `m1_*`, props `modes_*`, kf-qemu `the_lifecycle_mode_*` |
| N3 | Window -> Pointer re-anchoring untested: mutant A passes all 264 lifecycle tests; the second window opens inside a submission (MEDIUM) | **[measured]** mutant A: kf-chan lib 174, model 47, props 25, stress 2, unit 16 all green (`traces`-free; log `p1c_base_repro.log` in the scratchpad) | tests only (the code was right) | unit `the_second_window_of_a_channel_is_anchored_at_the_gp_put_of_the_turn_that_left_the_first`; model `a_second_window_of_the_same_channel_starts_at_its_own_anchor_never_inside_a_submission` (mutant A reaches `MARKER_IN_PACKET`) |
| N4 | `next_boundary` survives a ring switch: a marker believed safe at an index no `GP_PUT` of the new ring ever had (LOW) | **[measured]** probe `probe_stale_boundary_after_a_source_switch` FAILED on the base; the Relay-level test authors a marker at the stale index before the fix | `Lifecycle::source_switched` clears `next_boundary`, `here`, `counted` | unit `a_remembered_gp_put_boundary_of_the_old_ring_is_forgotten_at_a_source_switch`, `a_boundary_counted_before_the_switch_*`; Relay unit `the_switch_forgets_the_old_rings_gp_put_boundaries_*` |
| N5 | tautology assertion `x == x` in `markers_need_a_known_parser_state_or_an_observed_gp_put_boundary` (LOW) | [read] | replaced by both outcomes of a Window `marker_possible` | the same test |
| W | mutant W: `migrate` drops `source_switched()`: kf-qemu 271/271 (MEDIUM, pump wiring untested) | **[measured]** 271/271 green | the ring switch is production code in `kf_chan::user_turn::Relay::switch_source` / `new_ring_get` (generation, cursor 0, grace, pending write-back, the guest's GET, post-switch scan); the pump's `migrate` calls them and writes none of those fields | Relay units `switch_source_installs_the_new_ring_*`, `a_switch_outside_a_window_*`, `a_nonzero_guest_get_*`; model `i4_*` (its stand-in calls the same method); kf-qemu pins `migrate_switches_the_relay_through_switch_source_*`, `the_switch_call_is_inside_the_admission_*`; hardware: `post_switch_scanned == 8 * USER_RING_MIGRATED` (section 10) |
| R | mutant R: `registry_live` keyed by the channel instead of the software object: no Window ever opens, kf-qemu 271/271 (MEDIUM) | **[measured]** 271/271 green | `software_key` (the one place an `ObjKey` is built) and `live_registrations` (the count) in `userpump.rs` | kf-qemu `a_live_registration_of_the_channels_software_object_is_counted_and_nothing_else_is` (a real `Registry`: the object, the channel's handle, another client, another object), `every_registry_lookup_of_the_software_object_goes_through_software_key`, `registry_live_counts_the_software_objects_registrations_for_the_channels_client`, `registry_live_never_asks_the_registry_about_the_channels_own_handle` |

### The modes (`KF3_LIFECYCLE_MODE`, measurement class, never required to boot)

Read ONCE, when the first relay is created (a process-wide `OnceLock`; never per turn; logged as `USER_RING_LIFECYCLE_MODE <name>`). Unset or empty = `default`; an unknown value runs
`default` and says so in that line. Default = the lifecycle fully on.

| value | behaviour | observable difference (test) |
|---|---|---|
| `default` | Scan -> Pointer (markers at observed `GP_PUT` boundaries) -> anchored Windows | no pushbuffer read in Pointer (`m1_default_reads_no_pushbuffer_*`, `modes_*`) |
| `always_scan` | the machine never leaves Scan (the base behaviour); EVERY 1b counter keeps running: `put_boundaries_scan` / `put_boundary_open_scan` over the WHOLE run, `post_switch_scanned`, `switch_cursor_mismatch`, the SET_OBJECT tally | every entry's pushbuffer is read, `put_boundary_open_scan` sees a boundary the default cannot (`m1_always_scan_measures_*`); the open boundary is the exact hazard of the default (`MARKER_IN_PACKET` reachable there, clean here) |
| `no_pointer_markers` | Pointer mode places no marker (spin / counted backstop wake); anchored Windows stay | `pointer_markers == 0`, the unannounced wakes counted (`pointer_carried_no_marker`); deadlocks without a look, live with one (`m1_no_pointer_markers_*`) |
| `no_anchored_windows` | a Window starts at the entry that sees the registration (PHASE 1 start) | the seeded gate lands inside a spanning packet (`m1_no_anchored_windows_*`, same shape as `machine_anchor_ignored`) |

### The VM-wide latch (answers the review's "guard")

The first time ANY channel reports a packet open at a `GP_PUT` boundary with the parser state known (Scan, or a Window; Pointer mode cannot see one), Pointer markers are switched
off for EVERY channel of the VM: one `AtomicBool` (`MarkerLatch`, owned by `LifecycleStats`, one per VM, one-way). Markers are performance, never correctness: with them off the wake is
the bounded spin / the counted 50 ms backstop (`pointer_carried_no_marker`, `pointer_backstop`), no entry is lost or duplicated (model `l1_*`: every latched interleaving is free of
`MARKER_IN_PACKET`, loss, duplication and reordering; without a look the full-guest-ring run deadlocks exactly as `no_pointer_markers` does; props `latch_*`: a second channel that never saw an open
packet loses its Pointer markers too, bit-exact forwarding). Loud: `USER_RING_POINTER_MARKERS_LATCHED_OFF` once per VM, counter `pointer_markers_latched_off` (0 or 1 VM-wide). Anchored
Windows stay (they have their own counters). **The latch needs an observation:** a guest that only ends submissions inside packets while the channel is in Pointer mode is not detected (the model
keeps that interleaving reachable on purpose); that is what `always_scan` runs are for.

### Ring-full anchors

A `GP_PUT` read while the guest ring is FULL (`put + 1` equals the `GP_GET` last written back, tracked in `Relay::written_get`; 0 at birth and after a switch) never becomes an anchor
(`anchor_refused_ring_full`): a driver that fills its ring may have cut a packet to get space, and a full ring is also when Windows grows it. Consequence, stated: if every anchor since the
last good one was refused, the flip finds `abs > safe_end` and the Window starts at the entry that sees the registration, which is not a known `GP_PUT` boundary; that is counted
(`anchor_stale_start`, must be 0 in an ordinary run) and reported (`stale_anchor=true`). Markers are NOT refused at a full-ring `GP_PUT` (that is the moment the guest waits for the `GP_GET` write-back and the marker is
what wakes it; model `d3_*`).

### Log lines (printed by the pump from machine notices; the machine has no I/O)

* `USER_RING_PUT_BOUNDARY_OPEN <key> mode nth cursor_slot put_slot relayed put_abs open_packet`: the first 8 per channel, and every one inside a Window.
* `USER_RING_WINDOW_FRESH <key> cursor anchor anchor_forwarded seeded stale_anchor first_segment`: one per Pointer -> Window flip; `first_segment` is `Closed`, `EndsOpen`, `Refused` or `None` (the Window ended first).
* `USER_RING_POINTER_MARKERS_LATCHED_OFF <key> ...`: once per VM.
* `USER_RING_LIFECYCLE_MODE <name>`: once per process.
* ★ 2026-10-24: the per-channel `USER_RING_LIFECYCLE` line gained `pid= classes= engine= bound= scan_entries= at=` after its `Counts {..}`; new one-shot `USER_RING_BOUND`; a final `at=exit` line for relays alive at device stop; relay totals appended to `USER_RING_LIFECYCLE SUMMARY`; `BUILD` and `WORKLOAD` lines at realize. Logging only. Format and rules: [`V3_RELAY_LIFECYCLE_LOG.md`](V3_RELAY_LIFECYCLE_LOG.md).

### Precondition P1 over a whole run (owner request, additive)

`set_object_5080_total` (per channel, summed VM-wide), `set_object_5080_rebinds` (a SET_OBJECT on a subchannel that held the 5080 software object and bound another class) and
`set_object_5080_max_per_channel` (VM-wide maximum of the per-channel total) count EVERY scanned `SET_OBJECT` of the 5080 class, not only the first. They are printed in the
`USER_RING_LIFECYCLE SUMMARY` line and the per-channel `USER_RING_LIFECYCLE` line. The tally lives in `Scan` and commits with the scan (a refused segment is not counted), so it is
exact for everything the scan saw: the whole run in `always_scan`, only the Scan stretch in the default (Pointer mode cannot see a later rebind by construction; model
`m2_the_default_machine_cannot_see_a_binding_after_pointer_mode_began` pins the gap). Reading: max per channel 1 and rebinds 0 across the app matrix in `always_scan` is the evidence for assuming P1.

## 0a. PHASE 1b: what the independent review found and what changed (2026-10-16)

Labels as above. **[measured]** = reproduced in this branch by a named test or run; **[inferred]** = reasoning.

| id | finding (reviewer's severity) | reproduced on `6f9163848`? | fixed by | tests that guard it |
|---|---|---|---|---|
| D1 | the Window-start gate (barrier + tagged marker) can land INSIDE an open guest packet: the Fresh restart clears the open-packet state that would have blocked it (HIGH) | **[measured]** reviewer's checker patch: 130 065 states, `MARKER_IN_PACKET`; own scenario `d1_*` (packet spanning two entries of one submission): 535 799 states, violation | anchored windows (below) | model `d1_*`, `p2prime_*` (residual), props `g2_p2prime_*`; machine unit tests `a_window_starts_only_at_the_anchor_*`, `the_anchor_is_always_a_gp_put_value_already_read_*` |
| D2 | a window that starts inside a packet can fail SILENTLY (tail words parse without refusal): a missed or false trigger (HIGH) | **[measured]** `d2_*` (false trigger from packet data: `triggers_found = 1`; refused tail) | anchored windows | model `d2_*`, `a5_*`; props `g2_p2prime_*` (600 streams identical to always-scan) |
| D3 | Pointer mode has no marker: credit waits and the guest `GP_GET` write-back are woken by a timer (50 ms backstop) (MEDIUM-HIGH, performance) | **[measured]** `d3_*` deadlocks with time forbidden (`pointer_no_markers` machine mutant = the base behaviour) | Pointer-mode markers at observed `GP_PUT` boundaries; loud counters | model `d3_*` (2), unit `markers_need_a_known_parser_state_or_an_observed_gp_put_boundary`, `loud_rate` |
| D4 | a fresh resume forgets a stored sub-device mask (MEDIUM) | **[measured]** (read: the old unit test pinned the reset); props `d4_*` loses the channel with the reset | `Scan::fresh_resume` keeps the mask state | props `d4_*`, unit `a_fresh_resume_keeps_*` |
| D5 | after the ring switch the new ring is forwarded unscanned from its first entry (MEDIUM) | read (design + model `i4_*` now shows the Window held) | the first `POST_SWITCH_SCAN` = 8 entries of the new ring are scanned; `switch_cursor_mismatch` counts a nonzero `GP_GET` in the new USERD | unit `after_a_source_switch_*`, model `i4_*` (terminal mode Window after a switch), props `b_ten_thousand_*` |
| D6 | a torn-down machine returns a scan plan; Pointer mode never re-checks the software object; `inline_plan` model neither validates entries nor checks registration state (LOW) | read | `Plan::Stop`; Pointer mode refuses a freed software object (cheap `live` flag) and counts it; `inline_plan`: model-only note, section 7 question 11 | unit `teardown_is_terminal_*`, props `d6_*` (2) |
| T1 | the REAL `Pump::turn_body` was untested (replacing the seeded gate left 271 `kf-qemu` tests green) | **[measured]** by the reviewer | the per-entry turn is production code in `kf_chan::user_turn` behind `Io`; model / props / stress drive it | the mutation table (section 8): `core_ignore_seed` now fails the suites |
| T2 | the machine mutant "Pointer mode does not record a length-0 SYNC_WAIT" survived (the Edge / Win11 shape) | **[measured]** | scenario `t2_*` | `life_forget_len0_sync` (section 8) |
| T3 | four invariants guarded by one test each (double fetch, ignore events, gate reset, bound without datum) | read | second tests from another angle | section 8 (no mutant is caught by a single test unless listed) |
| I8 | (added by the owner) nothing at or beyond the `GP_PUT` loaded in the turn is read | n/a | stub ring / store events in every driver | section 3, I8 |

### The anchor (corrects 1.1, 1.2, 5 P2)

**Windows (and markers) are anchored at `GP_PUT` boundaries, never at an arbitrary entry boundary.** The machine keeps `safe_end`: the absolute index of the
`GP_PUT` value read in the NEWEST turn whose registry observation showed no live registration (the epoch unchanged since a count of 0, or moved and a locked
count of 0). At the flip (a later turn whose observation shows a registration) the entries `[cursor, safe_end)` are forwarded unscanned in Pointer mode
(`anchor_forwarded`), and the Window starts at the first entry at or above `safe_end` (Fresh start: parser restarted, bindings and sub-device mask kept, gate
reset, the gate seeded from a recorded SYNC_WAIT). A turn that showed the registration never moves `safe_end`.

*Why entries below the anchor need no scan [inferred, model-checked as I4].* The anchor turn read `GP_PUT` before it loaded the epoch (SeqCst fence between) and
the epoch it loaded was still the old one, so that read precedes the bump of any later registration, which precedes the registration's reply. Every entry below
that `GP_PUT` was published before the reply; a cooperative guest publishes a trigger only after the reply. *The interleaving where the guest publishes between
the worker's `GP_PUT` read and its epoch observation:* that turn sees the bump, so it is not a clean turn and its `GP_PUT` (which may include post-reply entries)
is never the anchor; the previous clean turn's is. The model explores it (the guest's script steps interleave with every micro-step: `i4_*`, `a3_*`, `flip`).
Hostile guests: a trigger published before the reply only resets the guest's own channel (unchanged, `i4_a_trigger_published_before_the_reply_*`).

*CORRECTED 1c (second review N1, N2): P2' is a STEADY-STATE assumption, not a window-start one (every Pointer batch-end marker rests on it), and the Scan-mode counters below only see the setup stretch
before the first `SET_OBJECT`. The decision rule "do not enable Pointer markers / anchored Windows for that workload" is now executable without a rebuild: `KF3_LIFECYCLE_MODE` (section 0b) and a
VM-wide latch. Measure P2' on the whole run with `KF3_LIFECYCLE_MODE=always_scan`.*

**Assumption P2' [inferred; falsifiable]: a `GP_PUT` value the guest wrote is a packet boundary (a cooperative driver does not leave the GPU waiting inside a
packet at the end of a submission).** P2 (an ARBITRARY entry boundary is a packet boundary) is contradicted by the traces (section 5); P2' is weaker: the
measured splits are inline uploads spread over several GP entries, which a driver emits inside one submission. **Falsifier [measured on hardware later]:** the
Scan-mode counters `put_boundaries_scan` / `put_boundary_open_scan` (and `_window`): the number of `GP_PUT` values the cursor reached with the parser state
known, and how many of them had a packet OPEN; uncapped, per channel at release and VM-wide in `USER_RING_LIFECYCLE SUMMARY`. A nonzero open count on the app
matrix falsifies P2' for that workload: do not enable Pointer markers / anchored Windows there (use the short-entry trigger signature, open question 3 (c)).
**If P2' is false** a Window starts inside a packet (a silent misparse, or a refusal counted as `window_fresh_start_parse_error`) and the seeded marker lands in
the packet: the guest's own channel resets (a TDR on Windows). Not a host-security issue (pushbuffer integrity is not a security boundary for USER channels).
The reviewer's own scenario (a SUBMISSION that ends inside a packet) is exactly this case and stays reachable on purpose: model `p2prime_*`, props
`g2_p2prime_a_guest_that_ends_submissions_inside_packets_*`.

*Judgement of the settled design.* Sound under P2'; it does not remove the need to measure P2'. Two limits are stated, not hidden: (1) a worker that lags
behind several rung submissions sees only the newest `GP_PUT`, so mid-batch it knows no boundary (section 2, D3 below); (2) the seeded gate at the anchor relies
on P2' for its marker, exactly like the Pointer-mode batch markers.

### Pointer-mode markers (corrects 2, 7 question 1)

Pointer mode authors a batch / credit marker only where the cursor sits at a `GP_PUT` value the worker has READ (`end_batch`: the turn found `GP_PUT == cursor`;
`credit_need`: the cursor equals the nearest observed `GP_PUT` ahead, tracked as `next_boundary`). Each one is counted (`pointer_markers`). A wake it cannot
announce (a credit stall mid-submission, or behind a lagging worker's unseen intermediate `GP_PUT` values) keeps today's bounded spin and is counted
(`pointer_carried_no_marker`, loud); a Pointer-mode channel woken by the 50 ms backstop is counted (`pointer_backstop`) and logged when more than one a second
(`USER_RING_POINTER_BACKSTOP`, `kf_chan::user_loop::loud_rate`; the clock only decides how loud a timer wake is reported). Model `d3_*` pins the live case with
time forbidden; `i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look` pins the residual (a single submission larger than the private ring).
The OPEN test `i6_open_pointer_mode_credit_waits_need_no_look` stays ignored for that residual only.

### Invariant I8 (added 2026-10-16, owner)

**★ CORRECTION, 2026-10-17 (lazy observe, section 0c): "`GP_PUT` is loaded ONCE per turn" below is superseded for Pointer mode.** A turn loads `GP_PUT` AT MOST once; with rule A it may load none and use the value a EARLIER turn of the same ring generation loaded
(Pointer mode only; Scan and Window turns still load exactly once). The invariant is unchanged in substance: no ring entry at or beyond the `GP_PUT` IN USE is read (the cursor reaching the cached value forces a fresh load), the value in use was loaded by the relay and
never exceeds the guest's real `GP_PUT` for a cooperative guest, and the stub ring / store in every driver now checks that a turn that loaded no `GP_PUT` used one that is a value the guest wrote at or below its current `GP_PUT`
(`Env::cached_put_ok`, `Ev::BeyondPut`). The falsifier "a second `GP_PUT` load in a turn" stands.


Within a turn the relay reads NO ring entry at an index at or beyond the `GP_PUT` it loaded in that turn (`GP_PUT` is loaded ONCE per turn [⊘ at most once, 2026-10-17: see the correction above]), and no pushbuffer
for an entry it did not load in that turn; entries below `GP_PUT` but behind an unreached SYNC_WAIT are not read until the SyncGate's witness proves the
hardware reached it (I3). Window anchors are only `GP_PUT` values the worker has already read. Falsifier: any read at an index `>=` the loaded `GP_PUT`
(cyclic distance 0), a second `GP_PUT` load in a turn, or a pushbuffer read for a value other than the entry loaded. Guards: the stub ring / store in every
driver (`Ev::BeyondPut`, model bit `V_BEYOND_PUT` checked in EVERY model scenario), model `i8_*` (GP_PUT advancing mid-turn, backwards, beyond the ring), props
`i8_*` (poison entries just above `GP_PUT`, hostile `GP_PUT` values), the thread stress (a hostile writer publishes valid-looking entries just above `GP_PUT` and
moves it beyond the ring), and machine unit test `the_anchor_is_always_a_gp_put_value_already_read_*`.

## 1. The state machine (`kf_chan::user_lifecycle::Lifecycle`, pure, no I/O, no clock)

| state | meaning | reads a pushbuffer? | forwards |
|---|---|---|---|
| **Scan** | from birth until the scan has committed a SET_OBJECT that binds the channel's software object on a subchannel >= 5 | yes, every entry | after the scan and the SyncGate, as before |
| **Pointer** | software object bound, no live registration | NEVER (`Plan::Forward`; a pushbuffer read needs a `ScanPermit` only a `Plan::Scan` carries) | the guest's 8-byte entry, bit-exact, loaded once |
| **Window** | at least one live registration on the channel's software object | yes, every entry, with the gate | as before; stage + action + source switch at the trigger |

A Window is **Exact** when it continues a Scan that never stopped and **Fresh** when it starts after Pointer.

| from | event (observed at an entry boundary) | to |
|---|---|---|
| Scan | software object bound, 0 live registrations (`Cause::SoftwareBound`) | Pointer |
| Scan | software object bound, >= 1 live registration (`SoftwareBoundRegistered`) | Window(Exact) |
| Pointer | registry epoch moved and >= 1 live registration, **at the anchor** (CORRECTED 1b: entries below the anchor are forwarded unscanned first; section 0a) (`Registration`) | Window(Fresh) |
| Window | 0 live registrations (trigger executed + implicit delete, `REMOVE_API`, object freed) and the relay is quiescent (no gate draining, no segment in flight) (`RegistryDrained`) | Pointer |
| any | teardown | Dead (absorbs every later event; a delayed callback cannot revive a freed channel) |

`next(mode, cause)` is the table; the machine uses only it (I5). There is no Pointer -> Scan, no Window -> Scan, no timer edge, and no edge that depends
on a count of entries (OWNER_RULINGS §AE). "Live registration" means ANY live entry of the channel's `5080` object in the deferred-API registry,
whatever its bundle kind, executed or not (Open questions 8 and 9: deliberately wider than "a pending `FIFO_UPDATE_CHANNEL_INFO`").
`quiescent` is conservative (leaving a Window while a gate drains would be safe, because a Fresh restart resets the gate), kept for simplicity.

### 1.1 The flip: how a registration reaches the worker, and the "held reply"

**CORRECTED 1b (review D1/D2).** The Window no longer starts at the entry the worker happens to hold when it sees the epoch move: it starts at the ANCHOR (the
`GP_PUT` of the newest turn that showed no registration; section 0a). The first bullet below ("the flip boundary is the worker's own cursor at that entry") is
SUPERSEDED 1b: the cursor at that entry can lie inside a guest packet; entries between the cursor and the anchor were published before the reply and are forwarded
unscanned. The cooperative-guest argument of the second bullet is unchanged and is now stated for the anchor's `GP_PUT` read instead of the entry load.

The registration is served on the RPC drainer (`kf_rm::defapi::Registry::register`), which must never block. **Phase-1 mechanism:** the registry carries
a monotone `epoch` (`AtomicU64`) bumped (acq-rel) AFTER every change of the set of live entries, while the state lock is still held, and so BEFORE the
registration's reply is built. The worker loads the 8-byte GP entry (ONE load), then a SeqCst fence, then the epoch; only if the epoch changed does it
take the registry lock to count the live entries (`Registry::live_count`). Consequences, checked by the model (I4):

* (SUPERSEDED 1b, see above) an entry loaded BEFORE the epoch load that sees the bump is the first Window entry (it is scanned); every entry already forwarded in Pointer mode was
  loaded before the bump became visible. The flip boundary is the worker's own cursor at that entry, never a re-read of `GP_PUT` (suspect A3);
* a cooperative guest cannot publish a trigger before it has the registration's reply, and the bump precedes the reply, so every entry published after
  the reply is loaded after the bump and is scanned before it is forwarded. (The cooperative proof needs the epoch read only AFTER the `GP_PUT` read;
  loading it after the entry is the stronger order and also covers a guest that rewrites a published slot.);
* entries published before the flip but not yet loaded are scanned too (a superset of "entries published before the flip are not scanned"; the cost is one
  scan, except for the P2 risk below);
* a hostile guest that publishes the trigger before the reply only resets its own channel (owner ruling 2026-10-13); the model counts it
  (`i4_a_trigger_published_before_the_reply_may_reach_the_hardware_unscanned_and_nothing_wedges`).

This is the brief's "held reply" with one stated difference: in Phase 1 the worker is the ONLY publisher, so the flip is a wait-free atomic bump and there
is nothing for the reply to wait for; the reply is "held" only in the sense that it is built after the bump (the object link's reply is synchronous on the
drainer, and a real hold would need a new deferred-reply disposition there). A genuine hold (park the reply until a claimant has exited) is needed in PHASE 2
when an inline vCPU claimant exists (`kf_trap::relay_dispatch::request_worker` / `HANDOFF`), and is not built. The model lets the reply be delivered at ANY
later step after the bump, which covers an immediate and a held reply; both satisfy I4.

### 1.2 SYNC_WAIT across the Pointer -> Window edge (suspect A1)

Pointer mode forwards a SYNC_WAIT entry like any other and records the monotone `u64` index of the newest one (`outstanding_sync`; never a masked ring index).
It cannot prove it passed: `GP_GET` runs ahead of processing **[measured]** (`V3_USER_RING_EVENT_LOOP_20261014.md` §3.3) and Pointer mode must not place
a marker (no parser state). At a Fresh window start with a recorded SYNC_WAIT the SyncGate is seeded: the first Window entry is gated as if it were a
SYNC_WAIT entry (barrier + tagged marker, read nothing until `USERD.REFERENCE` shows the marker processed), which proves every earlier entry, the recorded
one included, was processed. A Fresh restart also resets the SyncGate (a stale `cleared` key one ring lap later would wave a SYNC_WAIT entry through;
`a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`). The marker is authored at the Window's first boundary, which under P2 is a
packet boundary: Open question 2. **CORRECTED 1b:** the first boundary is now the ANCHOR, a `GP_PUT` value (assumption P2', section 0a), not an arbitrary entry
boundary; a SYNC_WAIT gate whose marker would land in an open packet is exactly the case the Scan-mode `put_boundary_open_*` counters measure. **[measured, S1]** 1.28 % of Heaven's GP entries carry SYNC_WAIT (11.8 per frame), so in practice every Fresh window is seeded.

## 2. Pointer-mode entry handling (the only check is the entry)

Per entry: ONE 8-byte load; `inspect_entry` (length cap, privileged/indirect/reserved bits, 2^40 span) on THAT value; a credit check; publish the SAME
value (`write_user_gp`) behind the guest-ring and GP source guards (no pushbuffer guard exists because none was read); advance the cursor; record SYNC_WAIT.
No `Poll`, no tick: the doorbell contract is unchanged. **SUPERSEDED 1b (review D3):** ~~Markers need a packet boundary, so Pointer mode authors none: the guest `GP_GET` write-back and
private-ring credit waits are announced by the spin, any engine event and the counted 50 ms backstop (Open question 1).~~ Pointer mode authors a marker at an
observed `GP_PUT` boundary (section 0a); only the waits it cannot announce fall back to the spin / backstop, counted and loud. One guest entry is published per
turn, exactly as in Scan mode (batching several per turn would cut the per-entry admission and doorbell cost; a follow-up, not done).
**[measured, S1]** the Scan-mode gates (410 k) and batch markers (109 k) of a Heaven run disappear in Pointer mode.

## 3. Invariants, falsifiers, tests (all in `crates/kf-chan`, GPU-free)

| id | invariant | falsifier (written before the run) | tests | mutation (see §8) |
|---|---|---|---|---|
| I1 | no pushbuffer byte is read in Pointer mode | the model sees a pushbuffer read while the machine says Pointer; `pointer_pb_read_violation != 0` | model `i1_*`; props `g1_*` (stub store: `pb_reads_in_pointer == 0`); unit `pushbuffer_reads_are_counted_per_mode_*`, `pointer_mode_forwards_and_never_asks_for_a_scan` | machine `pointer_scans` |
| I2 | every entry published before the last doorbell is forwarded exactly once, in order, bit-exact | a terminal state with a missing, duplicated or reordered private entry | model terminal check + `ORDER`; props `g2_*`, `corpus_*`, `b_ten_thousand_cycles_*`, `soak_*`; stress `a_well_behaved_guest_*` | glue `drop_flip_entry` |
| I3 | no entry at or after an unreached SYNC_WAIT is READ before the tagged-marker witness | `STALE` (a pushbuffer read before the guest wrote it) | model `a1_*`, `i3_*` | machine `no_seed`, `forget_sync`; glue `ignore_seed`, `no_gate_reset` |
| I4 | no entry published after a registration's reply is forwarded before a Window scanned it | a post-reply entry forwarded unscanned while a registration is live | model `i4_*`, `a3_*`, `a5_*`, `b_*`; props `g2_*`; stress `a_well_behaved_guest_*` | glue `epoch_before_put`, `flip_at_put`; machine `exit_ignores_live` |
| I5 | transitions only via the listed events | a (mode, cause) pair outside `next()` changes the mode | unit `the_transition_table_is_exactly_the_documented_one`; every model state checks it (`V_TRANSITION`) | machine `pointer_to_scan` |
| I6 | no progress depends on time | a terminal state with covered work unrelayed when the model has no timer step | every model test; `i6_*`; unit `the_machine_names_no_clock_no_sleep_and_no_poll` | glue `ignore_events` |
| I8 | (1b) nothing at or beyond the `GP_PUT` loaded in the turn is read; `GP_PUT` is loaded once per turn [⊘ at most once since 2026-10-17: Pointer mode may use an earlier read of the same generation, section 0c]; a pushbuffer only for the entry loaded; Window anchors are only `GP_PUT` values already read | any read at cyclic distance 0 from the loaded `GP_PUT`, a second `GP_PUT` load, a pushbuffer for another value | stub ring / store `Ev::BeyondPut` in every driver (model bit `V_BEYOND_PUT` in every scenario); model `i8_*`; props `i8_*`; stress hostile writer; unit `the_anchor_is_always_*` | source `core_reads_beyond_put` |
| I7 | the value tested is the value stored | a stored value differing from the loaded one while the guest changes ONE dword at any instant | model `a4_*`; props `a4_*` (every bit flip); stress hostile writer | glue `double_fetch`, `skip_validation` |

Suspects A1..A7 of the appendix map to `a1_*`..`a7_*` (A1 `a1_*`; A2 `a2_*`; A3 `a3_*`, `mutant_*gp_put*`; A4 `a4_*`; A5 `a5_*`; A6 `a6_*`; A7 `a7_*`,
`userpump::tests::the_lifecycle_is_reachable_only_*`, the unit test on teardown); B `b_*`; C is `kf-trap` `inline_plan_*` and `loom_a_sync_wait_stop_*`; D `g2_*`,
`corpus_*`, `soak_*`, `obs_*`.

## 4. Counters (loud where the assumption is falsifiable)

Per channel (`USER_RING_LIFECYCLE` line at release) and VM-wide (`kf_chan::user_loop::Stats::life`, `USER_RING_LIFECYCLE SUMMARY` at device stop and when the
last relay goes): `entries_scan/pointer/window`, `to_pointer`, `to_window_exact`, `to_window_fresh`, `to_pointer_from_window`, `restarts`, `seeded_gates`,
`window_entries_max`, `segments_scan/window`, `straddles_scan` and `straddles_window` (segments that END inside an open packet, per mode, uncapped: the P2
measurement; the older `USER_RING_TRACE` `straddles` is capped at 16 per channel), `window_fresh_start_parse_error` (a scan refusal in the first four segments of
a Fresh window, once per entry), `window_binding_changed` (the software binding changed inside a Window: P1 falsifier), `window_unscanned_long` (0 by construction:
nothing in a Window is skipped), `pb_bytes_scan/window`, `window_bar1_bytes`, `pointer_pb_read_violation` (must stay 0; also a `debug_assert!`),
`scan_no_software`, `after_teardown`. **1b adds:** `put_boundaries_scan/window` and `put_boundary_open_scan/window` (the P2' falsifier, uncapped),
`pointer_markers`, `pointer_carried_no_marker` (loud), `pointer_backstop` and `pointer_backstop_loud`, `anchor_forwarded`, `post_switch_scanned`,
`switch_cursor_mismatch` (loud), `pointer_binding_unchecked`, `pointer_software_gone`. **1c adds:** `anchor_refused_ring_full`, `anchor_stale_start` (must be 0), `pointer_markers_latched_off`
(0 or 1 VM-wide), `set_object_5080_total`, `set_object_5080_rebinds`, VM-wide `set_object_5080_max_per_channel` (section 0b). Existing counters keep their meaning (`carried_ungated` now counts only Scan/Window entries).

## 5. Assumptions and what the existing traces say about them

* **P1 (the `5080` SET_OBJECT occurs once at setup).** Not measurable in Pointer mode by construction. Falsifiable in a Window (`window_binding_changed`).
  Existing data neither supports nor contradicts it beyond the short Scan phases: **[measured, S1]** 5 299 pushbuffer reads before the binding over 71 channels
  of a Heaven run.
* **P2 (SUPERSEDED 1b by P2', section 0a: a GP_PUT value, not an arbitrary entry boundary, is a packet boundary) (the driver ends GP segments on packet boundaries, so a Window may start with fresh parser state at any entry boundary).** **CONTRADICTED as a general property;
  the exposure is a probability.** [measured] The Windows driver splits big inline uploads across GP entries (`INC1 sub1/sub2 m0x1b0 1/1057`: 276 STRADDLE records,
  sizes up to 3841 arguments, in the 2026-10-14 traces); the refusal traces (a sample taken around refusals, so biased) hold **55 of 196** recorded segments that
  END inside an open packet (`user_lifecycle_props::corpus_p2_*`, 124 of them replayable, none open); the per-channel `straddles` summary is capped at 16 and still
  totals 282 over 2.67 M segments (520 summary lines), 16 on the busiest Heaven channel (2 314 629 segments); Minecraft produced `carried_ungated=27 334` of
  1 772 109 gated SYNC_WAIT entries (1.5 %), i.e. 25 153 in six minutes on one channel; Heaven's relay reported `carried_ungated=0` over 51 M GP entries (S1). A flip at an
  arbitrary entry boundary lands inside an open packet with a probability that is workload-dependent and unmeasured; the sequential model (`g2_p2_*`) shows that
  EVERY difference between the lifecycle and the always-scan base comes from such a flip and is counted. See Open question 3.
* **P3 (the reply is held while the mode flips).** Satisfied by the ordering of §1.1 in Phase 1.
* **P4 (what keeps a channel scanning).** The registry empties (**[measured]** `implicit_deletes` equals `registered` in every measured Windows trace: 93 of 93 in the Heaven
  OpenGL run) or the object is freed.

## 6. Window details

* Fresh start: bindings are kept (the software subchannel is not re-learned), the packet in progress is forgotten, the sub-device mask state is KEPT as the last
  scan left it (CORRECTED 1b, D4: ~~returns to its birth value~~, which refused a later USE and lost the channel; an unscanned `SET/STORE/USE_SUBDEVICE_MASK` still
  cannot be seen: Open question 10), `SyncGate` is reset, `Grace` is cleared.
* Refusal in a Window: exactly today's scanner refusal (`Grace`, then `USER_RING_REFUSED`, channel dead). Never a recovery.
* VRAM-backed pushbuffers in a Window use the existing path and are counted (`window_bar1_bytes`). PHASE 3 replaces bulk MMIO reads in windows.
* A Window with no trigger never ends by itself and no timer ends it; it is counted (`window_entries_max`) and every other channel is served (one entry per turn).
* Source switch (1b, D5: the first 8 entries of the new ring are scanned before the Window may end, `POST_SWITCH_SCAN`; a nonzero `GP_GET` in the new USERD is counted
  and logged as `USER_RING_SWITCH_CURSOR_MISMATCH` while the cursor still resets to 0): the existing `migrate` path (`USER_RING_MIGRATED`): cursor 0 on the new ring, new USERD, generation + 1, private native GET untouched. Cursor semantics
  are unmeasured (Open question 4); the model checks both candidates.

## 7. Open questions (each has a test; `#[ignore = "OPEN: ..."]` where the expected behaviour cannot hold)

1. **Pointer mode has no marker, so no interrupt announces private-ring credit or the guest `GP_GET` write-back.** A marker segment is only safe at a packet boundary, which
   Pointer mode cannot know. Tests: `i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look` (passes: live with looks, deadlocked without) and
   `#[ignore] i6_open_pointer_mode_credit_waits_need_no_look` (expected: live without any look). Options: (a) spin + any engine event + the counted 50 ms backstop
   (implemented; correct, possibly slow when the 512-slot private ring fills: Heaven publishes 923 entries/frame); (b) author markers at P2-assumed boundaries (rejected: a
   wrong boundary corrupts the cooperative guest's stream); (c) a larger private ring (storage layout change); (d) a per-segment boundary witness (reads the pushbuffer: contradicts
   G1). **Recommendation: (a) for Phase 1; measure `credit_waits` / `backstop_carried` / `wake_backstop` on the host before deciding on (c).** This is the main performance risk of the design.
2. **The Window-start gate marker relies on P2.** §1.2. Options: (a) author it at the Fresh boundary (implemented); (b) prove "passed" from the channel's PB position in USERD
   (`TOP_LEVEL_GET`), unmeasured; (c) never leave a SYNC_WAIT unproven: impossible without a marker. **Recommendation: (a) with the counters; measure (b) on the host.**
3. **P2 is contradicted by measured straddles** (§5). Test `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh` shows the failure
   exactly (a loud refusal, counted once, then today's refusal behaviour) and `a5_a_flip_inside_a_packet_whose_data_looks_like_the_trigger_*` the false-positive form.
   Options: (a) accept and count (implemented); (b) delay the Window's scanning to the first entry whose scan returns to a boundary (a Window that cannot be trusted for its first
   segments, but nothing is refused before it); (c) state-free trigger signature for short entries (`V3_WINDOWS_USER_RING_RELAY.md` §10, "scan only SHORT GP entries").
   **Recommendation: (a) for Phase 1, then (b) or (c) if `window_fresh_start_parse_error` is nonzero on the host.**
4. **Cursor semantics at the source switch are unmeasured (P0 a).** Production keeps reset-to-0 on the new ring. Model: `b_the_source_switch_*` (both pass when the new ring is
   prepared before the trigger is visible), `b_mismatched_cursor_semantics_*` (the checker sees the loss), `#[ignore] b_open_continued_indices_need_the_new_gp_put_preset_before_the_switch`
   (with continued indices and a zero GP_PUT in the new USERD the relay forwards stale slots). Reset-to-0 does not depend on when the guest prepares the new ring
   (`b_reset_to_zero_does_not_depend_on_when_the_guest_prepares_the_new_ring`).
5. **A rebind of the software subchannel while in Pointer mode is invisible** (`#[ignore] a6_open_a_rebind_in_pointer_mode_is_not_known_to_the_window`; the in-Window rebind is
   seen and counted). Cannot be known without reading; options: accept (implemented), or count SET_OBJECT occurrences in Scan to bound the risk.
6. **A channel without a software object scans forever** (a registration needs the object, and it cannot be allocated after the first publication, so such a channel could
   safely be Pointer from its first entry). Implemented as the brief says, counted (`scan_no_software`). Recommendation: the owner may allow Pointer for it.
7. **Held reply** in Phase 2 (inline claimants): §1.1.
8. **"Pending" = any live entry, executed or not** (an explicit-delete entry stays live after its trigger and may be triggered again; **[measured]** every registration in the
   traces is implicit-delete). Cost: a Window until `REMOVE_API` for an explicit entry.
9. **All bundle kinds open a Window, not only `FIFO_UPDATE_CHANNEL_INFO`.** The relay also executes `DMA_INVALIDATE_TLB` and context bundles at their triggers
   (`Pump::software`); an unscanned trigger of those reaches the hardware, which has no such registration and resets the channel. Deviates from the brief's wording; required for correctness.
10. **Sub-device mask state after an unscanned stretch** is kept as the last scan left it (CORRECTED 1b; ~~reset to its birth value~~).
11. **(1b, D6) The PHASE-2 `kf_trap::inline_plan` model neither validates entries (`inspect_entry`) nor consults the lifecycle, the epoch or the registration
    state.** Fine while it is model-only; a MUST-FIX before it is wired: the inline vCPU path must run the same `user_turn::Relay` checks (entry validation, anchor,
    registration state). Not changed in 1b (no inline vCPU wiring).
12. **(1b, 1c) P2' is unmeasured.** (1c: measure it on the whole run with `KF3_LIFECYCLE_MODE=always_scan`; the VM-wide latch limits the damage of a violation seen in Scan or a Window, section 0b.) The first hardware run reads `put_boundary_open_scan` / `_window` on the app matrix (Heaven, GravityMark, Minecraft, Edge): any
    nonzero value means the anchor design must not be enabled for that workload (fallback: the short-entry trigger signature, question 3 (c)).
13. **(1b) Cursor semantics at the switch stay unmeasured (question 4).** The new loud detector is `switch_cursor_mismatch` (a nonzero `GP_GET` in the new USERD at
    the switch); scanning the first 8 entries catches garbage slots, not valid stale ones.
14. **(1b) Pointer-mode waits mid-submission.** A credit stall inside a submission larger than the private ring, or behind a worker that lags several rung
    submissions, has no known boundary: no marker, the spin / 50 ms backstop wakes it, counted (`pointer_carried_no_marker`, `pointer_backstop`). Option: a larger private ring.

## 8. Mutation checks

**CORRECTED 1b (T1/T3).** ~~Glue mutants were defects of a test transliteration of the turn.~~ The per-entry turn is production code now
(`kf_chan::user_turn::Relay`), so `scripts/lifecycle_mutation_check.sh` mutates the REAL source: the machine mutants of `kf_chan::user_lifecycle::Mutant`
(`KF_LIFECYCLE_MUTANT`, feature `lifecycle-mutants`, off by default and never enabled by another crate) plus **exact-text source mutants** (`core_*` in
`user_turn.rs`, `life_*` in `user_lifecycle.rs`, `scan_*` in `user_scan.rs`; the script refuses a patch that does not apply exactly once and restores the files
after every run). Each runs the whole GPU-free lifecycle suite (`--lib` and the five lifecycle test binaries) and lists the failing tests.

Result (baseline green; **27 mutants, every one caught by at least two tests**; counts include the new unit tests of `tests/user_turn_unit.rs`; the rows
`core_ignore_seed`, `core_no_gate_reset`, `core_double_fetch`, `core_bound_without_datum`, `core_pointer_gone_unchecked`, `core_carried_gate_removed`,
`core_no_fresh_resume` and `machine_pointer_markers_anywhere` are from the second pass after those tests were added; an earlier pass had caught `core_bound_without_datum`, `core_pointer_gone_unchecked` and
`core_carried_gate_removed` by ONE test each and `core_no_fresh_resume` by none, which is why the unit tests exist):

| mutation (+ = machine mutant via `KF_LIFECYCLE_MUTANT`; `core_*` / `life_*` / `scan_*` = exact-text patch of the production source) | invariant | failing tests |
|---|---|---|
| `machine_pointer_scans`: Pointer mode plans a scan (a pushbuffer is read in Pointer) | I1 | 51: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 48 more |
| `machine_no_seed`: a Fresh window never seeds its gate | I3 / A1 | 6: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing`, `i3_the_same_stream_with_the_guest_free_to_publish_b_early_is_found` and 3 more |
| `machine_forget_sync`: Pointer mode forgets the SYNC_WAIT it forwarded | I3 / A1 | 8: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing`, `b_the_monotone_entry_counter_survives_the_u32_boundary` and 5 more |
| `machine_pointer_to_scan`: a registration in Pointer mode moves the channel back to Scan | I5 | 35: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 32 more |
| `machine_revive_after_teardown`: a teardown does not stop the machine | A7 | 3: `a7_teardown_at_any_instant_stops_the_private_ring_for_good`, `d6_a_torn_down_machine_reads_nothing_and_publishes_nothing`, `unit:teardown_is_terminal_and_a_late_event_cannot_revive_the_channel` |
| `machine_exit_ignores_live`: a Window ends when quiescent whatever the registry says | I4 | 34: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 31 more |
| `machine_mask_frozen`: a Window never notices a moved software binding | A6 / P1 | 2: `a6_a_rebind_inside_a_window_is_seen_and_counted_and_the_new_binding_is_used`, `unit:a_changed_software_binding_inside_a_window_is_counted` |
| `machine_anchor_ignored`: a Window starts at the first entry that sees the registration, not at the anchor | D1 / D2 | 8: `d1_a_registration_inside_a_submission_never_puts_a_host_marker_inside_a_guest_packet`, `d2_data_that_looks_like_the_trigger_inside_a_packet_of_one_submission_is_never_a_trigger`, `d2_the_tail_of_a_packet_inside_one_submission_is_never_refused_by_a_window` and 5 more |
| `machine_pointer_markers_anywhere`: Pointer mode places a marker wherever it is asked | D1 / D3 | 5: `a_wake_pointer_mode_cannot_announce_is_counted_loudly_not_hidden`, `d3_a_credit_stall_inside_a_packet_never_gets_a_marker`, `i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look` and 2 more |
| `machine_pointer_no_markers`: Pointer mode never places a marker (the PHASE 1 behaviour) | D3 | 3: `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...`, `d3_pointer_mode_batch_ends_announce_pops_by_an_event_with_no_look`, `unit:markers_need_a_known_parser_state_or_an_observed_gp_put_boundary` |
| `core_ignore_seed`: `let gate_entry = self.raw;` (the seeded gate is ignored; the reviewer's T1 mutation) | I3 / T1 | 6: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a_window_that_follows_a_forwarded_sync_wait_gates_its_first_entry_before_reading_it`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing` and 3 more |
| `core_no_gate_reset`: the SyncGate is not reset at a Fresh restart (stale proof one ring lap later) | I3 / T3 | 3: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a_fresh_window_resets_the_gate_so_a_proof_from_an_earlier_lap_is_gone`, `t3_a_second_window_one_ring_lap_later_gates_its_sync_wait_entry_again` |
| `core_epoch_before_put`: the registry epoch is loaded first, before GP_PUT and the entry | I4 | 26: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 23 more |
| `core_double_fetch`: a second load of the entry is what gets stored | I7 / T3 | 4: `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_than_the_tested_one`, `a_hostile_writer_rewrites_ring_entries_and_gp_put_while_the_worker_runs`, `a_turn_loads_gp_put_once_then_the_entry_at_the_cursor_then_the_epoch_in_that_order` and 1 more |
| `core_drop_flip_entry`: the first entry of a Fresh window is skipped | I2 | 31: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 28 more |
| `core_skip_validation`: an entry inspect_entry refused is forwarded anyway | A4 | 4: `a4_every_single_bit_flip_of_a_valid_entry_is_refused_by_name_or_forwarded_validated`, `a4_hostile_entries_never_panic_and_never_store_an_unvalidated_value`, `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_than_the_tested_one` and 1 more |
| `core_bound_without_datum`: a SET_OBJECT header (datum unscanned) counts as the binding | A2 / T3 | 2: `a2_the_binding_packet_split_across_two_entries_switches_to_pointer_after_the_datum`, `the_binding_is_known_only_after_the_datum_not_after_the_set_object_header` |
| `core_reads_beyond_put`: GP_PUT == cursor is treated as work: the entry at GP_PUT is read | I8 | 63: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 60 more |
| `core_pointer_gone_unchecked`: Pointer mode never notices a freed software object | D6 | 2: `d6_pointer_mode_refuses_a_freed_software_object_like_the_scan_path_and_counts_it`, `pointer_mode_refuses_a_freed_software_object_and_publishes_nothing_for_it` |
| `core_carried_gate_removed`: a SYNC_WAIT behind an open packet is gated (a marker inside the packet) | D1 | 2: `a_sync_wait_entry_behind_an_open_packet_is_read_at_once_and_counted_without_a_gate`, `obs_a_sync_wait_behind_an_open_packet_is_read_at_once_and_counted_exactly_once_in_a_window` |
| `core_no_fresh_resume`: the parser is not restarted at a Fresh window (a stale open packet) | D2 | 2: `a_fresh_window_forgets_a_packet_the_scan_left_open_before_pointer_mode`, `d2_a_packet_left_open_by_the_scan_before_pointer_mode_does_not_swallow_a_later_trigger` |
| `life_stale_anchor`: a turn that showed the registration moves the anchor | D2 | 41: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...` and 38 more |
| `life_no_post_switch`: the Window may end right after a source switch | D5 | 2: `i4_no_entry_published_after_the_reply_is_forwarded_before_a_window_scanned_it`, `unit:after_a_source_switch_the_window_stays_open_for_a_bounded_number_of_entries` |
| `life_dead_scans`: a torn-down machine returns a scan plan | D6 | 2: `d6_a_torn_down_machine_reads_nothing_and_publishes_nothing`, `unit:teardown_is_terminal_and_a_late_event_cannot_revive_the_channel` |
| `life_forget_len0_sync`: Pointer mode does not record a LENGTH-0 SYNC_WAIT | T2 | 3: `t2_a1_with_a_length0_sync_wait_never_reads_the_late_pushbuffer_early`, `unit:the_newest_pointer_sync_wait_is_recorded_by_monotone_index_across_ring_laps`, `unit:the_restart_is_requested_until_applied_and_the_seed_until_the_witness_is_proven` |
| `life_no_boundary_tracking`: a Pointer-mode marker is never allowed (no GP_PUT boundary is believed) | D3 | 3: `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_f...`, `d3_pointer_mode_batch_ends_announce_pops_by_an_event_with_no_look`, `unit:markers_need_a_known_parser_state_or_an_observed_gp_put_boundary` |
| `scan_mask_reset`: the sub-device mask state returns to its birth value at a fresh resume | D4 | 2: `d4_a_stored_sub_device_mask_is_still_stored_when_a_window_scans_its_use`, `user_scan::tests::a_fresh_resume_keeps_the_bindings_and_the_mask_state_and_forgets_only...` |

The reviewer's T1 mutation (`core_ignore_seed`) fails 6 tests; against the PHASE 1 pump the same edit left all 271 `kf-qemu` tests green. T2
(`life_forget_len0_sync`) fails `t2_*`. I8's mutant (`core_reads_beyond_put`, "reads one entry beyond GP_PUT") is caught by the stub-store event in every model
scenario (63 failing tests) and, from a different angle, by the unit test `a_turn_with_gp_put_at_the_cursor_loads_no_ring_entry_and_no_pushbuffer`, the props
`i8_*` tests and the thread stress. Removed with the transliteration: the seven model tests that asserted "the checker finds this glue defect"
(`mutant_*`), replaced by the table above; the harness knob `ignore_events` (a test of the explorer's teeth) stays with two tests (`i6_a_worker_that_ignores_events_*`,
`d3_the_same_run_deadlocks_if_the_worker_ignores_events`).

### 8b. PHASE 1c mutation run (2026-10-17, branch head `fb8aedd80`, `scripts/lifecycle_mutation_check.sh`, 57 mutants)

Baseline green. The 27 mutants of the 1b table are re-run (all still caught) and 30 new ones added (the exact mutants of the second review: **A** = `life_no_window_reanchor`, **W** = `pump_w_no_switch` and
`core_switch_no_post_scan`, **R** = `pump_r_object_is_the_client`, `pump_r_channel_handle`, `pump_r_no_registry`; N4 = `life_stale_boundary`; the modes, the latch, the ring-full guard, the tally, the pump wiring). The first column
is the mutation, then the number of failing tests (kf-chan: `--lib` and the five lifecycle binaries; `pump_*`: `kf-qemu --lib`), then the first three. `core_drop_flip_entry` and `life_stale_anchor` make `user_turn_unit`
loop forever; the hung binary was killed by hand, so their counts do not include its tests (a hang is a detection). Mutants A, W and R were all caught by 2 or more tests; **12 new mutants are caught by exactly ONE test**:
`life_anchor_ignores_full_ring`, `life_stale_start_uncounted`, `life_latch_counted_twice`, `life_open_notices_unbounded`, `core_switch_keeps_pending_get`, `core_new_ring_get_uncounted`, `core_write_back_unrecorded`,
`pump_w_no_mismatch` and the four pump wiring pins (`pump_no_tally_sync`, `pump_no_notices`, `pump_no_latch`, `pump_mode_ignored`: one source-text pin each, no GPU-free harness exists). They are debt (section 11), not justified.

| mutation | failing tests | first failing |
|---|---|---|
| `machine_pointer_scans` | 70 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 67 more |
| `machine_no_seed` | 9 | `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a_window_that_follows_a_forwarded_sync_wait_gates_its_first_entry_befo`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing` and 6 more |
| `machine_forget_sync` | 11 | `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a_window_that_follows_a_forwarded_sync_wait_gates_its_first_entry_befo`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing` and 8 more |
| `machine_pointer_to_scan` | 51 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 48 more |
| `machine_revive_after_teardown` | 4 | `a7_teardown_at_any_instant_stops_the_private_ring_for_good`, `a_torn_down_machine_loads_nothing_after_the_plan_and_publishes_nothing`, `d6_a_torn_down_machine_reads_nothing_and_publishes_nothing` and 1 more |
| `machine_exit_ignores_live` | 38 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 35 more |
| `machine_mask_frozen` | 2 | `a6_a_rebind_inside_a_window_is_seen_and_counted_and_the_new_binding_is`, `a_changed_software_binding_inside_a_window_is_counted` |
| `machine_anchor_ignored` | 15 | `a_second_window_of_the_same_channel_starts_at_its_own_anchor_never_ins`, `d1_a_registration_inside_a_submission_never_puts_a_host_marker_inside_`, `d2_data_that_looks_like_the_trigger_inside_a_packet_of_one_submission_` and 12 more |
| `machine_pointer_markers_anywhere` | 15 | `a_wake_pointer_mode_cannot_announce_is_counted_loudly_not_hidden`, `d3_a_credit_stall_inside_a_packet_never_gets_a_marker`, `i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look` and 12 more |
| `machine_pointer_no_markers` | 16 | `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late`, `a_pointer_marker_is_authored_where_the_cursor_reached_gp_put_and_not_m`, `a_wake_pointer_mode_cannot_announce_is_counted_loudly_not_hidden` and 13 more |
| `core_ignore_seed` | 7 | `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a_window_that_follows_a_forwarded_sync_wait_gates_its_first_entry_befo`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing` and 4 more |
| `core_no_gate_reset` | 3 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a_fresh_window_resets_the_gate_so_a_proof_from_an_earlier_lap_is_gone`, `t3_a_second_window_one_ring_lap_later_gates_its_sync_wait_entry_again` |
| `core_epoch_before_put` | 31 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 28 more |
| `core_double_fetch` | 4 | `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_tha`, `a_hostile_writer_rewrites_ring_entries_and_gp_put_while_the_worker_run`, `a_turn_loads_gp_put_once_then_the_entry_at_the_cursor_then_the_epoch_i` and 1 more |
| `core_drop_flip_entry` | 36 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 33 more |
| `core_skip_validation` | 5 | `a4_every_single_bit_flip_of_a_valid_entry_is_refused_by_name_or_forwar`, `a4_hostile_entries_never_panic_and_never_store_an_unvalidated_value`, `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_tha` and 2 more |
| `core_bound_without_datum` | 2 | `a2_the_binding_packet_split_across_two_entries_switches_to_pointer_aft`, `the_binding_is_known_only_after_the_datum_not_after_the_set_object_hea` |
| `core_reads_beyond_put` | 79 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 76 more |
| `core_pointer_gone_unchecked` | 2 | `d6_pointer_mode_refuses_a_freed_software_object_like_the_scan_path_and`, `pointer_mode_refuses_a_freed_software_object_and_publishes_nothing_for` |
| `core_carried_gate_removed` | 2 | `a_sync_wait_entry_behind_an_open_packet_is_read_at_once_and_counted_wi`, `obs_a_sync_wait_behind_an_open_packet_is_read_at_once_and_counted_exac` |
| `core_no_fresh_resume` | 2 | `a_fresh_window_forgets_a_packet_the_scan_left_open_before_pointer_mode`, `d2_a_packet_left_open_by_the_scan_before_pointer_mode_does_not_swallow` |
| `life_stale_anchor` | 55 | `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_la`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late` and 52 more |
| `life_no_post_switch` | 3 | `i4_no_entry_published_after_the_reply_is_forwarded_before_a_window_sca`, `switch_source_installs_the_new_ring_and_keeps_the_window_open_over_its`, `after_a_source_switch_the_window_stays_open_for_a_bounded_number_of_en` |
| `life_dead_scans` | 3 | `a_torn_down_machine_loads_nothing_after_the_plan_and_publishes_nothing`, `d6_a_torn_down_machine_reads_nothing_and_publishes_nothing`, `teardown_is_terminal_and_a_late_event_cannot_revive_the_channel` |
| `life_forget_len0_sync` | 4 | `t2_a1_with_a_length0_sync_wait_never_reads_the_late_pushbuffer_early`, `the_newest_pointer_sync_wait_is_recorded_by_monotone_index_across_ring`, `the_restart_is_requested_until_applied_and_the_seed_until_the_witness_` and 1 more |
| `life_no_boundary_tracking` | 12 | `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_late`, `a_pointer_marker_is_authored_where_the_cursor_reached_gp_put_and_not_m`, `d3_pointer_mode_batch_ends_announce_pops_by_an_event_with_no_look` and 9 more |
| `scan_mask_reset` | 2 | `d4_a_stored_sub_device_mask_is_still_stored_when_a_window_scans_its_us`, `a_fresh_resume_keeps_the_bindings_and_the_mask_state_and_forgets_only_` |
| `life_no_window_reanchor` | 3 | `a_second_window_of_the_same_channel_starts_at_its_own_anchor_never_ins`, `the_first_anchor_and_the_anchor_after_a_window_also_refuse_a_full_ring`, `the_second_window_of_a_channel_is_anchored_at_the_gp_put_of_the_turn_t` |
| `life_anchor_ignores_full_ring` | 1 | `the_first_anchor_and_the_anchor_after_a_window_also_refuse_a_full_ring` |
| `life_pointer_anchor_ignores_full_ring` | 2 | `the_guest_ring_is_full_when_gp_put_leaves_one_slot_before_the_gp_get_t`, `a_gp_put_read_while_the_guest_ring_is_full_is_never_an_anchor` |
| `life_stale_start_uncounted` | 1 | `a_gp_put_read_while_the_guest_ring_is_full_is_never_an_anchor` |
| `life_latch_local_ignored` | 4 | `a_packet_open_at_gp_put_in_scan_mode_is_reported_with_the_packet_and_s`, `l1_latched_markers_off_the_wake_is_a_look_and_without_looks_the_run_de`, `l1_once_an_open_boundary_was_seen_no_marker_ever_lands_in_a_packet_and` and 1 more |
| `life_latch_shared_ignored` | 2 | `latch_an_open_gp_put_boundary_in_one_channel_switches_pointer_markers_`, `the_latch_is_vm_wide_and_counted_once` |
| `life_open_boundary_not_latched` | 7 | `a_packet_open_at_gp_put_in_scan_mode_is_reported_with_the_packet_and_s`, `l1_latched_markers_off_the_wake_is_a_look_and_without_looks_the_run_de`, `l1_once_an_open_boundary_was_seen_no_marker_ever_lands_in_a_packet_and` and 4 more |
| `life_latch_counted_twice` | 1 | `the_first_packet_open_at_a_gp_put_boundary_switches_pointer_markers_of` |
| `life_open_notices_unbounded` | 1 | `put_boundary_open_notices_are_bounded_outside_a_window_and_unbounded_i` |
| `life_stale_boundary` | 3 | `the_switch_forgets_the_old_rings_gp_put_boundaries_so_no_marker_lands_`, `a_boundary_counted_before_the_switch_does_not_hide_the_new_rings_bound`, `a_remembered_gp_put_boundary_of_the_old_ring_is_forgotten_at_a_source_` |
| `life_mode_no_pointer_markers_ignored` | 8 | `l1_latched_markers_off_the_wake_is_a_look_and_without_looks_the_run_de`, `l1_once_an_open_boundary_was_seen_no_marker_ever_lands_in_a_packet_and`, `latch_an_open_gp_put_boundary_in_one_channel_switches_pointer_markers_` and 5 more |
| `life_mode_no_anchored_ignored` | 3 | `m1_no_anchored_windows_is_the_phase_1_start_and_puts_the_marker_inside`, `a_gp_put_read_while_the_guest_ring_is_full_is_never_an_anchor`, `no_anchored_windows_starts_the_window_at_the_entry_that_sees_the_regis` |
| `life_mode_always_scan_leaves_scan` | 11 | `corpus_a_gravitymark_ring_growth_replays_identically_through_the_base_`, `d4_a_stored_sub_device_mask_is_still_stored_when_a_window_scans_its_us`, `g1_pointer_mode_reads_no_pushbuffer_byte_over_a_long_stream_and_the_co` and 8 more |
| `core_switch_no_post_scan` | 3 | `i4_no_entry_published_after_the_reply_is_forwarded_before_a_window_sca`, `switch_source_installs_the_new_ring_and_keeps_the_window_open_over_its`, `the_switch_forgets_the_old_rings_gp_put_boundaries_so_no_marker_lands_` |
| `core_switch_keeps_cursor` | 11 | `a3_the_flip_boundary_is_the_cursor_at_the_entry_not_a_reread_of_gp_put`, `a_switch_outside_a_window_resets_the_cursor_but_scans_nothing_extra`, `b_mismatched_cursor_semantics_lose_entries_and_the_checker_sees_it` and 8 more |
| `core_switch_keeps_pending_get` | 1 | `switch_source_installs_the_new_ring_and_keeps_the_window_open_over_its` |
| `core_new_ring_get_uncounted` | 1 | `a_nonzero_guest_get_in_the_new_userd_contradicts_the_reset_and_is_what` |
| `core_ring_never_full` | 2 | `a_fresh_relay_sees_a_ring_filled_to_the_last_slot_as_full`, `the_guest_ring_is_full_when_gp_put_leaves_one_slot_before_the_gp_get_t` |
| `core_write_back_unrecorded` | 1 | `the_guest_ring_is_full_when_gp_put_leaves_one_slot_before_the_gp_get_t` |
| `scan_tally_first_only` | 2 | `m2_always_scan_counts_every_5080_set_object_and_every_rebind_of_the_wh`, `the_tally_counts_every_software_set_object_and_every_rebind_and_commit` |
| `scan_tally_no_rebind` | 2 | `m2_always_scan_counts_every_5080_set_object_and_every_rebind_of_the_wh`, `the_tally_counts_every_software_set_object_and_every_rebind_and_commit` |
| `pump_w_no_switch` | 2 | `migrate_switches_the_relay_through_switch_source_and_nothing_else_touc`, `the_switch_call_is_inside_the_admission_that_installs_the_new_source` |
| `pump_w_no_mismatch` | 1 | `migrate_switches_the_relay_through_switch_source_and_nothing_else_touc` |
| `pump_r_object_is_the_client` | 2 | `a_live_registration_of_the_channels_software_object_is_counted_and_not`, `every_registry_lookup_of_the_software_object_goes_through_software_key` |
| `pump_r_channel_handle` | 2 | `registry_live_counts_the_software_objects_registrations_for_the_channe`, `registry_live_never_asks_the_registry_about_the_channels_own_handle` |
| `pump_r_no_registry` | 2 | `a_live_registration_of_the_channels_software_object_is_counted_and_not`, `every_registry_lookup_of_the_software_object_goes_through_software_key` |
| `pump_no_tally_sync` | 1 | `the_pump_wires_the_marker_latch_the_notices_and_the_set_object_tally` |
| `pump_no_notices` | 1 | `the_pump_wires_the_marker_latch_the_notices_and_the_set_object_tally` |
| `pump_no_latch` | 1 | `the_pump_wires_the_marker_latch_the_notices_and_the_set_object_tally` |
| `pump_mode_ignored` | 1 | `the_lifecycle_mode_is_read_once_at_relay_creation_and_never_per_turn` |

## 9. Results (GPU-free, `cargo test --release`)

| crate | base (`0a7461173`) | PHASE 1 (`6f9163848`) | PHASE 1b | new in 1b |
|---|---|---|---|---|
| `kf-chan` lib | 142 | 166 | 174 | `user_lifecycle` +7 (anchor, boundaries, post-switch, Stop, the anchor property test), `user_loop` +1 (`loud_rate`), `user_scan` (mask test rewritten) |
| `kf-chan` tests | 150 | 150 + 63 (+3 ignored) | 150 + 90 (+3 ignored) | `user_lifecycle_model` 47 (+3 ignored OPEN), `user_lifecycle_props` 25, `user_lifecycle_stress` 2, **`user_turn_unit` 16** |
| `kf-rm` lib | 150 | 154 | 154 | |
| `kf-trap` lib | 80 | 86 | 86 | |
| `kf-qemu` lib | 270 | 271 | 271 | the pump now runs `kf_chan::user_turn`; the G3 structural pin still holds |

All green with `cargo test --release -p kf-chan -p kf-rm -p kf-trap -p kf-qemu` (1 561 tests, 0 failed, 3 ignored OPEN). **PHASE 1c: 1 603 passed, 0 failed, 3 ignored OPEN (+42 over 1b), measured at `fb8aedd80`**: kf-chan lib 188 (+14), `user_lifecycle_model` 57 + 3 ignored (+10), `user_lifecycle_props` 27 (+2), `user_lifecycle_stress` 2, `user_turn_unit` 23 (+7), kf-qemu lib 280 (+9), kf-rm 154 and kf-trap 86 unchanged.
tests in about 2 minutes (the Pointer-mode markers added states; three scenarios were shortened and one limit raised, see the git history of `i1_*`, `a3_*`).
Failing first (the base `6f9163848` with the new tests, commit `0e209d0c0`): `d1_*`, both `d2_*` and `d3_*` fail; `t2_*` passes on the base and is the test the
length-0 mutant must fail; `p2prime_*` documents the residual risk. Assertions CHANGED on purpose (the intended behaviour moved, nothing was loosened to hide a
failure): `a2_an_open_packet_carried_*` additionally expects `MARKER_IN_PACKET` (its submission ends inside a packet: P2' false); `i4_*` expects terminal mode Window
after a ring switch (D5); the sequential simulator is now a cooperative guest (entries marked `post` are published after the reply) and counts an unscanned entry
while a registration is live only for `post` entries (entries below the anchor are legitimately unscanned); `g2_p2_*` became `g2_p2prime_*` (identical to always-scan
when submissions end on packet boundaries; counted when they do not).

**Lazy observe (2026-10-17, branch `claude/lazy-prod-20261017`): 1 690 passed, 0 failed, 3 ignored OPEN (+87 over the p1c base `736c097d4`, 1 603), `cargo test --release -j2 -p kf-chan -p kf-rm -p kf-trap -p kf-qemu`.** Per target: kf-chan lib 188 -> 198 (+10: `user_lazy` unit tests 9, `look_needed` 1),
`user_lazy_turn` 27 (new), `user_lazy_model` 17 (new, doorbell-contract lineage), `user_lifecycle_model` 57 -> 63 (+3 ignored unchanged), `user_lifecycle_props` 27 -> 38, `user_lifecycle_stress` 2 -> 4, `user_loop_model` 37 -> 38, kf-trap `doorbell_contract` 5 (new), kf-qemu lib 280 -> 287 (+7 pins),
`doorbell_no_put_poll` 1 (new); kf-rm unchanged. The model file takes about 4 minutes (the child run of 33 scenarios with `KF_TEST_LAZY=1` is about 2 of them).

What is NOT covered (honest list): the staged trigger path and the publication of a SCANNED segment (`Pump::segment_turn`: still pump code; the tests drive a
stand-in); the pump's wiring of `PumpIo` (compiles, the 271 `kf-qemu` tests pass, G3 pin; no GPU-free harness for the real accessors); a real hold of the
registration reply (PHASE 2); inline vCPU copying; BAR1 read volume and every performance claim; any Linux or Windows guest; **P2' itself** (the Scan-mode
counters are the falsifier); the cursor semantics of the source switch; `kf_trap::inline_plan` (question 11).

## 10. Hardware report checklist (PHASE 1c; what to read after a run, and in which order)

**Added 2026-10-17: lazy observe has its own checklist (section 0c, "Hardware checklist") and the run order below gains its split arms; read the `USER_RING_LAZY_OBSERVE_MODE` line first. The sanity and must-be-0 lists below are unchanged and still apply with lazy observe ON.**

Read after every Windows run, from the `kf3: USER_RING_LIFECYCLE SUMMARY` line (device stop), the per-channel `USER_RING_LIFECYCLE` lines (relay release), the lines named in section 0b, the relay stats
line, the host `dmesg` and the guest System log. Record the revision of the binary (`kf3-bins/<rev>/`) and `USER_RING_LIFECYCLE_MODE` with the report.

**Must be 0:** `put_boundary_open_scan`, `put_boundary_open_window`, `window_fresh_start_parse_error`, `switch_cursor_mismatch`, `pointer_pb_read_violation`, `window_binding_changed`,
`after_teardown`, `anchor_stale_start`, `pointer_markers_latched_off`. **Must be absent:** `USER_RING_REFUSED`, `USER_RING_POINTER_MARKERS_LATCHED_OFF`, `USER_RING_PUT_BOUNDARY_OPEN`,
`USER_RING_SOFTWARE_OBJECT_GONE` outside teardown, host NVRM Xid 13 / 31 / 32 / 43, guest `nvlddmkm` event 153, `Display` 4101.

**Sanity (these must hold; a miss is a bug in the counters or the relay):**
* `to_window_fresh == restarts` (every Fresh window restarted the parser once); `seeded_gates <= to_window_fresh`.
* `anchor_forwarded` is the number of entries forwarded unscanned between the registration and the anchor (the `anchor_forwarded` field of `USER_RING_WINDOW_FRESH`); `USER_RING_WINDOW_FRESH` lines == `to_window_fresh`.
* **`post_switch_scanned == 8 * (number of USER_RING_MIGRATED lines)`** whenever at least 8 entries followed each switch (mutant W: the call that makes it true is `Relay::switch_source`; a missing +8 means the call did not run). Holds in `always_scan` too.
* `entries_pointer` much larger than `entries_window` in the default mode; in `always_scan` `entries_pointer == 0` and `entries_scan` is every entry.
* `set_object_5080_max_per_channel == 1` and `set_object_5080_rebinds == 0` in an `always_scan` run is the evidence for assuming P1; any other value is a finding.
* `anchor_refused_ring_full` is information (how often the guest ring was full at a `GP_PUT`); it explains a nonzero `anchor_stale_start`.

**D3 performance (default mode; the fix of review D3 is only worth keeping if these stay quiet):** `pointer_markers` about the number of doorbell batches; `pointer_carried_no_marker`, `pointer_backstop` and
`pointer_backstop_loud` about 0 (a rising `pointer_carried_no_marker` with the latch set is the expected cost of the latch); relay `credit_waits` and `backstop_carried` low; fps against the S3 189 / 338 reference.

**Run order (one app at a time, each with the counters above):**
1. Heaven D3D11 and Heaven OpenGL with everything on (`default`): the S3 scratch build's mode 2 ran both without Xid, which is the only evidence P2' holds for any workload.
2. Minecraft, Edge, GravityMark each FIRST in `KF3_LIFECYCLE_MODE=always_scan` (the machine never leaves Scan; `put_boundary_open_scan` is measured over the whole run, `put_boundaries_scan` is the denominator).
3. Enable Pointer markers (the default mode) for an app only if its `put_boundary_open_scan` was 0 over a full `always_scan` run; keep anchored Windows only if in addition `window_fresh_start_parse_error == 0`
   and every ring growth passed without Xid. If an app shows a nonzero value: run it in `no_pointer_markers` (markers off, anchors on) or `always_scan` and record the open packets from `USER_RING_PUT_BOUNDARY_OPEN`.

## 11. Debt left by 1c

**Added 2026-10-17 (lazy observe):**
* `KF3_PUT_CACHE` / `KF3_LAZY_OBSERVE` are measurement switches (default ON). After the split arms (none / A / B / A+B) are measured on the production relay, hardwire ON, delete both flags and the `LazyConfig` switch parsing, and make `Relay::new` ON (today it is OFF so that every older test keeps its meaning; the pump opts in exactly once, pinned).
* The effective interval `min(16, E / 2)` and the D6 guard are reasoning plus GPU-free tests; whether `obs_forced_ring_full` matters for the real Windows guest ring is a hardware question.
* The identity `put_cache_hits + put_fresh_reads <= turns` and the per-reason look counters exist only in `relay_stats.lazy` and the per-relay retire line; the periodic lifecycle tick does not print them.
* `docs/RELEASE_CLEANUP_LEDGER.md` is not on this base; the first two bullets belong there once it is merged.

* `docs/RELEASE_CLEANUP_LEDGER.md` is not on this base (it is on `claude/release-ledger-20261016`); the entries below belong there once merged.
* `KF3_LIFECYCLE_MODE` is a measurement flag with four values. Once P2' is measured on the app matrix, the choice of the shipped behaviour should be hardwired (no flag) and the unused modes deleted; the VM-wide latch stays as the runtime guard.
* The ring-full anchor refusal can leave a Window starting at an entry that is not a known `GP_PUT` boundary (`anchor_stale_start`). A better fallback (for example delaying the Window to the first boundary the scan returns to) is open; it needs the hardware counters first.
* Pump wiring is covered by structural pins (source-text tests) and by the production code moved into `kf-chan` (`switch_source`, `new_ring_get`, `software_key`, `live_registrations`); there is still no GPU-free harness for `PumpIo` and `segment_turn`. The pins are single-angle for `pump_no_tally_sync`, `pump_no_notices`, `pump_no_latch` and `pump_mode_ignored` (see the 1c mutation table).
* The latch needs an observation in Scan or a Window; a guest that only ends submissions inside packets while the channel is in Pointer mode is not detected until `always_scan` is run.

## 12. Resume here (owner pause, 2026-10-17)

State: all four must-fix items of the 1c brief are implemented and pushed on `claude/win-lifecycle-p1c-20261016`; nothing ran on hardware.

| Item | Status |
|---|---|
| 1 modes (`KF3_LIFECYCLE_MODE`) | done (`no_anchored_windows` included) |
| 2 log lines + guard | done: three log lines, ring-full anchor guard, VM-wide latch, the P1 SET_OBJECT tally (coordinator addition) |
| 3 mutants A, W, R | done: each caught by 2 or more tests; the new pump wiring is still pinned by single source-text tests (section 8b) |
| 4 stale boundary + tautology | done |

**Lazy observe (2026-10-17) leaves, in order:** (A) Linux gates on the host, unchanged expectations: `scripts/bench/v3_gates.sh` (9/9), `scripts/bench/build_kf3.sh`, rebuild the raw client and the fast guest, `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180` (30/30); there must be no lazy line in a Linux-only run.
(B) Windows Heaven on the production profile with the new binary (`kf3-bins/<rev>/`): `REV=<rev> APPS="heaven_d3d11_win heaven_gl_win" bash scripts/bench/windows/regress_matrix.sh` (it prints the relay counters per app at guest reboot; read the `USER_RING_LAZY_OBSERVE` lines of the QEMU stderr as well), then the split arms from the SAME binary by setting
`KF3_PUT_CACHE=0` (B only), `KF3_LAZY_OBSERVE=0` (A only) and both (the p1c lifecycle alone, expected about 196 fps D3D11) in the launch environment (how `launch-2990.sh` forwards variables was not read here; confirm with the `USER_RING_LAZY_OBSERVE_MODE` line). Section 0c lists the counter relationships and the fps falsifier (D3D11 about 297 expected, about 220 or less = the port lost the gain).

Next, in order: (1) second tests for the 12 single-catch mutants of section 8b; (2) the Linux gates (`v3_gates.sh` 9/9, `fast_suite.sh` 30/30) on the host, which this work never touched; (3) the Windows run order of section 10, Heaven first; (4) merge `claude/release-ledger-20261016` and move section 11 into the ledger.
