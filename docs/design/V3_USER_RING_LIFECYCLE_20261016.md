**STATUS: DESIGN-ONLY, 2026-10-16** (branch `claude/win-lifecycle-20261016`, from `claude/perf-fix-20261015` `0a7461173`).
This line becomes LIVE (PHASE 1, worker-only, GPU-free results) when the implementation commit lands; hardware validation is a
separate, later step. Labels: **[measured]** = a number from a named run or test; **[read]** = code or public hardware text;
**[inferred]** = reasoning, never evidence.

# V3 — the Windows USER-ring lifecycle: Scan, Pointer, Window

## 0. What this is and why

The USER-ring relay (`crates/kf-qemu/src/userpump.rs`) reads and scans EVERY GP entry's pushbuffer of every Windows user-work
channel for the channel's whole life. **[measured, S1, `traces/perf_lifecycle_20261016/s1/`, Heaven OpenGL, 250 s window]** that is
923 GP entries/frame and 865 pushbuffer reads/frame (1.14 MB/frame, 3.2 BAR1 copies/frame), 100 % of the BAR1 pushbuffer copies of
channels that had already seen their `5080` SET_OBJECT and had no registration pending. The scan exists to find ONE rare thing: the
software-method trigger of a registered deferred API (`FIFO_UPDATE_CHANNEL_INFO` ring growth, `DMA_INVALIDATE_TLB`, context bundles) on
the `5080` software object's subchannel. A channel that has no registration cannot execute a valid trigger.

Settled design (owner/coordinator, 2026-10-13/16; `V3_WINDOWS_USER_RING_RELAY.md` §10): a channel is scanned from birth only until its
`5080` binding is known (**Scan**); after that, while no registration is live, GP entries are copied to the private ring AS IS and
no pushbuffer byte is read (**Pointer**, "migratable passthrough"); while a registration is live the relay scans again (**Window**)
until the trigger has been read and acted on and the registration is gone. This document is the contract, the invariants with their
falsifiers and the tests that enforce them. The hardware gates G1..G4 and the phases are in the brief; PHASE 1 (this document, the
tests, the worker-only implementation) is GPU-free.

Scope. Only Windows USER-work channels that the relay carries today (`userrelay::enabled`, sysmem USERD, private GPFIFO/USERD).
Linux guests, Translated and Emulated channels and vidmem-USERD channels never get a `Lifecycle` (G3; `unchanged_paths_never_enter_a_mode`).

## 1. The state machine (`kf_chan::user_lifecycle::Lifecycle`, pure, no I/O, no clock)

| state | meaning | reads a pushbuffer? | forwards |
|---|---|---|---|
| **Scan** | from birth until the scan has committed a SET_OBJECT that binds the channel's software object on a subchannel >= 5 | yes, every entry | after the scan and the SyncGate, as today |
| **Pointer** | software object bound, no live registration | NEVER (`pushbuffer_bytes_read == 0`) | the guest's 8-byte entry, bit-exact, loaded once |
| **Window** | at least one live registration on the channel's software object | yes, every entry, with the gate | as today; stage + action + source switch at the trigger |

A Window is **Exact** when it continues a Scan that never stopped (the parser state is exact) and **Fresh** when it starts after
Pointer (parser state restarted, assumption P2, §5).

Events and transitions (everything else is rejected by `Lifecycle` and counted; I5):

| from | event (observed at an entry boundary) | to |
|---|---|---|
| Scan | software object bound, 0 live registrations | Pointer |
| Scan | software object bound, >= 1 live registration | Window(Exact) |
| Pointer | registry epoch changed and >= 1 live registration | Window(Fresh) |
| Window | 0 live registrations (trigger executed + implicit delete, `REMOVE_API`, object freed) and the relay is quiescent (no gate draining, no segment in flight) | Pointer |
| any | teardown | Dead (absorbs every later event: a delayed callback cannot revive a freed channel) |

There is no Pointer -> Scan, no Window -> Scan, no timer edge and no edge that depends on a count of entries (OWNER_RULINGS §AE).
"Live registration" means ANY live entry of the channel's `5080` object in the deferred-API registry, whatever its bundle kind, executed or
not (see Open questions 8 and 9: this is deliberately wider than "a pending `FIFO_UPDATE_CHANNEL_INFO`").

### 1.1 The flip: how a registration reaches the worker, and the "held reply"

The registration is served on the RPC drainer (`kf_rm::defapi::Registry::register`), which must never block. **Phase-1 mechanism:** the
registry carries a monotone `epoch` (an `AtomicU64`) bumped, release-ordered, AFTER every change of the set of live entries and BEFORE the
registration's reply is built. The worker loads the 8-byte GP entry (ONE load), THEN loads the epoch (acquire, after a SeqCst fence), and
only if the epoch changed takes the registry lock to count the live entries. Consequences, all proven by the model (I4):

* an entry loaded BEFORE the epoch load that sees the bump is the first Window entry (it is scanned); every entry the worker already
  forwarded in Pointer mode was loaded before the bump became visible. The flip boundary is therefore the worker's own cursor at that
  entry (the private cursor), never a re-read of `GP_PUT` (suspect A3);
* a cooperative guest cannot publish a trigger before it has the registration's reply, and the bump precedes the reply, so every entry
  published after the reply is loaded after the bump and is scanned before it is forwarded;
* entries published before the flip but not yet loaded are scanned too (a superset of the rule "entries published before the flip are not
  scanned"; harmless: the cost is one scan, except for the P2 risk below);
* a hostile guest that publishes the trigger before the reply only resets its own channel (owner ruling 2026-10-13: pushbuffer integrity is
  not a security boundary for USER channels).

This is the brief's "held reply" with one difference that must be stated: in Phase 1 the worker is the ONLY publisher, so the flip is a
wait-free atomic bump and there is nothing for the reply to wait for. The reply is "held" only in the sense that it is built after the bump.
A genuine hold (the drainer parks the reply until a claimant has exited) is needed in PHASE 2, when an inline vCPU claimant exists
(`kf_trap::relay_dispatch::request_worker` / `HANDOFF`), and is not built here. The model checks BOTH policies (reply released at once, and
reply released only after the worker has seen the flip); both satisfy I4.

### 1.2 SYNC_WAIT across the Pointer -> Window edge (suspect A1)

Pointer mode forwards a SYNC_WAIT entry like any other (the hardware honours it) and records the monotone index (`u64`, never a masked ring
index) of the newest forwarded SYNC_WAIT entry. It cannot prove it passed: `GP_GET` runs ahead of processing **[measured]**
(`V3_USER_RING_EVENT_LOOP_20261014.md` §3.3) and Pointer mode must not place a marker (no parser state). At a Fresh window start with a
recorded SYNC_WAIT the SyncGate is seeded: the first Window entry is gated as if it were a SYNC_WAIT entry (barrier + tagged marker, read
nothing until `USERD.REFERENCE` shows the marker processed), which proves that every earlier entry, the recorded one included, was
processed. The marker is authored at the Window's first boundary, which under P2 is a packet boundary. This is the one place Pointer
mode's lack of parser state costs something, and it is Open question 2.

## 2. Pointer-mode entry handling (the only check is the entry)

For each entry: ONE 8-byte load; `inspect_entry` (length cap, privileged/indirect/reserved bits, 2^40 span) on THAT value; a credit check;
publish the SAME value (`write_user_gp`) behind the guest-ring and GP source guards (no pushbuffer guard exists because none was
read); advance the cursor; record SYNC_WAIT. No `Poll`, no tick: the doorbell contract is unchanged (a doorbell makes every earlier
`GP_PUT` write visible; `GP_PUT` ahead of the last doorbell is normal). Markers need a packet boundary, so Pointer mode authors none: the
guest `GP_GET` write-back and private-ring credit waits are announced by the spin, any engine event and the counted 50 ms backstop
(Open question 1: option list).

## 3. Invariants, falsifiers, tests

| id | invariant | falsifier (written before the run) | tests (in `crates/kf-chan`) | mutation (one each) |
|---|---|---|---|---|
| I1 | no pushbuffer byte is read in Pointer mode | a stub store sees a read while the machine says Pointer | `i1_*` (model, unit, stub-store gate G1) | Pointer plans `Scan` |
| I2 | every entry published before the last doorbell is forwarded exactly once, in order, bit-exact | a terminal state with a missing, duplicated or reordered private entry | `i2_*` (model terminal check, property differential) | skip one entry at the flip |
| I3 | no entry at or after an unreached SYNC_WAIT is READ before the tagged-marker witness | the model's `STALE_READ` (pushbuffer read before the guest wrote it) | `i3_*`, `a1_*` | no seed at window start |
| I4 | no entry published after a registration flip is forwarded before the Window scanned it | a forwarded entry tagged "published after the reply" that was not scanned | `i4_*`, `a3_*` | epoch loaded BEFORE the entry |
| I5 | transitions only via the listed events | a (mode, event) pair outside the table changes the mode | `i5_*` | allow Pointer -> Scan |
| I6 | no progress depends on time | a terminal state with covered work unrelayed when no timer step exists | every model test runs with time forbidden; `i6_*` | exit needs a tick |
| I7 | the value tested is the value stored | a stored value that differs from the loaded one while the guest changes ONE dword at any instant | `i7_*`, `a4_*` | double fetch |

Suspects A1..A7 of the appendix map to `a1_*`..`a7_*` tests; the mutation table with the failing test per mutation is in §8 (filled in with the
implementation commit).

## 4. Counters (loud where the assumption is falsifiable)

Per channel (printed at release, one bounded line) and VM-wide (`kf_chan::user_loop::Stats::life`, in `USER_RING_LOOP SUMMARY`):
`entries_scan/pointer/window`, `to_pointer`, `to_window_exact`, `to_window_fresh`, `to_pointer_from_window`, `window_entries_max`,
`straddles_scan` and `straddles_window` (segments that END inside an open packet; this is the P2 measurement, per mode),
`segments_scan/window`, `window_fresh_start_parse_error` (a scan refusal in the first four segments of a Fresh window),
`window_binding_changed` (the software binding the Window relies on changed: P1 falsifier), `window_unscanned_long` (0 by construction:
nothing in a Window is skipped), `window_bar1_bytes`, `pushbuffer_bytes_read_scan/window` and `pointer_pb_read_violation` (must stay 0; also a
`debug_assert!`), `scan_no_software` (channels that never allocate a software object keep scanning).

## 5. Assumptions and what the existing traces say about them

* **P1 (the `5080` SET_OBJECT occurs once at setup).** Not measurable in Pointer mode by construction (nothing is read). Made falsifiable
  in Windows (`window_binding_changed`) and in Scan. Existing data: 71 channels, `pb_reads_before_5080` 5 299 pushbuffer reads over the Scan phases of a
  Heaven run **[measured, S1]**: Scan phases are short.
* **P2 (the driver ends GP segments on packet boundaries).** **CONTRADICTED as a general property, supported as a rarity.** The Windows
  driver splits big inline uploads across GP entries (`INC1 sub1/sub2 m0x1b0 1/1057`, 276 STRADDLE records in the 2026-10-14 traces, dozens of
  sizes up to 3841 arguments) and Minecraft produces 25 153 SYNC_WAIT-behind-an-open-packet entries in six minutes **[measured]**. The
  per-channel `straddles` summary is capped at 16 (`TRACE_STRADDLES`), so it is a lower bound, e.g. 16 of 2 314 629 segments on the busiest
  Heaven channel. Heaven's relay reported `carried_ungated = 0` over 51 M GP entries (S1) **[measured]**. A flip at an arbitrary entry boundary
  therefore lands inside an open packet with a probability bounded below by about 7e-6 per boundary and above by an unmeasured
  value; on such a flip the Fresh scan misparses (a loud refusal or a missed trigger). `straddles_window` / `window_fresh_start_parse_error`
  turn this into a measurement. See Open question 3.
* **P3 (the reply is held while the mode flips).** Satisfied by the ordering of §1.1 in Phase 1.
* **P4 (what keeps a channel scanning).** The registry empties (`implicit_deletes` in every measured Windows trace) or the object is freed.

## 6. Window details

* Fresh start: bindings are kept (the software subchannel is not re-learned), `pending` is cleared, the sub-device mask state is reset
  to its birth value (`active`, nothing stored), `SyncGate` is reset (a stale `cleared` key could wave a SYNC_WAIT one ring lap later through,
  `V3_USER_RING_EVENT_LOOP_20261014.md` §5.1), `Grace` is cleared.
* Refusal in a Window: exactly today's scanner refusal (`Grace`, then `USER_RING_REFUSED`, channel dead). Never a recovery.
* VRAM-backed pushbuffers in a Window use the existing path (rare, bounded) and are counted (`window_bar1_bytes`). PHASE 3 replaces bulk
  MMIO reads in windows.
* A Window with no trigger never ends by itself and no timer ends it; it is counted (`window_entries_max`) and every other channel is
  served (one entry per turn, `REACT_ROUNDS` fairness unchanged).
* Source switch: the existing `migrate` path (`USER_RING_MIGRATED`): cursor 0 on the new ring, new USERD, generation + 1, private native
  GET untouched. Cursor semantics are unmeasured (Open question 4); the model checks both candidates.

## 7. Open questions (each has an `#[ignore = "OPEN: ..."]` test where it can be written)

1. **Pointer mode has no marker, so no interrupt announces private-ring credit or the guest `GP_GET` write-back.** A marker segment is only safe
   at a packet boundary, which Pointer mode cannot know. Options: (a) spin + any engine event + the counted backstop (implemented; correct, possibly
   slow when the 512-slot private ring fills at about 923 entries/frame); (b) author markers at the P2-assumed boundaries (rejected: a wrong boundary
   corrupts the cooperative guest's stream); (c) a larger private ring (needs the storage layout); (d) a cheap boundary witness per segment (reads the
   pushbuffer: contradicts G1). Recommendation: (a), measure `credit_waits` / `backstop_carried` on the host, then decide (c).
2. **Window-start gate marker relies on P2.** See §1.2. Options: (a) author it at the Fresh boundary (implemented); (b) prove "passed" from the
   channel's `USERD` PB position (`TOP_LEVEL_GET`), unmeasured; (c) never leave a SYNC_WAIT unproven: not possible without a marker. Recommendation: (a)
   with the counters, measure (b) on the host.
3. **P2 is contradicted by measured straddles.** Options: (a) accept and count (implemented); (b) delay the Window until the first entry whose scan
   returns to a boundary and refuse nothing before it (a scan that cannot be trusted for its first segments); (c) state-free trigger signature for
   short entries (§10 of the relay design). Recommendation: (a) for Phase 1, then (c) if the counters are nonzero.
4. **Cursor semantics at the source switch are unmeasured (P0 a).** Production keeps the existing reset-to-0 on the new ring. The model runs both.
5. **A rebind of the software subchannel while in Pointer mode is invisible** (A6). A trigger on the rebound subchannel is then missed and
   the hardware resets the channel. Cannot be known without reading; counted when it happens inside a Window.
6. **A channel without a software object scans forever** (a registration needs the object, and the object cannot be allocated after the first
   publication, so such a channel could safely be Pointer from its first entry). Implemented as the brief says (Scan until bound), counted
   (`scan_no_software`); recommendation: owner may allow Pointer for it.
7. **Held reply** in Phase 2 (inline claimants): see §1.1.
8. **`pending` = any live entry, executed or not** (an explicit-delete entry stays live after its trigger and may be triggered again). Measured
   Windows traces only use implicit delete (93 of 93). Cost of the wider rule: a Window until `REMOVE_API` for an explicit entry.
9. **All bundle kinds open a Window, not only `FIFO_UPDATE_CHANNEL_INFO`.** The relay also executes `DMA_INVALIDATE_TLB` and context bundles
   at their triggers (`Pump::software`); an unscanned trigger of those reaches the hardware, which has no such registration and resets the channel.
   This deviates from the brief's wording and is required for correctness.
10. **Sub-device mask state after an unscanned stretch** is reset to its birth value; unscanned `SET/STORE/USE_SUBDEVICE_MASK` cannot be seen.

## 8. Mutation checks

(Filled in with the implementation commit.)
