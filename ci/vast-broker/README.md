# Vast GPU CI broker

STATUS: LIVE, 2026-11-07, for the deployed rental path. The result store, tree key and
requirements check, the tiers, admin start and release run (sections below; branches
`claude/worker-store-20261105` and `claude/worker-tier-20261105`, merged in
`claude/integration-20261105b`) and the failed-run evidence upload (section "Failed-run evidence",
branch `claude/ci-evidence-bundle-20261106`) and the public full logs, structured run reports and host
sessions (section "Public logs, run reports and host sessions", branch `claude/report-20261107`) are
built and tested GPU-free only; NOT DEPLOYED.

The broker connects GitHub Actions to rented NVIDIA GPU machines on Vast.ai so kayfabe can run GPU checks beyond the hardware available to hosted runners. It is a Cloudflare Worker backed by a Durable Object ledger that tracks rentals, deadlines and spending. GitHub asks for a check; the Worker handles the provider account and relays the box's result and bounded logs back to the runner.

## How a GPU check uses it

1. The [GPU workflow](../../.github/workflows/gpu-vast.yml) decides whether a master push or PR needs a check. It fetches the controller from the trusted master base, without checking out proposed code on the GitHub runner. Two independent path decisions must agree before it reports a check as not needed.
2. The controller presents GitHub OIDC identity. The Worker checks the pinned workflow, the exact run attempt's owner triggering actor and the exact tested commit, then applies offer eligibility, concurrency and spending policy before creating a rental.
3. The box bootstrap checks out that exact commit and runs the [repository-side payload and reporter](../../scripts/ci/README.md). Its scoped run token lets it report progress, logs and a structured result, request its own destruction, and obtain permitted private asset download URLs.
4. The GitHub controller independently validates the result bindings and log hashes, scrubs displayed output and supplies the workflow's `gpu-vast` status. After fetching the final log it requests immediate destruction; result grace and Worker deadlines provide fallback cleanup.

## Trust and spending boundaries

The Vast box and its operator are untrusted. The Vast account key stays in the Worker, outside GitHub secrets and rented machines. A box receives only its run capability and, when configured, a Cloudflare Access service-token pair. GitHub communicates through the broker and accepts no executable or build artifact from the box. A GPU PASS is an unattested tripwire: a dishonest box or Worker can forge a verdict. It supplies no release provenance. See the [trust and release trace](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md).

The ledger reserves liability before rental and applies daily and rolling budgets, a prepaid credit reserve and limits on concurrent work. The default check policy is $1 effective/hour, $6/day, $30 per rolling 30 days and an $8 credit reserve, with one active rental and a 300-minute hard deadline for the check profile, which runs the Windows stage of the master gate ([howto](../../docs/howto/gpu-vast-windows.md)); the sweep keeps three hours. The separate owner-dispatched sweep profile has its own hourly and daily caps. Transfer allowances are reservations, not provider-enforced usage limits. Missed cron runs or provider failures can leave a box billing beyond its deadline; these controls do not guarantee a hard external bill ceiling.

## What the runner shows of the box log

STATUS: LIVE as code on branch `claude/runlog-20261105`, 2026-11-05; not yet on master, so no GitHub run has used it. Every behaviour below is CODE READ plus the GPU-free tests in `test/runner-runlog.test.js`; none is MEASURED on a real run. The box log is untrusted text: every displayed line is prefixed `[untrusted] ` and passes `sanitise()` in [runner.py](runner.py) (redaction of the Worker origin, path, Access values and OIDC token, applied on the raw text and again after the character strip so that a value split by a control character cannot reassemble; terminal sequences; characters outside printable ASCII and Latin-1 letters; `Bearer` and GitHub tokens; runs of 32 or more identifier characters; URLs; `::` and `##[`; lines over 1000 bytes). The runner writes no file and uploads nothing: `gpu-vast.yml` has no `uses:` step in the job that holds the Worker credentials. The full log is meant to come from the Worker (public logs bucket), not from the runner.

- **Display budget**: 65,000 bytes (unchanged) and 270 lines (was 200). The extra lines hold the key lines and the census below. The live head (24,000 bytes / 100 lines) is shown while the run polls; the final group then shows key lines, the middle-drop marker with its census, and the tail (16,000 bytes / 70 lines).
- **KEY LINES**: up to 60 lines and 12,000 bytes, each cut at 400 bytes, picked from the whole sanitised log by the fixed table `KEY_PATTERNS` in runner.py, never by the box: stage start/end, `KF_FINAL_SUMMARY`, `stage name=`, `FAST_CELL_ARM`/`BARE_CELL_ARM`, `WINDOWS_GATE_PASS|FAIL|RETRY|TIMING`, xid-watch before/after/delta and `NEW HOST Xid`, `GATE_HOSTREF`, and any `FAIL`/`FAILED`/`REFUSED`. Non-PASS lines come first, then the first and last line of each category, then the rest in log order. Each line is cut to 1000 characters before any pattern runs, so the work per line is bounded. A box line that imitates a key line is displayed like any other: `[untrusted]`. The verdict is the Worker's validated status, not any line. Each key line is a copy and is tagged `[key] ` (`[untrusted] [key] [box] ...`), so it never reads as the box line it copies: `gate_reuse.py` `summary_lines`, the only reader of the job log (a test asserts that), requires exactly one `[untrusted] [box] KF_FINAL_SUMMARY_BEGIN` line, and a regression test feeds it the runner's complete PASS output (key lines, census, tail). Shown only when the head did not already show the whole log.
- **Census of the dropped middle**: next to `[N bytes dropped from the middle]`, up to 12 lines `[dropped N lines / B bytes: <prefix>]`, largest bytes first, then one `[dropped others: N lines / B bytes]`. The prefix is the line's first tokens (after `[box] ` and a dmesg timestamp) up to and including the first token holding a number or `=`, numbers and hex shown as `#`, at most 60 characters, sanitised and displayed `[untrusted]`. Beyond 12 groups, and beyond 5,000 distinct prefixes, lines are only counted. Bytes are sanitised bytes; the middle marker keeps the raw count.
- **Honest markers**: `[N bytes removed by sanitisation]` (redaction, control sequences, characters outside the allow-list, lines cut at 1000 bytes) and `[N bytes dropped by size cap]` (whole lines that the line or byte cap did not show) are separate; the closing `[N bytes dropped by runner display budget]` counts only the second kind. The old single marker `[N bytes dropped by runner]` no longer exists.
- **DIAGNOSTICS**: when a run was admitted but ended without a PASS or FAIL result that passed the runner's checks, the runner prints, before the final group, the Worker's last polled state, the Worker-clock timestamps it returned (`started_at`, `last_post_at`, `result_at`, `ended_at`), the runner's own clock since the rent request, the last log text and the newest `KF_PAYLOAD_STAGE_START` line (name cut to `[A-Za-z0-9_.-]{1,40}`, displayed `[untrusted]`), and the Worker states its polling saw. It states no cause. "The last lines ... are shown below" is printed only when the Worker returned log text. It uses only fields the protocol already returns. The runner itself still returns neither the provider's instance status nor the close reason as fields; both now appear as Worker log lines (section "Provider status tolerance and close lines"), which the runner shows as `[untrusted]` text like any other log line.

## Provider status tolerance and close lines (built 2026-11-06, NOT DEPLOYED)

STATUS: LIVE as code on branch `claude/vast-tolerance-20261106`, 2026-11-06; not on master, not deployed.
Why: the vanish hunt (`traces/vanish_hunt_20261106/README.md`) found that the Worker destroyed a live
box on ONE Vast reading that was not `running`. MEASURED there (direct rentals, no Worker, 2026-11-06):
on machine 17049 the instance endpoint the Worker reads returned `actual_status: "offline"` in 16 of
174 polls (10 s apart) while `intended_status` stayed `running`. INFERRED there: the same flaps hit the
Worker's boxes in TESTING and caused the seven spurious UNATTRIBUTED gpu-vast attempts of that day. This
change is GPU-free tested only (`test/provider-tolerance.test.js`, `provider-tolerance-mutants.mjs`);
no Worker, rental or box has run it. Whether it removes the losses is OPEN until a deployed Worker
survives a flapping host (falsifier: `provider=offline` lines appear while POSTs continue and the run
survives; a box that stops POSTing first and then reads non-running points back at the host or box).

**Rule (src/index.js `update()`, `report()`, `badReading()`; constants in `defaults.json`).** Applies to
an existing instance whose `actual_status` is not `running`, and to a Vast API error:

- **BOOTING** (before the first box POST): `offline` is treated like `created`, `loading`, `starting` and
  a missing status: still booting, nothing counted, bounded by the existing boot timeout (75 minutes by
  default). A first POST while the status is one of these gets `NOT_READY` (409) as before. A pending
  reading clears a count made by an earlier non-pending one.
- **Definite words** (`exited`, `stopped`): counting readings. A reading counts if at least
  `providerGoneSpacingSeconds` (20) after the previous counted one, so a burst of runner polls is one
  reading. The rental closes as `ENDED` when `providerGoneReadings` (3) are counted, the first is at least
  `providerGoneWindowSeconds` (120) old, and no box POST arrived since the first.
- **Ambiguous words** (`offline`, `created` after the first POST, `loading`, `unknown`, a missing status,
  anything unrecognised) **and Vast API errors** (HTTP 5xx, timeout, unreadable body): the same window, and
  in addition the box must have posted nothing for `providerQuietSeconds` (660, the 600 s heartbeat plus a
  cron period). The box sends a heartbeat every 600 s and otherwise posts whenever it has output, so a
  live box does not stay quiet that long. A persistent API error closes as `UNAVAILABLE` on the same
  terms (the destroy then retries every cron tick until the API answers). A foreign instance (ownership
  mismatch) closes at once.
- Any `running` reading, any good inspection or any accepted box POST clears the count.
- **A box POST while the provider says non-running, or while the API errors,** is accepted (it was a 401
  plus destroy). The Worker counts the anomaly (`providerAnomalies`) and prints the status line with
  `(box post accepted)`. `/blob-url` and `/bench-data` still answer 401 while the provider does not read
  `running` (no close), so a box retries them.
- **Absence closes at once**, as before: Vast answering that the instance does not exist
  (`instances: null`, 404) closes as `GONE` on the first reading, in `update()` and `report()`.
- **`RUN_GONE` is evaluated whatever Vast reads** (the GitHub lifecycle read now runs before the status
  branch): a box that keeps posting through a stuck status cannot outlive its workflow. Not while a
  booting instance is still pending, not after the result, not for an admin or release leg. A GitHub
  failure does not stop the window for a non-running status; with `running` it throws as before.
- **Unchanged and checked before the status:** hard deadline, result deadline, boot timeout, heartbeat
  timeout (15 min without a POST), the cost contract recheck (`CONTRACT_PRICE`, `CONTRACT_BUDGET`);
  `STOPPED` (`/stop`, `/stop-all`, the kill route), `SUPERSEDED`, `TTL`, `LIMIT`, the daily, rolling and
  concurrency caps, settlement and the destroy-by-exact-id teardown with absence check.

**Worst case (INFERRED from the code and the 60 s cron; tests move a clock, nothing ran on Cloudflare).**
Delay of the close of a really gone instance, against the old close on the first reading, and the
billing of that delay at the profile price caps (check $1/h, release $2/h, sweep $2.5/h; default
boot limit 75 min, `defaults.json` `bootMinutes`):

| case | closes after | extra per box: check / release / sweep |
|---|---|---|
| `exited` or `stopped` | 120 to 180 s after the first non-running reading (up to one more cron period if a tick is late), about +4 min at most | $0.07 / $0.13 / $0.17 |
| ambiguous word or API error, box silent | 11 minutes after the box's last POST (not after the first reading); the heartbeat timeout (15 min) is the outer bound and is unchanged | $0.18 / $0.37 / $0.46 per 11 min |
| BOOTING `offline` / `created`, instance never boots | the boot timeout, up to 75 min (before: closed on the first `offline`; `created` already waited) | $1.25 / $2.50 / $3.13 |
| a token holder that keeps posting while Vast reads non-running | the hard deadline (checked first), or `RUN_GONE` at the next tick after the workflow ends | up to the pre-existing bound |

A dead box that Vast still reports `running` is not affected: the heartbeat timeout ends it as before.
The host `96607` (machine 17049; vanish hunt: never booted, `offline` flaps) is not in
`defaults.json` `badHosts` or `badMachines`; it should stay excluded by the owner's list when the other
exclusions are lifted, because the 75-minute boot wait above is its cost. That list is an owner decision
and this change does not edit it.

**Close lines.** Every close writes exactly one Worker line before the destroy:

    [Worker] close reason=<REASON> provider=<last raw actual_status> msg=<status_msg>

`provider` is the last status word the Worker read (`[a-z_]{1,16}` as Vast sent it; `unknown` for any
other string; `null` when Vast sent none; `absent` when Vast said the instance does not exist; `error` when the provider API failed; `none`
when it never read one), and `msg` is Vast's `status_msg` with box-text sanitising (control and escape
sequences, addresses, bearer and token-like runs, URLs, known secrets, the instance id and label),
sanitised first (on the first 8192 characters; a trailing identifier-like run is dropped when the input was cut), then joined to one line and cut to 120 characters (`-` when empty). The reading is the last one stored, not a
fresh call. The reasons that were silent before: `RUN_GONE`, `CONTRACT_PRICE`, `CONTRACT_BUDGET`, the
report-path liveness close (`UNAVAILABLE`), `TTL`, `LIMIT`, `STOPPED`, `SUPERSEDED`; the others
(`ENDED`, `GONE`, `DEADLINE`, `HARD_TIMEOUT`, `RESULT`, `BOOT_TIMEOUT`, `HEARTBEAT_TIMEOUT`,
`PROVIDER_FAULT`) keep their earlier `destroy reason=` or `provider_absent` line and gain this one. The
`TTL` close of `report()` cannot be reached with a valid token (the token expires at the same instant,
jose refuses it first); its line comes from the shared `close()`. The status reading line now prints the
raw word and message on each change: `provider=<word> phase=<PHASE> msg=<status_msg>`, with
`flaps=<n>` once a run has had a non-running episode. A closing tolerance adds
`provider gone: <n> readings over <s> s, no box post machine=<id> host=<id>`.

**Host policy.** Flaps are counted per run only (`providerFlaps`, shown as `flaps=` and in the run log
with the machine and host ids). No per-host ledger counter and no automatic bad-listing: a host is
bad-listed only by the existing provider-fault rules. Aggregating `flaps=` lines across runs is a
follow-up that needs a retention design.

## Failed-run evidence (built 2026-11-06, NOT DEPLOYED)

CODE READ (`src/evidence.js`, `src/index.js`, `scripts/ci/vast_evidence.py`,
`scripts/ci/fetch_evidence.py`); GPU-free tests only (`test/evidence.test.js`, `evidence-mutants.mjs`,
`scripts/ci/test_vast_evidence.py`, `evidence_mutants.py`, `test_vast_fetch_evidence.py`); no real R2
bucket, Worker or box has run it. Why: a box is destroyed after its grace period and the runner's
display drops the middle of the log, so the evidence of a failed run was lost with the box (examples:
gpu-vast run 42111518119 attempt 2, whose cause was found only by reproducing it on a direct rental,
and run 42152626663 attempt 1, whose timeout screendump was lost).

- **Box side.** After the payload ends (any status, any timeout) and before the result is reported,
  `vast_onstart.sh` runs `vast_evidence.py`: one bounded bundle (at most 8 MiB compressed, 48 MiB
  unpacked), scrubbed (GPU UUIDs, serials, MACs, public IPs) and scanned (run token, broker URL and
  secret path, Access pair, runner nonce, guest password, harness ssh key, Vast key, presigned URLs,
  JWTs, bearer headers, private keys: any hit REFUSES the upload and says which class and member),
  uploaded once with the run token. What it holds, and what is dropped first when the cap binds, is in
  [scripts/ci/VAST_PAYLOAD.md](../../scripts/ci/VAST_PAYLOAD.md), "Evidence bundle". It prints one line
  into the box log: `evidence uploaded: <run>/<attempt> <bytes> sha256 <12 hex>`, or
  `KF_EVIDENCE_REFUSED reason=...` (matches the runner's existing `REFUSED` key line) or
  `KF_EVIDENCE_SKIPPED reason=...`. Any failure is non-fatal: the exit status is always 0, the result
  file is never written, and the step is bounded to 110 s and to `KF_SOFT_DEADLINE + 150 s`. The runner
  is not changed: the line sits at the end of the log, inside the tail the runner shows.
- **Worker side.** `POST /evidence/<run>-<attempt>`, a box route with the same run-token
  authentication as `/report`, `/kill`, `/blob-url` and `/bench-data`, accepted until the rental is
  closed or ended (so also during the result grace). `Content-Length` is required and at most 8 MiB,
  checked in the gateway before a byte is read; the body must be gzip, must match the declared
  SHA-256, and must come with a metadata header of a fixed shape (`X-KF-Evidence-Meta`, at most 4 KiB).
  The gateway reads and writes the body itself; the ledger Durable Object only reserves and settles a
  once-per-rental PENDING/STORED flag (a few bytes each way), so the second upload is `409
  EVIDENCE_EXISTS` once stored and `409 EVIDENCE_PENDING` while the first is still being written
  (retried by the box, never counted as stored), at most 4 reservations are open across all rentals and at most 2 bodies are held
  per gateway isolate (the rest get `503`, retried by the box), and at most 6 uploads a minute per
  rental reach the gateway. The Worker never opens the archive. It writes
  `<profile>/<run>/<attempt>.tgz` (`<attempt>-r1` for a provider-fault replacement) and `.json`
  (manifest) to the PRIVATE bucket `kayfabe-ci-evidence`
  through the `EVIDENCE_BUCKET` write binding. Without the binding the route answers `404
  EVIDENCE_DISABLED`. The status object, the public API, the site and the runner never mention the
  bundle; no route lists or serves it. A verdict, a deadline or the budget never depends on it.
- **Retention and reading.** 30 days, by an R2 lifecycle rule the coordinator sets; reading is the
  coordinator's job only ([DEPLOY](DEPLOY.md), "Evidence bucket"; `scripts/ci/fetch_evidence.py`).
- **Tests.** `node --test test/evidence.test.js` (route, auth, size, idempotency, concurrency, disabled
  mode, gateway buffering) and `node evidence-mutants.mjs` (every mutant must fail at least two tests).
  Not exercised: real R2, the deployed gateway and Durable Object (the tests call the ledger service
  in-process), a real box.

## Public logs, run reports and host sessions (built 2026-11-07, NOT DEPLOYED)

CODE READ (`src/publiclog.js`, `src/runreport.js`, `src/publish.js`, `src/index.js`, `src/ledger.js`,
`scripts/ci/vast_run_report.py`); GPU-free tests only (`test/public-log.test.js`, `test/run-report.test.js`,
`report-mutants.mjs`, `scripts/ci/test_vast_run_report.py`, the run-report cases of `test_vast_box.py`); no
real R2, D1, Worker or box has run any of it. Contract: `/root/night/REPORT_CONTRACT.md`.

- **Full public log.** When a run is settled the cron writes `<run id>/log.txt` to the public R2 bucket
  `kayfabe-logs-public` (`https://logs.kayfabe.dev`, binding `LOGS_BUCKET`): the text the box streamed, whole,
  with secrets replaced by `[redacted]` and nothing else changed (no character allow-list, no display budget;
  a log that holds a secret is redacted, never refused), at most 16 MiB with a visible cut marker. What it can and
  cannot contain, exactly: [DEPLOY.md](DEPLOY.md), "What the full log can and cannot contain". The runner's
  display scrub (`runner.py`, `src/sanitize.js`) is a separate path and unchanged.
- **Off the teardown path.** `publishRuns` runs from the cron AFTER the serialized sweep, outside its queue,
  with a bounded wait per R2 call (20 s) and at most two runs and 40 s per tick; a failure is one log line with
  a fixed word, the entry is retried for about an hour and then dropped. The verdict, the teardown and the budget
  never depend on it.
- **Run report (box protocol 7, additive).** A box posts one bounded JSON claim to `POST /run-report/<run>-<attempt>`
  ([box protocol](docs/BOX_PROTOCOL.md), "Optional run report") only when the Worker sets `KF_RUN_REPORT=1`
  (only with a result store). A separate route, not a key of the result, because a report may be 256 KiB and a
  `/report` body is at most 64 KiB; the other direction (an older Worker) answers 404 and a box that is not told
  sends nothing. The Worker re-validates and bounds it (`src/runreport.js`), sets the run's identity itself, writes
  `<run id>/report.json` to the bucket, and stores it in D1 (migration 0004: `ci_run_reports`, `ci_subruns`,
  `ci_run_tests`, `ci_run_perf`, `ci_run_apps`, all additive). SPECviewperf data is dropped.
- **Host sessions and subruns.** A rental is a host session; its results belong to subruns: `host`,
  `linux-<driver>`, `windows-<driver>`. The public API ranks subruns by similarity
  ([ci/platform-api/README.md](../platform-api/README.md), `/v1/ci/subruns`).
- **Tests.** `node --test test/public-log.test.js test/run-report.test.js`, `node report-mutants.mjs` (every
  mutant must fail at least two tests), `python3 -m unittest test_vast_run_report` and
  `test_vast_box.Onstart` in `scripts/ci`. Not exercised: real R2 `put` options, real D1 `json_each` batches,
  the Durable Object storage limits, the cron timing, a real box.

## Current setup and further reading

[CODE READ: wrangler.toml, gpu-vast.yml, gpu-vast-sweep.yml] The public broker origin is
`https://ci.kayfabe.dev`; Worker var `WORKER_URL` and GitHub environment `gpu-vast` variable
`VAST_BROKER_URL` must match because the origin is the OIDC audience. The path and both Access
credentials stay secrets. Protect the whole `ci.kayfabe.dev` hostname with Access and the same
runner/box service tokens; keep workers.dev as a fallback only if it also stays behind Access.
The admin UI uses `https://admin.kayfabe.dev` and its separate owner-identity Access app. Before
the first var deploy the coordinator deletes the old `WORKER_URL` secret, following [DEPLOY](DEPLOY.md).

Renting is enabled by the deployed vars file, which is kept outside the repository ([deployment guide](DEPLOY.md)); `RENTING_ENABLED=false` in the committed [Worker configuration](wrangler.toml) is only the safe default. `RUN_TOKEN_SECRET` signs run capabilities and `OWNER_STOP_TOKEN` controls emergency cleanup. As of 2026-10-25 acceptance of the current hosted workflow with the protocol-7 payload is still open; historical local smoke results do not establish it.

- [Threat model](docs/THREAT_MODEL.md): who is trusted, and which behaviours are intended design rather than vulnerabilities.
- [Box protocol](docs/BOX_PROTOCOL.md): bootstrap environment, callbacks and result contract.
- [Placement decision](docs/PLACEMENT.md): US/Canada offer preference and the proposed ledger location.
- [Evidence summary](../../traces/vast_broker_summary/README.md): recorded runs, retained evidence and its limitations.
- [Internal implementation and evidence notes](docs/IMPLEMENTATION_NOTES.md): the former README's dated rounds, corrections and superseded behavior.

After deploying the matching Worker bundle, use a fresh GPU-relevant commit to start validation; reruns retain the original trusted BASE pin.

CODE READ (`src/provider-fault.js`, `src/ledger.js`, `runner.py`): provider failures stop and destroy the box before one replacement on a different known host. Two failed machines condemn their shared host; static seed `224600` comes from runs 40425285335 and 40478005010. An exhausted replacement stays a non-passing infrastructure error, with no kayfabe verdict. See [provider-fault handling](../../docs/howto/vast-provider-faults.md).

CODE READ 2026-10-27 (`src/github.js`): attempt3 and later have no special authorization ceiling. Admission reads the signed attempt's REST resource and retains strict actor, repository, status and SHA bindings. For a 401, use the fixed field-only observability code; see [rerun authorization](../../docs/howto/vast-rerun-auth.md).

CODE READ 2026-10-27 (`gate_reuse.py`): proposed master-push reuse preserves the existing GPU job with readable original result logs and summary. It requires identical trees and complete GitHub status/CI/run evidence; uncertainty uses the existing gate. See [reuse behavior](../../docs/howto/gpu-gate-reuse.md).

## Result store, tree key and requirements check (built 2026-11-05, NOT DEPLOYED)

Design: [V3_PLATFORM_TESTING.md](../../docs/design/V3_PLATFORM_TESTING.md) §4/§8 and
[V3_CI_TIERS_AND_IGNORE_LADDER.md](../../docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md) §1/§3.
Deployment steps: [DEPLOY](DEPLOY.md), "Result store (D1)". No gate reads the store yet: the
`gpu-vast` check still waits for its own rental, and nothing here starts a rental. Changes after
the review of PR #258 (2026-11-05) are marked "review".

- **Store** ([`src/results.js`](src/results.js), tables of
  [`0002_ci_results.sql`](../platform-api/migrations/0002_ci_results.sql)). `recordRun(env, row)`
  is idempotent on `run_id`: an in-flight row (no verdict) is finalised once, a repeated final
  row is `DUPLICATE`, a different final row is `CONFLICT` and changes nothing. Every field is
  checked against a fixed pattern (no box text is stored), a token-shaped value refuses the row,
  and each final row carries an HMAC tag over its fields and stages with the Worker secret
  `ROW_TAG_KEY` (not a run token). A tree hash supplied by a caller is stored as kind `claimed`;
  only the Worker's own ingest, with a key it computed, stores kind `tree` (review).
- **Requirements check** (`requirementsMet(env, {commit_sha, tree_hash, tier, requirements})`):
  answers `{met, missing, inflight}`. **Only rows of the same commit count** (review): until boxes
  delete ignored files after cloning, a tree key does not prove what a run tested. The public var
  `TREE_KEY_CARRY_OVER=true` (default off) also counts other commits' rows under a Worker-verified
  (`tree`) key; turn it on only together with the strip step. A final row whose tag does not
  verify makes the answer `TAMPERED` (not met); it is never skipped (review). Answers are ordered
  by the tagged `ended_at`. An in-flight row that still carries a tag (a final row set back to "in
  flight") is also `TAMPERED`, and `closeStale` never re-signs it. Residual (not detectable):
  deleting a row; or setting the newest FAIL back to "in flight" AND erasing its tag, which lets an
  older PASS answer until `closeStale` re-signs that row as UNATTRIBUTED after 8 h (and an
  UNATTRIBUTED row does not block the older PASS either).
- **Ingest** ([`src/ingest.js`](src/ingest.js)). The ledger's verdict is not changed. A queue entry
  `store:<run>-<attempt>:<leg>` is written when a rental is reserved. Tree keys are computed
  OUTSIDE the ledger's serial queue (cron entry point, one commit per tick, 25 s budget, the
  attempt saved before the computation, the per-commit key after 3 failures) and cached as
  `treekey:<sha>` for 7 days (review). The serialized sweep runs `ledger.expire` first, then the
  store work for at most 10 s, recording an in-flight row (so a caller can join it) and, after
  settlement, the final row, as run id `vast-<run>-<attempt>-<leg>`; each attempt is saved before
  its D1 write (review). A dropped queue entry, and any row in flight for more than 8 hours, is
  closed as UNATTRIBUTED (review). Store verdicts: PASS only for a verified box PASS; FAIL for a box
  FAIL or a box that reported and then hit a test-side limit (hard or heartbeat timeout, report
  limits); RETRY for a provider fault; UNATTRIBUTED for everything else (no box report, owner
  stop, supersede, provider absent). RETRY and UNATTRIBUTED never meet a requirement. The stored
  tier is the record's own `tier` (set at admission: a master push `small`, a PR to master
  `middle`, a release leg `large`, stored as `largest`; the sweep `adhoc`); a record without that
  field falls back to its budget class (`check` -> `middle`, `release` -> `largest`, `sweep` ->
  `adhoc`). The stored profile is the budget class (`release` for release legs, which the ledger
  keeps as profile `check` plus `budgetClass: 'release'`); `sweep` is platform `linux`, the others
  `both`. A box PASS counts as PASS only when the box reported the record's die (GPU name and
  compute capability through the die rules) and a known driver; otherwise the row is UNATTRIBUTED
  (integration of #258 with #259, review rev259 F7, 2026-11-05). Release legs go through the same
  `admit()` as every rental, so each leg is one row; nothing else calls `recordRun`.
- **Tree key** ([`src/treekey.js`](src/treekey.js)). SHA-256 over `kf-tree-v2`, the SHA-256 of the
  ignore set's effective lines (non-blank, not `#`; review: two different ignore sets never share a
  key) and, for every file not ignored, `path NUL mode NUL object-id LF` in UTF-8 byte order of the
  path. A new file under an existing exclusion keeps the key; a new exclusion changes it. Without a
  `.workerignore` the built-in default `/archive/`, `/docs/`, `/site/` applies (INFERRED from a
  2026-11-05 grep: no build or box script reads them; `traces/` stays because `kf-abi` includes a
  TSV from it). Fetched only by this Worker, through the GitHub git data API: the commit, its root
  tree, the `.workerignore` blob and one recursive listing per top-level directory that is not
  wholly ignored. No tree key (the run is stored under a per-commit key nothing else shares) for: a
  truncated or empty listing, a filtered tree with no file, a GitHub error, a symlinked
  `.workerignore`, a pattern outside the subset, a path with U+FFFD, or an ignore set that covers a
  code root: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`, `crates/`, `scripts/`,
  `ci/`, `ci/vast-broker/`, `qemu/`, `cuda/` (so `*` is refused) (review).
- **`.workerignore` subset** (read identically by git; `test/treekey-parity.test.js` compares the
  matcher with `git check-ignore --no-index` on every path of this repository through
  [`treekey_ref.py`](treekey_ref.py)): one pattern per line, `#` comments; characters
  `A-Z a-z 0-9 . _ - + @ / * ?` only (no `!`, `\`, `[...]`, spaces); a trailing `/` matches
  directories (and submodules) only; a `/` at the start or in the middle anchors the pattern at the
  root, otherwise it matches a name at any depth (`docs/` ignores every directory called docs,
  `/docs/` only the top-level one); `*` stays within one path segment and `?` matches one byte;
  `**` only as a whole segment of an anchored pattern (`**/x`, `a/**/b`, `a/**`); a path is ignored
  when it or any parent directory matches. The matcher uses no regular expression (review: a
  backtracking RegExp took 10 s for `*a*a*a*a*a*b` on a 100-character name). Its cost is bounded
  deterministically, not by a timer (review N1: a timer cannot interrupt a synchronous loop): one
  table pass per path and pattern, at most 2 `**` per pattern, at most 64 path segments, and a step
  counter of 20,000,000 matcher steps per tree key; a deeper path or an exhausted counter means no
  key. MEASURED 2026-11-05 (node 22.22.1, this repository's 25,539 paths): the default list costs
  557,648 steps, a 12-pattern set using every rule 6,692,930 steps (234 ms). Docker reads unanchored patterns differently; the ladder lint
  of the design is not built.

### Requirements file

[`ci/requirements.toml`](../requirements.toml) is read from a BASE branch the Worker picks, never
the caller (review): `master` (the default branch) for the middle and release tiers, `next` for
small; never from the PR. [`src/requirements.js`](src/requirements.js) parses it. It is a
strict TOML subset (`test/requirements.test.js` also parses it with Python's `tomllib`):
`version = 1`, then one `[[require]]` block per rule with `tier` (`small`, `middle`, `large`),
`family` (a family id or `"any"`), `dies` (die ids such as `"GA102"` or `"CC_89"`, or `["any"]`),
optional `min_dies` (with `["any"]`: distinct dies that must pass, default 1), `platforms`
(`"linux"`, `"windows"`; a `both` run covers each), optional `stages` (suite rows that must be
PASS in the answering run) and optional `min_passes` (default 1). Strings are `"..."` without
escapes; any other key, value or table is refused. The newest verified PASS or FAIL of a die and
platform answers (a re-run that passes after a FAIL counts: re-run, never bypass); `min_passes`
counts only the trailing streak of PASSes, newest first, stopped by the first FAIL, so PASS, FAIL,
PASS does not satisfy 2 (owner rule as decided by the coordinator, 2026-11-05). Optional
`min_windows_runs` (1-8, only with platform `windows`; review of PR #269, R2) counts, on the
Windows cell, the passing Windows gate runs (`windows_gate`, `windows_gate_2`) of that same streak,
as the release verdict does. The committed `large` blocks use `min_passes = 1` and
`min_windows_runs = 2`, so one passing release with one row per die (a box that ran the gate twice)
or two rows per die (two machines) is met (tested GPU-free 2026-11-05: `test/requirements.test.js`
and the release + store test in `test/batch.test.js`). Not checked by the store (the release verdict
checks them): that the two runs were on two machines, and that the rows come from one release
batch; rows of the same commit count, newest first. A run of a larger
tier also answers a smaller tier (`large` > `middle` > `small`; `adhoc` answers none). A tier
without blocks, a missing file or a refused file is never met.

### `POST /v1/requirements/check`

Authenticated exactly like `/availability` (GitHub OIDC of the pinned master workflow, the
`X-KF-Protocol` header, an owner-triggered in-progress run), with its own admission counter of the
same size so that polling never starves `/rent` (review). It runs outside the ledger's serial queue
(review). Body: `{"commit_sha": "<40 hex>", "tier": "small"|"middle"|"large"}` (no other keys).
Answer: `{met, missing: [{family, die, platform, reason, need?, have?}], inflight: [run_id], tier,
base, tree_hash, tree_kind, requirements_sha256}`. Reasons: `NO_RESULT`, `FAIL`, `STAGE`,
`TOO_FEW_PASSES`, `INFLIGHT`, `TAMPERED`, `NO_REQUIREMENTS`, `REQUIREMENTS_INVALID`. It never
reserves, rents or queues; its writes are the content-addressed copy of the requirements text
(`ci_requirement_snapshots`) and the `treekey:<sha>` cache. Without the D1 binding or
`ROW_TAG_KEY` it answers 503 `STORE_UNAVAILABLE`. Forks cannot reach it (the OIDC identity is
the master workflow's).

### Tests

GPU-free, from this directory (Node 22.13+ for `node:sqlite`; the D1 tests skip on older Node):
`node --test test/treekey.test.js test/treekey-parity.test.js test/requirements.test.js
test/results-store.test.js test/store-ingest.test.js`, `node --test workerd/store.test.js`
(Miniflare's local D1), and `node store-mutants.mjs` (every mutant must be killed). The D1 tests
run against the real migrations on `node:sqlite` with a D1-shaped fake (`test/d1.js`); real
Cloudflare D1, the GitHub trees API and the deployed Worker are not exercised.

## Tiers, the admin start and the release run (how to)

CODE READ (`src/tier.js`, `src/release.js`, `src/index.js`, branch `claude/worker-tier-20261105`,
2026-11-05; not deployed, no rental made). Design: `docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md`
(PR #256, DESIGN-ONLY). Every number here is a configuration default, not a measurement.

**Which tier a run is.** The event decides (the signed OIDC claims `event_name`, `ref` and `base_ref`, through `tierForClaims` in `src/tier.js`; review of PR #269, R4); no request field can. A PR to master is **middle**: one
box, Windows required, the check profile's caps and its 300-minute limit, and the die comes from a
**family rotation** (Turing, Ampere, Ada, Blackwell; the next family after the last one that ran,
falling through to the next family that has an eligible offer; the run records its family). A master
push is **small**: today's check run (the cheapest of `CHECK_DIES`), until the small tier's own
driver-patch matrix exists. The die sweep stays outside the tiers (`adhoc`). **large** is the
release; a release tag cannot reach the Worker by itself (GitHub admission accepts master workflows
only), so the owner starts it from the admin page. The middle tier's VRAM floor
(`MIDDLE_VRAM_FLOOR_MB`) is off (0) until the footprints of the first trio run exist (TBD).

**What a release rents.** Two measured dies per family from `releaseDies` in `defaults.json`
(TU106, TU116; GA106, GA104, then GA102; AD104, AD102, then AD103, AD106; GB205, GB206, then GB202,
GB203), falling to the next listed die when one has no eligible offer. Every box runs Linux and
Windows. Each die gets two Windows runs: two boxes on different machines (different hosts when
offered), or one box that runs the Windows gate twice when only one machine offers the die. A
PASS needs every family, two dies each, a Linux pass and two Windows passes per die and no FAIL
anywhere. RETRY and UNATTRIBUTED (a failure without a green same-box host reference) are never a
pass: each schedules one fresh-box re-run on another machine (`RELEASE_RERUNS_PER_LEG`, default 1)
inside the release cost cap. A FAIL is never re-rented. An owner stop is not a verdict and is not
re-run. Release cells count only this release's own boxes of the same commit: no result is carried
over from another commit or tree, and one FAIL in a cell fails it whatever passed before or after.

**Limits of the release profile (explicit, per tier).** A release runs only with the kill switch
`RELEASE_ENABLED=true` (default false). Price at most 2 USD/hour per box, hard limit 300 minutes per
box (the check rule: one box runs the Linux bar, the Windows install and up to two Windows gate runs;
installs took 1436 s and 1825 s and gates 906 s to 3454 s on 2026-11-04 at rev `0c515b3b`,
`traces/die_coverage_20261104/`; `RELEASE_HARD_RUNTIME_MINUTES` may lower it but not raise it,
because the reaper deletes `kf-ci-*` boxes after 300 + 15 minutes), at most the account cap minus 1
boxes at once (`MAX_CONCURRENT_RELEASE`, at most 8; one slot always stays for the master gate, so 8
release boxes need `MAX_CONCURRENT_RENTALS=9`; the committed account cap of 1 gives a release no
slot), 40 USD and 24 rentals per day for the release profile, a **release cost cap of 40 USD**
(`RELEASE_COST_CAP_USD`) and an account-wide cap of 50 USD per UTC day over all profiles
(`ACCOUNT_DAILY_USD`). The shared 30-day rolling cap and the credit reserve still apply to every
box. A release whose plan exceeds any of these is refused before the first rental; every later box
(pending legs, re-runs) is admitted only while spent plus reserved plus its own worst case stays
under the cap, and a box whose contract price (as the provider reports it) breaks the cap is closed. Turning
`RENTING_ENABLED` or `RELEASE_ENABLED` off stops a running release at the next cron tick. A release
writes no commit-gap marker, so the master gate of the same commit still rents. A PASS counts only
when the box itself reports the planned die (its GPU name and compute capability) and a known
driver; anything else is a RETRY.

**Plan first (rents nothing).** On the admin page (`kayfabe-vast-admin`, owner Access identity),
enter the commit, choose the tier and press **Plan**: the Worker evaluates the live market with the
release rules and prints the dies, machines, legs, the estimate per family and any refusal
(`INCOMPLETE`, `COST_CAP`, `NO_SLOTS`, `RELEASE_DISABLED`, `DAILY_USD`, `ACCOUNT_DAILY_USD`,
`DAILY_COUNT`, `ROLLING_USD`, `CREDIT_RESERVE`). Offline,
the same planner runs on an offer list:

```sh
node ci/vast-broker/release-plan.mjs --market ci/vast-broker/test/fixtures/release-market-synthetic.json \
  --var MAX_CONCURRENT_RENTALS=9 --var RELEASE_ENABLED=true
```

**Start.** Press **Start** (it asks for confirmation). The commit must be on this repository's
`master` or `next` history (a fork commit is refused) and must declare the Worker's box protocol.
Each box goes through the same admission as a PR-triggered rental: offer rules, caps, concurrency,
own-instance labels (`kf-ci-<batch>-<leg>`), teardown by exact id, the cron's deadlines. The page
lists the batch with its state, verdict, estimate and spend; **Stop** on a row stops one box, the
batch stop (`{"batch": "<id>"}` to `/api/stop`) or **Stop all** ends the release without
re-runs. A small or middle start is one box and, like a PR rental, is not rented for a commit that
already rented within `SHA_GAP_HOURS` (the leg stays pending and shows `BUDGET:sha-gap`).

**Die lists, caveat (review F10).** The fallbacks GB202 (Windows gate 1 FAIL in 3 runs at
`0c515b3b`, `traces/die_coverage_20261104/blackwell/`) and AD103/AD106 (Windows not run there) are in
`releaseDies`, and GA102 and GB203 in `middleRotation` have no Windows PASS at that revision (GA102
passed once through the Worker path: gpu-vast run 41936606326, rev `d74f6063`, 2026-11-05); a
plan that falls back to them can fail on the die, not on the change.

**Stages.** `specviewperf` (release only) is plumbing: enabled by `RELEASE_STAGES=["specviewperf"]`,
off by default, and until the SPECviewperf lane exists its row is `specviewperf_retry_not_built`,
never a pass; scores stay on the box (`scripts/ci/vast_stage_specviewperf.sh`). `two_guests` and
`trio` are reserved names, DESIGN-ONLY, refused if requested, and part of no required check.

**Results.** Every box of a batch is an ordinary ledger rental (`kf-ci-<batch>-<leg>`), so the
results store (`src/ingest.js`, section above) records it like any other rental, as tier `largest`; each
record carries `tier`, `family`, `source` (`github` or `admin`) and `batch`. The release verdict
itself lives in the batch (`batch:<id>` in the ledger, shown on the admin page): per (family, die,
platform) cells with their runs, passes and machines.
