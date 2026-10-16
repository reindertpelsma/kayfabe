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

Within a turn the relay reads NO ring entry at an index at or beyond the `GP_PUT` it loaded in that turn (`GP_PUT` is loaded ONCE per turn), and no pushbuffer
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
| I8 | (1b) nothing at or beyond the `GP_PUT` loaded in the turn is read; `GP_PUT` is loaded once per turn; a pushbuffer only for the entry loaded; Window anchors are only `GP_PUT` values already read | any read at cyclic distance 0 from the loaded `GP_PUT`, a second `GP_PUT` load, a pushbuffer for another value | stub ring / store `Ev::BeyondPut` in every driver (model bit `V_BEYOND_PUT` in every scenario); model `i8_*`; props `i8_*`; stress hostile writer; unit `the_anchor_is_always_*` | source `core_reads_beyond_put` |
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
`switch_cursor_mismatch` (loud), `pointer_binding_unchecked`, `pointer_software_gone`. Existing counters keep their meaning (`carried_ungated` now counts only Scan/Window entries).

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
12. **(1b) P2' is unmeasured.** The first hardware run reads `put_boundary_open_scan` / `_window` on the app matrix (Heaven, GravityMark, Minecraft, Edge): any
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
`core_no_fresh_resume` and `machine_pointer_markers_anywhere` are from the second pass after those tests were added; an earlier pass had caught the last five
of them by ONE test each and `core_no_fresh_resume` by none, which is why the tests exist):

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

## 9. Results (GPU-free, `cargo test --release`)

| crate | base (`0a7461173`) | PHASE 1 (`6f9163848`) | PHASE 1b | new in 1b |
|---|---|---|---|---|
| `kf-chan` lib | 142 | 166 | 174 | `user_lifecycle` +7 (anchor, boundaries, post-switch, Stop, the anchor property test), `user_loop` +1 (`loud_rate`), `user_scan` (mask test rewritten) |
| `kf-chan` tests | 150 | 150 + 63 (+3 ignored) | 150 + 90 (+3 ignored) | `user_lifecycle_model` 47 (+3 ignored OPEN), `user_lifecycle_props` 25, `user_lifecycle_stress` 2, **`user_turn_unit` 16** |
| `kf-rm` lib | 150 | 154 | 154 | |
| `kf-trap` lib | 80 | 86 | 86 | |
| `kf-qemu` lib | 270 | 271 | 271 | the pump now runs `kf_chan::user_turn`; the G3 structural pin still holds |

All green with `cargo test --release -p kf-chan -p kf-rm -p kf-trap -p kf-qemu` (1 561 tests, 0 failed, 3 ignored OPEN). The interleaving model needs `--release`: 47
tests in about 2 minutes (the Pointer-mode markers added states; three scenarios were shortened and one limit raised, see the git history of `i1_*`, `a3_*`).
Failing first (the base `6f9163848` with the new tests, commit `0e209d0c0`): `d1_*`, both `d2_*` and `d3_*` fail; `t2_*` passes on the base and is the test the
length-0 mutant must fail; `p2prime_*` documents the residual risk. Assertions CHANGED on purpose (the intended behaviour moved, nothing was loosened to hide a
failure): `a2_an_open_packet_carried_*` additionally expects `MARKER_IN_PACKET` (its submission ends inside a packet: P2' false); `i4_*` expects terminal mode Window
after a ring switch (D5); the sequential simulator is now a cooperative guest (entries marked `post` are published after the reply) and counts an unscanned entry
while a registration is live only for `post` entries (entries below the anchor are legitimately unscanned); `g2_p2_*` became `g2_p2prime_*` (identical to always-scan
when submissions end on packet boundaries; counted when they do not).

What is NOT covered (honest list): the staged trigger path and the publication of a SCANNED segment (`Pump::segment_turn`: still pump code; the tests drive a
stand-in); the pump's wiring of `PumpIo` (compiles, the 271 `kf-qemu` tests pass, G3 pin; no GPU-free harness for the real accessors); a real hold of the
registration reply (PHASE 2); inline vCPU copying; BAR1 read volume and every performance claim; any Linux or Windows guest; **P2' itself** (the Scan-mode
counters are the falsifier); the cursor semantics of the source switch; `kf_trap::inline_plan` (question 11).
