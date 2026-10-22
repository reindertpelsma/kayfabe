# Vast box scripts (repo side of the CI broker)
[CODE READ: PR #104 e63af44dd; scripts/ci/vast_onstart.sh] Repo-side implementation; real-box acceptance remains pending.
The Worker's tiny onstart clones the repo, checks out `KF_SHA`, execs `scripts/ci/vast_onstart.sh`.
Per-run data is `KF_*` environment variables. Files: `vast_onstart.sh` (tools, env check, root-only
env file `/run/kf-ci/env`, helpers, reporter, payload under `timeout`); `vast_report.py` (stdlib:
`check-env [--write F]`, `daemon --log L --done D --result R`); `kf-box-kill`, `kf-box-hold`,
`kf-ci-hold.sh` (`/etc/profile.d` hook); `test_vast_box.py`, `vast_box_mutants.py`.

Env (protocol 4 or 5). Required: `KF_PROTOCOL`, `KF_BROKER_URL` (https origin + secret path, no
trailing slash), `KF_RUN_ID`, `KF_RUN_TOKEN` (must match the run id), `KF_NONCE` (32 hex), `KF_SHA`
(40), `KF_RUNNER_NONCE` (64), `KF_REPO`, `KF_SOFT_DEADLINE` (epoch s), `KF_HEARTBEAT_SECONDS`,
`KF_GRACE_FAIL`. Checked if present: `KF_BOOT_DEADLINE`, `KF_HARD_DEADLINE`, `KF_GRACE_PASS`. A
violation posts a log naming variables (never values) and, if nonce/sha/runner nonce are valid, a
FAIL result `env_contract`; the payload does not run.

Payload: `bash scripts/ci/vast_payload.sh <sha> <nonce>`, cwd = checkout, stdin closed, output to
`/workspace/kf-ci-output.log` (offline backup). It gets `KF_SOFT_DEADLINE_MS` and loses the token,
broker URL, run id and runner nonce. It writes `/workspace/kf-result.json`:
`{verdict, suites:[{name,pass,fail,skip}], hardware?}`, hardware = `{gpu_name, compute_cap (int,
8.6 -> 860), driver_version}`. Nonce/sha/runner nonce come from the environment; a conflicting
value in the file makes it invalid. PASS is withheld (FAIL + failing row) on non-zero exit, skips,
zero passes, unknown hardware or compute_cap < 750. Soft deadline: TERM, KILL after 30 s; result is
FAIL with the suites already written plus `timeout`. Absent payload: FAIL `payload_missing`; absent
or invalid result file: FAIL `result_missing`/`result_invalid`. Exit text never makes a PASS.

Reporter: `POST <KF_BROKER_URL>/report/<run>`, Bearer token, JSON `{seq,kind,text?,result?}`. A
message keeps its sequence number and bytes until acknowledged (idempotent, no gaps); 401/403/404
stop reporting; 429, 5xx, 409 NOT_READY retry with backoff; 3xx is never followed. Caps: 8 KiB per
post, 4000 bytes per line, 1500 log posts, 3 MB of log bodies, 1950 requests, 1 s between requests.
Heartbeat (`log`, empty text) when no post succeeded for `KF_HEARTBEAT_SECONDS`. Nothing is
stripped (Worker and runner sanitise). The token goes only to `KF_BROKER_URL`, HTTPS, no proxy.

`kf-box-kill [reason]` POSTs `/kill/<run>`, `kf-box-hold` POSTs `/hold/<run>`: curl `--proto =https
--max-redirs 0`, token on stdin, non-zero unless HTTP 200. The hook runs `kf-box-hold` detached once
per interactive SSH login and ignores every failure.
[CODE READ: ci/vast-broker/src/index.js, ci/vast-broker/src/results.js,
ci/vast-broker/docs/BOX_PROTOCOL.md] endpoints, schema, caps, rate 20 per 10 s, hold needs a recorded result.
[MEASURED: v5-pr104-20261022; traces/vast_broker_v5_20261018/pr104-alignment.json] Canonical
Python suite: 72/72. Mutation evidence distinguishes the Worker campaign from PR #104
's own campaign; no real box has run these scripts yet.
[INFERRED: scripts/ci/vast_onstart.sh] apt, `/run`, profile.d, sshd on the real KVM image. The payload is code from the SHA under
test, runs as root and can read the env file.
