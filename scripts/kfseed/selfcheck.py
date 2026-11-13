# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Self-check of a seeder URL: probe, HEAD, Range GETs, sha256, and the negative cases. No redirects."""
import hashlib
import http.client
import urllib.parse

from .core import PROBE_CONTENT, PROBE_NAME, SeedError

CHUNK = 8 * 1024 * 1024


def _req(base, method, rel, headers=None, read=True):
    u = urllib.parse.urlsplit(base)
    if u.scheme != "http" or not u.hostname:
        raise SeedError("selfcheck URL must be http://host:port/TOKEN/")
    conn = http.client.HTTPConnection(u.hostname, u.port or 80, timeout=10)
    try:
        conn.request(method, u.path.rstrip("/") + "/" + rel if rel is not None else "/", headers=headers or {})
        r = conn.getresponse()
        return r.status, dict((k.lower(), v) for k, v in r.getheaders()), (r.read() if read and method != "HEAD" else b"")
    finally:
        conn.close()


def silent(host, port, method, path):
    """True when the server closes the connection without sending any reply (nginx 444)."""
    c = http.client.HTTPConnection(host, port, timeout=10)
    try:
        c.request(method, path)
        c.getresponse().read()
        return False
    except (http.client.RemoteDisconnected, ConnectionResetError, BrokenPipeError):
        return True
    finally:
        c.close()


def selfcheck(url, artifacts, chunk=CHUNK, report=None):
    """Return a list of failure strings (empty = pass). artifacts: [(name, sha256)].

    Invalid requests must get NO reply (silent drop); each such case is appended to report as a line.
    """
    bad = []
    report = report if report is not None else []
    u = urllib.parse.urlsplit(url)
    s, _, body = _req(url, "GET", PROBE_NAME)
    if s != 200 or body != PROBE_CONTENT:
        bad.append("probe: status %s or content mismatch" % s)
    parts = u.path.strip("/").split("/")
    base = u.path.rstrip("/")
    other = "zzzzzzzz" if parts[0] != "zzzzzzzz" else "yyyyyyyy"
    for label, method, path in (
            ("GET /", "GET", "/"),
            ("wrong token", "GET", "/%s/%s" % (other, PROBE_NAME)),
            ("directory under token", "GET", base + "/"),
            ("missing file", "GET", base + "/no-such-file"),
            ("POST", "POST", base + "/" + PROBE_NAME)):
        if silent(u.hostname, u.port or 80, method, path):
            report.append("%s: no reply (ok)" % label)
        else:
            bad.append("%s must get no reply (connection closed)" % label)
    for name, sha in artifacts:
        s, h, _ = _req(url, "HEAD", name)
        try:
            size = int(h.get("content-length", ""))
        except ValueError:
            size = -1
        if s != 200 or size < 0:
            bad.append("%s: HEAD status %s" % (name, s))
            continue
        digest = hashlib.sha256()
        pos = 0
        while pos < size:
            end = min(pos + chunk, size) - 1
            s, h, data = _req(url, "GET", name, {"Range": "bytes=%d-%d" % (pos, end)})
            if s != 206 or h.get("content-range") != "bytes %d-%d/%d" % (pos, end, size) or len(data) != end - pos + 1:
                bad.append("%s: Range %d-%d answered %s" % (name, pos, end, s))
                break
            digest.update(data)
            pos = end + 1
        else:
            if digest.hexdigest() != sha:
                bad.append("%s: sha256 mismatch" % name)
        if size:
            s, _, data = _req(url, "GET", name, {"Range": "bytes=-1"})
            if s != 206 or len(data) != 1:
                bad.append("%s: suffix Range answered %s" % (name, s))
    return bad
