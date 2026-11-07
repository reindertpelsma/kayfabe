# Box interface

STATUS: LIVE, 2026-11-07. Box protocol 7 (since 2026-10-23; unchanged by the evidence upload, the optional host claims and the optional run report below,
which are additive); runner status protocol 6 (was 5 until the edits2 round of 2026-10-24). Owner
rulings 2026-10-24: right after the runner's verified final log fetch it sends `POST /stop/<run>`
and the Worker destroys the box at once, so the result grace below is only the fallback; box
log text is relayed, not kept: the Worker deletes it `LOG_RETENTION_HOURS` (default 6) after the
run ended.

CODE READ: box protocol 7; the repo reporter declares `BOX_PROTOCOL = "7"` and accepts only
that value. Before renting, the Worker reads `scripts/ci/vast_report.py` at the exact tested SHA
and refuses (`NOT_ELIGIBLE`) unless that line names the Worker's protocol. The bootstrap clones the
public repository, fetches and checks out the exact approved SHA, verifies HEAD, and execs the
configured `scripts/ci/vast_onstart.sh`. Transport and payload logic live in the checkout.

| Variable | Format |
| --- | --- |
| KF_PROTOCOL | Decimal protocol, currently 7 |
| KF_BROKER_URL | HTTPS origin plus required secret path, no trailing slash |
| KF_RUN_ID / KF_RUN_TOKEN | Run-attempt / HS256 JWT run, attempt, sha, nonce, exp (Worker checks) |
| KF_NONCE / KF_SHA / KF_RUNNER_NONCE | 32 / 40 / 64 lowercase hex |
| KF_REPO | Public owner/repository |
| KF_SOFT_DEADLINE | Required Worker-clock absolute epoch seconds |
| KF_BOOT_DEADLINE / KF_HARD_DEADLINE | Optional absolute epoch seconds; validated when present |
| KF_HEARTBEAT_SECONDS | Interval in seconds |
| KF_GRACE_FAIL | Required grace ceiling in seconds |
| KF_GRACE_PASS | Optional grace ceiling in seconds |
| KF_ACCESS_ID / KF_ACCESS_SECRET | Optional Cloudflare Access service-token pair; both or none |
| KF_MERGE_WINDOWS | Optional `0` or `1` (2026-11-04): the tier, a pure function of the profile (`src/tier.js`); absent means the master gate |

[SUPERSEDED 2026-10-24, edits2 C: status protocol 5 below is now 6; the status object gained
`machine_id` and `host_id` and lost `pr`; the runner sends `X-KF-Protocol: 6`.]
The runner status protocol is separate: 5 since 2026-10-23 (the runner sends `X-KF-Protocol: 5`;
each side refuses the other version).

Protocol 7 (2026-10-23): the box sends `CF-Access-Client-Id` / `CF-Access-Client-Secret` on every
broker request when the Worker injected the pair (Worker secrets `BOX_ACCESS_ID` /
`BOX_ACCESS_SECRET`, both set). The Worker passes the pair through only; Cloudflare Access
checks it at the edge, and the Worker never reads Access headers or JWTs. The payload process
does not get the pair (it is removed from its environment like the run token), but the box
code from the tested SHA runs as root and can read `/run/kf-ci/env`, so it does hold the pair.
That is accepted; see the [threat model](THREAT_MODEL.md).

CODE READ: Vast create `env` is a JSON object mapping variable names to string values; values use a restricted shell-safe alphabet. Repo scripts write the environment to root-only `/run/kf-ci/env`. No Vast key, owner stop token, signing secret or private-repository token reaches a box. Its operator can inspect its own scoped capability.

CODE READ: `POST /report/<run>` uses Bearer auth and JSON `{seq,kind,text?,result?}`; kind is
progress/log/result, sequences increase by one. A result binds nonce, SHA, runner_nonce, verdict,
suite counts and hardware. `POST /kill/<run>` takes a short reason. Retries reuse the same
payload/sequence. The backup is `/workspace/kf-ci-output.log`. Missing/invalid payload output fails
closed. HTTPS only; no redirects. Capabilities expire with their own run. GPU results remain
unattested tripwires; approved code precedes rental.

CODE READ: `GET /blob-url/<run>/<tag>/<name>` uses the same run capability. Strict ASCII tag/name, no query string. It is available while the provider confirms this run is running and its next deadline has not expired, including unexpired result grace. Default limits are eight mint attempts/run and 64/day; failed upstream attempts count. BLOB_REPO, BLOB_GITHUB_TOKEN and optional JSON BLOB_ALLOWED_TAG_PREFIXES configure the private store. The GitHub credential stays in the Worker. Metadata is bounded to 64 KiB; the asset endpoint must return a single manual 302 to an exact allowed GitHub asset hostname. The Worker cancels its body without reading binary bytes and returns only `{url,size,name}`. URLs are secret, neither stored nor logged; both Worker and runner redact URL text independently. The payload's own hash lock is authoritative; downloads use HTTPS without a repository auth header or redirects. `kf-box-blob-url <tag> <name>` is installed repo-side.

CODE READ 2026-10-31 (`src/benchdata.js`, `src/index.js`; not deployed by the change that added it): `GET /bench-data/<run>-<attempt>/<name>` uses the same run capability and the same live-run checks as `/blob-url`, no query string, `<name>` from a fixed four-name map. A permitted request gets `302` with `Cache-Control: no-store` to a presigned S3 GET URL for the private R2 benchmark bucket, valid at most 1200 s and not past the run's next deadline; limits are 4 URLs per object per run; there is no country gate (owner ruling 2026-11-01). Refusals: 401 (token/run state), 404 (name), 429 (URL limit), 503 (R2 settings missing). The box helper is `scripts/bench/box/fetch_bench_data.sh`; the details are in DEPLOY.md.

[SUPERSEDED in part 2026-10-24, final review D1] The last sentence of the next paragraph names
the protocol-6 Worker; the Worker is protocol 7 (it still supplies every optional deadline/grace
field).
[CODE READ: scripts/ci/README.md, scripts/ci/vast_report.py; PR #104 e63af44dd] The payload writes `/workspace/kf-result.json` containing `{verdict,suites,hardware?}`. Each suite has `{name,pass,fail,skip}`; hardware has `{gpu_name,compute_cap,driver_version}` with integer capability (8.6 becomes 860). The reporter adds SHA, nonce and runner nonce from the validated environment. Conflicting bindings in the file fail closed. Both reporter and Worker refuse PASS without valid hardware. The protocol-6 Worker supplies every optional deadline/grace field too.

[SUPERSEDED 2026-10-25, history kept] The sentence "The Worker injects no Cloudflare Access
credential into a box." was removed from the next paragraph. It was wrong since box protocol 7
(owner ruling 2026-10-23 21:45): when both Worker secrets `BOX_ACCESS_ID` / `BOX_ACCESS_SECRET`
are set, the Worker injects them as `KF_ACCESS_ID` / `KF_ACCESS_SECRET` (pair B, see above).
[CODE READ: src/index.js, src/routes.js; owner ruling 2026-10-23 19:10] There is no hold, manual or
owner timer-extension mode: a run ends at its result grace, heartbeat, boot or hard deadline, by
owner stop/stop-all or by `kf-box-kill`. Worker caps remain 20 requests/10 seconds, 2000 requests and 4 MiB per run.

[SUPERSEDED in part 2026-10-24, final review D1] In the next paragraph, "protocol-4 metadata
field" reads status protocol 6 (`country` is still the one optional status key; the status
object has no `pr` and has `machine_id` / `host_id` since 2026-10-24) and "the environment is
protocol 6" reads box protocol 7.
[CODE READ: src/index.js, runner.py] Structured Worker status includes the selected provider `country` (two uppercase letters or `UNKNOWN`), separate from payload hardware. The runner accepts it as an explicitly optional protocol-4 metadata field, validates it independently, and labels it untrusted. The repo-side result-file contract is unchanged; the environment is protocol 6.

CODE READ 2026-11-06 (`src/evidence.js`, `src/index.js`, `scripts/ci/vast_evidence.py`; NOT deployed;
GPU-free tests only), "Evidence upload": `POST /evidence/<run>-<attempt>` uses the same run capability
as `/report`; no query string; the body is one gzip bundle (`Content-Type: application/gzip`), at most
8 MiB, with a required `Content-Length` (`411` without one, `413` above the cap, checked before the body
is read). Headers: `X-KF-Evidence-SHA256` (64 lowercase hex of the exact bytes) and `X-KF-Evidence-Meta`
(compact ASCII JSON, at most 4096 bytes: `{"v":1,"sections":[{"name","files","raw_bytes","dropped"}],
"dropped","raw_bytes","scrub":{"uuid","serial","mac","ipv4","ipv6"},"scan":"clean"}`; any other key or
shape is `400`). It is accepted while the rental is not closed or ended, so also during the result grace,
and it is sent BEFORE the result report. Answers: `200 {"state":"STORED","run","size","sha256"}`; `400`
(not gzip, digest or length mismatch, bad metadata); `401` (token or rental state); `404
EVIDENCE_DISABLED` (the Worker has no bucket binding) or an unknown route (an older Worker); `409
EVIDENCE_EXISTS` (one upload per rental; a retry whose first try did arrive sees it; the box counts
ONLY this state as stored, and a `409 BUSY` from the ledger queue is retried); `409 EVIDENCE_PENDING`
(an earlier upload of this rental is still being written, at most 60 s; retried, never counted as stored); `411`; `413`; `429` (20
requests per 10 s per rental, and 6 uploads a minute at the gateway); `503` (bucket error, or
`EVIDENCE_BUSY`: too many uploads in flight, retry). The box tries at most 3 times (network error, 409
BUSY, 429, 5xx) and treats every other answer as a skipped upload: the upload never changes
a result, a verdict, a deadline or the exit status. The Worker keeps the bundle in a private bucket and
never serves it. The object key is `<profile>/<run>/<attempt>.tgz`, and `<attempt>-r1.tgz` for the
replacement rental after a provider fault (same run key, so a distinct object). The body is read and
written by the gateway Worker; the ledger Durable Object only reserves and settles a flag (DEPLOY.md,
"Evidence bucket"). The box protocol version stays 7: an old box never calls the route, and a box that calls
a Worker without it gets 404. The Worker's check of `vast_report.py` (`BOX_PROTOCOL = "7"`) is unchanged.

## Optional host claims (2026-11-06)

[CODE READ: scripts/ci/vast_report.py, ci/vast-broker/src/results.js, src/index.js] The box protocol
stays 7. The Worker adds the optional environment variable `KF_HOST_CLAIMS=1` to the box environment;
a reporter that sees it may add one optional key to the structured result, `host_claims`, a flat
object of at most eight bounded strings (`cpu_vendor` amd, intel, arm or other; `cpu_model`;
`cpu_arch`; `host_driver`; `linux_os_version`, `linux_driver`; `windows_os_version`,
`windows_driver`). They are claims: the Worker checks each with the same patterns as the box
(`cleanClaims`, shared vectors in `scripts/ci/host_claims_vectors.json`), drops an invalid field
instead of refusing the report, and no verdict reads them. Without the variable (an older Worker) the
reporter sends none, because an older Worker refuses any other top-level key of a result; a newer
Worker accepts a result without the key (an older box). `host_driver` and the GPU name already travel
in `hardware`; the Worker takes the host driver from `hardware.driver_version`.

## Optional run report (2026-11-07)

[CODE READ: scripts/ci/vast_run_report.py, scripts/ci/vast_onstart.sh, ci/vast-broker/src/runreport.js,
src/index.js; GPU-free tests only, nothing deployed] The box protocol stays 7. A Worker that has a
result store adds the optional environment variable `KF_RUN_REPORT=1` to the box environment (as for
`KF_HOST_CLAIMS`: the reporter's contract lists it as an optional name that must be `1`). Only a box that
sees it posts a report, so a newer box never calls a route an older Worker lacks, and an older box on a
newer Worker simply sends none. Why a separate route and not a key of the result (as `host_claims`):
the structured result must fit one report request of at most 65536 bytes, and a report can be 256 KiB.

`POST /run-report/<run>-<attempt>` uses the same run capability as `/report` (the live run token of this
rental, `Authorization: Bearer`), no query string, body `Content-Type: application/json`, at most
262144 bytes (`413 PAYLOAD_TOO_LARGE`; the gateway buffers at most that plus one byte). It is accepted
while the rental is not closed or ended (so before the result report; the box sends it after the payload and
before the result, bounded to 30 s by `vast_onstart.sh`). The body is a JSON object with `"schema": 1` and the
claims below; the Worker keeps the shape and drops the rest (unknown keys, rows of an unknown subrun, bad
enums, non-finite numbers, strings that are not printable ASCII of at most 200 characters, anything
token-shaped, URL-shaped or about SPECviewperf), fits it into 256 KiB, and stores it in the ledger until the
rental is settled, when the cron publishes it ([DEPLOY.md](../DEPLOY.md), "Public logs and run reports").
Answers: `200 {"state":"STORED"}`; `400 BAD_REQUEST` (not JSON, not an object, `schema` is not 1); `401`
(token or rental state); `404 REPORTS_DISABLED` (no result store) or an unknown route (an older Worker);
`409 REPORT_EXISTS` (one report per rental; the box counts it as sent); `413`; `429` (the per-rental rate of
`/report`). The box tries at most 3 times (network error, 429, 5xx) and treats every other answer as a
skipped report: the report never changes a result, a verdict, a deadline or the exit status.

Claims (`vast_run_report.py` builds them only from output the payload already prints; a field with no
source is empty, never invented): `guest_drivers` {linux, windows}; `stages` [{name, result PASS, FAIL,
SKIP, NOT_RUN, RETRY or UNATTRIBUTED, rc, duration_ms}]; `suites` [{name, pass, fail, skip}] (the graded rows of
the result file); `subruns` [{sub, kind, os, guest_driver, host_driver, verdict (PASS, FAIL, RETRY,
UNATTRIBUTED or NOT_RUN), reason (always present for NOT_RUN), stages, log_stages, started_at, ended_at,
duration_ms, counts {pass, fail, skip}}]; `tests` [{sub, suite, name, result PASS, FAIL, SKIP or REFUSED, ms}];
`apps` (always `[]` today: the application matrix is not a payload stage); `perf` [{sub, name, value, unit,
higher_is_better, suite, ref_value, ref_ratio}]; `windows` {gate, install_s, probes, resets,
live_dumps_193}. A host session (one rental) holds several subruns: `host` (bare-metal arms and the host CUDA
ladder, host driver), `linux-<guest driver>` (thin suite, birth census and the guest CUDA ladder),
`windows-<driver>` (the Windows gate); a subrun that did not run is NOT_RUN with a reason.
`sub` ids are `host`, `linux-<driver>`, `linux-unknown`, `windows-<driver>` or `windows`. Every test, perf and
app row names its `sub`. `duration_ms` of a subrun is the sum of the timed tests in it (the payload
prints no wall-clock timestamps). The offsets of a subrun in the published log (`log_range`) are NOT sent by
the box: the Worker finds them from the `KF_PAYLOAD_STAGE_START/END name=<stage>` lines of the log it wrote
(`log_stages` names the stages), because redaction and the Worker's own lines move every byte of the box's
file.

What the Worker sets itself when it publishes (never taken from the box): `run_id`, `commit`, `tree`, `tier`,
`die`, `family`, `gpu_name`, `host_driver`, `started_at`, `ended_at`, `verdict`, and `log` {url, bytes, sha256}
of the published log. Every other value is a claim of an untrusted box: shown as such, never a gate, never
an input to a requirement. SPECviewperf scores are never part of a report (docs/SPEC_RESULTS.md).

