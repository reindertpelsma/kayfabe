**STATUS: LIVE, 2026-11-09. CODE READ hostile-guest review of the UR1 Phase 1 port; GPU-free verification is recorded below. Phase 2 remains DESIGN-ONLY.**

# USER descriptor admission on next

CODE READ: authority is [the 2026-10-22 decision](V3_USER_DESCRIPTOR_DECISION_20261022.md).
This port separates C56F format admission from CPU copying. Pointer forwards the captured
8-byte descriptor unchanged, without resolving its PB. Scan and Window retain the
917,504-byte snapshot/staging ceiling and stop before a read or gate above it.
No descriptor is split, no limit is raised and admission creates no completion.

## Evidence and limits

MEASURED BY THE COORDINATOR, 2026-11-08, not rerun by UR1: `/root/night/iter2/NOTES.md`
and `/root/night/iter2/evidence/` record AD102, kf-iter1, `8d495e90` versus `5fcf9458`
(the latter is `8d495e90` plus `cf79374e3`, with mechanical test/doc conflict resolution).
The two next runs fail catia-07 subtests 03 and 06 with Budget refusals around 1.02 MB
and 0.98 MB against 917,504 bytes, then RM unloads and guest 153/141/193 events.
Both candidate runs pass all 6 subtests with zero refusals, RM unloads, guest events
and host Xids. Each candidate run records 4,799 Pointer entries above the old bound,
maximum 1,042,740 bytes, and zero `refused_scan_budget`.
These local evidence paths are operator records, not inputs required to build or test.

INFERRED: these runs support the intervention on that die/configuration; they do not
qualify this final port. One die, one RAM/FB configuration and one driver combination
were exercised. UR1 runs no hardware, VM, enscape, SPEC stage or Windows gate lane.
The historical [Phase 1 proof](../../traces/descriptor_bound_20261018/phase1.md) names
its own revision and measurements; the new checks below do not inherit its green status.

## Conflict and test decisions

CODE READ: the historical `run3440.rs` fixture is restored because both real Storage
and production Relay tests consume its twelve public interface words. It holds no PB
bytes or captured per-die constants. Its content is an adversarial input corpus, never
an admission rule. The old `V3_USER_DESCRIPTOR_BOUND_20261022.md` stays absent: it
recommended continuation/complete-from-birth approaches explicitly rejected by the
authoritative decision. Its analysis is preserved in git at `468a35c9d`, already marked
superseded there. Restoring it as a current design would misstate the selected contract.
The existing next mock's native reference observation is retained.

CODE READ: the reconciliation retains format validity, tested/stored equality, bounded
work, mapping/source guards and real-credit assertions:

- Storage's over-copy-bound refusal cases become hardware end-address refusal cases;
  larger legal Pointer words are checked for exact storage and unchanged GET/reference.
- The lifecycle fixture decoder identifies format-valid entries independently of copy size.
  Otherwise it would mislabel a legal large word as a torn capture.
- Property bit-flips check every one of 384 words against the hardware ceiling: 30 invalid,
  354 valid. Random hostile Pointer words must still pass format validation.
- Stress checks require hardware format for every publication and additionally retain the
  snapshot bound for scanned publications. Tested/stored equality remains mandatory.
- The long small-entry I/O golden omits only the three new observation fields; every old
  counter, action and state stays in its hash. The old golden value is retained.

## Hostile-guest size/count inventory

CODE READ: this table covers the production USER relay paths affected by admission,
including diagnostic reads and authored emissions. GPU length is bounded by format;
CPU read/emit bounds remain independent. A guest-reachable unbounded CPU read or emit
would be a security bug. **No such path was found in this review.** This is source
review and bounded GPU-free testing, not a proof of GPU fault/reset containment.

| Guest-controlled input / operation | Read, copy, stage, forward or emit site | Bound and retained authority |
|---|---|---|
| Declared GP base/count | `userpump::user_ring_serve`, `userread::mapped` | Power of two, 2..2^20 entries; 8 MiB ring; 8-byte alignment; checked end strictly below 2^40; at most 2,049 placement/RAM pieces; topology generation and row identity re-admitted. Whole ring mapping validation does not copy the ring. |
| GP_PUT and cursor/slot | `user_turn::put_step`, `PumpIo::gp_put/load_dword` | PUT < admitted entries; cursor wraps by entries-1; checked slot*8+base; exactly 4 USERD bytes and one owned 8-byte GP snapshot, with both dwords from that snapshot. |
| GP descriptor length/base/control | `user_stream::inspect_format`, `Relay::plan_step` | Direct C56F supported bits, maximum (2^21-1)*4 = 8,388,604 bytes, checked end < 2^40; only supported CONTROL NOP encodings. Malformed entries stop without publication. |
| Pointer PB length | `Plan::Forward`, `PumpIo::publish_entry`, `Storage::write_user_gp` | No PB read, copy, stage or allocation by length; one intact 8-byte word; format rechecked in Storage; slot < 512; private Fetch ring credit retained. USER privilege, deny-physical and own restricted VAS remain the GPU access boundary (§AI). |
| Scan/Window PB length | `Relay::plan_step` Scan arm | `max_scan_bytes` = 917,504; checked before gate or `read_pb`, including Pointer-to-Window transitions. Oversize refusal grants no GET or native receipt. |
| PB snapshot bytes/pieces | `PumpIo::read_pb`, `snapshot_with`, `userread::snapshot/resolve` | 1..917,504 bytes; at most 225 placement pieces and RAM pieces; checked additions and backing lengths; allocation failure refuses; row identity/epoch and birth RAM generation checked before copying and again at publication. CPU views validate offsets inside the unsafe perimeter. |
| Owned scan words and patched copy | `PumpIo::stage`, `stage_plan`, `user_scan::Scan::segment` | At most 229,376 u32s from bounded snapshot; patched copy has identical length. Trigger replacement fills validated slices, never expands them. Packet count is a public method-header field and consumption is constrained by supplied words; carried state contains metadata only. |
| Trigger walk / suffix positions | `stage_plan`, `segment_turn`, `suffix_after_trigger`, `gapless_remainder` | Checked slice positions and checked multiplication; trigger offsets advance within the same bounded snapshot. A partial unsupported suffix refuses. END stops interpretation; no descriptor splitting is introduced. |
| Staged bytes and authored GP spans | `Storage::write_authored/write_gp_level`, `segment_turn` | Checked offset+bytes <= GP_OFFSET = 917,504; 4-byte alignment; authored GP nonempty and slot < 512. Entire patched copy must fit before any representing span is published. |
| Native GP publication/count | `Fetch::new/available/publish`, `Storage::publish_put` | Private ring power of two, 4..512; spare slot retained; Pointer/scanned forwarding reserves marker credit, trigger prefix reserves 2 or 3 slots. Each emission is one bounded word/span/barrier/marker; no guest-count loop emits an unchecked number of entries. |
| Native GET / REFERENCE and guest GET | `Fetch::observe`, `Marks`, `PumpIo::hw_words/write_guest_get` | Native indices/counts validated against issued ring credit and tags. Receipt attribution, not admission or a timer, drives writeback; one 4-byte guest GET store under channel/allocation guards. |
| Software trigger data | `segment_turn`, `software`, registry and source admission | Channel-owned software identity and registered bundle lookup; host action authored from validated verbs. Raw PB data never grants a host address or privileged verb. Trigger count <= bounded snapshot words; execution waits for real ordered witness. |
| Shadow observation | `shadow_step`, `inspect_entry`, `usershadow::charge/decode_words` | Separate diagnostic path retains `inspect_entry(..., MAX_BYTES)`, 917,504-byte snapshots, 4 MiB per-channel observation charge, bounded samples; opaque shadow publication has its existing own-USER eligibility and slot guards. |
| Trace/re-read sizes | `trace_segment`, `trace_rereads`, snapshot/trace helpers | Derived only from already bounded scanned entries; rereads still pass snapshot bounds. Fixed trace history/sample/straddle/verify caps remain; Pointer path does not create a PB trace. |
| Number/lifetime of relay allocations | `NativeQuota::reserve/attach/release_confirmed`, `VmCaps` | Runlist budget charged before native allocation; uncertain attempts retain charge; token retirement alone grants no release. Each Storage remains 1 MiB; guest length cannot grow it. |
| Observation counters | `Lifecycle::entry`, `Counts::since`, `LifecycleStats::add` | No authority or admission decisions. Large count is additive; maximum length is aggregated with fetch_max, never summed; refusal count cannot forge progress. |

## Preregistered checks

INFERRED H1: the global budget is the reason legal large Pointer words fail on next.
Falsifier: the imported expected-red tests pass unchanged next, or the port still refuses
those words before publication. Test results are recorded below, including failures.

INFERRED H2: the reported `user_loop_model_v2` SIGSEGV predates this port.
Falsifier: unchanged origin/next passes and only the port crashes under the same command
and build configuration. Each source variant uses its own target directory.

INFERRED H3: changing admission does not alter small-entry decisions or I/O.
Falsifier: the retained legacy I/O golden changes, or existing safety/model tests fail
for a reason other than the explicitly reconciled admission contract.

## Verification

MEASURED: pending local runs. This section will record exact counts, mutant killers,
clippy/fmt and claim/doc/privacy/UUID gate results before the draft PR is opened.
