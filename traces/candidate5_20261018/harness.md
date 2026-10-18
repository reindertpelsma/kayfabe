# Integration harness maintenance, 2026-10-18

CODE READ: hitch added a second `ticks_since_open >= 1` occurrence for diagnostics
and changed `spend` from `is_some` to `if let`. Three old mutant anchors therefore
had two or zero matches. The anchors now retain the same intended decision defects;
none of the 35 mutants or required suites was removed. MEASURED: all 35 match once.

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
