# Review: claude/win-lifecycle-p1-20261016 (6f9163848) vs claude/perf-fix-20261015 (0a7461173)

Reviewer: Opus 5.5, 2026-10-16. Read-only. GPU-free runs were made in a scratch worktree (since removed). Labels:
MEASURED = a run I made or a number from a named trace; READ = code I read; INFERRED = reasoning only.

Baseline (MEASURED): `cargo test --release -p kf-chan` is all green at 6f9163848 (lib 166, model 44 + 3 ignored,
props 17, stress 2, the other suites unchanged). `cargo test --release -p kf-qemu --lib` gives 271/271.

## 1. Defects, ranked

### D1 (HIGH, confidence high). The seeded window-start marker can land inside an open guest packet
READ: at a Fresh start, `turn_body` calls `self.scan = self.scan.fresh_resume()` (so `pending = None`), resets the
SyncGate, and ORs `GP_SYNC_WAIT` into `gate_entry`. The guard that existed for exactly this hazard
(`gate_entry >> 63 & 1 == 1 && self.scan.carried().is_some()` -> carried_ungated, no marker) is false *by construction*
after `fresh_resume`. So `SyncGate::entry` returns `PublishBarrier`, and the relay publishes barrier + tagged marker
`[SET_REFERENCE, NON_STALL_INTERRUPT]` straight after the last Pointer-forwarded guest entry. If that entry ended
inside a packet, the hardware takes the marker's words as packet data.

MEASURED (scratch model): I added a hardware-view checker to `user_lifecycle_model.rs`. `ModelEnv` re-parses the
published private stream as ONE packet stream across segments (with the real `Scan`) and sets `V_MARKER_IN_PACKET`
when a `Pub::Marker` is published while that parse has a packet open (patch:
`scratchpad/lifecycle_review_checker.patch`). Scenario: SetObj; `p(1).sync()`; `OpenHead(2)` (`h(1,0x1b0,3)`, 1 of 3
arguments); doorbell; Register; reply; `OpenTail(3).post()`; `p(4).post()`. Result: 130 065 states, violation
`MARKER_IN_PACKET`, log `[Guest(SetObj, scanned), Guest(p1 SYNC, ptr), Guest(OpenHead, ptr), Barrier, Marker(1)]`.
All 45 existing model tests stay clean with the checker added. So the existing model never puts a marker in a packet
in its scenarios, and it could not have noticed one: its PBDMA treats each `Pub` as standalone.

What follows on hardware (INFERRED, from the original Xid-32 PBENTRY history): the marker's 4 words go in as packet
arguments. The guest's tail is then shifted by the words the marker used. The PBDMA parses guest data as headers,
which gives a PBDMA/illegal-method error, a channel or TSG reset, and on Windows a TDR. The marker's `REFERENCE` write
also never happens as a method, so the gate (`Need::Marker`) is never satisfied. The relay waits on that channel
indefinitely, with backstop looks every 50 ms that never succeed, and the guest's TDR comes first.

Exposure (MEASURED inputs, INFERRED rate): S1 Heaven has 11.8 SYNC_WAIT entries per frame, so almost every Fresh
window is seeded. Straddles are at least 282 per 2.67 M segments (the count is capped, so this is a lower bound).
Minecraft has 27 334 SYNC_WAIT entries behind open packets in 6 minutes. A flip lands at the worker's cursor when the
bump becomes visible. That is an arbitrary boundary whenever the worker has a backlog, and Pointer mode with
credit waits (D3) makes a backlog more likely.

### D2 (HIGH, confidence high). A flip inside a packet can be SILENT, so the P2 falsifier undercounts
MEASURED: in the same scenario the Fresh scan of the tail `[0x22, 0x33]` did NOT refuse (no `V_REFUSED`).
`window_fresh_start_parse_error` counts refusals only. A misparse that decodes as headers desyncs the Window's parser
for an unbounded stretch. The consequences: (a) the trigger is missed, reaches the hardware unscanned, and the channel
resets; (b) a false trigger fires. The model's own `a5_a_flip_inside_a_packet_whose_data_looks_like_the_trigger_*`
shows (b) consuming the registration wrongly. For `FIFO_UPDATE_CHANNEL_INFO` that is a source switch at the wrong
position, which loses the channel. When the misparse does refuse, "today's refusal behaviour" is: Grace re-reads
(`REFUSE_RETRIES`), then `USER_RING_REFUSED`, the relay for that channel stops, the guest's fence never signals, and
the guest has a TDR within 2 s (Windows resets the adapter, and apps see DEVICE_REMOVED). Every branch hurts a
cooperative guest. None of them is a host security issue.

### D3 (MEDIUM-HIGH, performance; confidence medium). Pointer-mode progress is woken by a timer when no doorbell or event comes
READ: in Pointer mode `marker_possible()` is false, so `end_batch` and `credit_need` always end in
`Need::Popped`. `user_loop::found(Popped) = Found::Carried`, which is counted as *expected* (`backstop_carried`) and
is not a bug. The only wakes are the 64-iteration spin, a doorbell, any engine event, and the 50 ms backstop. The
model's own `i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look` passes only with looks.
No deadlock: the backstop is always armed, and a native ring would stall on the same acquires (INFERRED). But a
private ring that is full when the guest stops ringing stalls for up to 50 ms per 510 entries. S1 has 923
entries/frame and 2.6 doorbells/frame, and Scan mode had `credit_waits=0` only because the scan was the bottleneck.
The guest `GP_GET` write-back also lags by up to 50 ms. Windows grows rings when they fill, so ring-full is not
hypothetical. A TDR is unlikely, since every stall makes progress (INFERRED). This breaks the owner's "performance
never depends on timing". The counters do not separate Pointer-mode backstops from the old expected carried case.

### D4 (MEDIUM, confidence medium). Sub-device mask state reset to "nothing stored"
READ: `fresh_resume` sets `stored: None`. The unit test pins that a `USE_SUBDEVICE_MASK` with nothing stored is
refused. If the guest STOREs at setup, which is scanned, and USEs later, the first USE after a Fresh start refuses and
the channel is killed. UNVERIFIED whether the Windows streams use STORE/USE at all.

### D5 (MEDIUM, confidence medium). After the source switch, Pointer mode removes the loud detector for wrong cursor semantics
READ: an implicit delete at the trigger closes the Window at the next entry, so the new ring is forwarded unscanned
from its first entry. If P0(a) is wrong (indices continue), the relay forwards stale slots and the hardware *replays
old commands* silently. In Scan mode the same mistake would most likely have refused loudly. GravityMark is the case
that exercises this.

### D6 (LOW). Smaller items
* `Lifecycle::entry` in `Dead` returns a Scan plan ("the safe plan"), which permits a pushbuffer read. It should be a
  stop. No caller reaches it today (READ).
* Pointer mode never calls `software_snapshot`. A freed or replaced 5080 object goes unnoticed: `self.sw` keeps the
  old key, and a registration on a new object never opens a Window. P1-class. Counted nowhere.
* `quiescent` ignores an unproven `carried_ungated` SYNC_WAIT, and `outstanding` is cleared at Window exit. The next
  Fresh window then does not seed. The exposure is the same as the base's carried case.
* A `scan_refused` for a Fresh window is counted only for refusals that start with `"USER_RING_SCAN "`.
  `USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED` is not counted.
* PHASE-2 `kf_trap::inline_plan` neither validates entries (`inspect_entry`) nor consults the lifecycle or epoch.
  That is fine while it is model-only, and a must-fix before it is wired.

### Checked and found sound
* (a) Epoch/flip ordering (READ, INFERRED x86-TSO): the bump is under the registry lock and after the insert, and
  the reply is built after `register` returns (`rmrpc/policy.rs::deferred_api` returns the status synchronously). The
  worker does entry load -> SeqCst fence -> Acquire epoch, and `live_count` takes the lock. The bump is a monotone
  `fetch_add`, so there is no ABA, and every race resolves conservatively (a recount at the next entry). A
  cooperative guest's post-reply entry cannot be forwarded in Pointer mode.
* (b) Window length (MEASURED, S1 registry samples every ~5 s): live=0 at every sample. 63-74 registrations scale
  with channel creation and are implicitly deleted. `rare_segments` was 0-1, so almost none of them are on a user
  channel's own 5080 object. In Heaven, Windows are rare and short. Unmeasured for GravityMark and Minecraft.
* (e) One 8-byte snapshot, and the local value is validated and stored (I7 holds). `inspect_entry` refuses
  reserved, privileged and indirect bits, the length cap and the 2^40 span. The hardware stays inside the channel's
  VA space. The pushbuffer TOCTOU existed before.
* (h) Linux, Translated and Emulated channels never construct a `Lifecycle`. The only shared changes are additive:
  a registry bump (one atomic RMW per change), `Stats.life`, and a print. The structural pin test holds.
* (i) No new `unsafe`. No guest-reachable panic in the production diff. Still one entry per turn.

## 2. Test quality

* The real glue is untested. MEASURED: I replaced the seeded gate in `userpump.rs` with `let gate_entry = entry;`
  (the real-code twin of glue mutant `ignore_seed`). Result: `kf-qemu --lib` 271/271 pass and kf-chan is unaffected.
  So the mutation survives every suite. The same holds for any edit to `lifecycle_plan`, `forward_pointer`, the
  fence, or the gate reset in the real pump.
* MEASURED surviving mutant (machine): "Pointer mode does not record a LENGTH-0 SYNC_WAIT" (the
  `outstanding = Some(abs)` condition restricted to entries with a segment). The model, props and stress suites all
  pass. Only two lib unit tests fail, and only because their fixture `SYNC|0x10` happens to be length-0. With a
  length-0 SYNC_WAIT the model finds STALE + REFUSED + DEADLOCK (my `review_a1_with_a_length0_sync_wait` scenario;
  it is clean without the mutant). Length-0 SYNC_WAIT is the Edge/Win11 shape. The model needs this scenario.
* The model's PBDMA has no packet continuity (D1), so the original Xid-32 class is invisible to all 16 mutants.
* Invariants covered by a single test: I7 double-fetch (1), I6 ignore_events (1), `no_gate_reset` (1),
  `bound_without_datum` (1), `mask_frozen` (2), teardown (2).
* The cheapest way to test the real function: move the per-entry body of `turn_body` (PUT read -> load -> plan ->
  gate -> publish/credit/end_batch) into kf-chan as a function generic over an `Io` trait. The test `Env` already
  names that surface (about 12 methods). kf-qemu then implements `Io` with the real accessors, and the model, props
  and stress drive the production code instead of a transliteration. A cheaper stopgap: a source-diff test that pins
  the transliteration's line map to `userpump.rs` hunks. That is weak.

## 3. Verdict
Not safe for Windows hardware as is. It is acceptable on Linux: inert there by construction, but run the Linux gates
anyway.

Must change before Windows:
1. D1/D2: do not author any marker, and do not restart the parser, at an arbitrary boundary. Proposed anchor
   (P2'): start the Window at `safe_put`. That is the last `GP_PUT` value the worker read in a turn whose epoch load
   still returned the old epoch. Entries before it were published before the reply, so I4 still holds by the same
   causality argument. Forward [cursor, safe_put) as Pointer and flip there. A `GP_PUT` value is a guest
   submission end, which is far more plausible as a packet boundary than an arbitrary entry. The same anchor lets
   Pointer mode place batch markers at `end_batch` (cursor == the observed PUT), which also fixes D3.
2. Before relying on P2', measure it in Scan mode on the existing matrix: a new counter `put_boundary_open` (at
   `end_batch` or each observed PUT, `scan.carried().is_some()`). If it is nonzero anywhere, use the short-entry
   trigger signature instead.
3. D4: keep the scanned sub-device mask state across `fresh_resume`, or refuse to go Pointer once mask ops were seen.
4. Add the model checker (patch provided) and the length-0 scenario. Fail-closed rule: if the hardware-view
   parse is open, never publish a marker; take the old carried path instead.

Measure on hardware (counters exist or are cheap): `straddles_scan` (uncapped), `to_window_fresh`,
`window_fresh_start_parse_error`, `seeded_gates`, `entries_window/pointer`, `pb_bytes_window`. Split
`credit_waits`, `wake_backstop` and `backstop_carried` by mode. New: `put_boundary_open` and Pointer-mode
`backstop_carried`. Make it loud if Pointer-mode backstop wakes exceed about 1/s.

Could not verify: the hardware behaviour of a marker inside a packet (inferred from history); whether Windows
streams use STORE/USE_SUBDEVICE_MASK; cursor semantics P0(a); window lengths outside Heaven; the real
`turn_body` (no GPU-free harness).
