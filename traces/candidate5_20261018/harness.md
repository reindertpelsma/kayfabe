# Integration harness maintenance, 2026-10-18

CODE READ: hitch added a second `ticks_since_open >= 1` occurrence for diagnostics
and changed `spend` from `is_some` to `if let`. Three old mutant anchors therefore
had two or zero matches. The anchors now retain the same intended decision defects;
at that anchor-maintenance step no mutant or required suite was removed.
MEASURED: all 35 match once. The owner-approved expectation correction below
is a subsequent, separate change.

CODE READ: the relay fixture returned Full forever without detecting a retry loop,
so the required `send_blocks_on_full` mutant could strand its test process. The soak
fixture already asserts after 512 Full results per entry (READ_BATCH packets may
try several sends each). The relay fixture now uses that same assertion and resets
its count at each fixture entry; production code and timers are untouched.
Falsifier stated before the run: a baseline turn trips the guard, or the blocking
mutant escapes it. MEASURED: tests-only `c289c5534` fails both finite fixture
self-tests; the guarded full broker suite passes 162 tests with 9 ignored.
Removing the guard kills both self-tests, without an unbounded test loop.

MEASURED: the two new hitch test-initializer clippy warnings were removed by
constructing exactly the same ScanState field values in the initializer. The
original latch test still passes; comparable clippy fingerprints on changed files
have no additions relative to candidate-4. Existing denied lints outside changed
files remain baseline debt; the lint comparison uses the same cap-lints command
on both trees to expose all targets without suppressing any new diagnostic.

MEASURED: the first broker normal relink after a sanitizer run encountered stale
ASan protocol objects. A clean rebuild links and passes; every subsequent normal
broker verification starts with `make clean`. This is build-output hygiene, not a
product change or a test assertion failure.

Owner Amendment 3 approves the progress oracle and scope correction in
`harness_proposal.patch`. CODE READ: the carried-credit mutant removes release of
an AWAIT_TICK credit on the next accepted tick; B previously checked upper bounds
without the corresponding progress obligation. The new oracle checks that release.
The stale-tick mutant changes conn.rs, which form A explicitly models rather than
executes; its expected kill forms are therefore B and relay. All 35 mutants remain.
FALSIFIER before the sweep: any required form survives, any infrastructure error
is counted as a kill, or restored-source tests fail. EXPECTED RED evidence on the
carried-credit mutant: seven distinct B test failures; unmutated B 22/0/1.

MEASURED approved sweep: 35/35 mutants, 78/78 required-suite assertion kills;
no survivors, compile failures or infrastructure failures. Restored broker/display
verification: 409 passed, zero failed, 12 ignored. Changed Rust files format clean;
comparable clippy has zero added diagnostics on changed files.
