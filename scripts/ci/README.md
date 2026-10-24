# Vast box scripts (repo side of the CI broker)
STATUS: LIVE, 2026-10-24: box protocol 7 (ported from protocol 4/5 on branch
claude/vast-payload-p7-20261024; no real box has run protocol 7 yet). The evidence below is for
the pre-port payload. MEASURED: run `v9-xid-kvm-20261023`, source
`2c908a0e8`: bound PASS, host/fat CUDA4/4 each, tests3749/0, gates9/9+10,
raw30/30, thin30/30 and census. [Latest evidence and gaps](../../traces/vast_xid_v9_20261018/LIVE_RUN.md).
The earlier V8 run stopped at raw defer-liveness wrapper rc6; that finding is superseded by
S4 exact-source admission and the complete V9 result. Its journal was not retained for replay.
The Worker's tiny onstart clones the repo, checks out `KF_SHA`, execs `scripts/ci/vast_onstart.sh`.
Per-run data is `KF_*` environment variables. Files: `vast_onstart.sh` (tools, env check, root-only
env file `/run/kf-ci/env`, helpers, reporter, payload under `timeout`); `vast_report.py` (stdlib:
`check-env [--write F]`, `daemon --log L --done D --result R`); `kf-box-kill`, `kf-box-blob-url`;
`test_vast_box.py`, `vast_box_mutants.py`. There is no hold helper or login hook (owner ruling
2026-10-23 19:10; the Worker has no hold route). The box sends the Worker-injected Cloudflare
Access pair (`KF_ACCESS_ID`, `KF_ACCESS_SECRET`) as `CF-Access-Client-Id`/`-Secret` on every
broker call when both are set (owner ruling 2026-10-23 21:45); the payload never sees it.

Env (protocol 7, declared by `BOX_PROTOCOL` in `vast_report.py`; the Worker reads that line at
the tested SHA before renting). Required: `KF_PROTOCOL`, `KF_BROKER_URL` (https origin + secret
path, no trailing slash), `KF_RUN_ID`, `KF_RUN_TOKEN` (a compact HS256 JWT; the box only checks
its shape), `KF_NONCE` (32 hex), `KF_SHA` (40), `KF_RUNNER_NONCE` (64), `KF_REPO`,
`KF_SOFT_DEADLINE` (epoch s), `KF_HEARTBEAT_SECONDS`, `KF_GRACE_FAIL`. Checked if present:
`KF_BOOT_DEADLINE`, `KF_HARD_DEADLINE`, `KF_GRACE_PASS`, `KF_ACCESS_ID`, `KF_ACCESS_SECRET`. A
violation posts a log naming variables (never values) and, if nonce/sha/runner nonce are valid, a
FAIL result `env_contract`; the payload does not run.

Payload: `bash scripts/ci/vast_payload.sh <sha> <nonce>`, cwd = checkout, stdin closed, output to
`/workspace/kf-ci-output.log` (offline backup). It gets `KF_SOFT_DEADLINE_MS` and loses the token,
broker URL, run id, runner nonce and Access pair. It writes `/workspace/kf-result.json`:
`{verdict, suites:[{name,pass,fail,skip}], hardware?, diagnostics?}`, hardware = `{gpu_name, compute_cap (int,
8.6 -> 860), driver_version}`. Nonce/sha/runner nonce come from the environment; a conflicting
value in the file makes it invalid. PASS is withheld (FAIL + failing row) on non-zero exit, skips,
zero passes, unknown hardware or compute_cap < 750. Soft deadline: TERM, KILL after 30 s; result is
FAIL with the suites already written plus `timeout`. Absent payload: FAIL `payload_missing`; absent
or invalid result file: FAIL `result_missing`/`result_invalid`. Exit text never makes a PASS.

CODE READ: optional local `diagnostics` records contain `arm, mode, rc, admitted, text,
sha256, source_bytes, dropped_bytes`. Previews retain head/tail:256 UTF-8 bytes for a clean
arm,2048 for a refused arm; the first refusal is preserved when compacting. Limits:128
records,49152 bytes of JSON including escapes, within the existing65536-byte result-file cap.
Hashes identify the original UTF-8 input text before preview truncation; they confer no trust.
The reporter validates this local field and emits the same existing wire result keys; the
full bounded per-arm context continues through the authenticated log stream. No Worker
schema, bundle, admission rule or verdict comes from diagnostic text. This local-file change
was rehearsed without a new rental; the V9 live proof predates it.

Reporter: `POST <KF_BROKER_URL>/report/<run>`, Bearer token, JSON `{seq,kind,text?,result?}`. A
message keeps its sequence number and bytes until acknowledged (idempotent, no gaps); 401/403/404
stop reporting; 429, 5xx, 409 NOT_READY retry with backoff; 3xx is never followed. Caps: 8 KiB per
post, 4000 bytes per line, 1500 log posts, 3 MB of log bodies, 1950 requests, 1 s between requests.
Heartbeat (`log`, empty text) when no post succeeded for `KF_HEARTBEAT_SECONDS`. Nothing is
stripped (Worker and runner sanitise). The token goes only to `KF_BROKER_URL`, HTTPS, no proxy.

`kf-box-kill [reason]` POSTs `/kill/<run>`: curl `--proto =https --max-redirs 0`, token (and the
Access pair, when both halves are set) on stdin, non-zero unless HTTP 200.
CODE READ (Worker on master: `ci/vast-broker/src/{routes,reports,results,onstart}.js`,
`docs/BOX_PROTOCOL.md`): endpoints, schema, caps, rate 20 per 10 s.
MEASURED: `python3 -m unittest discover -s scripts/ci -p 'test_vast*.py'`,
`scripts/ci/vast_box_mutants.py`; `ci/vast-broker/test/box-scripts-p7.test.js` runs the real
reporter against the in-process Worker.
INFERRED: apt, `/run`, sshd on the real KVM image. The payload is code from the SHA under
test, runs as root and can read the env file.

CODE READ: `kf-box-blob-url <tag> <name>` resolves a private release asset using only the run's
scoped token; the token stays on curl stdin. The payload first restores the dependency cache
from the [reviewed lock](setup-cache.lock.json), then delegates to the existing box provision
and merge bar. [Cache design and timing table](../../docs/design/VAST_SETUP_CACHE_20261023.md).

CODE READ: dependency setup waits for the reporter's first accepted progress acknowledgement
through the internal KF_BROKER_READY_FD pipe. The pipe carries no token; reporter auth
refusals remain terminal. Ordinary payloads retain the offline log path.
The Worker serves blob URLs only once Vast reports running. The box helper retries failed GETs,
including 401 during created, with curl's exponential backoff (8 retries, 20 s per attempt),
under a 299 s hard deadline. The Worker's running-state requirement is unchanged.

CODE READ: host APT runs in the existing guarded provisioners; cache restore prepares files only.
The test runtime is Ubuntu 22.04 with GNU coreutils (`setup-test.Dockerfile`), matching the box.

CODE READ: if cloud-init omits standard HOME or leaves it empty, repo-side onstart restores the
current OS account's home directory. An explicit process HOME is preserved; no Worker var is added.
