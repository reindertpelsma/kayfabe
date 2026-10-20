STATUS: GPU-FREE SCOPED IMPORT ACCEPTED / HARDWARE PENDING, 2026-10-20.

MEASURED supersession: coordinator authorized the wrapper correction and
resumption. Four wrapper tests pass. All six mutants were rerun and killed by
4 / 6 / 10 / 6 / 2 / 2 tests; restored scoped run: 250 pass / 0 fail / 5 ignored
(existing unit tests plus new caps integration and example; other integration
targets wait for the full Stage 4 run). The old wrapper stop below is historical.
The ordinary Linux page constructor matches candidate-5 source exactly.

# C6 independent review: scoped derived display caps

STATUS: HISTORICAL STOP DETAILS BELOW; superseded by the accepted import above.

CODE READ: the coordinator approved a separate caps replacement, not the full
sweep. Source product checkpoint `24906bf33` on
`codex/sweep-integrate-20261018`; originating derivation/topic design is on
`claude/display-caps-derive-20261014` at `7c12dfd14`. C6 independently read the
parser, class/branch/die matching, layer rules, provenance tracking, constructor
call site, source test suite and generated family-page comparison.

CODE READ: required imports are `capsderive.rs`, the three caps TSVs, the pure
`caps::diff_pages`/`ClassTable::rows_under` helpers, the caps tests and their
native test oracle. No address-named surface/ISO changes, display pacing,
cursor, DmaKey/cache-key change, scrub fixture, walker, doorbell or drainer
change belongs to this import. The old sweep `None` scrub fixture stays out.

INFERRED acceptance contract, registered before C6 integration: only an explicit
Windows profile selects the new page. FALSIFIER: any change to the ordinary
Linux page constructor or Linux default selection; loss of an existing test;
selection of a different die/branch's measured row; any captured whole page
reachable in a production build; or a scope dependency requiring a protected
unrelated subsystem change. Stop on such a dependency instead of importing
the rest of the sweep automatically.

CODE READ planned scope adjustment: sweep's virtual Blackwell tile authoring
will live inside the new derivation, rather than changing `caps::page` for all
guests. The layer-family listing test must still reproduce the source's exact
listing. Existing Linux constructors and class/version admission stay intact.
The source's public 590/595 vocabulary and surface-address changes are not
dependencies of its `580.65.06` caps derivation tests and will not be imported.

CODE READ provenance: rules use generated public class fields and maintained
family topology. The unnamed required word is an exact die/class/branch
measurement row; missing rows produce a named unmeasured result, not a neighbor
fallback. The runtime parser rejects non-measured source tags. Its inputs are
committed data, not guest-supplied text. The captured native page is only a test
oracle. Keep the inherited QEMU caps-oracle test by compiling its old table and
helper only under `cfg(test)`, while removing the runtime captured-page path.

CODE READ source limitation: strict full-page AD104/native equality remains
RED (100 words in the source report). The passing bisection/derived-field tests
do not imply native equality. The older design says its final binary had not
booted; sweep's later partial hardware matrix is not C6 hardware acceptance.
The originating design file was absent from candidate-5/sweep's integrated
tree and was read from its named source branch; do not add a dangling local
documentation link or pretend the source report is a new C6 measurement.

INFERRED pre-run expected strict-oracle result: full-page comparison will still
return RED with the recorded differences; a different result requires review,
not patching the derived page with native bytes. New C6 tests, mutation kills,
counts and exact import files will be appended after execution. Hardware n=0.

INFERRED mutation falsifiers, registered before injection: removing exact-die
matching, branch matching, required-word application, layer application or
source-tag rejection must each fail at least two tests without compilation
errors. Omitting virtual tiles must fail the committed family listing plus an
independent tile assertion. A mutant that survives is a failed verification
gate, not permission to reduce the required kill count.


MEASURED checkpoint: EXPECTED RED tests committed alone at `b1ea24127`:
missing caps module, comparison API and family listing. The uncommitted scoped
implementation passed 20 tests / 0 failures / 2 inherited ignored diagnostic
emitters (17 integration tests plus 3 example tests). A later independent tile
assertion was added; the exact-die mutant run compiled it and it passed.

MEASURED: removing exact-die matching failed four tests; [kill names](mutations.json).
The wrapper nevertheless rejected the run because its compile-error detector
matched Cargo's expected `error: test failed` line. This is a C6 wrapper bug.
The wrapper restored the original exact-die condition in `finally`; no mutant
is left applied. Remaining mutants, a restored final run, full native-oracle
comparison and full-crate regression are NOT RUN. Therefore no accepted caps
implementation or successful mutation campaign is claimed.

CODE READ stop rule applied: lane AGENTS.md requires STOP/report when an own
safety check refuses a step. C6 stopped rather than editing that detector and
continuing without a fresh instruction. Recommended resumption: distinguish
compiler errors from expected failing-test exit diagnostics, rerun the entire
mutation campaign, then resume the approved profile wiring. The existing
captured page remains reachable on the published product base: the runtime
replacement is not implemented or shipped at this checkpoint.

MEASURED final strict-oracle rerun at `c27134846`: exit1,
`EXACT_NATIVE_PAGE_EQUAL=false differing_words=100`. This confirms the stated
limitation; the runtime constructor is not patched with the native oracle.
