# kfseed

STATUS: DESIGN, NOT RUN ON HARDWARE, 2026-11-12. Implements W3 of `CUSTOM_HOSTS_SPEC.md` (the seeder side). Unit tests and a real-nginx integration test run on a laptop-class host; nothing here has run on a Vast box or a custom host, and the Worker probe (W1) is not wired yet.

A box serves one or more artifacts to the other boxes of the same session over plain HTTP, under a secret path token. Logic is Python (stdlib only); nginx does the serving.

## Use

```
python3 -m kfseed serve --root DIR --port INNER --token TOKEN --artifact NAME:SHA256 [--serve-until UNIX_TS]
python3 -m kfseed check            # same as: serve --check; read-only
python3 -m kfseed selfcheck --url http://127.0.0.1:PORT/TOKEN/ --artifact NAME:SHA256
```

- `--token` is optional; if omitted an 8 character `[a-z0-9]` token is generated with `secrets`. A given token must have that shape.
- `serve` creates a temp runtime dir (mode 0755, own pid file, own temp paths, removed on exit), a staging dir holding hard links (else copies) of ONLY the named artifacts plus `probe.txt` (`kfseed-probe-v1\n`), writes the nginx config (about 25 lines: `location /<token>/` with `alias`, `autoindex off`, `limit_except GET HEAD { deny all; }`, `sendfile on`, `access_log off`, `server_tokens off`, `location /` and `@drop` return 444), runs `nginx -t`, then `nginx -c <conf> -p <dir>` with `daemon off`. No system config or service is touched. It stops on SIGTERM/SIGINT or at `--serve-until`.
- The first stdout line is JSON: `{"token", "port", "payloads": [seed_register payloads]}`. A payload is `{"artifact": {"name","sha256","size"}, "path": "/<token>/<name>"}` (`kfseed.core.seed_register_payload`, for the daemon).
- nginx missing: exit 2 and the install instructions (Debian/Ubuntu, Fedora, Alpine). `--fallback-python-server` is DEV/TEST ONLY (a range-capable stdlib server with the same layout); it is used only when nginx is absent (or with `--force-fallback`) and prints a warning.
- Silent drop (owner decision): every invalid request gets NO reply, so a port scanner cannot tell an HTTP server is there. nginx uses `error_page 400 403 404 405 = @drop;` with `return 444` (connection closed, zero bytes). This covers `/`, a wrong token, a directory or missing file under the token, and any method other than GET/HEAD. Consequence: a consumer that gets a wrong token or a missing artifact sees a connection error (`RemoteDisconnected`, curl exit 52), not a 404, and must treat it as "seeder unavailable" and fall back. Valid GET/HEAD/Range requests under the token behave as before (200/206). The dev python server does the same (it closes the socket without a reply).
- `selfcheck` verifies: probe content, and that `GET /`, a wrong token, a directory under the token, a missing file and POST each get no reply (reported as `PASS ...: no reply (ok)`; any HTTP answer fails), and per artifact HEAD, sequential Range GETs (206 and exact Content-Range) with the sha256 of the assembled bytes, and a suffix range. Exit 1 and `FAIL` lines on any problem. Run it against the box's own inner port before sending `seed_register`; it does not prove the OUTER port is reachable (that is the Worker probe).

## nginx version and updates

nginx is not pinned and must stay current with security updates. `kfseed check` (or `serve --check`) prints `nginx installed: <nginx -v>` and, when `apt-cache`, `dnf` or `apk` is on PATH, `nginx candidate: <repo version>`; with no package manager it prints `candidate unknown`. It prints a `WARNING:` line when installed differs from the candidate (apt: package version strings, dnf/apk: upstream version numbers; the dnf and apk parsers are covered only by design, not by tests). It exits 2 only when nginx is missing. It never installs or updates anything, and neither does `serve`.

- Vast boxes: the box setup (a separate step, not in kfseed) runs `apt-get update`, then installs `nginx-light` when seeding is chosen.
- Long-lived custom hosts: enable unattended upgrades; `kfseed check` shows drift.

## Consumers (documentation only)

`scripts/ci/kf_fetch.py` already does what a consumer needs, with no change:

```
KF_FETCH_STREAMS=8 python3 -I scripts/ci/kf_fetch.py http://IP:PORT/TOKEN/NAME out.bin --sha256 SHA --size SIZE
```

It uses parallel Range streams, checkpoints, and pins the sha256 (a wrong byte fails). The URL, sha256 and size come from the Worker. The fallback is the caller's: wrap the fetch in a deadline (for example `timeout $((DEADLINE-NOW)) ...`); on non-zero exit, build the artifact locally. A seeder URL is plain `http://`; do not pass `--worker-auth` to it (the run token must go only to the Worker).

## Tests

`python3 -B -m unittest kfseed.tests.test_kfseed` from `scripts/`. The real-nginx class is skipped with the reason `nginx-not-installed` when nginx is not on PATH.

## Two-box test procedure (coordinator; not yet run)

1. Box A (seeder, any box with nginx and an open inbound TCP port, external port P mapped to inner port Q): build or fetch an artifact `X`, `sha256sum X`.
2. `python3 -m kfseed serve --root DIR --port Q --artifact X:SHA` and note the token.
3. On A: `python3 -m kfseed selfcheck --url http://127.0.0.1:Q/TOKEN/ --artifact X:SHA` must print `selfcheck: ok`.
4. On box B (different network): `curl -s http://A_IP:P/TOKEN/probe.txt` must print `kfseed-probe-v1`; `curl -si http://A_IP:P/` must fail with exit 52 (empty reply); then the `kf_fetch.py` command above must produce a file with the same sha256. Time it against the local build.
5. Stop A with SIGTERM; B's request must now fail to connect. Check `ls /tmp/kfseed-*` is empty on A.
6. Record: nginx version, A and B providers, speed, and whether the Worker (W1) can fetch the probe from a raw IP and non-standard port (UNVERIFIED so far).
