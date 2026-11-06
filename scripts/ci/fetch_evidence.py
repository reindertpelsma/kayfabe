#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Coordinator tool: list and download failed-run evidence bundles from the private R2 bucket.

STATUS: LIVE, 2026-11-06 (GPU-free tests only: test_vast_fetch_evidence.py; never run against a
real bucket). NOT part of CI and never run by a box, a runner or a Worker: it needs the
coordinator's own R2 credentials, read from the ENVIRONMENT only (never from an argument, never
printed): CF_ACCOUNT_ID (the 32-hex Cloudflare account id), R2_ACCESS_KEY_ID and
R2_SECRET_ACCESS_KEY (an R2 API token with Object Read on the evidence bucket; the Worker's own
write binding is not an S3 key). Bucket: kayfabe-ci-evidence (override with --bucket).

    fetch_evidence.py list [--run RUN]            keys, sizes and times (run = the numeric run id)
    fetch_evidence.py get KEY|RUN-ATTEMPT... --out DIR
                                                  download <profile>/<run>/<attempt>.tgz and .json
                                                  into DIR (new or empty), verify SHA-256 against
                                                  the manifest, never extract
    fetch_evidence.py members FILE.tgz            list the member names of a downloaded bundle

Without credentials, `wrangler r2 object get kayfabe-ci-evidence/<profile>/<run>/<attempt>.tgz
--file out.tgz --remote` does the same for one known key (DEPLOY.md, "Evidence bucket").

A bundle is DATA from a box that ran untrusted code: read it, never run anything from it, and
never extract it with a tool that follows member paths. `members` lists names only; to read one
file use `python3 -I -c` with tarfile and `extractfile`, or extract into an empty scratch
directory with `tar --no-same-owner --no-same-permissions` and open the files as text.
"""

import argparse
import datetime
import hashlib
import hmac
import json
import os
import re
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

BUCKET = "kayfabe-ci-evidence"
MAX_OBJECT = 16 * 1024 * 1024
MAX_LIST_PAGES = 20
KEY = re.compile(r"^(check|sweep|release)/([1-9][0-9]{0,19})/([1-9][0-9]{0,19})\.(tgz|json)$")
RUN_ATTEMPT = re.compile(r"^([1-9][0-9]{0,19})-([1-9][0-9]{0,19})$")
EMPTY_SHA = hashlib.sha256(b"").hexdigest()


class Refused(Exception):
    pass


def _hmac(key, text):
    return hmac.new(key, text.encode(), hashlib.sha256).digest()


def sigv4_headers(method, url, headers, access, secret, region, service, now, payload=EMPTY_SHA):
    """AWS Signature V4 (header form) for one request; returns the headers to send.

    `headers` are signed as given (lowercased); x-amz-date is added. Checked against the AWS
    test-suite vector `get-vanilla` in test_vast_fetch_evidence.py.
    """
    parts = urllib.parse.urlsplit(url)
    stamp = now.strftime("%Y%m%dT%H%M%SZ")
    day = stamp[:8]
    signed = {k.lower(): str(v).strip() for k, v in headers.items()}
    signed["host"] = parts.netloc
    signed["x-amz-date"] = stamp
    query = sorted(
        (urllib.parse.quote(k, safe="-_.~"), urllib.parse.quote(v, safe="-_.~"))
        for k, v in urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
    )
    names = sorted(signed)
    canonical = "\n".join(
        [
            method,
            urllib.parse.quote(parts.path or "/", safe="/-_.~"),
            "&".join("%s=%s" % kv for kv in query),
            "".join("%s:%s\n" % (n, signed[n]) for n in names),
            ";".join(names),
            payload,
        ]
    )
    scope = "%s/%s/%s/aws4_request" % (day, region, service)
    to_sign = "\n".join(
        ["AWS4-HMAC-SHA256", stamp, scope, hashlib.sha256(canonical.encode()).hexdigest()]
    )
    key = _hmac(("AWS4" + secret).encode(), day)
    for part in (region, service, "aws4_request"):
        key = _hmac(key, part)
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    out = dict(signed)
    out["authorization"] = "AWS4-HMAC-SHA256 Credential=%s/%s, SignedHeaders=%s, Signature=%s" % (
        access,
        scope,
        ";".join(names),
        signature,
    )
    return out


class R2:
    def __init__(self, env, bucket=BUCKET, opener=None, now=None):
        account = env.get("CF_ACCOUNT_ID", "")
        access, secret = env.get("R2_ACCESS_KEY_ID", ""), env.get("R2_SECRET_ACCESS_KEY", "")
        if not re.fullmatch(r"[0-9a-f]{32}", account) or not access or not secret:
            raise Refused("set CF_ACCOUNT_ID, R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY")
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,61}[a-z0-9]", bucket):
            raise Refused("bad bucket name")
        self.base = "https://%s.r2.cloudflarestorage.com/%s" % (account, bucket)
        self.access, self.secret = access, secret
        self.opener = opener or urllib.request.build_opener(urllib.request.HTTPSHandler())
        self.now = now or (lambda: datetime.datetime.now(datetime.timezone.utc))

    def get(self, path="", query=None, limit=MAX_OBJECT):
        url = self.base + ("/" + path if path else "")
        if query:
            url += "?" + urllib.parse.urlencode(query)
        headers = sigv4_headers(
            "GET",
            url,
            {"x-amz-content-sha256": EMPTY_SHA},
            self.access,
            self.secret,
            "auto",
            "s3",
            self.now(),
        )
        headers.pop("host")
        request = urllib.request.Request(url, headers=headers, method="GET")
        try:
            with self.opener.open(request, timeout=60) as response:
                data = response.read(limit + 1)
        except urllib.error.HTTPError as error:
            raise Refused("R2 answered HTTP %d" % error.code) from None
        except (urllib.error.URLError, OSError) as error:
            raise Refused("R2 not reachable: %s" % type(error).__name__) from None
        if len(data) > limit:
            raise Refused("object larger than %d bytes" % limit)
        return data

    def list(self, run=None):
        rows, token = [], None
        for _ in range(MAX_LIST_PAGES):
            query = {"list-type": "2", "max-keys": "1000"}
            if token:
                query["continuation-token"] = token
            root = ET.fromstring(self.get("", query, limit=4 * 1024 * 1024))
            ns = root.tag.split("}")[0] + "}" if root.tag.startswith("{") else ""
            for item in root.findall(ns + "Contents"):
                key = item.findtext(ns + "Key") or ""
                match = KEY.match(key)
                if match and (run is None or match.group(2) == run):
                    rows.append(
                        (
                            key,
                            int(item.findtext(ns + "Size") or 0),
                            item.findtext(ns + "LastModified"),
                        )
                    )
            if root.findtext(ns + "IsTruncated") != "true":
                break
            token = root.findtext(ns + "NextContinuationToken")
        return sorted(rows)


def keys_for(spec, rows):
    """A KEY or RUN-ATTEMPT (profile found by listing) -> the bundle key; refuses anything else."""
    match = KEY.match(spec)
    if match and match.group(4) == "tgz":
        return spec
    pair = RUN_ATTEMPT.match(spec)
    if not pair:
        raise Refused("not a key or run-attempt: %r" % spec[:60])
    hits = [k for k, _, _ in rows if k.endswith("/%s/%s.tgz" % pair.groups())]
    if len(hits) != 1:
        raise Refused("%s: %d bundles match" % (spec, len(hits)))
    return hits[0]


def download(r2, key, out):
    """Fetch <key> and its manifest into `out`; verify the SHA-256; never extract."""
    run, attempt = KEY.match(key).group(2, 3)
    bundle = r2.get(key)
    sha = hashlib.sha256(bundle).hexdigest()
    try:
        manifest = json.loads(r2.get(key[: -len(".tgz")] + ".json", limit=1024 * 1024))
    except (Refused, ValueError):
        manifest = None
    out.mkdir(parents=True, exist_ok=True)
    stem = "%s-%s" % (run, attempt)
    (out / (stem + ".tgz")).write_bytes(bundle)
    status = "no manifest"
    if manifest is not None:
        (out / (stem + ".json")).write_text(json.dumps(manifest, indent=1, sort_keys=True))
        status = "sha256 ok" if manifest.get("sha256") == sha else "SHA256 MISMATCH"
    print("%s %d bytes sha256 %s (%s)" % (stem, len(bundle), sha[:12], status))
    return status != "SHA256 MISMATCH"


def members(path):
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            kind = "f" if member.isreg() else "?"
            print("%s %9d %s" % (kind, member.size, re.sub(r"[^\x20-\x7e]", "?", member.name)))


def main(argv=None, env=None, r2=None):
    top = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    top.add_argument("--bucket", default=BUCKET)
    sub = top.add_subparsers(dest="command", required=True)
    p = sub.add_parser("list")
    p.add_argument("--run")
    p = sub.add_parser("get")
    p.add_argument("specs", nargs="+")
    p.add_argument("--out", required=True)
    p = sub.add_parser("members")
    p.add_argument("file")
    args = top.parse_args(argv)
    try:
        if args.command == "members":
            members(args.file)
            return 0
        r2 = r2 or R2(os.environ if env is None else env, args.bucket)
        if args.command == "list":
            if args.run is not None and not re.fullmatch(r"[1-9][0-9]{0,19}", args.run):
                raise Refused("--run is the numeric run id")
            for key, size, when in r2.list(args.run):
                print("%s %d %s" % (key, size, when))
            return 0
        out = Path(args.out)
        if out.exists() and (not out.is_dir() or any(out.iterdir())):
            raise Refused("--out must be a new or empty directory")
        rows = r2.list() if any(not KEY.match(s) for s in args.specs) else []
        ok = all(download(r2, keys_for(s, rows), out) for s in args.specs)
        return 0 if ok else 1
    except Refused as error:
        print("fetch_evidence: %s" % error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
