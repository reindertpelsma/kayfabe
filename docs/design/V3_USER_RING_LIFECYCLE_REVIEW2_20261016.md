# Review 2: claude/win-lifecycle-p1b-20261016 (e1e9947cc) — lifecycle PHASE 1b

Reviewer: Opus 5.5, 2026-10-16. Read-only. GPU-free runs in a scratch worktree (/workspace/kf-review2, removed after).
Labels: MEASURED = a run I made or a number I computed from a committed trace; READ = code I read; INFERRED = reasoning only.

Baseline (MEASURED, e1e9947cc, `--release`): kf-chan lib 174, model 47 (+3 ignored, 153 s), props 25, stress 2,
user_turn_unit 16, every other kf-chan suite green. kf-qemu --lib 271/271 (measured with mutant W applied; W touches only
`migrate`, so the baseline cannot pass fewer).

## 1. Review-one findings: fixed?

| id | verdict | how verified | residual |
|---|---|---|---|
| D1 marker in packet at Fresh start | FIXED under P2' | READ: in Pointer mode `relayed <= safe_end` always holds (every clean turn sets `safe_end = max(safe_end, put_abs)` with `put_abs >= relayed+1`; the Scan->Pointer and Window->Pointer edges set it to that turn's `put_abs`; registered turns forward only while `abs < safe_end`), so a Fresh window starts at exactly `abs == safe_end`, a GP_PUT value read in a turn whose epoch observation showed 0 live. MEASURED: reviewer-1's checker is now in the model (`V_MARKER_IN_PACKET`, checked for every `Pub::Marker`, Pointer markers included); `d1_*` clean; reviewer-1's exact scenario is `p2prime_*` and still violates (by design: its GP_PUT is mid-packet). | P2'. Plus: the second and later windows of a channel are not covered by any test (mutant A survives, section 4). |
| D2 silent misparse | FIXED under P2' (same anchor) | MEASURED `d2_*` (3 tests) clean | P2'. When P2' fails the Window's own `put_boundary_open_window` is blind (fresh parser believes "closed"). |
| D3 timer-woken Pointer mode | FIXED for the common case | READ: `end_batch` runs only when `GP_PUT == cursor`, so `here == Some(relayed)` and a marker is placed at every caught-up batch end; credit stalls at a remembered boundary (`next_boundary`) too. MEASURED `d3_*` live with looks forbidden. | Mid-submission credit stalls and lagging workers (counted, loud). New exposure: see N1 — P2' is now a STEADY-STATE assumption. |
| D4 mask reset | FIXED | READ `fresh_resume` keeps `subdevice`; props `d4_*` | A mask changed inside an unscanned stretch is invisible (open question 10, accepted). |
| D5 unscanned new ring | PARTLY | READ: 8 entries scanned + `USER_RING_SWITCH_CURSOR_MISMATCH` on a nonzero new GP_GET. MEASURED: deleting the only production call `self.core.life.source_switched()` in `userpump.rs::migrate` survives every suite (kf-qemu 271/271; kf-chan untouched because its stand-in calls it itself). | The D5 fix's wiring is untested; valid stale slots still pass the 8-entry scan (author says so). |
| D6 small items | FIXED except inline_plan (deferred, OK while unwired) | READ: `Plan::Stop` in Dead; Pointer `gone` check; `PARTIAL_SUFFIX` counted | carried_ungated/outstanding-at-exit exposure unchanged from base. |
| T1 real turn untested | LARGELY FIXED | READ: the per-entry turn is production `kf_chan::user_turn::Relay`; model/props/stress/unit drive it through `EnvIo`. | `PumpIo` (11 methods), `segment_turn`, `software`, `migrate` remain pump code with no GPU-free harness (mutant W survives). |
| T2 len-0 SYNC_WAIT | FIXED | READ `t2_*` scenario present | — |
| T3 single-test invariants | IMPROVED | table claims >= 2 tests each; I did not re-run the 27-mutant script (80 min). | Not verified by me beyond the mutants below. |

## 2. New defects / behaviour changes (ranked)

N1 (HIGH, design-level, INFERRED from READ). **P2' moved from a rare window-start assumption to a steady-state one.**
In PHASE 1 the arbitrary-boundary risk existed only at window starts (Heaven: one trigger per session). In 1b every
Pointer-mode batch end (`GP_PUT == cursor`, i.e. ~every doorbell batch, Heaven 2.6/frame -> several hundred/s) authors a
host marker that is safe only if that GP_PUT is a packet boundary. One violation corrupts the guest channel.

N2 (HIGH for the run plan). **The named falsifier cannot falsify P2' where it now matters, and there is no switch.**
`put_boundary_open_scan` counts only while the channel is in Scan, i.e. the setup prefix before the 5080 SET_OBJECT
(S1: 5 299 pushbuffer reads over 71 channels, ~75 entries per channel). The steady-state boundaries where Pointer markers
land are never observed with a known parser state; `_window` is rare and self-blinding after a wrong anchor. The pump
hardwires `Lifecycle::new()` (userpump.rs:900): no measurement mode (always-scan + counters) and no kill switch for
Pointer markers / anchored windows. The design's own decision rule ("do not enable Pointer markers / anchored Windows
for that workload") cannot be executed without a rebuild.

N3 (MEDIUM, test gap, MEASURED). Window -> Pointer re-anchoring is untested: removing `self.safe_end = o.put_abs` from the
Window exit (mutant A) passes all 264 lifecycle tests; my probe shows the mutant opens the SECOND window at the first
entry that sees the registration (inside the submission). D1 regression for every window after the first.

N4 (LOW, MEASURED by probe). `next_boundary` is an absolute index that survives a ring switch. If the old ring's GP_PUT
was ahead of the trigger, that index is later believed to be a GP_PUT boundary of the NEW ring (`marker_possible`
returns true at abs 30 where no GP_PUT was ever 30; probe `probe_stale_boundary_after_a_source_switch` fails on
e1e9947cc). Only reached through a Pointer-mode credit stall at exactly that index, and only if the old ring had
published entries after the trigger (which reset-to-0 would also lose). Fix: clear `next_boundary`/`here` in
`source_switched` (and on any generation change).

N5 (LOW, READ). `user_lifecycle.rs` unit test `markers_need_a_known_parser_state_or_an_observed_gp_put_boundary` ends with
`assert!(l.marker_possible(false, 5) == l.marker_possible(false, 5))` — a tautology.

Checked and found sound (READ unless noted): anchor vs epoch vs guest publishing in between (a turn that sees the bump is
never an anchor; the anchor turn's GP_PUT load precedes its SeqCst fence + epoch load that returned the pre-bump value;
the recount path takes the lock after loading the epoch, conservative in every race); no new `unsafe`; no new
guest-reachable panic (release has `overflow-checks = true`; every new subtraction is guarded or monotone, `entries - 1`
only after `put >= entries` refused); one entry per turn; Linux/Translated/Emulated never construct a Lifecycle (G3 pin
holds); registry bump is O(1) under the existing lock, `live_count` O(log n). G2 "bit-identical" is true for GUEST
entries only: the private ring differs by design (no SYNC_WAIT gates in Pointer, markers only at GP_PUT boundaries).
Behaviour difference vs the old pump in Scan mode: an extra `software_snapshot` per entry (`software(refresh=true)`) —
cost only.

## 3. P2' against the trace facts

* MEASURED (my computation over the 392 unique `USER_RING_TRACE STRADDLE` records committed in
  `traces/user_ring_scan_20261014/*` and `traces/boots/w277`): `put - cursor` is never 1 (min 3; 223 records = 4, the
  1/1057 inline-shader-upload idiom). No straddling segment was the last entry before the GP_PUT the worker read.
  Weak support: records are capped at 16/channel, early-life biased, Edge/desktop only, and a lagging worker inflates
  `put - cursor`.
* READ (traces/win11_userring_20261014): Minecraft's `SYNC_CARRIED` records are one idiom — a segment holding a packet
  header + 3-4 args, then a SYNC_WAIT segment of exactly the remaining args (4/29 then 25 words, 4/84 then 80 words).
  The two GP entries are emitted together; whether a GP_PUT ever falls between them is unmeasured.
* INFERRED: the most plausible way a cooperative driver ends a submission inside a packet is a full GPFIFO in the middle
  of such a sequence (it must kick what it has to get space) — and a full ring is exactly when Windows grows the ring
  (the FIFO_UPDATE_CHANNEL_INFO registration). So the anchor is most exposed at the one moment it is used for the
  ring-growth window.
* READ + MEASURED: the S3 scratch build 28a0ee9c (mode 2) skipped the scan with an empty word list, so its parser state
  stayed "closed" and it placed batch markers at every caught-up GP_PUT and SYNC_WAIT gates at arbitrary boundaries in
  the unscanned stretch; Heaven D3D11 and OpenGL ran 3 x 260 s each without Xid. That is real evidence that P2' (and even
  P2 at SYNC_WAIT entries) holds for Heaven. Nothing comparable exists for Minecraft, Edge or GravityMark.

What fails on hardware (INFERRED from PBDMA semantics + the measured mode-1 failure):
* Pointer batch marker inside a packet: the marker's words are consumed as the open packet's data (random method writes,
  or inline-upload payload = silent corruption); the packet ends early and the guest's last words of the next segment
  are decoded as headers. Typical result: PBDMA interrupt, host Xid 32 (mode 1 saw `PBDMA INTR_0 0x00800000`), or Xid
  13/31 if decoded methods reach the engine or a random address, or a hang on a decoded semaphore acquire. Host RM RC
  resets the channel/TSG; guest nvlddmkm event 153 / TDR, Windows resets the adapter, apps get DEVICE_REMOVED. The
  marker's REFERENCE/interrupt never happen; the relay is woken by the 50 ms backstop (`pointer_backstop` rises).
* Anchored window inside a packet: the seeded gate marker does the same, AND the gate waits for a reference that never
  comes, so the relay stalls that channel until the guest's TDR. The fresh parser misparses: silent missed trigger
  (unscanned FIFO_UPDATE_CHANNEL_INFO -> Xid 32 at ring growth, the mode-1 signature), false trigger, or a refusal
  (`window_fresh_start_parse_error`, then USER_RING_REFUSED and a TDR).
* None of this is a host-security issue.

Guard: the per-channel Scan latch has almost no power (Scan sees only the setup prefix). Worth adding instead:
1. A measurement mode (env, owner rules allow measurement flags): `Lifecycle::always_scan()` with the 1b counters —
   `observe_put` already counts every caught-up GP_PUT in Scan mode under legacy, so this measures P2' on the
   steady-state population. Run the app matrix in it before Pointer markers are enabled for that app.
2. A VM-wide latch fed by every known-state boundary (Scan, Exact windows, measurement mode): once any GP_PUT boundary
   is seen open, stop authoring Pointer markers VM-wide (fall back to spin/backstop, counted) — markers are performance,
   not correctness, so the fallback is safe.
3. Guest-ring-full guard: a GP_PUT read while the guest ring is full (`put + 1 == last written-back GP_GET`) is never
   an anchor or a marker boundary; count `put_at_ring_full`.
4. Log lines: `USER_RING_PUT_BOUNDARY_OPEN` (first N per channel, cursor/put/open packet) and a per-flip
   `USER_RING_WINDOW_FRESH` (cursor, safe_end, anchor_forwarded, seeded, first-segment result).

## 4. Mutations (each applied in the scratch worktree, suites run, reverted)

| mutant | suites | result |
|---|---|---|
| W: `userpump.rs::migrate` drops `self.core.life.source_switched();` (D5 wiring) | kf-qemu --lib (kf-chan does not compile pump code) | SURVIVED 271/271 |
| A: `Lifecycle::entry` Window->Pointer drops `self.safe_end = o.put_abs;` | kf-chan lib 174, model 47, props 25, stress 2, unit 16 | SURVIVED (all green); probe shows a real defect (second window below the anchor) |
| A2: Scan->Pointer sets `safe_end = o.abs` instead of `o.put_abs` | same | KILLED (I predicted survival; wrong): 2 lib unit, model `d2_*` x2, props `g2_p2prime_*` |
| R: `PumpIo::registry_live` keys the registry by `r.key.1` (the channel) instead of `sw.guest` — no Window ever opens, every trigger reaches the hardware unscanned (the mode-1 Xid 32) | kf-qemu --lib | SURVIVED 271/271 |
| (spot check of the author's table) `core_ignore_seed` | user_turn_unit | KILLED (`a_window_that_follows_a_forwarded_sync_wait_*`), as claimed |

Probes (scratch test file, not committed): `probe_second_window_starts_at_the_anchor` passes on e1e9947cc and fails under
mutant A; `probe_stale_boundary_after_a_source_switch` FAILS on e1e9947cc (N4).

Conclusion: the machine and the core turn are well tested; the pump-side wiring (`PumpIo`, `migrate`) is not — the two
lines that decide whether a Window ever opens (W, R) can be broken with all 271 + 264 tests green.

## 5. Verdict

Linux gates: GO (the lifecycle is inert there by construction; run v3_gates 9/9 and fast_suite 30/30 as usual).

First Windows run on a dev VM: GO for Heaven ONLY, after these must-fix items (all small):
1. A measurement/kill switch (env, measurement-class): `always_scan` (Lifecycle::always_scan + all 1b counters) and
   `no_pointer_markers`. Without it the P2' decision cannot be acted on without a rebuild, and P2' cannot be measured on
   the steady-state population at all.
2. Log lines: `USER_RING_PUT_BOUNDARY_OPEN` (first N per channel + all in windows: cursor, put, open packet) and
   `USER_RING_WINDOW_FRESH` per flip (cursor, safe_end, anchor_forwarded, seeded, outcome of the first segment).
3. Two GPU-free tests: Window->Pointer re-anchoring (mutant A) and a pump-side check that `registry_live` /
   `source_switched` are wired (mutants R, W) — e.g. a kf-qemu unit test over `PumpIo::registry_live` with a populated
   registry, or a source-text pin like the G3 pin.
4. Clear `next_boundary`/`here` on a source switch (N4).
Should-fix: guest-ring-full anchor guard + counter; VM-wide P2' latch for Pointer markers.

Run order: Heaven D3D11 + OpenGL with full lifecycle (the S3 mode-2 evidence supports P2' there) and counters; then
Minecraft, Edge, GravityMark in `always_scan` measurement mode FIRST; enable Pointer markers / anchors for an app only
if its `put_boundary_open_scan` is 0 over a full run.

Read after the run (USER_RING_LIFECYCLE SUMMARY + per-channel lines + relay stats + host dmesg + guest System log):
* must be 0: `put_boundary_open_scan`, `put_boundary_open_window`, `window_fresh_start_parse_error`,
  `switch_cursor_mismatch`, `pointer_pb_read_violation`, `window_binding_changed`, `after_teardown`; no
  `USER_RING_REFUSED`, no `USER_RING_SOFTWARE_OBJECT_GONE` outside teardown; no NVRM Xid 13/31/32/43; no guest
  nvlddmkm 153 / Display 4101.
* sanity: `put_boundaries_scan` large in measurement mode (the denominator), `to_window_fresh == restarts`,
  `seeded_gates <= to_window_fresh`, every `USER_RING_MIGRATED` followed by `post_switch_scanned` += 8 and no Xid,
  `anchor_forwarded` (how far behind the anchor the flip was seen), `entries_pointer` >> `entries_window`.
* performance of the D3 fix: `pointer_markers` ~ doorbell batches, `pointer_carried_no_marker`, `pointer_backstop`,
  `pointer_backstop_loud` ~0, relay `backstop_carried`, `credit_waits`; fps vs the S3 189 / 338.
* Decision: Pointer markers stay only if `put_boundary_open_*` == 0 and no Xid/TDR; anchored windows stay only if, in
  addition, `window_fresh_start_parse_error` == 0 and every ring growth passed without Xid.

Could not verify: P2' on any workload other than Heaven (and Heaven only indirectly, via the S3 scratch build's code
plus its reported runs); the hardware behaviour of a marker inside a packet (inferred from PBDMA semantics and the
mode-1 Xid 32); whether Windows' ring-growth registration coincides with a full GPFIFO; cursor semantics at the switch;
the author's 27-row mutation table beyond `core_ignore_seed`; anything in `PumpIo`/`segment_turn` beyond reading.
