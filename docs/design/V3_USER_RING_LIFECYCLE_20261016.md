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

`scripts/lifecycle_mutation_check.sh` re-runs the whole GPU-free suite (`--features lifecycle-mutants`, off by default and never enabled by another crate) once per
deliberate defect and lists the failing tests. Machine mutants are `kf_chan::user_lifecycle::Mutant` (`KF_LIFECYCLE_MUTANT`), glue mutants are defects of the test
transliteration of the turn (`KF_LIFECYCLE_GLUE_MUTANT`; the transliteration in `tests/lifecycle_support/mod.rs` uses the REAL `Lifecycle`, `SyncGate`, `Marks`, `Grace`,
`Scan`, `Fetch`, `inspect_entry`, but the pump's I/O is abstracted: a mapping by line is in its comments).

Result of the last run (baseline green; every one of the 16 deliberate defects is caught; none survived, so no test had to be added after the run):

| mutation | invariant / suspect | failing tests (count: names) |
|---|---|---|
| `machine_pointer_scans`: Pointer mode plans a scan (a pushbuffer is read in Pointer) | I1 | 45: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `machine_no_seed`: a Fresh window never seeds its gate | I3 / A1 | 5: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing`, `i3_the_same_stream_with_the_guest_free_to_publish_b_early_is_found`, and 2 more |
| `machine_forget_sync`: Pointer mode forgets the SYNC_WAIT it forwarded | I3 / A1 | 7: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing`, `b_the_monotone_entry_counter_survives_the_u32_boundary`, and 3 more |
| `machine_pointer_to_scan`: a registration in Pointer mode moves the channel back to Scan | I5 | 28: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `machine_revive_after_teardown`: a teardown does not stop the machine | A7 | 2: `a7_teardown_at_any_instant_stops_the_private_ring_for_good`, `teardown_is_terminal_and_a_late_event_cannot_revive_the_channel` |
| `machine_exit_ignores_live`: a Window ends when quiescent whatever the registry says | I4 | 31: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `machine_mask_frozen`: a Window never notices a moved software binding | A6 / P1 | 2: `a6_a_rebind_inside_a_window_is_seen_and_counted_and_the_new_binding_is_used`, `a_changed_software_binding_inside_a_window_is_counted` |
| `glue_epoch_before_put`: registry epoch loaded before GP_PUT and the entry | I4 / A3 | 21: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `glue_double_fetch`: a second load of the entry is what gets stored | I7 / A4 | 1: `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_than_the_tested_one` |
| `glue_no_gate_reset`: the SyncGate is not reset at a Fresh restart (stale proof one ring lap later) | I3 / A1 | 1: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof` |
| `glue_flip_at_put`: the flip boundary is a re-read of GP_PUT, not the cursor | I4 / A3 | 25: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `glue_ignore_seed`: the seeded gate is ignored at window start | I3 / A1 | 4: `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing`, `i3_the_same_stream_with_the_guest_free_to_publish_b_early_is_found`, and 1 more |
| `glue_ignore_events`: the worker never looks at its waits on an event (only a timer would) | I6 | 1: `i6_every_announced_wait_is_woken_by_an_event_with_no_look_and_no_timer` |
| `glue_drop_flip_entry`: the first entry of a Fresh window is skipped | I2 | 28: `a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof`, `a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer`, `a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh`, and 3 more |
| `glue_skip_validation`: an entry inspect_entry refused is forwarded anyway | A4 | 3: `a4_every_single_bit_flip_of_a_valid_entry_is_refused_by_name_or_forwarded_validated`, `a4_hostile_entries_never_panic_and_never_store_an_unvalidated_value`, `a4_one_dword_changing_between_the_loads_never_stores_a_value_other_than_the_tested_one` |
| `glue_bound_without_datum`: a SET_OBJECT header (datum unscanned) counts as the binding | A2 | 1: `a2_the_binding_packet_split_across_two_entries_switches_to_pointer_after_the_datum` |

## 9. Results (GPU-free, `cargo test --release`)

| crate | base (`0a7461173`) | this branch | new |
|---|---|---|---|
| `kf-chan` lib | 142 | 166 | `user_lifecycle` 22 (machine, counters, transition table, G4 source pin, reused key), `user_scan` +2 (`software_mask`, `fresh_resume`) |
| `kf-chan` tests | 150 | 150 + 63 (+3 ignored OPEN) | `user_lifecycle_model` 44 (+3 ignored), `user_lifecycle_props` 17, `user_lifecycle_stress` 2 |
| `kf-rm` lib | 150 | 154 | registry `epoch` / `live_count`: 4 (one is a two-thread ordering test) |
| `kf-trap` lib | 80 | 86 | `inline_plan` 5 + 1 loom model (PHASE 2 protocol, model level only) |
| `kf-qemu` lib | 270 | 271 | the G3 structural pin |

All green with `cargo test --release` (the interleaving model needs `--release`: 44 tests in about 30 s; the 3 OPEN tests are `#[ignore]` with the
expected-versus-observed text). Failing-first: against the stub machine (= the base behaviour, every entry scanned) 22 `kf-chan` lib tests, 4 `kf-rm` tests,
21 of 32 model tests and 4 of 5 props tests failed (commit `de719e337`); the tests added afterwards (A-items, mutants, corpus, stress) are validated by the mutation table
above instead. Clippy: no warning or error of the new code in `kf-chan`, `kf-trap`, `kf-rm`, `kf-qemu` (kf-qemu has 91 pre-existing denied-lint errors: the base is not clippy-clean there).
Soak: 10.7 M entries over 3 092 streams in 2 s (`KF3_STRESS_SECS` longer); stress: 6.6 M entries and 242 k staged triggers with a concurrent registrar, 137 k hostile turns
with 464 channels killed by their own hostile entries and no violation of I7.

What is NOT covered (honest list): the pump itself (`Pump::turn_body`) cannot run GPU-free, so the model and the property tests drive a line-for-line transliteration that uses the
same machine and the same scan / gate primitives (G3, the compile and the structural pin cover the wiring only); a real hold of the registration reply (PHASE 2); inline vCPU
copying (only the `inline_plan` model); BAR1 read volume (G1 in bytes) and every performance claim; any Linux or Windows guest; the cursor semantics of the source switch.

