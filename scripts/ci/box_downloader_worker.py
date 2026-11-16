#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bootstrap-layer helper: ask the Vast CI Worker for presigned URLs and write the URL file box_downloader.py reads.

STATUS: LIVE, 2026-11-15 (GPU-free tests only). This is the ONLY code of the downloader that knows the Worker, the run
token or the broker URL; box_downloader.py never reads them, and the payload never runs this. The bootstrap layer
(which already holds the token, OWNER_RULINGS section AR) runs it, then starts the downloader with
`--urls FILE`; the payload stays tokenless. Route and rules are those of scripts/bench/box/fetch_bench_data.sh:
GET <broker>/bench-data/<run>/<name> with the Bearer token (and the Access pair when both are set), no redirect
followed, HTTPS only, no proxy; the 302 Location is the presigned URL (never sent the token, never printed).
Each URL lives at most 20 minutes and the Worker allows 4 per object per run, so refresh sparingly.

    KF_RUN_TOKEN=... KF_BROKER_URL=... KF_RUN_ID=... box_downloader_worker.py resolve MANIFEST --urls-out FILE
"""
import argparse
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
import box_downloader as bd  # noqa: E402
import kf_fetch  # noqa: E402

sys.path.pop(0)


class WorkerError(Exception):
    pass


def worker_signed_url(broker, run, token, ref, access=None, *, allow_http=False, opener=None):
    """One Worker call: the presigned URL for object `ref`. `access` is an optional (id, secret) pair."""
    if not re.fullmatch(r"[0-9]+-[0-9]+", run) or not re.fullmatch(r"[A-Za-z0-9._-]+", token) or not bd.NAME_RE.fullmatch(ref):
        raise WorkerError("arguments")
    base = broker.rstrip("/")
    try:
        kf_fetch.valid_url(base, secure=not allow_http)
    except kf_fetch.FetchError:
        raise WorkerError("broker") from None
    headers = {"Authorization": "Bearer " + token, "User-Agent": "kayfabe-bench/1.0"}
    if access:
        if not all(re.fullmatch(r"[A-Za-z0-9._-]+", v) for v in access):
            raise WorkerError("access")
        headers["CF-Access-Client-Id"], headers["CF-Access-Client-Secret"] = access
    request = urllib.request.Request(f"{base}/bench-data/{run}/{ref}", headers=headers)
    opener = opener or urllib.request.build_opener(urllib.request.ProxyHandler({}), kf_fetch.NoRedirect())
    try:
        response = opener.open(request, timeout=30)
    except urllib.error.HTTPError as e:
        response = e
    except (OSError, ValueError):
        raise WorkerError("network") from None
    with response:
        if response.status not in (301, 302, 303, 307, 308):
            raise WorkerError("status")
        location = response.headers.get("Location", "")
    try:
        return kf_fetch.valid_url(location, secure=not allow_http)
    except kf_fetch.FetchError:
        raise WorkerError("location") from None


def write_url_file(path, urls):
    """Atomic, mode 0600; the content is a set of capability URLs."""
    tmp = Path(str(path) + ".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump({"schema": bd.URLS_SCHEMA, "urls": urls}, f, sort_keys=True)
    os.replace(tmp, path)


def main(argv=None, environ=None, allow_http=False):
    environ = os.environ if environ is None else environ
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("cmd", choices=["resolve"])
    p.add_argument("manifest")
    p.add_argument("--urls-out", required=True)
    p.add_argument("--only", nargs="+", default=[])
    p.add_argument("--stages", default="")
    args = p.parse_args(argv)
    try:
        manifest = bd.load_manifest(args.manifest, allow_http)
        access = None
        if environ.get("KF_ACCESS_ID") or environ.get("KF_ACCESS_SECRET"):
            if not (environ.get("KF_ACCESS_ID") and environ.get("KF_ACCESS_SECRET")):
                raise WorkerError("access")
            access = (environ["KF_ACCESS_ID"], environ["KF_ACCESS_SECRET"])
        refs = sorted({s.ref for i in manifest.ordered(tuple(args.only), tuple(x for x in args.stages.split(":") if x))
                       for s in i.sources if s.kind == "signed"})
        urls = {r: worker_signed_url(environ.get("KF_BROKER_URL") or environ.get("WORKER_URL") or "", environ.get("KF_RUN_ID", ""),
                                     environ.get("KF_RUN_TOKEN", ""), r, access, allow_http=allow_http) for r in refs}
        write_url_file(args.urls_out, urls)
        print(f"KF_URLS_WRITTEN refs={len(urls)}", flush=True)
        return 0
    except (WorkerError, bd.ManifestError, OSError):
        print("KF_URLS_FAIL", flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
