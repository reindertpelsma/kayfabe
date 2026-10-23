# Box interface

CODE READ: box protocol 6; the repo reporter declares `BOX_PROTOCOL = "6"` and accepts only
that value. Before renting, the Worker reads `scripts/ci/vast_report.py` at the exact tested SHA
and refuses (`NOT_ELIGIBLE`) unless that line names the Worker's protocol. Runner response protocol remains 4. The bootstrap clones the public repository, fetches and checks out the exact approved SHA, verifies HEAD, and execs the configured `scripts/ci/vast_onstart.sh`. Transport and payload logic live in the checkout.

| Variable | Format |
| --- | --- |
| KF_PROTOCOL | Decimal protocol, currently 6 |
| KF_BROKER_URL | HTTPS origin plus required secret path, no trailing slash |
| KF_RUN_ID / KF_RUN_TOKEN | Positive run-attempt / this run's HMAC capability |
| KF_NONCE / KF_SHA / KF_RUNNER_NONCE | 32 / 40 / 64 lowercase hex |
| KF_REPO | Public owner/repository |
| KF_SOFT_DEADLINE | Required Worker-clock absolute epoch seconds |
| KF_BOOT_DEADLINE / KF_HARD_DEADLINE | Optional absolute epoch seconds; validated when present |
| KF_HEARTBEAT_SECONDS | Interval in seconds |
| KF_GRACE_FAIL | Required grace ceiling in seconds |
| KF_GRACE_PASS | Optional grace ceiling in seconds |

CODE READ: Vast create `env` is a JSON object mapping variable names to string values; values use a restricted shell-safe alphabet. Repo scripts write the environment to root-only `/run/kf-ci/env`. No Vast key, owner stop token, signing secret or private-repository token reaches a box. Its operator can inspect its own scoped capability.

CODE READ: `POST /report/<run>` uses Bearer auth and JSON `{seq,kind,text?,result?}`; kind is
progress/log/result, sequences increase by one. A result binds nonce, SHA, runner_nonce, verdict,
suite counts and hardware. `POST /kill/<run>` takes a short reason. Retries reuse the same
payload/sequence. The backup is `/workspace/kf-ci-output.log`. Missing/invalid payload output fails
closed. HTTPS only; no redirects. Capabilities expire with their own run. GPU results remain
unattested tripwires; approved code precedes rental.

CODE READ: `GET /blob-url/<run>/<tag>/<name>` uses the same run capability. Strict ASCII tag/name, no query string. It is available while the provider confirms this run is running and its next deadline has not expired, including unexpired result grace. Default limits are eight mint attempts/run and 64/day; failed upstream attempts count. BLOB_REPO, BLOB_GITHUB_TOKEN and optional JSON BLOB_ALLOWED_TAG_PREFIXES configure the private store. The GitHub credential stays in the Worker. Metadata is bounded to 64 KiB; the asset endpoint must return a single manual 302 to an exact allowed GitHub asset hostname. The Worker cancels its body without reading binary bytes and returns only `{url,size,name}`. URLs are secret, neither stored nor logged; both Worker and runner redact URL text independently. The payload's own hash lock is authoritative; downloads use HTTPS without a repository auth header or redirects. `kf-box-blob-url <tag> <name>` is installed repo-side.

[CODE READ: scripts/ci/README.md, scripts/ci/vast_report.py; PR #104 e63af44dd] The payload writes `/workspace/kf-result.json` containing `{verdict,suites,hardware?}`. Each suite has `{name,pass,fail,skip}`; hardware has `{gpu_name,compute_cap,driver_version}` with integer capability (8.6 becomes 860). The reporter adds SHA, nonce and runner nonce from the validated environment. Conflicting bindings in the file fail closed. Both reporter and Worker refuse PASS without valid hardware. The protocol-6 Worker supplies every optional deadline/grace field too.

[CODE READ: src/index.js, src/routes.js; owner ruling 2026-10-23 19:10] There is no hold, manual or
owner timer-extension mode: a run ends at its result grace, heartbeat, boot or hard deadline, by
owner stop/stop-all or by `kf-box-kill`. The Worker injects no Cloudflare Access credential into a
box. Worker caps remain 20 requests/10 seconds, 2000 requests and 4 MiB per run.

[CODE READ: src/index.js, runner.py] Structured Worker status includes the selected provider `country` (two uppercase letters or `UNKNOWN`), separate from payload hardware. The runner accepts it as an explicitly optional protocol-4 metadata field, validates it independently, and labels it untrusted. The repo-side result-file contract is unchanged; the environment is protocol 6.
