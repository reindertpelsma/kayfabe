# Oversized USER descriptors: the decision

**STATUS: LIVE-DESIGN, 2026-10-22. Design authority decision on G1's stop
([`V3_USER_DESCRIPTOR_BOUND_20261022.md`](V3_USER_DESCRIPTOR_BOUND_20261022.md)) using T1's evidence
([`V3_TDR_SPECVIEWPERF_20261022.md`](V3_TDR_SPECVIEWPERF_20261022.md)). Nothing here is implemented. The
base is `03715b7d3`. Product code is unchanged by this note.**

Tags: **MEASURED** means observed in a run, a log or a probe executed for this note. **CODE READ** means read in source at
`03715b7d3`. **INFERRED** means reasoning that a test or hardware run still has to confirm.

## 0. Decision in one paragraph

**Mode-scoped admission. The 917,504-byte figure limits a snapshot or a staged span, never a descriptor.**
The staging limit (`user_storage::MAX_SOURCE_BYTES`) bounds host memory and CPU only where the relay copies
guest pushbuffer bytes. CODE READ: Pointer mode copies none, yet today it applies the limit before it selects a
mode. Phase 1 therefore admits Pointer-mode and control descriptors on the hardware format alone, up to
(2^21 - 1) × 4 = 8,388,604 bytes with the end below 2^40, and forwards the guest's 8-byte word unchanged, as it
already does for small entries. Phase 2 lets Scan and Window mode classify a descriptor larger than one snapshot
by streaming it through bounded snapshots of at most 917,504 bytes, one part per worker turn, carrying parser
state within the one descriptor. The original word is forwarded unchanged and is never split. The one real
ceiling is staging: when a descriptor needs host-authored staging (a registered software trigger before any
`END_PB_SEGMENT`), its whole patched copy must fit in 917,504 bytes. Above that the relay refuses it by a new
name and stops, because the only alternatives are to skip the entry (a forged consumption) or to hold it forever.
The constant is not raised, no descriptor is split, and no timer or heuristic is added.

## 1. Evidence

- MEASURED (T1, run 3440): there are twelve `USER_RING_GP_FORMAT Budget` refusals of 947,340..1,026,596 bytes against
  917,504. Each one is followed by `poisoned` +1, a cancelled relay, a refused `RESET_CHANNEL`, a BAR1 unload and a
  viewperf OpenGL error. The first is at line 113366, word `0x0faa26023110102c`.
- MEASURED (this note: a scratch crate outside the repo linked `kf-chan`/`kf-abi` at `03715b7d3` and was deleted afterwards):
  all twelve words decode as direct C56F segments with **LEVEL = SUBROUTINE and no SYNC_WAIT**, with VAs from 0x1dd1002c to
  0x2c42c002c. `inspect_entry(w, 917504)` returns `Err(Budget)` for all twelve, and `inspect_entry(w, 8388604)` returns
  `Ok(Segment)` for all twelve. `gp_entry(va, 8388604)` encodes, and `gp_entry(va, 8388608)` returns `None`: a larger
  length cannot be expressed.
- CODE READ of T1's report: the first refusing relay was in Pointer mode at retirement (211,919 Pointer entries and
  13 Scan entries). **INFERRED, not measured:** the mode at the other eleven refusals. They are probably Pointer too, because
  long-lived channels spend almost all of their entries there. §9 adds the counter that measures it.

## 2. The production path, end to end (CODE READ)

| Step | Site | What happens to a descriptor of length L |
|---|---|---|
| Load | `userpump.rs` `PumpIo::load_dword` (746-790) | One owned 8-byte snapshot of the guest GP slot (`aux.entry`, `aux.source` guard). Lo and Hi come from that one value (invariant I7). |
| Admit | `user_turn.rs` `plan_step` (677-) → `user_stream::inspect_entry(raw, io.max_entry_bytes())` | Format check, then **`L > 917504` → `Budget`**. This runs **before** `Lifecycle::entry` chooses Pointer or Scan. |
| Refuse | `user_turn.rs` `step` (509-517), `userpump.rs` `user_ring_serve` (2863-2871) | `Fail::Refused` → `dead = true`, lazy state invalidated, `poisoned` +1, `r.dead = Some(why)`, `record.cancel()`. No completion and no reuse credit. The guest's channel never progresses again, so the guest driver TDRs. |
| Pointer plan | `user_lifecycle.rs` 895-900 → `Plan::Forward` → `publish_guest` (810-) | No pushbuffer read (invariant I1, counted by `pointer_pb_read_violation`). |
| Pointer publish | `userpump.rs` `PumpIo::publish_entry` (860-885) → `publish_guarded(.., ring, gp=source, pb=None, ..)` → `Storage::write_user_gp(slot, entry)` → `publish_put` → doorbell | The **raw word** goes into the private GP ring. **`pb` is `None`: the PB is never resolved, read or staged.** `write_user_gp` (`user_storage.rs` 522-530) **re-applies `inspect_entry(entry, MAX_SOURCE_BYTES)`**. |
| Scan or Window, no trigger | `Phase::PbRead` → `read_pb` → `userread::snapshot` (`bytes ≤ MAX_BYTES`, ≤ 225 pieces) → `stage` → `segment_turn` non-rare branch (2262-2300) | The PB snapshot is scanned to classify it and to carry parser state. Then the **raw word** is forwarded through `write_user_gp` behind ring, GP and **PB** source guards. |
| Scan or Window, trigger | `segment_turn` rare branch → `storage.write_authored(0, &patched)` (2331) → `write_gp_level` spans | **The whole patched segment is staged** in `[0, GP_OFFSET)`, and `write_authored` and `write_gp_level` both refuse `end > GP_OFFSET`. **This is the only place where 917,504 is a resource fact.** |

So the limit is enforced in three places: `inspect_entry` in `plan_step`, `write_user_gp`, and `userread::snapshot`
together with the staging writers. It exists because a hostile guest must not be able to make the worker allocate,
copy or scan an unbounded span, or overflow the 1 MiB private object. CODE READ: in Pointer mode none of those
resources is touched, so the first two checks bound nothing there. The only effect they have is to kill legal work.

**Is "dead" the right response?**

- Dead is correct for an entry the relay *cannot* relay faithfully: a malformed word, or a span that needs staging
  larger than staging. The ring is ordered. Skipping the entry would forge a consumption and reorder the guest's
  work. Holding it forever is the same stop with resources still held.
- Dead is **wrong** for a hardware-legal entry the relay *can* relay. Every Pointer entry, and every Scan or Window
  entry without a trigger, is relayable at any representable length.
- Refusing "without killing the relay" therefore does not exist as a correct state. The fix is to stop refusing
  relayable entries, and to keep the refusal (renamed, counted) only where the relay cannot relay the entry.
- INFERRED, out of scope: surfacing such a stop to the guest as a channel error rather than a silent stall would turn
  a TDR into an application error.

## 3. Options

| Option | Correctness | Hostile-guest bound | Worker rules | Complexity | Verdict |
|---|---|---|---|---|---|
| **A. Raise `MAX_SOURCE_BYTES`** to ≥ 8 MiB | Correct for this size class. | Staging object ≥ 9 MiB per relay instead of 1 MiB, times the runlist channel budget. INFERRED example: 64 relays ≈ 576 MiB of RO GPU-mapped memory, against 64 MiB. Every Scan/Window entry can force 8 MiB allocation + copy + scan **in one turn**. `PIECES` rises from 225 to 2049. | Worst-case turn ≈ 9× longer, which delays every other relay on that worker. | Trivial | **Rejected** (the owner already ruled it out). It also pays the cost in Pointer mode, which needs no bound. |
| **B. Complete mode from birth** (G1 option 1: stage every descriptor through N bounded segments) | Possible, but needs parser continuity, staging recycling and witness proofs everywhere. | CPU copy of **all** GPU command traffic. N × 917,504 staging per channel. | Many turns per entry, always. | High | **Rejected.** It contradicts §AI 2026-10-12 ("a malicious mutation of its own ordinary commands is not a reason to snapshot every pushbuffer"). It throws away the Pointer-mode gain (+97 % D3D11 in the 2026-10-16 lifecycle results, as reported in the project notes and not re-measured here), and it still ends at a ceiling. |
| **C. Split the native descriptor into ≤ 917,504-byte authored GP entries** (segment-wise continuation) | **Unsound in Pointer mode.** With no parser state, a cut can fall after `END_PB_SEGMENT`, so the suffix the original ignored gets executed (`user_scan.rs` 215, test `end_pb_segment_does_not_interpret_ignored_suffix`). A marker cannot go inside an open packet. | Fine | Fine | Medium | **Rejected.** It also has no benefit, because the hardware accepts the original length. |
| **D. Mode-scoped admission: forward intact** (G1 option 2) | Same semantics as today's small Pointer entries: the raw word, native execution in the channel's own restricted VAS, SYNC_WAIT and LEVEL bits untouched. | **Zero host cost per byte.** Per entry: one 8-byte load, O(1) work, no allocation sized by L. The GPU fetch is the guest's own channel time, in its own VAS (unmapped means a native fault on its own channel, as for any small entry today). | Unchanged | Small | **Chosen (Phase 1).** §AI 2026-10-12 is the determination G1 lacked: copying ordinary USER GP pointers needs no CPU interpretation while the channel stays USER, physical operands stay forbidden and its own VAS protects host resources. Size does not change the authority. |
| **E. Streamed classification through a fixed window** (Scan/Window only) | The forwarded word is unchanged. Only the CPU-side classification is chunked, with parser state carried **within** one descriptor. `END` stops reading further parts, which is exactly the hardware's view. | ≤ 917,504 bytes copied and scanned per turn (today's maximum). ≤ ⌈8,388,604 / 917,504⌉ = **10 parts** per attempt. Receipts (span lists without bytes) ≤ 10 × 225 per relay. One stream per relay. Relays are bounded by `VmCaps` runlist admission (`userquota.rs`). | One part per turn, `Need::Again` between parts (the existing internal ring, not a timer). | Medium | **Chosen (Phase 2).** |
| **F. Author a different host submission for big triggered segments** (guest-VA sub-spans plus a staged trigger, or staging recycled at witness-proven packet boundaries) | Plausible, but needs new proofs: cuts only at closed packets, before END, LEVEL on every piece, SYNC_WAIT on the first, and source receipts per original entry. | Bounded if designed | Multi-turn | High | **Deferred.** It is built only if the falsifier of H-E (§9) fires. |
| **G. Keep the refusal** | It kills relayable legal work. | — | — | None | Rejected as a disposition. It survives only as the named refusal above the staging ceiling. |

## 4. The chosen design

### 4.1 Invariants (each needs a test, see §5)

- **N1** A descriptor is refused for size only if relaying it requires host staging larger than staging. Pointer
  mode and control (NOP) entries are bounded by the hardware format alone.
- **N2** Pointer mode reads no pushbuffer byte and allocates nothing sized by L, for every L (this extends I1).
- **N3** The value stored in the private ring is the 8-byte value tested (I7, unchanged), and that value is the guest's
  word, never a split or rewritten form.
- **N4** No single snapshot exceeds `userread::MAX_BYTES` = 917,504, and no turn reads more than one snapshot of a streamed
  descriptor.
- **N5** A streamed descriptor is published only after every part up to `END` or the end of the descriptor has been scanned
  with no trigger. Bytes after `END` are never read. Parser state is committed only at publication.
- **N6** Publication of a streamed descriptor re-admits **every** part's source receipt (row identity plus RAM generation), together with
  the ring and GP guards, under the existing `publish_guarded` order. Any mismatch is `Retry`, and nothing is published.
- **N7** A `Retry` or a teardown during a stream drops the stream. The next turn re-plans the entry from `GP_PUT`, exactly as for an
  unstaged segment today (`userpump.rs` 2856).
- **N8** No completion, receipt or `GP_GET` credit is created by admission, by a part, or by a refusal. Doorbells and the internal
  `Need::Again` ring are wakes only. Nothing depends on time.

### 4.2 Phase 1: mode-scoped admission (fixes the measured case)

1. `kf_chan::user_stream`: add `pub fn inspect_format(entry: u64) -> Result<Source, Refusal>`. It is today's `inspect_entry`
   without the budget comparison: control NOP, direct C56F framing, and `end < 2^40`. Keep `inspect_entry(entry, max)` as
   `inspect_format` plus the budget, for the diagnostic shadow path (`userpump.rs` 2082, unchanged) and for tests.
2. `user_turn::plan_step`: call `inspect_format(self.raw)`. The format refusal text stays `USER_RING_GP_FORMAT Entry ...`.
3. `user_turn::plan_step`, `Plan::Scan` arm, **before** the gate: if `d.len_bytes > io.max_scan_bytes()`, then in
   **Phase 1** emit `Ev::ScanBudget`, count `refused_scan_budget` and return
   `Err("USER_RING_SCAN_BUDGET len=… max=… mode=… entry=… cursor=… put=…")`. Nothing is read and no gate is published. In
   **Phase 2**, set `stream_planned = true` instead.
4. Rename `Io::max_entry_bytes` to `Io::max_scan_bytes`, documented as the per-snapshot bound. It is consulted only on a Scan plan
   (this is a mechanical change to the four mocks).
5. `Storage::write_user_gp`: call `inspect_format(entry)`. Fix its doc comment: Pointer forwarding resolves no PB, by §AI.
   `write_gp_level` and `write_authored` keep their `GP_OFFSET` bounds, which are the real staging ceiling.
6. Measurement counters, which decide nothing: `pointer_large_entries` (forwarded with L > 917,504) and `pointer_entry_max_bytes` in
   the lifecycle counts and the `USER_RING_LIFECYCLE` line, plus `refused_scan_budget` and the mode at refusal in the refusal
   text.

### 4.3 Phase 2: streamed classification (Scan and Window)

New module `crates/kf-chan/src/user_chunk.rs`. It is pure, with no I/O, clock or lock.

```text
Stream { entry: u64, va: u64, total: u64, next: u64, part: u8, scan: Scan, cursor: PartCursor, ended: bool }
Stream::begin(desc, entry, &core_scan, max) -> Result<Stream, Refusal>   // requires total > max; parts = ceil(total/max) <= 10
Stream::next_read(&self) -> Option<(va: u64, bytes: u64)>             // fixed cuts at multiples of max; None once ended or exhausted
Stream::feed(&mut self, words, software) -> Result<Fed, StreamRefusal> // Fed::{More, Done}; StreamRefusal::{Scan(Refusal), StagedSpanBudget{at_word}}
Stream::into_scan(self) -> Scan                                       // committed by the caller at publication only
```

`Scan::part(&mut self, words, cursor: &mut PartCursor, software) -> Result<PartOutcome, Refusal>`, with
`PartOutcome::{Exhausted, Ended{at}, Trigger(Trigger)}`. Positions are relative to the descriptor. `PartCursor` carries the
word base and the absolute `packet_start` of a packet still open across a part cut. A one-datum software packet whose header
ends one part and whose datum starts the next is therefore a `Trigger`, as the whole-descriptor scan would report, **not** a
`SoftwarePacket` cross-segment refusal. `Scan::segment` is re-implemented as a single `part` over the whole words, so one
parser serves both paths. The existing `user_scan` tests are the regression proof.

The turn machine (`user_turn.rs`), with the same code the models drive:

```text
Plan::Scan & L > max --(gate as today)--> Gate::Proceed --> Phase::Part (Stream::begin)
Phase::Part: (va,bytes)=next_read; io.read_part(raw, va, bytes, part) -> words (pump keeps the receipt, drops the bytes)
     feed -> More  : need = Again; turn ends; next turn: Obs -> Wb -> Phase::Part   (no PUT read, no re-plan)
     feed -> Done  : Phase::PubStream
     StagedSpanBudget -> Err("USER_RING_STAGED_SPAN_BUDGET len=.. trigger_at=.. max=..")  (dead; nothing published, no action)
     Scan(r)       -> the existing REFUSE_RETRIES grace keyed by (generation, cursor): drop the stream, re-read from part 0
Phase::PubStream: credit check (2 slots, as publish_guest); io.publish_streamed(fetch, generation, next_get, raw)
     -> core.scan = stream.into_scan(); life.segment_scanned(carried); cursor = next_get; note_relayed(raw); unnotified = true
Retry anywhere / retire / switch_source: stream = None; io.drop_receipts()
```

- `observe_decision` and `Phase::Wb` treat `stream.is_some()` as they treat `segment_pending` today.
- `note_pb_read(permit, bytes, vram)` is called once per part. The `ScanPermit` from the `Plan::Scan` is held by the stream.

The pump (`userpump.rs`) adds the following:

- `read_part`: `snapshot_with(va, bytes)`, converts the bytes to words, then `snapshot.into_receipt()` and pushes it into
  `aux.receipts`. The receipt vector is `try_reserve_exact(10)` at stream begin, and allocation failure is a named refusal. `into_receipt`
  drops `bytes` and keeps `spans/va/end/rows/ram/generation`.
- `publish_streamed`: `publish_guarded(.., ring, gp=source, pbs=&receipts, ..)` with `write_user_gp(slot, raw)` plus `publish_put`.
- `userread::admit_all(receipts, publish)`. All receipts must share one `rows` (`Weak::ptr_eq`), one `ram` and one generation,
  otherwise the result is `Changed`. It takes `try_read` on rows once, re-resolves every receipt's spans, takes `try_read` on
  `ram.blocks` once, checks the generation, then publishes. It never blocks, and `Busy` or `Changed` becomes `Retry`.
- `user_ring_serve` `Retry` arm: drop an in-flight stream as it drops an unstaged segment.

### 4.4 Ceilings, and what happens above them

| Class | Ceiling | Above it |
|---|---|---|
| Pointer entry, control NOP | Hardware: 8,388,604 bytes, end < 2^40 (no larger value is encodable) | Not representable. A malformed word gets `USER_RING_GP_FORMAT Entry`, and the relay stops (unchanged). |
| Scan/Window entry, no trigger before END | Same hardware ceiling. ≤ 10 parts of ≤ 917,504 bytes, one per turn. | As above. |
| Scan/Window entry, trigger before END | **Staging: the whole patched copy ≤ 917,504 bytes** (`write_authored`, `GP_OFFSET`) | Phase 1 refuses every Scan/Window entry above 917,504 with `USER_RING_SCAN_BUDGET`. Phase 2 refuses only the triggered ones, with `USER_RING_STAGED_SPAN_BUDGET`. Both are counted, grant no credit and stop the relay. That stop is correct here: the relay cannot relay the entry, and skipping it would be a forgery. |

## 5. Tests (test-first; the EXPECTED RED commit holds tests only and is pushed alone)

The production path for GPU-free tests is the real `kf_chan::user_turn::Relay`, driven through its `Io` mock (the
same code `userpump` runs). Storage writes go through the real `Storage` (`anonymous_live_storage`). Snapshot and receipt
admission go through the real `userread` with its fake `Store` and rows. The twelve run-3440 words go in one shared fixture constant.

**EXPECTED RED, Phase 1** (each fails on `03715b7d3`):

1. `pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer`. Twelve words. Each turn yields `Call::Entry(raw)`
   bit-exact, no `read_pb`, `refused_entry == 0`, `!dead`, `pointer_pb_read_violation == 0`.
2. `a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback`. The entry is forwarded at a `GP_PUT` boundary,
   `end_batch` publishes a marker, the marker is processed, and the guest `GP_GET` is written back as cursor + 1. This is the GPU-free meaning
   of "completes". GPU execution is a hardware question (§6).
3. `pointer_forwards_the_largest_representable_descriptor` (8,388,604 bytes, end < 2^40).
4. `write_user_gp_stores_each_run3440_descriptor` and `write_user_gp_stores_the_largest_representable_descriptor` (real `Storage`).
5. `scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate` and the same test for a seeded Fresh Window. The name
   and the absence of `publish_gate`/`read_pb` are asserted. They are RED on the name today.
6. Lifecycle props and stress oracles (`user_lifecycle_props.rs` 1086/1129, `user_lifecycle_stress.rs` 558) become mode-aware.
   Pointer publications are checked with `inspect_format` and Scan publications with `inspect_format` plus L ≤ max. They are RED today,
   because a bit flip that only enlarges the length is refused. This changes the contract and must not weaken the oracle.
7. Existing rows that move: the `MAX_SOURCE_BYTES + 4` rows of `bad_user_descriptor_cannot_modify_private_ring_or_userd` and
   `shadow_writer_preserves_opaque_bits_…` (`user_storage.rs` 842-900). The shadow row stays refused by the shadow path's own
   `inspect_entry(.., MAX)`. The bad-descriptor row is replaced by `end == 2^40`. Record this in the RED commit message.

**Hostile cases, Phase 1** (green before and after unless noted):

- One dword changes between Lo and Hi, giving a large length: the stored value equals the tested one (extend `a4_*` with lengths > max).
- Reserved bits 0, 1 and 40, non-NOP control opcodes, `end == 2^40`: still refused as `GP_FORMAT Entry`.
- A ring of 32,768 maximum-length descriptors: per-entry `Io` call count is constant, `read_pb` is never called, and nothing is allocated from L.
- A large SYNC_WAIT word in Pointer mode is recorded as `outstanding` and seeds the next Fresh Window gate (A1 unchanged).

**EXPECTED RED, Phase 2:**

- S1 A 1,026,596-byte Window descriptor is streamed in **two turns** (917,504 then 109,092 bytes) and forwarded unchanged. The committed
  `Scan` equals the whole-descriptor scan of the same words.
- S2 Property: random valid word streams (packets of every form, SET_OBJECT, mask ops, END, triggers, carried-in packets),
  cut at random points and at the fixed 917,504 cut. `part` outcomes, absolute trigger positions and the committed state must equal `segment`.
- S3 END in part 0 and trigger-shaped words in part 1: forwarded, exactly one read.
- S4 A trigger in part 1 gives `USER_RING_STAGED_SPAN_BUDGET`, no `Call::Entry`, no software action, `dead`, counted.
- S5 A software-packet header in the last word of part 0 and its datum in part 1 is reported as a trigger (then S4), not as `SoftwarePacket`.
- S6 A packet carried in from the previous descriptor continues through the parts. A stream that ends in an open packet carries it out to the next descriptor.
- S7 Rows replaced between parts, or between the last part and publication, give `Retry`: nothing is published and the stream restarts from part 0
  on the next turn (`userread` fake rows).
- S8 `admit_all` refuses receipts with mixed rows, RAM or generation.
- S9 One part per turn, each read ≤ 917,504 bytes, `Need::Again` between parts, and no `GP_PUT` read in a continuing turn.
- S10 A maximum-length descriptor takes ≤ 10 parts. Receipt reservation failure is refused by name.
- S11 A scan refusal in part k uses the existing grace and re-reads from part 0 with the **uncommitted** state.
- S12 A SYNC_WAIT streamed descriptor publishes its gate before part 0 is read (I3).
- S13 `retire` or `switch_source` mid-stream publishes nothing, and no receipt survives.
- Equality: for every corpus and soak input with all L ≤ 917,504, the `Io` action sequence is identical before and after
  Phase 1 and Phase 2 (`corpus_*`, `soak_*`, `g2_*` unchanged and green).

**Mutants** (hand-applied and restored; each must be killed by ≥ 2 independent tests):

| Mutant | Killers |
|---|---|
| Pointer admission keeps the staging budget | 1, 2, 3 |
| `write_user_gp` keeps the staging budget | 4 (both) |
| Format check skipped in Pointer | `an_entry_inspect_entry_refuses_…`, a4 bit-flip prop, `end == 2^40` hostile test |
| `write_user_gp` format check removed | `bad_user_descriptor_…`, `write_user_gp_refuses_end_at_2_40` |
| Scan path reads a snapshot > max | 5, mock assertion `read_pb.len ≤ max`, `userread` Bounds test |
| Scan budget checked after the gate | 5 (Window variant), S12 |
| Pointer reads PB for large L | 1, ring-of-32,768 hostile test |
| Large-entry counter changes behaviour | 2, equality test |
| Pending packet forgotten at a part cut | S2, S5, S6 |
| Reading continues after END | S2, S3 |
| Triggered stream forwarded | S4, S2 |
| Only the last receipt admitted | S7, S8 |
| Whole descriptor read in one turn | S1, S9 |
| Scan state committed per part | S11, S7 |
| Stream kept on `Retry` | S7, S11 |

**Lints and unsafe:**

- No `unsafe` anywhere.
- The `kf-qemu` changes sit under `chan.rs`'s non-test deny set (`arithmetic_side_effects`, `indexing_slicing`, `unwrap_used`,
  `expect_used`, `panic`, `unreachable`).
- `user_chunk.rs` opens with the same `#![cfg_attr(not(test), deny(…))]`. New or changed code in `user_stream`, `user_scan` and
  `user_turn` uses `checked_*` and `.get()` only. In particular, `Scan::part` must not inherit `segment_probed`'s `words[i]`.
- Run clippy with the identical command for the baseline and the final state. Zero new diagnostics.
- Local cargo: `flock -w 600 /tmp/claude-0/kf-local-cargo.lock nice -n 19 cargo … -j2`, with a private target dir that is deleted afterwards.

## 6. Hardware: what only hardware confirms, and the regression proof

**Only hardware can confirm (INFERRED until run):**

1. The original descriptors execute natively and to completion when forwarded intact.
2. catia-07 subtest `03_ThreeLoftJetsDarkMirror` survives on the run-3440 configuration (same image, driver and viewset,
   `KF3_PUT_CACHE`=on, `KF3_LAZY_OBSERVE=16`, fresh VM, the whole catia viewset in order, then isolated 03 three times).

**Pass criteria for that run:**

- `pointer_large_entries > 0` and `pointer_entry_max_bytes ≥ 947,340`. This proves the case was exercised. If they are zero, the run proves nothing.
- Zero `USER_RING_GP_FORMAT Budget`, `USER_RING_SCAN_BUDGET` and `USER_RING_STAGED_SPAN_BUDGET`.
- `poisoned` unchanged by user relays, and markers after the large entries processed.
- catia 03 completes, with no event 153 or 4101 in that window and no new host Xid.
- The seed-5-style reset that had no Budget refusal is tracked separately (T1 H1/H2). It is not part of this fix.

**Regression for Heaven, Minecraft and Steel Nomad.** CODE READ: Phase 1 changes behaviour only for L > 917,504, which today is
terminal. A workload that runs today has made no such entry, or it would have logged `USER_RING_GP_FORMAT Budget` and lost a relay.

**The proof:**

- (a) MEASURE that precondition by grepping those runs' kept logs for `GP_FORMAT Budget`. This was not done here.
- (b) Run the GPU-free equality test above.
- (c) Same box, old and new binary: the same lanes, with fps inside run-to-run variance and the `USER_RING_LIFECYCLE` counters
  (`entries_pointer`, markers, `to_window_*`, `pointer_large_entries`) in the same order.
- (d) `v3_gates` 9/9, `fast_suite` 30/30, and the Linux broker lane. Linux Passthrough has no user-ring relay, but the rule applies.

Phase 2 changes behaviour only for Scan/Window entries above 917,504, which are also terminal today. It gets the same proof.

## 7. Hypotheses and falsifiers (stated before any run)

- **H-A (CODE READ, test to MEASURE):** Pointer mode reads and stages no PB bytes, so the budget protects no host resource there.
  **Falsifier:** any Pointer path that calls `read_pb`, `snapshot` or `write_authored`, or a nonzero `pointer_pb_read_violation` (tests 1 and 3).
- **H-B (INFERRED):** forwarding the 0.95–1.03 MB descriptors intact is what native hardware runs, as under VFIO.
  **Falsifier:** with the descriptors forwarded (`pointer_large_entries > 0`), a PBDMA or MMU fault or Xid tied to that channel, or markers
  behind them never processed.
- **H-C (INFERRED, T1's leading cause):** the Budget refusal causes the catia app kills.
  **Falsifier:** catia 03 still fails with zero refusals of any budget name. Then pursue T1 H1/H2 with T1's diagnostics.
- **H-D (INFERRED):** all twelve refusals were in Pointer mode.
  **Falsifier:** a `USER_RING_SCAN_BUDGET` under Phase 1 in the catia run. Phase 2 then becomes required for catia, not just for completeness.
- **H-E (INFERRED):** production workloads do not put a registered trigger inside a segment above 917,504 bytes.
  **Falsifier:** `USER_RING_STAGED_SPAN_BUDGET > 0` in any lane. Option F then becomes required, with its own design review.

**Measurements missing (none of them blocks the decision):**

- The mode at eleven of the twelve refusals (H-D).
- Big segments in Scan/Window and their trigger content (H-E).
- (a) above.
- A matched VFIO catia run (still unavailable).

## 8. Order of work

1. Phase 1 RED tests, pushed alone.
2. Phase 1 green, with mutants and clippy.
3. Hardware run (§6): Phase 1 is enough for the measured case if H-D holds.
4. Phase 2 RED tests, then green, mutants and clippy, plus the equality proof.

No README edit, no boot flag and no constant change.
