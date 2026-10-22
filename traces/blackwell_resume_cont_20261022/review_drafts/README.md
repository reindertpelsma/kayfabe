STATUS: RESEARCH, 2026-10-22. Unapplied pre-steering CPU experiments, archived for review only.

The owner subsequently limited this task to documentation/derivation and no hardware/rentals.
The working changes were parked without enabling a new runtime guard or native fixture. These small
zero-context text patches preserve that work in git; they are not ready-to-run campaign instructions.

Base: b22e1be62 on codex/bw-resume-cont-20261022. Native test-first 524ea6c4d and guard test-first
b22e1be62 remain intentionally EXPECTED RED on this draft sweep branch. The clean CAPC PR #69
contains neither experiment. No old test is removed to hide those failures.

- native_ce_fixture.patch: selects the actual host CE class rather than the historical fixture bind;
  prior CPU-only full kf-harness release 15 ->19 passed, no failures or ignored cases; two mutants
  each killed by two tests; clippy 284 ->284, added 0. Hardware counterfactual remains UNRUN.
- phase_guard.patch: 40 CPU-only tests passed on the parked candidate, but mutation checks incomplete.
  Review found additional multiple-marker parsing, subprocess-timeout abort and route/provenance gaps.
  Do not apply/deploy it as a guard. Native-source fixtures in these tests reuse AD106 text with explicit
  synthetic channel substitutions; they are unit-test inputs, not Blackwell hardware proof.
- classifier_review_probes.json: observed weaknesses of the committed cc593e3be classifier;
  every unexpected native Xid stays fatal. No new Xid exception is approved here.

Retained logs describe those prior experiments, not the current checked-out implementation.
The native release output is filtered to test names/counts/failures, with full build logs remaining
in the lane's local preparation directory. Documentation task validation reruns the existing
18-case capture/classifier suite; no claim that the unfinished guard suite is green.

Patch format uses zero context to avoid trailing-space context records in the evidence files.
Any later application must deliberately use --unidiff-zero against the stated base and be reviewed.
