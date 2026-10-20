# V3 — the FOUND-IN-PRODUCTION ledger: bugs a production-like workload found, and the tests they became

**STATUS: LIVE, 2026-10-20.** Branch `claude/found-in-production-20261020` (from `claude/relay-fairness-20261020`).
Labels: **MEASURED** = a run, a counter or a test result named here; **CODE READ** = kayfabe code or a commit read while writing this
page; **INFERRED** = reasoning, never evidence. Test names were grepped on the branch named in each entry (base branch unless stated).

## 0. Why this ledger exists (owner idea, 2026-10-20)

Hunting bugs with a production-like workload (two plain GPU clients on the relay route, a Windows guest doing sign-in, Edge, Minecraft)
and turning each one into a unit or model test yields tests nobody would have designed up front. It is a family distinct from
standard fuzzing: the workload finds the *property* that was never stated, and the test then states it for every future change.
Today's example (entry A): an exhaustive model had proved LIVENESS ("every covered entry is eventually served"), and could not see
a client served after 59 s. Only the new BOUNDED-FAIRNESS property saw it.

One entry per bug. Each entry: date, symptom, how it was found, the minimal schedule in plain English, the tests that now pin it,
the PROPERTY CLASS it introduced, status, and the lesson for review. §2 turns the classes into a checklist for anyone adding a wake
source, a protocol or a lock. §3 is the recipe for the next entry.

## 1. The entries

### A. The worker loop starved every wake but the doorbell ("the baton") — 2026-10-20

- **Symptom (MEASURED, owner's hardware runs).** On the relay route (Linux guest with `KF3_DEBUG_FORCE_MIGRATABLE=1`; the Windows guest shows
  the same signature) about one plain two-client run in three starves ONE client for 6-59 s while the other completes thousands of iterations.
  GPU 94-97 %, no Xid, `KF3_DIAG_USER_STALL` never fired, `backstop_found_*` stay 0; an `strace`-wrapped run is clean (timing); the
  Passthrough route 0 of ~16 (`V3_USER_RING_EVENT_LOOP_20261014.md` section 10).
- **Found by.** The workload (two plain clients, relay route, owner's runs). The probes in `scripts/bench/baton/` (branch
  `claude/baton-debug-20261020`, head `993a71499`: `baton.c`, `stallprobe.c`, `qsampler.py`, `wd_parse.py`) were built to measure it. The
  cause was found by CODE READ and a new model property (P11-P13); the old model checked only that a wait is eventually woken.
- **Minimal schedule (plain English; CODE READ + MEASURED in the model).** One worker, one hot relay that ends every turn `Need::Again` and
  rings its own token. The cold relay's marker interrupt fires. The worker scans, claims the hot token, serves it up to 9 times,
  republishes it RUNG, and the next scan claims it again, for ever. `run_with_relay` looked at the world (poll(0): engine events,
  completions, doorbell eventfd; `relay.peek`) only when a pass served NOTHING, so the event is never dispatched and the wait never peeked.
  With two workers the hot token is handed from one to the other (the baton) and each is idle only by timing: hence "1 in 3", hence clean
  under `strace`.
- **Tests that pin it.** `crates/kf-chan/tests/relay_fairness.rs`: `p11_p12_one_worker_one_hot_relay_the_cold_wait_and_the_engine_event_are_attended_within_k_passes`,
  `p13_one_worker_one_hot_relay_a_carried_wait_is_found_within_k_passes_without_the_safety_net`, `p13_one_worker_a_lost_try_lock_is_re_rung_within_k_passes`,
  `p11_two_workers_one_hot_relay_the_baton_does_not_starve_the_cold_wait`, `p12_two_workers_one_hot_relay_the_baton_does_not_delay_the_engine_event`,
  the control `control_a_flagged_cold_token_is_claimed_within_two_passes`, the seven `mutant_*` tests, and the source pins `pin_the_busy_look_*`,
  `pin_a_busy_pass_asks_the_policy`. `crates/kf-chan/src/worker.rs`: `a_busy_worker_looks_every_look_every_passes_and_an_empty_pass_idles`.
  The structural guard (this branch): `crates/kf-chan/tests/wake_registry.rs` (§2, "wake sources").
- **Property class introduced.** BOUNDED FAIRNESS / no starvation under legal adversarial schedules (class 1, §2).
- **Status.** Fix `worker::LoopPolicy::after_pass` (a look every `LOOK_EVERY = 4` busy passes), commit `cfdcaf73b`, branch
  `origin/claude/relay-fairness-20261020` (head `d1fc50c4a`), not merged. GPU-free tested. Hardware A/B is in progress: **cause pending hardware
  confirmation**. Falsifier (before the run): a starvation that persists with `busy_looks` > 0.
- **Lesson for review.** A loop that serves work before it looks at anything else needs a bound on how long it may serve. "Eventually" is
  not a requirement a user can feel. When reviewing any loop, list every wake source it visits and ask, for each, "what is the longest a
  neighbour can keep me from visiting it?".

### B. The VA-refresh (MMU invalidate) protocol: stale clear, lost specific refresh, unbounded backlog — 2026-10-20

- **Symptom.** None observed as a failure on hardware (INFERRED link, not shown): the owner's requirement of 2026-10-20 was that refreshes
  coalesce like doorbells, after the Windows stalls and the 2026-10-09 hang (entry E4) made the invalidate path the suspect. The defects
  were found before they were seen.
- **Found by.** An exhaustive interleaving checker over the REAL `InvalidatePort` and `VaManager` (guest Prep/Arm/Sig, manager Drain/Observe/Ready,
  GPU finish, every order, a guest that may arm specific or ALL_PDB at any time), with an oracle independent of the code. Tests-first commit
  `a4c1d56b8` (EXPECTED RED), fix `372435ac1`, mutation table `5c1a864d2`, branch `origin/claude/va-refresh-interleavings-20261020`.
- **Minimal schedules (MEASURED by the checker).** (I1, stale clear, 8 events) the guest writes the PDB latch for its NEXT invalidate before
  the manager rebuilds the armed request from the latches; the earlier arm is redirected to another space, the walk of that space finishes, and
  the earlier trigger is cleared though its own space was never walked. (I1/I4, lost specific) arms A, A, B are all armed before the manager
  looks; the trigger names only the newest, so the earlier specific arms are never refreshed. (I3, backlog) 200 arms during one walk leave
  200 pending wants.
- **Tests that pin it** (branch above). `crates/kf-mem/src/vasmgr/tests/interleave.rs`: `adversarial_guest_n3_no_stale_clear_and_no_lost_arm`,
  `adversarial_guest_n3_walks_never_exceed_arms_and_progress_needs_no_timer`, `adversarial_guest_n4_two_spaces_every_invariant`,
  `polite_guest_n4_every_invariant_over_every_interleaving`, `a_later_pdb_latch_must_not_redirect_an_earlier_arm`,
  `an_unobserved_specific_arm_is_not_lost_when_a_later_arm_overwrites_it`, `a_storm_of_arms_during_one_walk_leaves_one_bounded_follow_up`,
  `real_threads_hostile_arm_storms_lose_no_pte_and_build_no_backlog`; `crates/kf-trap/src/mmuinval.rs`: `more_unobserved_arms_than_the_ring_keeps_set_the_overflow_flag`,
  `an_arm_without_a_readable_record_is_treated_as_all_pdb`, `a_reader_never_sees_an_arm_before_its_record`. Its own header carries the 17-mutant table (>= 2 killers each).
- **Property classes introduced.** STALE-DATA CLEAR (class 2) and COALESCING WITHOUT LOSS (class 3, §2).
- **Status.** Fixed in the branch (every arm's scope is recorded in its own trap and read back by sequence; refreshes merge, bounded by the number of
  spaces; an unreadable record is answered by ALL_PDB). Not merged. Hardware: not run (CODE READ + checker only).
- **Lesson for review.** A completion (here: clearing a trigger bit) may only retire an arm that PRECEDED the work that produced it. If the "which
  arm is this for?" answer is rebuilt later from state the other side may be rewriting (latches), it is wrong under some interleaving. Record the
  scope with the arm, in the same atomic step that publishes the arm.

### C. A legal first submission on a kernel copy-engine ring was refused (`NoCeObject`) — 2026-10-19

- **Symptom (MEASURED, Windows run 3017).** Token `0x806` (channel `0xc1d00019:0xff040004`, COPY2, engine `0xb`): GP entry 34 carried
  `SET_SEMAPHORE_A/B/PAYLOAD` + `LAUNCH_DMA` on subchannel 4 with no `SET_OBJECT` before it, was refused `NoCeObject { subch: 4, method: 576 }`
  and the channel was declared DEAD (refused-and-poisoned) during the guest's TDR recovery.
- **Found by.** The Windows 11 workload (run 3017, the guest's TDR recovery) and kayfabe's refusal log; the same class as entry E5.
- **Minimal schedule.** A guest-kernel CE ring's very first method after birth is a CE method; hardware subchannels 0-4 are fixed to the channel's
  engine and no engine requires `SET_OBJECT`; the T-mode decoder required a guest-supplied binding and refused.
- **Tests that pin it** (branch `origin/claude/ce2-kernel-ring-20261019`, head `95e23df77`). `crates/kf-chan/src/tmode_ce_tests.rs`:
  `the_measured_run3017_segment_is_decoded_against_the_rings_own_ce_class`, `an_unconfigured_ring_refuses_the_measured_segment_as_before`,
  `the_measured_segment_binds_through_the_windows_and_emits_no_guest_va`, `a_ce_object_that_is_not_a_known_copy_class_gives_no_leniency`,
  `software_subchannels_gain_nothing`, `a_guest_set_object_still_binds_and_a_foreign_class_is_still_refused`,
  `the_guests_binding_wins_over_the_derived_class_when_newer`, `the_host_semaphore_width_follows_the_derived_class`, the census tests;
  `crates/kf-chan/src/ce_implicit.rs`: `a_kernel_copy_engine_ring_gets_its_host_ce_class`, `a_user_tier_channel_never_gains_it`,
  `a_non_copy_engine_channel_never_gains_it`, `no_class_reaches_a_user_tier_or_non_copy_engine_channel`.
- **Property class introduced.** DERIVE-NEVER-CAPTURE FOR THE LEGALITY OF A FIRST SUBMISSION (class 4, §2).
- **Status.** Fixed in the branch (`4085b0851`: the class comes from the ring's own host channel, never from guest bytes, never authored as a
  bind; a guest `SET_OBJECT` still wins; hostile cases unchanged). Not merged. Hardware effect on the TDR recovery is INFERRED, not measured.
- **Lesson for review.** For every decoder that refuses "before it was told X", ask whether the hardware needs to be told X. If not, the
  refusal is a bug that only a real driver's first submission will find. The legal set of first submissions is a property of the HARDWARE,
  and the host already holds the answer (its own channel).

### D. Host memory compaction multiplied VM exits (kcompactd0 TLB-shootdown storm) — 2026-10-19

- **This is HOST CONFIGURATION, not code.** No kayfabe change fixes it, and no unit test can pin it.
- **Symptom.** A guest stall of about 10 s with `kcompactd0` busy on the host and about 540k VM exits/s (figures as given by the coordinator's
  brief for this page; not re-measured here).
- **Evidence.** Host path `/data/claude-stall10s-20261019` (on the host; **not readable from the dev box and not read while writing this page**).
- **Found by.** A production-like run's stall capture (the evidence directory is named for a 10 s stall); the per-thread host view (kcompactd0 busy,
  the exit rate) is from the coordinator's brief. Which tool produced which number is not recorded in the repo.
- **Mechanism (INFERRED, Linux behaviour, not shown from the evidence).** Compaction migrates pages; every migration of a page that a
  nested/KVM guest maps flushes TLB entries on the CPUs that run vCPUs (IPI shootdowns), and each shootdown that hits a vCPU is a VM exit. The
  repo's own bench README already says an exit costs 10-40x bare metal on a nested vast box (`CLAUDE.md`, *Bench traps*).
- **Knobs (INFERRED: the standard Linux ones; the values in force and the values applied are in the evidence directory and were not read here,
  so fill them in when the host path is read).** `vm.compaction_proactiveness`, `vm.extfrag_threshold`, `vm.compact_unevictable_allowed`,
  `vm.watermark_scale_factor`, `vm.min_free_kbytes`, and `/sys/kernel/mm/transparent_hugepage/{enabled,defrag}` (THP defrag is the usual
  trigger of direct and background compaction on a host that backs a large guest).
- **Tests that pin it.** None (host state is outside the repo). A host preflight that records these sysctls in every bench evidence directory and
  fails a lane on a nonzero `kcompactd` CPU share during the lane is the guard; PROPOSED, not built.
- **Property class introduced.** HOST CONFIGURATION THAT MULTIPLIES EXITS (class 5, §2).
- **Status.** Host configuration. Fix applied/confirmed: not recorded in the repo: the owner or coordinator records the sysctl values here.
- **Lesson for review.** A stall with no guest-visible cause and no Xid needs the HOST's per-thread CPU and exit counters before any code
  hypothesis. "Nothing in kayfabe changed" is itself a finding.

### E. Earlier ones, documented elsewhere in the repo

- **E1. `SyncGate::cleared` is never forgotten (2026-10-14).** CODE READ + MEASURED (`V3_USER_RING_EVENT_LOOP_20261014.md` §5.1). After one ring lap
  the same `(generation, cursor)` carried a NEW SYNC_WAIT entry and was waved through, so the relay read a pushbuffer the hardware had not fetched.
  The model reaches the stale read in 75 steps. Found by the tests-first model (`95a9ad422`), fixed on master. Tests (`crates/kf-chan/tests/`):
  `user_gate_rules.rs::a_sync_wait_entry_one_ring_lap_later_is_gated_again`, `user_gate_rules.rs::the_same_slot_gated_twice_a_lap_apart_authors_two_barriers`,
  `user_gate_rules.rs::a_retried_turn_at_the_same_gated_entry_publishes_one_barrier` (the retry guarantee that must not break),
  `user_loop_model.rs::design_a_sync_wait_in_the_same_slot_one_lap_later_is_not_read_early`. Class: STALE-DATA (class 2): memory of a decision must end with its premise.
  Lesson: a "done" latch keyed by position is wrong once positions repeat; ask what makes the key unique again.
- **E2. `Grace` finality was a function of the wall clock (2026-10-14).** CODE READ + MEASURED (§5.2). `Grace::refused` returned "retry" until 50 ms had
  passed; with the clock frozen a refusal never became final. Fixed: finality counts re-reads (`REFUSE_RETRIES`), never time. Test:
  `user_gate_rules.rs::a_refusal_becomes_final_after_a_bounded_number_of_rereads_without_any_clock`. Class: NO CORRECTNESS BY CLOCK (a model with no timer
  is a proof; see §2 "also seen"). Lesson: if a test needs `sleep` to make a rule fire, the rule depends on time.
- **E3. `GP_GET` runs ahead of processing (2026-10-14).** MEASURED on bare metal (RTX 4070 / 595.91.07; §3.3): with an acquire stalling entry 0,
  `GP_GET` already covers all entries, the segment behind the SYNC_WAIT barrier has not executed and `USERD.REFERENCE` is 0. The first gate
  ("`GP_GET` past the witness") was unsound: `kf-syncwait-stress --families barrier` made the relay refuse 49 of 60 late-bytes plans.
  Failing-first commit `b07aecc9c`; fixed by the tagged marker (proof = a processed effect, `USERD.REFERENCE`). Tests:
  `user_loop_model.rs::the_retired_gp_get_proof_read_a_pushbuffer_before_the_drain`, `user_loop_model.rs::design_a_sync_wait_behind_an_open_packet_cannot_protect_late_bytes`
  (the known limit, pinned), `user_loop_model.rs::design_credit_waits_with_a_shallow_pop_window_are_event_driven`. Class: A PROOF MUST BE A PROCESSED
  EFFECT, NOT A POINTER (the model's hardware must be able to run ahead). Lesson: put the hardware's measured reordering in the model BEFORE designing on it.
- **E4. The uncleared `MMU_INVALIDATE` guest hang (2026-10-09).** MEASURED from a host memory dump of the hung Windows run 223 and kayfabe's logs
  (qemu.log `mem[inval=N cleared=N-1 unreconciled=1]` in runs 221, 190, 181, 162, 223). One vCPU polled BAR0 `0xB830B0` bit 31 (`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE`) forever while
  holding a dxgkrnl resource shared x13; every other thread queued behind dxgkrnl locks and Windows raised 0x117. kf-mem refused one unmappable range and,
  by the 2026-09-25 design, did not clear. Owner ruling 2026-10-09 (`OWNER_RULINGS.md` §AA): an invalidate completes over ABSENCE (hardware has no error channel); a refused UNMAP still holds
  the clear. Fix `003f861e0` is on master. Tests (`crates/kf-mem/src/vasmgr.rs`): `clear_over_absence_completes_the_invalidate_by_default`,
  `a_refused_map_completes_the_invalidate_and_is_retried_by_the_next_one`, `a_refused_unmap_still_holds_the_clear_by_default`, `clear_over_absence_can_be_switched_off`.
  Class: A REFUSAL BY DESIGN MUST NOT BECOME AN UNBOUNDED WAIT FOR THE OTHER SIDE (the other side has no error channel). Lesson: for every "we refuse" ask what the
  guest does while it waits, and what locks it holds. Hardware re-confirmation of this entry alone is not recorded in the repo.
- **E5. NVDEC kernel channel refused as a copy-engine channel (2026-10-14).** MEASURED: Win11 Edge/YouTube TDR, run 2990 `0c5fedff`, kernel NVDEC channel
  `0xc1d00018:0xff040003`, `NoCeObject { subch: 4, method: 576 }` (the video runlist has no copy engine; subchannels 0-4 reach the channel's one video engine).
  Fix `1c503f87e` (on master): a VIDEO tier of the T-mode rewriter whose method space is DERIVED from NVIDIA's public class headers
  (`crates/kf-chan/src/video_generated.rs`, `vtables.rs`), disposition keyed by method NAME. Tests: `crates/kf-chan/src/vtables.rs::the_ada_decoder_is_derived_with_its_measured_semaphore_layout`,
  `crates/kf-chan/src/vtables.rs::every_derived_video_class_has_the_semaphore_rows_the_tier_needs`,
  `crates/kf-chan/src/tmode_video_tests.rs::the_measured_nvdec_segment_is_admitted_and_bound_through_the_windows`, `…::hostile_video_words_are_refused_by_name`,
  `…::a_video_semaphore_outside_the_vms_rows_is_refused_at_bind`. Class: derive-never-capture for the legality of a first submission (class 4); entry C is the second case of it.
  Lesson: after the SECOND such bug, sweep every engine family for the same assumption instead of waiting for a third.

## 2. PROPERTY CLASSES TO CHECK WHEN ADDING A WAKE SOURCE, A PROTOCOL OR A LOCK

1. **Bounded fairness / no starvation under legal adversarial schedules.** Liveness ("eventually served") is not enough. State a bound in
   passes or events, give the adversary every legal move (a neighbour that re-rings itself, a worker that hands a token to its peer, a lost
   `try_lock`, an OS that schedules unfairly but not never), and check the bound. *Wake sources:* every source the idle path visits must also be
   visited by the busy path, on the same look. The guard is `worker::WAKE_SOURCES` and `crates/kf-chan/tests/wake_registry.rs`: it fails if
   `dispatch` handles a tag class the registry does not list, if a peek reason or an `on_*` callback is unaccounted for, if the busy `Look` arm
   and the idle spin make different calls, and the property test fails if `LoopPolicy` goes more than `LOOK_EVERY` busy passes without a look.
   A *lock* adds the same question: what is the longest a holder can keep a waiter out, and who re-rings the waiter on release?
2. **Stale-data clear.** A completion may only retire an arm that PRECEDED the work that produced it. Record the arm's scope with the arm
   (same atomic step), never rebuild it later from state the other side can rewrite; forget a decision when its premise ends (a position key that
   repeats after a lap).
3. **Coalescing without loss.** Merging waiting work must keep a bound that does not depend on the number of arrivals, and must never drop a scope:
   when the record of a request is gone, answer with the SUPERSET (ALL_PDB), never with nothing.
4. **Derive-never-capture for the legality of a first submission.** What a guest may legally send first (no `SET_OBJECT`, a semaphore on subchannel 4
   of a video runlist) is a property of the hardware and of the host's own channel. A decoder that refuses "before it was told X" must be
   checked against what the hardware needs. Sweep all families when one is found (all families are first-class).
5. **Host configuration that multiplies exits.** Compaction, THP defrag, KSM, NUMA balancing, frequency scaling and anything that
   shoots down TLBs or takes a lock the vCPU threads take turns a rare event into a stall. Record the host's sysctls and its per-thread CPU
   in every evidence directory; check the host before the code.

Also seen (ledger E): **no correctness by clock** (a finality that waits for wall time is not final in a model with no timers); **a proof must be a
processed effect, not a pointer** (model hardware that can run ahead); **a refusal must not become an unbounded wait** for a side with no error channel.

## 3. How to add a ledger entry

1. **Workload.** Name the production-like run that showed the symptom (guest, route, flags, revision, box). Keep the log path.
2. **Counter / probe.** The cheapest measurement that tells the candidate causes apart (a counter that is 0 when the hypothesis is false, a thread
   sampler, a stall probe, a start marker and an exit line). State the falsifier BEFORE the run.
3. **Minimal schedule.** Reduce the trace to the shortest schedule in plain English (who does what, in what order, what is never looked at). If the
   model cannot reach it, the model is missing a move: add the move first.
4. **Failing test.** Write the test of the PROPERTY (not only of the instance), commit it alone with the message `EXPECTED RED`, then the fix. Add
   a source pin when the test drives a transliteration of the production code. Mutation-check the fix: each mutant killed by >= 2 tests.
5. **Fix.** Smallest change that restores the property; production behaviour otherwise byte-identical.
6. **Hardware falsifier.** Say what result on hardware would prove the fix wrong (the counter that must move, the stall that must not appear) and write
   `cause pending hardware confirmation` until a run says so, with the revision, box and evidence path.
7. **Ledger.** Add the entry here (date, symptom, how found, schedule, tests with file and name verified by `grep`, property class, status, lesson), add
   the class to §2 if it is new, and keep MEASURED / CODE READ / INFERRED apart.
