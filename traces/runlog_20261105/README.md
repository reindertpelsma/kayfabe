# gpu-vast runner run-log work: mutants

STATUS: LIVE, 2026-11-05. Branch `claude/runlog-20261105`, base `origin/next` ce4a1fd74. No GitHub run, rental or Worker contact was used; all tests are GPU-free.

What changed is described in [`ci/vast-broker/README.md`](../../ci/vast-broker/README.md), section "What the runner shows of the box log". This directory holds the mutation check of that change.

Falsifier stated before the run: a mutant of one new behaviour (key-line selection, census, drop markers, full-log file, DIAGNOSTICS block, upload-step guard, lint exception, request bodies) that no named test notices.

Reproducer: `nice -n 19 python3 traces/runlog_20261105/mutants.py`. It copies `ci/vast-broker`, `.github/workflows` and `scripts/ci` to a private directory, checks that the unmutated tests pass, applies one mutant at a time and requires at least one failing named test. Result file: `mutations.json` (mutant name, number of failing tests, their names).

MEASURED 2026-11-05 on the final tree of this branch (before the PR commit): the unmutated tests pass, and all 51 mutants below fail at least one named test (`mutations.json` lists the names). Two further mutants were written and are NOT killed; both are equivalent in effect, so they were removed from the script and are recorded here: `census-line-uncut` (the `line[:400]` slice before normalising only bounds the work per line, the key is cut at 60 characters afterwards) and `guard-upload-in-sweep` (the sweep's own forbidden-list entry for `upload-artifact`: the broker-step rule rejects any extra step in the sweep's `gpu` job as well).

| Mutant | Failing named tests |
| --- | ---: |
| key-lines-unframed | 8 |
| key-lines-no-cell-pattern | 3 |
| key-lines-no-gate-fail | 2 |
| key-lines-no-xid | 2 |
| key-lines-byte-budget-ignored | 1 |
| key-lines-line-budget-ignored | 3 |
| key-lines-no-red-priority | 1 |
| key-lines-no-first-last | 1 |
| key-lines-raw-not-sanitised | 1 |
| key-lines-line-uncut | 1 |
| key-lines-shown-when-small | 1 |
| key-lines-before-group | 1 |
| display-lines-raised-far | 2 |
| display-bytes-raised | 1 |
| census-prefix-uncapped | 1 |
| census-all-groups-named | 3 |
| census-keys-unbounded | 1 |
| census-unframed | 6 |
| census-not-sanitised | 1 |
| census-others-uncounted | 2 |
| census-not-before-tail | 5 |
| census-digits-in-names-collapsed | 1 |
| marker-sanitisation-wording | 4 |
| marker-cap-uncounted | 3 |
| runlog-follows-symlinks | 1 |
| runlog-cap-removed | 1 |
| runlog-marker-unprefixed | 2 |
| runlog-unsanitised | 3 |
| runlog-line-prefix-missing | 28 |
| runlog-failure-escapes | 1 |
| runlog-dir-traversal | 1 |
| runlog-dir-symlink | 1 |
| runlog-dir-from-anywhere | 1 |
| diag-always | 1 |
| diag-never | 2 |
| diag-not-in-artifact | 1 |
| diag-missing-fields-unlisted | 2 |
| diag-stage-name-unbounded | 1 |
| diag-box-line-unframed | 82 |
| diag-no-transitions | 1 |
| diag-artifact-unsanitised | 1 |
| protocol-rent-body-changed | 1 |
| protocol-version | 1 |
| guard-upload-step-any | 1 |
| guard-upload-text-anywhere | 1 |
| bootstrap-no-log-dir | 2 |
| bootstrap-sweep-gets-dir | 1 |
| upload-retention | 4 |
| upload-pin | 3 |
| lint-exception-any-action | 5 |
| lint-exception-any-job | 1 |
