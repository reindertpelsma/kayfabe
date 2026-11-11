# kfhost: the custom-host daemon (work package W2)

STATUS: DESIGN, NOT RUN ON HARDWARE (2026-11-12). Written against the shared contract `CUSTOM_HOSTS_SPEC.md` (coordinator, 2026-11-12) and tested only against a fake transport. Nothing is deployed. The daemon never runs on Vast boxes.

## What it is

An enrolled, non-Vast machine runs `python3 -m kfhost run --config FILE`. The daemon holds one outbound WebSocket to the Worker's per-box Durable Object (W1), reports measured capabilities, takes job offers and, on `owned` boxes only, remote commands. Logs, progress and results never travel over the socket: they go over the existing HTTPS box protocol (`POST /report/<run>`, Bearer run token from the offer) through `scripts/ci/vast_report.py`'s `Reporter`.

## Commands

- `python3 -m kfhost setup --venv DIR`: explicit; creates DIR as a venv and installs `requirements.lock` (`websockets` at an exact version, `--require-hashes`). It touches only DIR. It refuses while the lock carries the `HASHES TODO` marker: the coordinator generates the hashes; none are invented here.
- `python3 -m kfhost check`: prints what is missing and how to install it; changes nothing.
- `python3 -m kfhost run --config FILE`: the daemon. The config is JSON, mode `0600` (group/world readable: refuses to start): `worker_url`, `box_id`, `box_token`, `access_id`, `access_secret`, `profile_ceiling` (`owned|container|borrowed`), `work_dir`; optional `repo_allowlist` (default: the public kayfabe repo), `ws_path`, `heartbeat_s`, `stage_allowlist`.

## Behaviour

- `hello` carries capabilities measured on the host (nvidia-smi, `/proc/driver/nvidia/version`, `/dev/kvm`, cpu, RAM, disk, container, display, `can_swap_driver` = root and not a container). Unmeasurable fields are `null`; nothing is guessed. `compute_cap` is the integer form of the box protocol (8.9 is 890).
- Heartbeat every 15 s (the welcome may change it); reconnect with exponential backoff and jitter (cap 60 s). A dropped socket never stops a running job; the next heartbeat reports `busy` with the run id.
- Close codes: 4401 and 4403 stop with exit 3 and no retry; 4409 (replaced by a newer connection) exits 0.
- Effective profile = the strictest of the server's welcome profile and the local ceiling (an unknown value counts as `borrowed`).
- `borrowed`: offers carry only `repo`, `commit`, `tree_key`, `stages`; repo must be in the allowlist, commit 40 lowercase hex, stages from a fixed list. The commit is fetched with `git` from the public URL into a temp dir under `work_dir` and `git rev-parse HEAD` must equal the commit, else the offer is nacked and nothing runs. Runs in standalone mode (no sudo, no installs, minimal environment, no `cmd`).
- `container`: no `cmd` frames, no driver swaps; `env` (`KF_*` names) allowed. `owned`: `cmd` frames run one at a time with a timeout (default 300 s, max 3600 s), 256 KiB output bound, chunked `cmd_output` and a `cmd_done`; never while a run is active unless the frame sets `allow_during_run: true`.
- `seed_register`: a job calls `Daemon.seed_register`; anything but `seed_ack {ok: true}` (or no answer in time) marks seeding unavailable until the next connection.

## Not verified / open (do not read these as claims)

- The real `websockets` adapter (`transport.WebsocketsTransport`) is not exercised by any test; the Worker route (`ws_path`, provisional) and the box-token header are guesses until W1 fixes them.
- The stage allowlist, the stage-to-kfci-step mapping and the result shape (nonce binding) in `jobs.py` are provisional integration points, marked in code. `tree_key` is validated for shape only, not compared to the checkout.
- Never run on a GPU or against the real Worker.

## Tests

`python3 -m unittest scripts/kfhost/test_kfhost.py` (from `scripts/`: `python3 -m unittest kfhost.test_kfhost`). Stdlib only, fake transport and fake probes, local git fixture.
