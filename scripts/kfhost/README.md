# kfhost: the custom-host daemon (work package W2)

STATUS: DESIGN, NOT RUN ON HARDWARE (2026-11-12). Revised the same day after the trust review (PR #345-#347) and four owner rulings; the borrowed mode is PARKED (see below). Written against the shared contract `CUSTOM_HOSTS_SPEC.md` (coordinator, 2026-11-12) and tested only against a fake transport. Nothing is deployed. The daemon never runs on Vast boxes.

## What it is

An enrolled, non-Vast machine runs `python3 -m kfhost run --config FILE`. The daemon holds one outbound WebSocket to the Worker's per-box Durable Object (W1), reports measured capabilities, takes job offers and, on `owned` boxes only, remote commands. Logs, progress and results never travel over the socket: they go over the existing HTTPS box protocol (`POST /report/<run>`, Bearer run token from the offer) through `scripts/ci/vast_report.py`'s `Reporter`.

## Commands

- `python3 -m kfhost setup --venv DIR`: explicit; creates DIR as a venv and installs `requirements.lock` (`websockets` at an exact version, `--require-hashes`). It touches only DIR. It refuses while the lock carries the `HASHES TODO` marker: the coordinator generates the hashes; none are invented here.
- `python3 -m kfhost check`: prints what is missing and how to install it; changes nothing.
- `python3 -m kfhost run --config FILE`: the daemon. The config is JSON, mode `0600` (group/world readable: refuses to start): `worker_url`, `box_id`, `box_token`, `access_id`, `access_secret`, `profile_ceiling` (`owned|container|borrowed`), `work_dir`; optional `repo_allowlist` (default: the public kayfabe repo), `ws_path`, `heartbeat_s`, `stage_allowlist`.

## Behaviour

Local flags decide what the daemon enforces; the Worker cannot loosen them. Config keys or command-line flags: `trust_git_only` / `--trust-git-only` (restriction; hidden old alias `--distrust-us`), `container` / `--container` (restriction), `allow_cmd` / `--allow-cmd` (power, default off). Default = `standard`. The effective profile is derived (`profile.derive`): restrictions apply if either side sets them (the welcome may add `profile: borrowed|container` or `trust_git_only`/`container: true`, and everything else in it is ignored, so it can never loosen); powers need both (`owned` = local `allow_cmd` AND welcome profile `owned`). `hello` carries `enforced` (the local flags).

- `hello` carries capabilities measured on the host (nvidia-smi, `/proc/driver/nvidia/version`, `/dev/kvm`, cpu, RAM, disk, container, display, `can_swap_driver`). Unmeasurable fields are `null`; nothing is guessed. Wire format follows the Worker: `compute_cap` is the string nvidia-smi prints (`"8.9"`), `pci_device_id` the 4-hex device id (lowercase, from `pci.device_id`), `gpu_name`. (Whether the Worker wants the 4-hex form or a `0x` prefix is an assumption.)
- Heartbeat every 15 s; reconnect with exponential backoff and jitter (cap 60 s). A dropped socket never stops a running job; the next heartbeat reports `busy` with the run id.
- Close codes: 4401 (bad token) stops with exit 3 and a clear log line. 4409 (another connection holds the slot) and 4403 (box disabled or a transient Access 403) do NOT exit: they log and retry slowly (4409: 120 s base, 600 s cap; 4403: 30 s base, 300 s cap).
- Frames: larger than 256 KiB or nested deeper than 32 are dropped before `json.loads`; a handler exception never kills the loop.
- Offers (`profile.check_offer`, every profile): `repo` must EXACTLY equal an entry of `repo_allowlist` (canonical URLs, default only the public kayfabe repo; no prefix or lookalike matching; git runs with `http.followRedirects=false`); `commit` 40 lowercase hex; `tree_key` 64 lowercase hex; stages from the one `profile.STAGES` constant; `deadline` is an integer in epoch milliseconds (seconds or a past value is refused). The offer is acked EARLY, as soon as these checks pass; after the fetch is verified a `ready` frame follows, a failed verification sends `nack`. The Worker re-offers idempotently by `offer_id`.
- Verification (`fetch.py`): standard, container and owned do what the Worker says, including the commit: any 40-hex SHA is fetched with a plain `git fetch origin <sha>` (redirects off, e.g. a PR-branch commit), and `git rev-parse HEAD` must equal it after checkout. The tree key is recomputed on the host (`treekey.py`, a port of `ci/vast-broker/src/treekey.js`, kf-tree-v2, checked against JS-generated vectors in `tests/fixtures/treekey_vectors.json`); a mismatch is a loud WARNING in the job log and `tree_key_mismatch: true` in the result (open question for W1: whether the result schema accepts that key), not a refusal. Only under the parked `--trust-git-only`: fetch ONLY `refs/heads/master` and `refs/heads/next` (no `refs/pull/*`, no arbitrary SHA), require the commit to be an ancestor of one of them (logic in `profile_borrowed.py`), and refuse a tree key mismatch.
- The job time cap comes from `deadline`: the run is cancelled when it passes.
- Jobs get a scrubbed environment: no config path, no box or Access token, no run token; a throwaway HOME/XDG/TMPDIR under `work_dir` (owned keeps its real HOME). The config file may not live inside `work_dir`.
- `cmd` frames: only with local `allow_cmd` AND profile `owned` (the Worker enabled it) AND an active `commands_until_ms` from the welcome. One at a time, timeout (default 300 s, max 3600 s), 256 KiB output bound; never during a run unless the frame sets `allow_during_run: true`.
- `prepare_seeder()` (`seeder.py`), once per session before the first `seed_register`: only profile `owned` AND root runs `apt-get update` then `apt-get install -y --only-upgrade nginx-light` (plain install if missing), logging `nginx -v` before and after. Every other profile never changes packages: it runs `kfseed check`, reads drift from its output (a `WARNING:` line = drifted; `candidate: unknown`; exit 0 = nginx present, exit 2 only when missing, any other exit = check unavailable, warn only) and warns in the job log and in the `seed_register` frame (`warnings`). Missing nginx that cannot be installed marks seeding unavailable for the session; an apt failure with nginx present only warns. kfseed runs as `python3 -I`, from a fresh temp cwd, with an absolute module location, so a planted module is never imported.
- `seed_register`: anything but `seed_ack {ok: true}` (or no answer in time) marks seeding unavailable until the next connection.

## One command for a host from any provider (hourly rentals)

`bootstrap.py` (stdlib only) pins the daemon to a commit, builds the venv, writes the 0600 config and starts the daemon, all inside one work dir; `--dry-run` prints the plan and changes nothing. Secrets come from files (mode 600) or stdin, never argv. Recipe the admin page would print (token written to a file first; `<sha>` is the pinned commit):

```
umask 077; mkdir -p kfh && cd kfh && read -rs T && printf %s "$T" > tok && read -rs S && printf %s "$S" > sec
curl -fsSLO https://raw.githubusercontent.com/reindertpelsma/kayfabe/<sha>/scripts/kfhost/bootstrap.py
python3 bootstrap.py --repo https://github.com/reindertpelsma/kayfabe --commit <sha> --worker-url <worker> \
  --box-id <id> --token-file tok --access-id <access-id> --access-secret-file sec -- --exit-when-idle 50
```

`--exit-when-idle MINUTES` (default off) makes the daemon exit 0 with a log line after that long with no job and no cmd running; the clock restarts after every job, and heartbeats report `idle_since`, so the owner or a supervisor can release the rental. Not verified: the bootstrap and recipe have only been run against a local git repo with fakes (no venv was built, `setup` is refused until the lock hashes exist, nothing was fetched over HTTPS).

## Running from a pinned commit

The daemon is the Python in the checked-out commit itself: Concretely: `git clone <public repo>`, `git checkout <sha>`, `cd scripts`, `python3 -m kfhost setup --venv DIR`, then `DIR/bin/python -m kfhost run --config FILE [--allow-cmd] [--container]`. Pin the sha you reviewed; nothing the Worker sends changes the daemon's own code.

## PARKED: borrowed mode (`--trust-git-only`), owner decision pending (2026-11-12)

The coordinator may move this mode out of the first merge. Its code is isolated for a one-step move: `profile_borrowed.py` (build flag `ENABLED = False`: the daemon refuses to start with `--trust-git-only` while it is False; a Worker-added restriction, which only tightens, is still applied), `downloads.py` (repo-pinned download resolver `pins_from_checkout`, `verify_attestation` interface that always answers False with "attestation verification not available"; never wired into a real fetch yet) and the `borrowed` branches in `profile.py`, `daemon.py`, `jobs.py`, `fetch.py`. Borrowed refuses root, sets `PR_SET_NO_NEW_PRIVS` (ctypes, best effort), refuses assignments naming a seed_url or other box artifacts, and may still seed. Not extended further until the decision.

## Wire contract (round 2)

`tests/fixtures/hosts-wire.json` is the Worker's golden fixture, copied byte for byte; `test_wire.py` parses every valid server frame with the daemon's real parsers, checks that every invalid offer in it is refused, and builds the daemon's real outgoing frames (`frames.py`) and checks their keys and value kinds against the fixture's valid frames. PENDING in the fixture (W1 is adding them; re-copy the fixture and the test must still pass): `ready`, `hello.enforced` (booleans, at most 8), `heartbeat.idle_since` (epoch milliseconds), `seed_register.warnings` (at most 5 strings of 200 chars), `welcome.commands_until_ms`, `cmd.commands_until_ms`, `offer.run` ({nonce, runner_nonce, sha}) and `assignment.seed`.

- Offer: `deadline_ms` (epoch ms; the old seconds field is refused); `offer_id` and `cmd_id` 16 hex; `run_id` `<run>-<attempt>`; the `run` section supplies the nonces; `assignment.seed` ({url, fallback_url, sha256, size, name}) is fetched by non-trust-git-only profiles with `kf_fetch` (SHA-256 and size pinned; the fallback URL is tried second by the daemon because `kf_fetch` has no `--fallback-url` flag) and any failure builds locally.
- Result: through the existing HTTPS result route with exactly {hardware, nonce, runner_nonce, sha, suites, verdict} (the Worker's `checkedResult`); `hardware` from `vast_payload.hardware()`. Without a `run` section no valid result exists, so none is sent and the job log says so. `tree_key_mismatch` goes only to `POST /run-report/<run>` (via `vast_run_report.build_report` and its Transport), never in the result.
- Ack semantics: `ack` once the profile checks pass, `ready` after fetch and verification, `nack {offer_id, reason}` on any failure after the ack too (a cancel or the deadline during the fetch aborts before the run starts). `cmd` frames honour `commands_until_ms` in the frame (else the welcome's); `cmd_output` is paced to 10 frames per second at most, 4 KiB per frame, 256 KiB per command.
- Upgrade refusals: a non-101 answer (3xx including an Access redirect, 403, 5xx; the adapter maps the status to 4000 plus the status) retries slowly, exponential from 30 s to 300 s. Box ids are `[a-z0-9][a-z0-9-]{0,39}`; seed names `[A-Za-z0-9][A-Za-z0-9._-]{0,99}` without `..`, size at least 1, path `/<8-char token>/<name>`.
- Secrets: `bootstrap.py` writes the config as `DIR/secrets/config.json` (0600) in a 0700 directory that is not the jobs' cwd and not an ancestor of it; jobs get a scrubbed environment. A job under the same uid can still read any file that uid owns: real isolation needs a separate uid or a sandbox (not built).
- Tree key: a port of `computeTreeKey` including the skip of wholly ignored top-level directories; on this repository the keys and the matcher budgets are equal (7,647,460 steps each, measured 2026-11-12).

## Not verified / open (do not read these as claims)

- The real `websockets` adapter (`transport.WebsocketsTransport`) is not exercised by any test; the Worker route (`ws_path`, provisional) and the box-token header are guesses until W1 fixes them.
- The stage-to-kfci-step mapping and the result shape (nonce binding) in `jobs.py` are provisional integration points, marked in code. The pending fields listed above are as the coordinator described them, not read from Worker code. Not run against real git over HTTPS (blob-filter fetch, GitHub redirects): tests use a local repo.
- Never run on a GPU or against the real Worker.

## Tests

from `scripts/`: `python3 -m unittest kfhost.test_kfhost kfhost.test_hardening kfhost.test_bootstrap_idle kfhost.test_wire`. Stdlib only, fake transport and fake probes, local git fixture.
