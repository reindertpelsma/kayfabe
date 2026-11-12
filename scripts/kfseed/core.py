# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure pieces: token, artifact specs, nginx config text, staging dir, seed_register payload."""
import os
import re
import secrets
import shutil
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
    if not sep or not NAME_RE.match(name) or name == PROBE_NAME or not SHA_RE.match(sha):
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
        error_page 400 401 403 404 405 408 413 414 431 494 500 502 503 504 = /_drop;
        absolute_redirect off;
        merge_slashes off;
        if ($request !~ "^(GET|HEAD) /{token}/({alts}) HTTP/1\\.[01]$") {{ return 444; }}
        location = /_drop {{ internal; return 444; }}
        if ($request_method !~ ^(GET|HEAD)$) {{ return 444; }}
        if ($http_content_length) {{ return 444; }}
        if ($http_transfer_encoding) {{ return 444; }}
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
        if not NAME_RE.match(n) or n == PROBE_NAME:
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


def build_stage(stage_dir, root, artifacts):
    """Create stage_dir with ONLY the registered artifacts (hard link, else copy) and probe.txt."""
    os.makedirs(stage_dir, mode=0o755, exist_ok=True)
    os.chmod(stage_dir, 0o755)
    sizes = {}
    for name, _sha in artifacts:
        src = os.path.join(root, name)
        if os.path.islink(src) or not os.path.isfile(src):
            raise SeedError("artifact %s is not a regular file under the root" % name)
        dst = os.path.join(stage_dir, name)
        try:
            os.link(src, dst)
            if not os.stat(dst).st_mode & stat.S_IROTH:
                os.unlink(dst)
                raise OSError
        except OSError:
            shutil.copyfile(src, dst)
            os.chmod(dst, 0o644)
        sizes[name] = os.stat(dst).st_size
    probe = os.path.join(stage_dir, PROBE_NAME)
    with open(probe, "wb") as f:
        f.write(PROBE_CONTENT)
    os.chmod(probe, 0o644)
    return sizes


def seed_register_payload(name, sha256, size, token):
    check_token(token)
    if not NAME_RE.match(name) or not SHA_RE.match(sha256) or not isinstance(size, int) or size < 0:
        raise SeedError("bad artifact fields")
    return {"artifact": {"name": name, "sha256": sha256, "size": size}, "path": "/%s/%s" % (token, name)}
