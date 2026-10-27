# AUTH401 on a third workflow attempt

STATUS: ANSWERED IN SOURCE, 2026-10-27; live deployment and fresh master gate pending coordinator action. Base commit 2b9fa6947bcf5570c30d7edb552760605f5639cb; standalone EXPECTED RED commit 9934156f4. This scoped change does not merge PR172 or deploy a Worker.

## What the incident establishes

MEASURED 2026-10-27: [run40503100558 attempt3](https://github.com/reindertpelsma/kayfabe/actions/runs/40503100558/attempts/3), head2b9fa6947, failed at availability with HTTP401. The read-only IPv4 Worker query13:25:30–13:27:00 UTC returned7 events without truncation. At13:25:47.205 UTC its fixed refusal code is `github:run`; two GET availability401 responses follow, then stop40503100558-3 returns404. [Sanitized telemetry](telemetry.json) contains only timestamps, route words, run key, method, status and fixed check codes.

CODE READ 2026-10-27 at commit 2b9fa6947: `github:run` occurs after OIDC signature/time/audience and signed identity/workflow validation. It groups comparisons of the GitHub REST run metadata. There is no attempt>=3 ceiling. The bundle comparison is a separate later boundary. Stop404 follows an authenticated run-key lookup with no granted record; this availability refusal creates no rental or spend row.

**The exact historical differing REST field is unknown.** The deployed bundle logged one aggregate code. No historical token or upstream response was retained or reconstructed. This evidence does not prove that run_attempt or status specifically differed.

## Hypothesis, falsifier and change

Hypothesis stated before reproduction: a mutable latest-run response can describe the preceding attempt while OIDC names attempt3/4. Falsifier: admission already reads the exact attempt or succeeds when the latest-run endpoint returns the prior attempt.

MEASURED 2026-10-27 in EXPECTED RED commit 9934156f4: the stale-latest-run scenario rejects otherwise valid attempts3/4 in Node and real workerd. The focused30-test suite has26 failures and4 passing controls. Tests also show that a valid canonical response cannot authorize invalid exact-attempt metadata. Those synthetic scenarios prove the resource-selection defect; they do not identify the incident's unlogged field.

MEASURED 2026-10-27 13:59:34 UTC: the public [canonical run](https://api.github.com/repos/reindertpelsma/kayfabe/actions/runs/40503100558) and [attempt3](https://api.github.com/repos/reindertpelsma/kayfabe/actions/runs/40503100558/attempts/3) both now describe completed attempt3 with matching public identity/SHA facts, and both advertise `public, max-age=60, s-maxage=60`. [Selected public snapshots](public-run-snapshots.json) retain the observation time and response headers. INFERRED: prior-attempt metadata is a plausible incident cause. These later snapshots cannot show what the Worker received at13:25:47.

CODE READ 2026-10-27 (`src/github.js`): admission now reads the [documented exact attempt resource](https://docs.github.com/en/rest/actions/workflow-runs#get-a-workflow-run-attempt) using signed positive numeric run/attempt identifiers. Returned run id, attempt, event, in-progress status, repository, head repository, valid head SHA, nonempty branch, owner triggering actor and event-specific OIDC SHA equality remain mandatory. Requested tested SHA still binds to the attempt head. Admission reads remain uncached by the Worker. Lifecycle cleanup keeps its separate latest-run read. No timer, retry, budget, gap, protocol or workflow policy was changed.

CODE READ 2026-10-27 (`src/github.js`, `test/refusal-log.test.js`): a 401 now logs only `github:run:<fixed_field>`. Values, tokens, headers and URLs stay out of that telemetry and the public response. The existing runner phase/state/status diagnostic remains unchanged because no intentional attempt ceiling exists.

## Validation and operational answer

MEASURED 2026-10-27: all2346 Node tests and105 real-workerd tests pass without skips, including deterministic dry-run bundle verification. [Validation counts](validation.json) and [source hashes](source-sha256.json) bind this result. The workerd tests authenticate local signed JWTs, call actual availability and rent handlers through independent request closures, and confirm one SQLite-backed reservation for each attempt. Every outbound response is simulated; no provider call or deployment occurs.

MEASURED 2026-10-27: all14 private-copy [mutants](mutants.py) have at least2 named killing tests in [mutations.json](mutations.json). They cover canonical resource regression in real workerd, an attempt3 ceiling, all10 REST binding checks, removal of field diagnostics and logging metadata values. Copies use real dependencies and are disposed at the end.

CODE READ 2026-10-27 (`src/github.js`, `runner.py`, `deployment.json`): attempt4 is permitted against the currently deployed8f44 bundle, subject to current metadata, SHA gap and spend/concurrency caps. Its availability401 predecessor did not consume admission. A rerun cannot be promised green because the old aggregate log leaves the exact upstream mismatch unknown.

The coordinator should deploy the reviewed replacement bundle2828bf41ed2574b638a7b6fb535e16b6dba9467c997972917050aa58d0bb0460 and obtain a fresh GPU-relevant master commit carrying its matching manifest. After that deployment, old run40503100558 still expects its original trusted BASE8f44 pin and cannot validate the replacement bundle by rerunning attempt4. Preserve the pin check. A fresh matching commit is required for validation after the fix; master2b9's original bytes retain their historical failed gate. No live fix or fresh master gate is claimed here. PR172 remains for coordinator merge after diagnosis.
