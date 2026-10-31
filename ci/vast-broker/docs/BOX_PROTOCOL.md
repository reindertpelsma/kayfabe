# Box interface

STATUS: LIVE, 2026-10-24. Box protocol 7 (since 2026-10-23); runner status protocol 6 (was 5
until the edits2 round of 2026-10-24). Owner
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

CODE READ 2026-10-31 (`src/benchdata.js`, `src/index.js`; not deployed by the change that added it): `GET /bench-data/<run>-<attempt>/<name>` uses the same run capability and the same live-run checks as `/blob-url`, no query string, `<name>` from a fixed four-name map. A permitted request gets `302` with `Cache-Control: no-store` to a presigned S3 GET URL for the private R2 benchmark bucket, valid at most 1200 s and not past the run's next deadline; limits are 4 URLs per object per run and a country allow-list (`BENCH_DATA_ALLOWED_COUNTRIES`, default `US,CA`, judged on the request's `CF-IPCountry`). Refusals: 401 (token/run state), 403 (country), 404 (name), 429 (URL limit), 503 (R2 settings missing). The box helper is `scripts/bench/box/fetch_bench_data.sh`; the details are in DEPLOY.md.

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
