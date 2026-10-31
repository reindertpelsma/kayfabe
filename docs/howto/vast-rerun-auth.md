# Diagnose a Vast rerun authorization refusal

STATUS: LIVE, 2026-10-27. CODE READ: `ci/vast-broker/src/github.js`, `src/oidc.js`, `src/index.js` and `runner.py`.

The Worker accepts positive numeric run attempts, including 3 and later. A rerun still needs the owner's triggering actor, the pinned master workflow, matching repository and run identity, an in-progress attempt and the exact tested commit. It does not bypass spending, concurrency or the commit gap.

If the runner reports `phase=availability state=UNAUTHORIZED ... status=401`:

1. Use the owner's read-only Worker observability to find the `refused` event near the runner timestamp. Keep broker path prefixes, authorization headers, OIDC tokens and Access credentials private.
2. Read the fixed `why` code. `oidc:*` names signature or token-claim validation; `identity:*` names a signed identity check; `github:run:<field>` names a failed comparison against GitHub's exact attempt metadata. No claim or metadata value is logged. Older bundles log only `github:run`; that cannot establish which field differed.
3. Inspect the public GitHub run and exact attempt through read-only requests. Admission uses `GET /repos/<owner>/<repo>/actions/runs/<id>/attempts/<attempt>` and checks the returned attempt again. A later snapshot cannot prove what GitHub returned during the refusal. GitHub's [documented attempt endpoint](https://docs.github.com/en/rest/actions/workflow-runs#get-a-workflow-run-attempt) identifies a specific attempt.
4. Keep the binding checks intact. A stop404 after an availability401 is expected if no rental record was granted. An availability refusal creates no spend row, but prior rentals of the same SHA may still enforce a gap.

A rerun keeps its original trusted BASE and expected Worker bundle hash. If the coordinator deploys a changed bundle, start a fresh GPU-relevant master commit with the matching committed manifest. Rerunning the old BASE after deployment will fail the bundle binding. Do not relax that check or adjust a timeout to obtain a gate result.

For the 2026-10-27 incident, read the [sanitized evidence](../../traces/vast_auth401_20261018/README.md). The old log proves a GitHub metadata refusal, not an intentional attempt limit; the exact historical mismatching field remains unknown.
