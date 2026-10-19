# Exact protected dependencies, 2026-10-19

STATUS: LIVE. STOP for these two narrow topics; independent work continues.

2026-10-19 APPROVAL supersedes the two pending STOPs below. The scrub fixture
gets only None (94518f95b). Heartbeat is an isolated failing-first series with
nonblocking clock/metadata access and unchanged500ms; see HEARTBEAT_PLAN.md.
No excluded USER measurement/batching edit is restored. Five held historical
heartbeat source commits are represented by this new reviewed topic, not blindly
reapplied: drop the old blocking lock/latest/clone implementation and command_seq
freshness heuristic; replace them with try-lock borrowed reads and a per-life
fn72-ready bit. The old diagnostic PUT-ahead field remains excluded.


1. Heartbeat epoch topic: the original file-level audit missed that Device's
   existing GSP drainer contains the publication hunk. The import briefly
   reached 30a43bc9d and has now been removed in a dedicated corrective commit.
   The product again uses candidate-5 heartbeat publication and console wiring.
   Unapplied heartbeat_drainer_PROPOSAL.patch contains the exact dependency:
   Device's bounded fn72 cell at realize, FSM wiring, and existing 500ms drainer
   heartbeat conversion using the validated native timer and guest offset, plus
   the isolated helper/tests, GOP comment and host timer helper. No interval,
   doorbell, IRQ, worker scheduling or guest permission change is requested.
   Source commits held: 787f55728,277e5d895,60ad041ac,304ca9adb,a954ac468
   (aliases 68363c35e,086ed1183,deff13df8,e0b4c28e6,ac01b7bd3).
   All their dependent Device state/call sites, helper/module, host timer method
   and GOP correction are held, not partially activated. The native timer is
   already a production default in candidate-5 (field doc says opt-in but its
   constructor explicitly opens the unprivileged view without an opt-in).

2. Blackwell address scanout extends LatchedDmas's cache key from three to four
   fields. A current candidate-5 life-reset/scrub test directly initializes that
   key and fails compilation (expected4tuple,found3tuple). Exact required edit:
   other.latched.by_window[3] = Some(((1,2,3,None),dma));
   only the test fixture gains None for the optional surface address. Production
   reset/scrub code and every existing assertion remain unchanged. Proposal:
   scrub_test_address_key_PROPOSAL.patch. This is held rather than rewriting
   the protected test or changing cache design to evade the scope boundary.

The full integrated release batch at 30a43bc9d passed 13 crates; kf-qemu failed
only to compile that existing fixture. These are checkpoint results, not Phase1
acceptance or hardware authorization. Imported constants notice bounds and
other independent tests/mutations continue while the scope decisions are pending.

# Applied integration ledger, 2026-10-19

STATUS: LIVE. Candidate-5 f8f9d1a5f merged intact at 67208ea0f.
GPU-types code/tests imported in 35cd6d276, 797ca55eb, e634e495b and 668032fb7;
source executable modes restored in 939d31382. No historical evidence tree imported.
The main-stack and separate draft-topic records are pinned in stack_imports.json
and stack_extras_imports.json; source-to-new-commit mappings retain original
test/fix order. Historical red commits were imported, not claimed as newly
executed red runs. Fresh integrated validation is in progress.

Applied conflict dispositions:

- device.rs channel-budget formatting collision: preserve candidate-5 exactly;
  import only the independent GSP model selector and display die argument.
- mem.rs reset/scrub formatting collision: preserve candidate-5 byte for byte.
- user_loop, user_loop_model and userpump: exclude all old batching/dependencies
  as already approved; preserve candidate-5 byte for byte. No PUT-ahead field.
- Other nine formatting-only manifest paths: preserve current integration files.
- display.rs appended test modules: keep all candidate-5 diagnostic/scrub modules
  verbatim and append address_scan_tests after them; no pacing/scrub edit.
- caps_measured.tsv independent GB205 595 and 580 row appends: retain existing
  AD104/AD102/TU106/GB206 rows; add only new keyed measured rows, refusing a
  duplicate key with different content. No per-branch or neighbouring-die inference.
- kf-chip FSP test: keep the original BLACKWELL compatibility-row assertion
  verbatim and add the per-die selector/header assertion. Neither test dropped.
- Old captured caps probe: remove from runtime as requested by gpu-types; retain
  the original current test plus its captured oracle exclusively under cfg(test).
- Tier-2 and design include_str dependencies: project only the five consumed
  columns/eight manifest lines into test fixtures; do not copy evidence trees.
- Imported tooling hygiene: remove one obsolete private-path comment, normalize
  trailing whitespace/batch-file line endings, preserve source executable modes.
  The historical owner-host image relay is not run; host remains off-limits.

The C5 protected-file hash receipt records byte-identical chan.rs, userpump,
userbirth/userquota/userstore, mem/reset/scrub/bootclean and excluded USER files.
The TSG mirror branch is untouched. F-2 and family gate remain the only approved
protected product topics. Ada admission stays exactly c56f (c36f/c46f refused).

# Family topic verification, 2026-10-19

STATUS: LIVE. Narrow family approval completed: EXPECTED RED 2fb8aa503,
fix 871c38fad. Release kf-qemu 312 -> 320 passing, no dropped tests; mutants
3/2/4/2 kills, clippy 451 -> 451 added=0. Ada c36f/c46f remain refused;
exact accepted set c56f is exhaustively pinned. See USERRELAY_RESULT.md.

# Family admission approval, 2026-10-19 11:55 coordinator

STATUS: LIVE. The USER relay STOP below is superseded by narrow approval for
source_declaration, its Birth::run call site and tests in userrelay.rs only.
Generated Ada extras c36f and c46f are deliberately refused; the accepted Ada
set stays exactly c56f. See USERRELAY_PLAN.md. No native-birth widening.
The TSG mirror branch c2685412e is not imported or edited; groups/latejoin/
birth_group remain protected. Candidate-5 f8f9d1a5f will be merged before hardware.

# Current approval and verification update, 2026-10-19

STATUS: LIVE. F-2 APPROVED and VERIFIED for exactly grtables.rs and tmode.rs.

The coordinator approved the independent public-header class-width topic as a
separate test-first/fix topic. EXPECTED RED commit `08424cf11` adds nine tests
and no production change; three wide-address tests fail and six boundary/default
tests pass. The focused fix passes nine tests, four mutants are killed by
3/2/2/3 tests, and clippy has 217 baseline / 217 fixed diagnostics (added=0).
The measured full kf-chan baseline is 521 passed / 0 failed / 3 ignored; the
full fixed run passed 530 / 0 / 3 (fix commit 6a32d4e8e). See F2_PLAN.md, f2_clippy.json and
f2_mutations.json. The F-2 request below is superseded, not still awaiting approval.

A separate retained Turing topic requires userrelay.rs: current declaration
validation accepts only AMPERE_CHANNEL_GPFIFO_A. Exact dependency and the
bounded unapplied proposal are in USERRELAY_DEPENDENCY.md and
userrelay_family_gate_PROPOSAL.patch. The latest coordinator explicitly leaves
userrelay protected, so that edit is STOPPED and an asynchronous narrow scope
question is pending. No userrelay/birth/group/doorbell/lifecycle code was edited.

The broader stack audit found only that family topic among the retained,
non-F2 protected source paths examined. Candidate-5 mem.rs reset and device.rs
budget raw-merge conflicts are incidental old formatting, not required semantic
changes; retain current files/hunks there. No candidate-5 merge is applied yet.

The earlier ledger and exact source/hunk inventories below remain historical
records; their superseded F-2 STOP does not undo the new two-file approval.

# Current scope decision and exclusion ledger

STATUS: LIVE, 2026-10-18. Original exclusion APPROVED; separate F-2 protected edit STOP.

## Applied authority, not a product edit

OWNER/COORDINATOR DECISION, 2026-10-18: exclude the obsolete USER relay batching
and measurement topic, including dependent nonconflicting edits; retain current
integration relay/lifecycle/doorbell code and ALL current candidate-4 tests.
The old behavior is not ported. Original conflict winners are now EXCLUDE OLD
TOPIC / PRESERVE CURRENT, by explicit authorization. The historical pre-approval
hunk snapshot is retained below and no longer represents an unresolved request.
No merge result or product/test change has been applied while preparing the
exclusion audit. Every current test is still byte-identical to candidate 4.

| Explicitly excluded commit | Topic | Reason |
|---|---|---|
| `24e7a297a6b9f18d4de359f4e55b0e51d58ea209` | Flow/stage counters and timers | Approved exclusion; retain current lifecycle/lazy diagnostics |
| `639acf57d0550ac9638dd87205657c489dd2250a` | Histograms and stress flood mode | Approved exclusion; relies on excluded instrumentation |
| `7476ef69f218e352be8c8a2ed7e8d8e249d6303c` | EXPECTED RED batching tests | Approved exclusion of incoming topic only; no current test dropped |
| `edcaaca67a10ecbf64169e017e461041c02c958b` | Claim batching/deferred host bell | Approved exclusion; current relay/doorbells retained |
| `29eae05a46061bcbfd5bfe67d72b7fa3039ad774` | Merge of old event-loop topic | Approved exclusion including dependent edits, enumerated below |

Other commits arriving only through that excluded merge are documented in full
in `exclusion_inventory.json`: `a4027e16e`, `8f4610bc5`, `bd079a8df` are historical
batching documentation; `d5be61ed1` and `6c2cb4371` introduce and explain the
independent userread span-reservation optimization. Their merge-contained edits
are held outside the import too. The userread optimization is **not** claimed
to depend on batching and is not silently smuggled into the die integration.


| Additional commit held with the excluded merge | Reason |
|---|---|
| `a4027e16eb9369634d21c84f0d42becef81df5a3` | Historical batching design/results/handoff; not imported as product code. |
| `d5be61ed11b87043f7df651f5b0defbbc4516bf2` | Independent span-reservation change/comment carried only by excluded merge; deferred, not a batching dependency. |
| `6c2cb43717584947c85f0609b4c24f5345d5d56d` | Independent span-reservation change/comment carried only by excluded merge; deferred, not a batching dependency. |
| `8f4610bc515268534fdcb7ec510c341ee5689d46` | Historical batching design/results/handoff; not imported as product code. |
| `bd079a8df979ce2be3ec68a7450b96dfe79ababb` | Historical batching design/results/handoff; not imported as product code. |

The old-only `backstop_put_ahead` field, its `put_ahead` formatting argument and
call site, stage timing fields/methods, flow/histogram counters and call sites,
`chain_more` helper, chained model, deferred-bell state and dependent bookkeeping
are all excluded. Nonconflicting hunks of the excluded topic are not carried.
This concerns incoming old code; no current diagnostic or test is removed.

## New exact STOP dependency: retained F-2 class-width rule

CODE READ: commit `3970937f6f417d2779b5ffbb63a0b8fa9c382224` is an independent
per-class ABI topic in gpu-types, **not** part of the approved excluded topic.
Its entire product edit is 97 additions / 4 deletions in two protected files:

| File | Required change | Why it triggers the new condition |
|---|---|---|
| `crates/kf-chan/src/grtables.rs` | Add `address_upper_bits` / `reauthor_in_class`, class-aware upper-address validation and two tests | Changes kf-chan, explicitly named in approval condition 2 |
| `crates/kf-chan/src/tmode.rs` | `gr_write` calls `reauthor_in_class(Some(class), ...)` instead of `reauthor(...)` | Required caller dependency; changing the table alone leaves execution on the narrow path |

Current source allows only 8 upper-address bits. The incoming rule admits
25 bits for class `0xCB97` and `0xCDC0`; others retain the narrow default.
CODE READ confirmation: public ogkm 580.65.06 `clcb97.h` lines 42/3913 declare
24:0, and ogkm 595.91.07 `clcdc0.h` lines 39/116 declare 24:0. The 580.65.06
`clcdc0.h` is an id-only header, so no width is inferred from that older stub.
No hardware test was run and no missing-address failure is asserted for a
specific sweep cell. The exact dependency is ABI rule -> `gr_write` authoring
in kf-chan, not an inferred dependency on old USER batching.

STOP under coordinator condition 2: a retained ABI topic needs a kf-chan change.
No F-2 hunk was applied. No other protected area is implicitly authorized.
Recommendation: authorize only this two-file F-2 topic, with the current narrow
behavior preserved for every other class, current tests retained, failing-first
coverage and >=2 killing tests before acceptance. Alternatively explicitly defer
F-2; do not silently drop it from the requested gpu-types code integration.

This approval would not authorize USER relay, birth/groups/latejoin,
doorbell/drainer, display pacing or boot-layer scrub changes. Any such new
required dependency still stops the lane. The two affected candidate-4 blobs
are `497939a6ff4c325efa8c840896615f62f35dbc70` (grtables) and
`1b05d02b75c70c158e7bec676a69bc7d152f1262` (tmode), both unchanged locally.

## Complete dropped-edit inventory (CODE READ)

The rows below enumerate every hunk introduced by excluded merge
`29eae05a4` relative to its first parent. Every row is excluded from the
planned import; none is a deletion from candidate 4. Hunk numbering and hashes
in `exclusion_inventory.json` reproduce the exact old source without importing it.
Other incidental source differences are not automatically included or discarded.

| Source path | Every dropped hunk (old/new ranges, zero context) | Reason |
|---|---|---|
| `crates/kf-chan/src/user_loop.rs` | H1 `-166,0 +167,22` (+22 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H2 `-209,0 +232,69` (+69 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H3 `-216,0 +308,35` (+35 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H4 `-226,0 +353,12` (+12 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H5 `-228 +366` (+1 / -1) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H6 `-243,0 +382,7` (+7 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/src/user_loop.rs` | H7 `-251,0 +397,11` (+11 / -0) | Old flow/stage/PUT-ahead diagnostics and claim-batching helper/tests; no port authorized. |
| `crates/kf-chan/tests/user_loop_model.rs` | H1 `-152,0 +153,2` (+2 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H2 `-195,0 +198,24` (+24 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H3 `-213,0 +240,3` (+3 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H4 `-330,0 +360,14` (+14 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H5 `-390,0 +434` (+1 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H6 `-432,0 +477,4` (+4 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H7 `-487,0 +536,7` (+7 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H8 `-504,0 +560` (+1 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H9 `-610,0 +667,6` (+6 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H10 `-616 +678` (+1 / -1) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H11 `-779,0 +842,14` (+14 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H12 `-1058,0 +1135,4` (+4 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H13 `-1091,2 +1171,8` (+8 / -2) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H14 `-1234,0 +1321,8` (+8 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H15 `-1248,2 +1342,12` (+12 / -2) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H16 `-1263 +1367` (+1 / -1) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H17 `-1346,0 +1451,3` (+3 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H18 `-1379,0 +1487` (+1 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H19 `-1407,0 +1516,7` (+7 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H20 `-1428 +1543,5` (+5 / -1) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H21 `-1987,0 +2107,2` (+2 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H22 `-1988,0 +2110` (+1 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H23 `-2003,0 +2126,28` (+28 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H24 `-2021,0 +2172` (+1 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H25 `-2022,0 +2174,7` (+7 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-chan/tests/user_loop_model.rs` | H26 `-2405,0 +2564,173` (+173 / -0) | Old claim-batching model, final-bell mutant and associated tests; current model/tests preserved. |
| `crates/kf-harness/src/bin/kf-syncwait-stress.rs` | H1 `-716,0 +717,35` (+35 / -0) | Histogram/flood instrumentation dependent on the excluded old measurement topic. |
| `crates/kf-qemu/src/userpump.rs` | H1 `-434,0 +435,14` (+14 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H2 `-451,0 +466,2` (+2 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H3 `-501,0 +518,5` (+5 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H4 `-1260 +1281,41` (+41 / -1) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H5 `-1285,12 +1346,16` (+16 / -12) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H6 `-1298,16 +1363,14` (+14 / -16) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H7 `-1315 +1378,11` (+11 / -1) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H8 `-1317,2 +1389,0` (+0 / -2) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H9 `-1328,17 +1399,30` (+30 / -17) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H10 `-1350,0 +1435` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H11 `-1375,0 +1461` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H12 `-1376,0 +1463` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H13 `-1412,0 +1500` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H14 `-1433,0 +1522` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H15 `-1519,0 +1609` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H16 `-1524,0 +1615` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H17 `-1530,0 +1622,2` (+2 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H18 `-1542 +1635,4` (+4 / -1) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H19 `-1599,0 +1696` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H20 `-1627,0 +1725,7` (+7 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H21 `-1865,0 +1970` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H22 `-1925 +2030,19` (+19 / -1) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H23 `-1927 +2050,3` (+3 / -1) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H24 `-2014,0 +2140` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H25 `-2096,0 +2223,4` (+4 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H26 `-2479,0 +2610` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userpump.rs` | H27 `-2530,0 +2662` (+1 / -0) | All old counter/timer call sites, chained-turn state/control flow, deferred doorbell and dependent bookkeeping excluded together. |
| `crates/kf-qemu/src/userread.rs` | H1 `-199,0 +200,2` (+2 / -0) | Span-reservation optimization and its test/comment arrived through excluded merge; independent of die rules, deferred with that merge, not described as a batching dependency. |
| `crates/kf-qemu/src/userread.rs` | H2 `-201,3 +202,0` (+0 / -3) | Span-reservation optimization and its test/comment arrived through excluded merge; independent of die rules, deferred with that merge, not described as a batching dependency. |
| `crates/kf-qemu/src/userread.rs` | H3 `-208,0 +208` (+1 / -0) | Span-reservation optimization and its test/comment arrived through excluded merge; independent of die rules, deferred with that merge, not described as a batching dependency. |
| `crates/kf-qemu/src/userread.rs` | H4 `-341,0 +342,29` (+29 / -0) | Span-reservation optimization and its test/comment arrived through excluded merge; independent of die rules, deferred with that merge, not described as a batching dependency. |
| `docs/STATUS_AND_HANDOFF.md` | H1 `-2,0 +3,14` (+14 / -0) | Old batching/performance handoff; historical documentation is not part of code import. |
| `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` | H1 `-405,0 +406,167` (+167 / -0) | Old measurement/batching design/results; historical documentation is not part of code import. |


## Historical pre-approval preflight (superseded authority)

# Sweep integration conflict ledger

STATUS: SUPERSEDED SNAPSHOT, 2026-10-18. The original USER-topic decision was approved; see the current update above.

MEASURED: the GPU-free, unapplied `git merge-tree --write-tree --messages -z`
preflight of candidate 4 `0452613b5` and gpu-types `1cb048610` reports content
conflicts in three protected code/test files, comprising twelve hunks. It also
reports a documentation conflict in `docs/STATUS_AND_HANDOFF.md`; that document
is outside the intended code-only import. No merge result was checked out, no
conflict was resolved, and no historical evidence tree was imported.

The preflight result tree is `921827adbd65d90061eb1a6236e327363592e2dd`.
Ranges below identify that tree's conflict-marker lines, **not** candidate 4's
source line numbers. `preflight_conflicts.json` records stage 1/2/3 blob IDs,
per-side hunk hashes, sizes, and short source excerpts for reproduction.

| File | Hunk / result lines | Current side | Incoming side | Winner and reason |
|---|---|---|---|---|
| `crates/kf-chan/src/user_loop.rs` | H1, 254–331 | Lifecycle and lazy-observe statistics | Older flow statistics and stage timers | UNRESOLVED; protected lifecycle/USER relay |
| same | H2, 397–401 | Current summary format | Batching/flow summary including PUT-ahead field | UNRESOLVED; protected relay diagnostics |
| `crates/kf-chan/tests/user_loop_model.rs` | H1, 153–159 | Retired-PUT-poll mutant | Missing-final-doorbell mutant | UNRESOLVED; protected USER/doorbell model; no test may be silently dropped |
| `crates/kf-qemu/src/userpump.rs` | H1, 481–523 | `core` relay with `Aux` and software state | Inline marker/gate/batching fields | UNRESOLVED; protected relay state architecture |
| same | H2, 1039–1059 | Initialize `Aux` and software state | Initialize inline relay/batch state | UNRESOLVED; coupled to H1 |
| same | H3, 1881–1947 | Current relay turn, lifecycle deltas and lazy flush | Chained turns, deferred bell and timing counters | UNRESOLVED; protected lifecycle/lazyobserve/doorbell |
| same | H4, 2014–2242 | `kf_chan::user_turn::Relay::step` | Inline observation/read/gate/scan | UNRESOLVED; protected production relay execution |
| same | H5, 2262–2383 | No old duplicate inline body | Inline no-op/segment handling | UNRESOLVED; coupled to H4, not an independent addition |
| same | H6, 2405–2504 | Current scan-failure diagnostic | Inline marker/end-batch/credit methods | UNRESOLVED; protected completion/relay behavior |
| same | H7, 2666–2759 | Shared `publish_guarded` admission | Inline admission plus deferred `ring_owed` | UNRESOLVED; protected doorbell/publication ordering |
| same | H8, 2844–2855 | `core` scan/cursor and lifecycle accounting | Old scan/cursor and segment-turn count | UNRESOLVED; protected lifecycle/relay |
| same | H9, 2939–2948 | `core` scan plus lifecycle accounting | Old scan plus rare-entry turn counter | UNRESOLVED; protected lifecycle/relay |

CODE READ: incoming history affecting these files includes `24e7a297a`
(flow/stage measurement), `639acf57d` (histograms), `7476ef69f` (EXPECTED RED
batching tests), and `edcaaca67` (claim batching), merged through `29eae05a4`.
The conflict is not merely additive die data: it overlaps the later production
relay implementation. A blanket incoming-side choice would discard current
relay/lifecycle code; a blanket current-side choice alone would leave incoming
nonconflicting dependent edits unreviewed. Neither choice was applied.

## Historical owner request — APPROVED by the subsequent coordinator decision

INFERRED recommendation: authorize excluding this obsolete USER relay batching
and measurement topic, including its dependent nonconflicting edits, from the
gpu-types import. Preserve current integration relay/lifecycle/doorbell behavior
and its tests; integrate the independent die/ABI/constants/display topics only.
If those topics themselves require changes in a protected subsystem or channel
birth/groups, stop again with the exact dependency. Any desired measurement
port would be a separate topic against the current relay, with failing-first
tests and mutation checks, after explicit direction.

Alternative: request a dedicated port of the old batching/measurement behavior
onto the current relay before continuing the sweep. This is not covered by an
automatic conflict resolution and would require its own tests and review.

H-I1 FALSIFIED: protected conflicts were measured. The mandatory STOP rule in
the owner's 2026-10-18 steering, Phase 1.3, fired. Phase 1 integration and all
Phase 2 hardware work remain pending that decision.
