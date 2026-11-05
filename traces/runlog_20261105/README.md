# gpu-vast runner display work: mutants

STATUS: LIVE, 2026-11-05. Branch `claude/runlog-20261105`, base `origin/next` ce4a1fd74. No GitHub run, rental or Worker contact was used; all tests are GPU-free. The first version of this branch (f51c1dac7) also wrote a sanitised full-log file and uploaded it as an artifact; the owner ruled on 2026-11-05 (after the security review of PR #267) that the full log comes from the Worker instead, so the file, the upload step and the guard/lint exceptions were removed.

What changed is described in [`ci/vast-broker/README.md`](../../ci/vast-broker/README.md), section "What the runner shows of the box log". This directory holds the mutation check of that change.

Falsifier stated before the run: a mutant of one new behaviour (key-line selection and bounds, census, drop markers, redaction order, DIAGNOSTICS block, request bodies, a file output coming back) that no named test notices.

Reproducer: `nice -n 19 python3 traces/runlog_20261105/mutants.py`. It copies `ci/vast-broker` and `.github/workflows` to a private directory, checks that the unmutated tests pass, applies one mutant at a time and requires at least one failing named test. Result file: `mutations.json` (mutant name, number of failing tests, their names).

