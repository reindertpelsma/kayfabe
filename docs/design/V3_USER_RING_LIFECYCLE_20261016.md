**STATUS: LIVE (PHASE 1, worker-only, GPU-free), 2026-10-16** (branch `claude/win-lifecycle-20261016`, from
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
| Pointer | registry epoch moved and >= 1 live registration (`Registration`) | Window(Fresh) |
| Window | 0 live registrations (trigger executed + implicit delete, `REMOVE_API`, object freed) and the relay is quiescent (no gate draining, no segment in flight) (`RegistryDrained`) | Pointer |
| any | teardown | Dead (absorbs every later event; a delayed callback cannot revive a freed channel) |

`next(mode, cause)` is the table; the machine uses only it (I5). There is no Pointer -> Scan, no Window -> Scan, no timer edge, and no edge that depends
on a count of entries (OWNER_RULINGS §AE). "Live registration" means ANY live entry of the channel's `5080` object in the deferred-API registry,
whatever its bundle kind, executed or not (Open questions 8 and 9: deliberately wider than "a pending `FIFO_UPDATE_CHANNEL_INFO`").
`quiescent` is conservative (leaving a Window while a gate drains would be safe, because a Fresh restart resets the gate), kept for simplicity.

### 1.1 The flip: how a registration reaches the worker, and the "held reply"

The registration is served on the RPC drainer (`kf_rm::defapi::Registry::register`), which must never block. **Phase-1 mechanism:** the registry carries
a monotone `epoch` (`AtomicU64`) bumped (acq-rel) AFTER every change of the set of live entries, while the state lock is still held, and so BEFORE the
registration's reply is built. The worker loads the 8-byte GP entry (ONE load), then a SeqCst fence, then the epoch; only if the epoch changed does it
take the registry lock to count the live entries (`Registry::live_count`). Consequences, checked by the model (I4):

* an entry loaded BEFORE the epoch load that sees the bump is the first Window entry (it is scanned); every entry already forwarded in Pointer mode was
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
packet boundary: Open question 2. **[measured, S1]** 1.28 % of Heaven's GP entries carry SYNC_WAIT (11.8 per frame), so in practice every Fresh window is seeded.

## 2. Pointer-mode entry handling (the only check is the entry)

Per entry: ONE 8-byte load; `inspect_entry` (length cap, privileged/indirect/reserved bits, 2^40 span) on THAT value; a credit check; publish the SAME
value (`write_user_gp`) behind the guest-ring and GP source guards (no pushbuffer guard exists because none was read); advance the cursor; record SYNC_WAIT.
No `Poll`, no tick: the doorbell contract is unchanged. Markers need a packet boundary, so Pointer mode authors none: the guest `GP_GET` write-back and
private-ring credit waits are announced by the spin, any engine event and the counted 50 ms backstop (Open question 1). One guest entry is published per
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
`scan_no_software`, `after_teardown`. Existing counters keep their meaning (`carried_ungated` now counts only Scan/Window entries).

## 5. Assumptions and what the existing traces say about them

* **P1 (the `5080` SET_OBJECT occurs once at setup).** Not measurable in Pointer mode by construction. Falsifiable in a Window (`window_binding_changed`).
  Existing data neither supports nor contradicts it beyond the short Scan phases: **[measured, S1]** 5 299 pushbuffer reads before the binding over 71 channels
  of a Heaven run.
* **P2 (the driver ends GP segments on packet boundaries, so a Window may start with fresh parser state at any entry boundary).** **CONTRADICTED as a general property;
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

* Fresh start: bindings are kept (the software subchannel is not re-learned), the packet in progress is forgotten, the sub-device mask state returns to its birth value
  (`active`, nothing stored; unscanned `SET/STORE/USE_SUBDEVICE_MASK` cannot be seen: Open question 10), `SyncGate` is reset, `Grace` is cleared.
* Refusal in a Window: exactly today's scanner refusal (`Grace`, then `USER_RING_REFUSED`, channel dead). Never a recovery.
* VRAM-backed pushbuffers in a Window use the existing path and are counted (`window_bar1_bytes`). PHASE 3 replaces bulk MMIO reads in windows.
* A Window with no trigger never ends by itself and no timer ends it; it is counted (`window_entries_max`) and every other channel is served (one entry per turn).
* Source switch: the existing `migrate` path (`USER_RING_MIGRATED`): cursor 0 on the new ring, new USERD, generation + 1, private native GET untouched. Cursor semantics
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
10. **Sub-device mask state after an unscanned stretch** is reset to its birth value.

## 8. Mutation checks

`scripts/lifecycle_mutation_check.sh` re-runs the whole GPU-free suite (`--features lifecycle-mutants`, off by default and never enabled by another crate) once per
deliberate defect and lists the failing tests. Machine mutants are `kf_chan::user_lifecycle::Mutant` (`KF_LIFECYCLE_MUTANT`), glue mutants are defects of the test
transliteration of the turn (`KF_LIFECYCLE_GLUE_MUTANT`; the transliteration in `tests/lifecycle_support/mod.rs` uses the REAL `Lifecycle`, `SyncGate`, `Marks`, `Grace`,
`Scan`, `Fetch`, `inspect_entry`, but the pump's I/O is abstracted: a mapping by line is in its comments).

MUTATION_TABLE_PLACEHOLDER

## 9. Results (GPU-free, `cargo test --release`)

RESULTS_PLACEHOLDER
