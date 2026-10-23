# Vast box scripts (repo side of the CI broker)
STATUS: code as of 2026-10-23. MEASURED: KVM run `v8-kvm-20261023` returned FAIL/setup_cache;
unattended environment loading and original-grace MANUAL retention worked. The follow-up
acknowledgement fix has GPU-free tests only. [V8 evidence](../../traces/vast_setup_v8_20261018/LIVE_RUN.md).
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
CODE READ (Worker, `origin/codex/vast-broker-smoke-20261022`: `src/index.js`, `src/results.js`,
`docs/BOX_PROTOCOL.md`): endpoints, schema, caps, rate 20 per 10 s, hold needs a recorded result.
MEASURED: `python3 -m unittest scripts/ci/test_vast_box.py`, `scripts/ci/vast_box_mutants.py`.
INFERRED: apt, `/run`, profile.d, sshd on the real KVM image. The payload is code from the SHA under
test, runs as root and can read the env file.

CODE READ: `kf-box-blob-url <tag> <name>` resolves a private release asset using only the run's
scoped token; the token stays on curl stdin. The payload first restores the dependency cache
from the [reviewed lock](setup-cache.lock.json), then delegates to the existing box provision
and merge bar. [Cache design and timing table](../../docs/design/VAST_SETUP_CACHE_20261023.md).

CODE READ: dependency setup waits for the reporter's first accepted progress acknowledgement
through the internal KF_BROKER_READY_FD pipe. This prevents private blob fetches racing Vast's
created-to-running transition. The pipe carries no token; existing deadlines and terminal auth
refusals remain in force. Ordinary payloads retain the offline log path.
