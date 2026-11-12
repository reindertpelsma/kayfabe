# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure pieces: token, artifact specs, nginx config text, staging dir, seed_register payload."""
import hashlib
import os
import re
import secrets
import stat

PROBE_NAME = "probe.txt"
PROBE_CONTENT = b"kfseed-probe-v1\n"
TOKEN_RE = re.compile(r"^[a-z0-9]{8}$")
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"


class SeedError(Exception):
    pass


def make_token():
    return "".join(secrets.choice(ALPHABET) for _ in range(8))


def check_token(token):
    if not isinstance(token, str) or not TOKEN_RE.match(token):
        raise SeedError("token must be 8 characters [a-z0-9]")
    return token


def parse_artifact(spec):
    name, sep, sha = spec.partition(":")
    if not sep or not NAME_RE.match(name) or ".." in name or name == PROBE_NAME or not SHA_RE.match(sha):
        raise SeedError("artifact must be NAME:SHA256 (NAME [A-Za-z0-9._-], sha256 lowercase hex, not %s)" % PROBE_NAME)
    return name, sha


CONFIG_TEMPLATE = """\
daemon off;
pid {run}/nginx.pid;
error_log {run}/error.log warn;
events {{ worker_connections 256; }}
http {{
    access_log off;
    server_tokens off;
    sendfile on;
    client_body_temp_path {run}/tmp/body;
    proxy_temp_path {run}/tmp/proxy;
    fastcgi_temp_path {run}/tmp/fastcgi;
    uwsgi_temp_path {run}/tmp/uwsgi;
    scgi_temp_path {run}/tmp/scgi;
    server {{
        listen {port} default_server;
        error_page 400 401 403 404 405 408 413 414 416 431 494 500 502 503 504 505 = /_drop;
        absolute_redirect off;
        merge_slashes off;
        if ($request !~ "^(GET|HEAD) /{token}/({alts}) HTTP/1\\.[01]$") {{ return 444; }}
        location = /_drop {{ internal; return 444; }}
        if ($request_method !~ ^(GET|HEAD)$) {{ return 444; }}
        if ($http_content_length) {{ return 444; }}
        if ($http_transfer_encoding) {{ return 444; }}
        if ($http_expect) {{ return 444; }}
        if ($http_upgrade) {{ return 444; }}
        client_max_body_size 1;
        client_body_buffer_size 1k;
        client_body_timeout 5s;
        client_header_timeout 5s;
        client_header_buffer_size 1k;
        large_client_header_buffers 2 1k;
        send_timeout 30s;
        keepalive_timeout 15s;
        keepalive_requests 100;
        reset_timedout_connection on;
{locations}
        location / {{ return 444; }}
    }}
}}
"""


LOCATION = """\
        location = /{token}/{name} {{
            alias {stage}/{name};
            autoindex off;
            limit_except GET HEAD {{ deny all; }}
        }}"""


def render_config(run_dir, stage_dir, port, token, names):
    """names: the registered artifact names; only exact-match locations for them and probe.txt are served."""
    check_token(token)
    for n in names:
        if not NAME_RE.match(n) or ".." in n or n == PROBE_NAME:
            raise SeedError("unsafe artifact name %r" % n)
    if not isinstance(port, int) or not 1 <= port <= 65535:
        raise SeedError("port out of range")
    for p in (run_dir, stage_dir):
        if not re.match(r"^/[A-Za-z0-9._/-]+$", p):
            raise SeedError("unsafe path for nginx config: %r" % p)
    stage = stage_dir.rstrip("/")
    alts = "|".join(re.escape(n) for n in [PROBE_NAME, *names])
    locs = "\n".join(LOCATION.format(token=token, name=n, stage=stage) for n in [PROBE_NAME, *names])
    return CONFIG_TEMPLATE.format(run=run_dir, stage=stage, port=port, token=token, locations=locs, alts=alts)


def build_stage(stage_dir, root, artifacts, hooks=None):
    """Create stage_dir with ONLY the registered artifacts (copied) and probe.txt.

    Each source is opened with O_NOFOLLOW and checked with fstat on the open fd (regular file); the bytes served are
    the bytes copied from that fd, hashed while copying, and must match the registered sha256. Returns
    {name: (sha256, size)} of the STAGED bytes. hooks: test seam {"before_open": f(path), "after_open": f(path)}.
    """
    hooks = hooks or {}
    os.makedirs(stage_dir, mode=0o755, exist_ok=True)
    os.chmod(stage_dir, 0o755)
    staged = {}
    for name, sha in artifacts:
        if not NAME_RE.match(name) or ".." in name or name == PROBE_NAME:
            raise SeedError("unsafe artifact name %r" % name)
        src = os.path.join(root, name)
        st = os.lstat(src) if os.path.lexists(src) else None
        if st is None or not stat.S_ISREG(st.st_mode):
            raise SeedError("artifact %s is not a regular file under the root (symlinks are refused)" % name)
        if hooks.get("before_open"):
            hooks["before_open"](src)
        try:
            fd = os.open(src, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError as e:
            raise SeedError("artifact %s cannot be opened without following links: %s" % (name, e.strerror)) from None
        try:
            fst = os.fstat(fd)
            if not stat.S_ISREG(fst.st_mode) or (fst.st_dev, fst.st_ino) != (st.st_dev, st.st_ino):
                raise SeedError("artifact %s changed or is not a regular file" % name)
            if hooks.get("after_open"):
                hooks["after_open"](src)
            dst = os.path.join(stage_dir, name)
            h = hashlib.sha256()
            size = 0
            out = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o644)
            try:
                while True:
                    data = os.read(fd, 1 << 20)
                    if not data:
                        break
                    h.update(data)
                    size += len(data)
                    view = memoryview(data)
                    while view:
                        view = view[os.write(out, view):]
                os.fchmod(out, 0o644)
            finally:
                os.close(out)
        finally:
            os.close(fd)
        if h.hexdigest() != sha:
            os.unlink(dst)
            raise SeedError("artifact %s: sha256 of the staged bytes differs from the registered value" % name)
        staged[name] = (h.hexdigest(), size)
    probe = os.path.join(stage_dir, PROBE_NAME)
    with open(probe, "wb") as f:
        f.write(PROBE_CONTENT)
    os.chmod(probe, 0o644)
    return staged


def seed_register_payload(name, sha256, size, token):
    check_token(token)
    if not NAME_RE.match(name) or not SHA_RE.match(sha256) or not isinstance(size, int) or size < 0:
        raise SeedError("bad artifact fields")
    return {"artifact": {"name": name, "sha256": sha256, "size": size}, "path": "/%s/%s" % (token, name)}
