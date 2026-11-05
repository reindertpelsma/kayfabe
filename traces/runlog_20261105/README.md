# gpu-vast runner display work: mutants

STATUS: LIVE, 2026-11-05. Branch `claude/runlog-20261105`, base `origin/next` ce4a1fd74. No GitHub run, rental or Worker contact was used; all tests are GPU-free. The first version of this branch (f51c1dac7) also wrote a sanitised full-log file and uploaded it as an artifact; the owner ruled on 2026-11-05 (after the security review of PR #267) that the full log comes from the Worker instead, so the file, the upload step and the guard/lint exceptions were removed.

What changed is described in [`ci/vast-broker/README.md`](../../ci/vast-broker/README.md), section "What the runner shows of the box log". This directory holds the mutation check of that change.

Falsifier stated before the run: a mutant of one new behaviour (key-line selection and bounds, census, drop markers, redaction order, DIAGNOSTICS block, request bodies, a file output coming back) that no named test notices.

Reproducer: `nice -n 19 python3 traces/runlog_20261105/mutants.py`. It copies `ci/vast-broker` and `.github/workflows` to a private directory, checks that the unmutated tests pass, applies one mutant at a time and requires at least one failing named test. Result file: `mutations.json` (mutant name, number of failing tests, their names).


MEASURED 2026-11-05 at code revision 670f74e4b (node 22.22.1, `npm ci --ignore-scripts`): `node --test ci/vast-broker/test/*.test.js` 2785 pass, 0 fail; the unmutated runner tests pass and all 35 mutants below fail at least one named test (`mutations.json` lists the names). Not killed, and removed from the script as equivalent in effect: `census-line-uncut` (the `line[:400]` slice before normalising only bounds the work per line; the key is cut at 60 characters afterwards).

| Mutant | Failing named tests |
| --- | ---: |
| key-lines-unframed | 7 |
| key-lines-no-cell-pattern | 4 |
| key-lines-no-gate-fail | 2 |
| key-lines-byte-budget-ignored | 1 |
| key-lines-line-budget-ignored | 3 |
| key-lines-no-red-priority | 1 |
| key-lines-no-first-last | 1 |
| key-lines-raw-not-sanitised | 1 |
| key-lines-line-uncut | 2 |
| key-lines-scan-uncut | 1 |
| key-lines-shown-when-small | 1 |
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
| marker-sanitisation-wording | 3 |
| marker-cap-uncounted | 2 |
| redact-before-strip-only | 1 |
| redact-after-strip-only | 1 |
| diag-always | 1 |
| diag-never | 3 |
| diag-missing-fields-unlisted | 1 |
| diag-stage-name-unbounded | 1 |
| diag-box-line-unframed | 2 |
| diag-no-transitions | 1 |
| diag-claims-log-below-always | 1 |
| protocol-rent-body-changed | 1 |
| protocol-version | 1 |
| file-output-comes-back | 1 |
