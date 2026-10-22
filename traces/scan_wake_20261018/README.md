# RC-03 / scan-limit wake: interrupted test setup

STATUS: RESEARCH, 2026-10-22. Base origin/master e64b8e5f6.

INFERRED hypothesis, registered before inspection: bitmap-word rotation reaches
victims inside a summary, and an exhausted bounded scan must preserve a wake
for any unscanned inventory. FALSIFIER: starvation, dropped/duplicate bits,
exceeding the scan limit, or parking beside a pending live token.

CODE READ: the audit's summary-word starvation test and additional word/wrap
candidates are backed up in this branch. There is no product change.

MEASURED: test setup failed before any tests executed. The core tests directory
was missing when copying the lost-wake audit test. The combined Cargo invocation
also selected a missing test target across the selected packages. Cargo exited
101 with no test summaries; the red-proof guard refused 0 failed tests where
at least 2 were required. This is NOT evidence of a red regression or a mutant
kill. The prescribed shared lock, own target, nice 19 and two jobs were used;
this was not a lock timeout.

CODE READ disposition: STOP under the lane's explicit own-safety-check rule.
The setup was not repaired/retried after that refusal. Pending scan tests are
unexecuted candidates, not a verified fix or a ready PR. A later authorized
resume needs a real core test file and separate valid per-crate test selections.
The remaining ORDER items (AT-1, SA-04, SA-06, SA-14, SA-07, SA-01) were not
executed in this resumed run. No GPU/VM/remote host was used.

MEASURED completed work remains in draft PR73 (SA-02, restored six-test check),
PR74 (SA-03/VA-2, 199 passed, 3 existing ignored, four mutants killed), and PR75
(VA-4 tests only, all 3 expected red; fix deferred after a root-withdrawal lifetime
counterexample). Each is independent from master; nothing was merged.
