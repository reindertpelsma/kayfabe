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
- Startup is checked before anything is printed: `serve` refuses (exit 3, message on stderr) if the inner port is already in use, before launching nginx. After launching, it waits up to 5 s, fails on early exit or an nginx `[emerg]` line, and runs an internal `GET /<token>/probe.txt` on 127.0.0.1 that must return the exact content. On any failure it prints the reason and the nginx error-log tail (token redacted), stops nginx, removes the runtime dir and exits 3, with nothing on stdout.
- The first stdout line (printed only after that probe passed) is JSON: `{"token", "port", "payloads": [seed_register payloads]}`. A payload is `{"artifact": {"name","sha256","size"}, "path": "/<token>/<name>"}` (`kfseed.core.seed_register_payload`, for the daemon).
- nginx missing: exit 2 and the install instructions (Debian/Ubuntu, Fedora, Alpine). `--fallback-python-server` is DEV/TEST ONLY (a range-capable stdlib server with the same layout); it is used only when nginx is absent (or with `--force-fallback`) and prints a warning.
- Strict serving and silent drop (owner decisions): only `GET`/`HEAD` of EXACT paths `/<token>/probe.txt` and `/<token>/<name>` (one `location =` per registered artifact, plus a server-level check of the raw request line) are served; Range works. Everything else gets NO reply: nginx closes the connection with `return 444` (zero bytes). That covers `/`, `/<token>` (no redirect: `absolute_redirect off`), `/<token>/`, `//`, `/./`, `%2e%2e`, `%2f`, query strings, absolute-URI requests, wrong token, unknown names, any other or lowercase method, any body (`Content-Length`/`Transfer-Encoding` present), and every error class: `error_page 400 401 403 404 405 408 413 414 416 431 494 500 502 503 504 505 = /_drop;` with `location = /_drop { internal; return 444; }`. (A named location `@drop` fails in nginx 1.28 with a 500 for `CONNECT`, which has no URI, so an internal URI is used.) No `return 30x` exists. Artifact names are limited to `[A-Za-z0-9._-]`, at most 100 characters, so they are safe to render into the config; anything else is refused at registration.
- Hardening directives (server block): `client_max_body_size 1;` (never 0, which means unlimited), `client_body_buffer_size 1k;`, `client_body_timeout 5s;`, `client_header_timeout 5s;`, `client_header_buffer_size 1k;`, `large_client_header_buffers 2 1k;`, `send_timeout 30s;` (the gap between writes, so large downloads are not cut; a 20 MB slow-reader test passes), `keepalive_timeout 15s;`, `keepalive_requests 100;`, `reset_timedout_connection on;` (a timed-out client sees a reset instead of a reply), `merge_slashes off;`.
- Known nginx limits (observed, needs the token): bare-LF line endings and a header line without a colon (`BadHeader`) are accepted and answered 200 on a valid path (`ignore_invalid_headers off` does not change that, so it is not set). `Expect`, `Upgrade`, `Range` that cannot be satisfied (416) and `HTTP/9.9` (505) are dropped for any path and token.
- Consequence for consumers: a wrong token or missing artifact looks like a connection error (`RemoteDisconnected`, curl exit 52), not a 404; treat it as "seeder unavailable" and fall back. The dev python server also closes without a reply for invalid paths and methods (it is less strict about encodings and bodies).
- `selfcheck` verifies: probe content, and that `GET /`, a wrong token, a directory under the token, a missing file and POST each get no reply (reported as `PASS ...: no reply (ok)`; any HTTP answer fails), and per artifact HEAD, sequential Range GETs (206 and exact Content-Range) with the sha256 of the assembled bytes, and a suffix range. Exit 1 and `FAIL` lines on any problem. Run it against the box's own inner port before sending `seed_register`; it does not prove the OUTER port is reachable (that is the Worker probe).

## Guarantees about what is served

- kfseed hashes what it serves: each artifact is copied (never linked by path) into the staging dir from a file descriptor opened with `O_NOFOLLOW` and checked with `fstat` (regular file, same device and inode as the pre-check), hashing while copying. A sha256 that differs from `--artifact NAME:SHA256` is refused (exit 3). The `seed_register` payload carries the sha256 and size computed from the STAGED bytes, so the Worker can treat them as seeder-reported.
- A swap of the source between validation and staging cannot publish another file: a symlink or a different inode is refused, and a swap after the open is irrelevant because the bytes come from the open descriptor. Symlinks and non-regular files are refused. The staging dir is 0755 and its files 0644, owned by the seeder user. Tests use a seam (`hooks`) for both swap points and assert a planted secret is never staged.
- Names are exactly `[A-Za-z0-9._-]{1,100}`, no leading dot, no `..`, not `probe.txt`. The token is exactly 8 characters `[a-z0-9]`. The probe text is `kfseed-probe-v1\n`.
- `kfseed check` can be run from any cwd: `python3 -I /abs/path/kfseed/__main__.py check` loads the package from the file's own directory and takes nothing from the cwd (a test plants shadowing `kfseed`, `json`, `argparse`, `subprocess`, `shutil` modules there and asserts none runs).
- TLS ClientHello and the HTTP/2 preface are answered with silence (checked by the coordinator by raw socket on 2026-11-12; the other raw-socket cases are in the tests).

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
