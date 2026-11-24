# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Stream one object of a presigned-URL JSON file to stdout, checking its sha256 on the fly (stdlib only).

STATUS: CODE, 2026-11-12 (branch claude/spec-pc-20261112; PC run, not on the Worker path); 2026-11-16 it also reads the standalone release-run URL file (pick_url). The stage's normal input is
the Worker's /bench-data route; this helper serves a run where the disk cannot hold the archive: the bytes go to stdout
(zstd | tarfilter | guest), the hash is computed as they pass, and the exit status says whether they matched the pin.
Exit 0 = whole object read and the sha256 equals the pin; 3 = read to the end and the sha256 differs; 1 = transport failure
after the retries. A resume uses a Range request at the byte already delivered. The URL is never printed or logged.

2026-11-17 (docs/OWNER_RULINGS.md section AX): the URL file is the USER'S input, so our git pin is not a reason to refuse it. `--sha256` is
the pin that is ENFORCED (the Worker/strict path, or the user's own --expect-sha256); `--note-pin` is a pin that is only COMPARED: the
status line says pin=match|differs and the exit status is 0 either way. With neither, the status line records the computed sha256 and size.
"""

import argparse
import hashlib
import json
import sys
import time
import urllib.request
from pathlib import Path


URLS_SCHEMA = "kayfabe.download-urls/1"


def pick_url(doc, key):
    """The URL for `key` from either file shape: the flat PC-run map {key: url}, or the standalone-run file of
    scripts/ci/box_downloader.py (schema kayfabe.download-urls/1, {"urls": {ref: url}}), where the ref is the object's
    basename (the name the Worker's bench-data route knows: specviewperf-15.0.0.tar.zst). Exactly one shape is accepted:
    a document that names the schema is never read as a flat map. A missing key is a KeyError (no guessing)."""
    if type(doc) is dict and doc.get("schema") == URLS_SCHEMA:
        urls = doc.get("urls")
        if type(urls) is not dict:
            raise KeyError("urls")
        return urls[key] if key in urls else urls[key.rsplit("/", 1)[-1]]
    return doc[key]


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--sha256", help="enforced pin: exit 3 when the stream differs")
    ap.add_argument("--note-pin", help="recorded pin: compared, never enforced")
    ap.add_argument("--status", required=True)
    ap.add_argument("--retries", type=int, default=20)
    a = ap.parse_args(argv)
    url = pick_url(json.loads(Path(a.url_file).read_text()), a.key)
    out = sys.stdout.buffer
    h = hashlib.sha256()
    pos = 0
    total = None
    sink_open = True
    tries = 0
    t0 = time.time()
    while True:
        req = urllib.request.Request(url, headers={"Range": "bytes=%d-" % pos} if pos else {})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                if total is None:
                    cr = r.headers.get("Content-Range")
                    total = int(cr.split("/")[1]) if cr else int(r.headers.get("Content-Length", "0"))
                while True:
                    b = r.read(1 << 20)
                    if not b:
                        break
                    h.update(b)
                    pos += len(b)
                    if sink_open:
                        try:
                            out.write(b)
                        except BrokenPipeError:
                            sink_open = False   # the consumer ended early: keep reading, the hash needs every byte
            if total and pos < total:
                raise OSError("short read")
            break
        except Exception as e:  # noqa: BLE001 - any transport error: resume
            tries += 1
            print("stream_url: transport error %s at byte %d (try %d)" % (type(e).__name__, pos, tries), file=sys.stderr)
            if tries > a.retries:
                Path(a.status).write_text("STREAM_FAIL bytes=%d\n" % pos)
                return 1
            time.sleep(min(30, 2 * tries))
    try:
        out.flush()
    except BrokenPipeError:
        pass
    ok = a.sha256 is None or h.hexdigest() == a.sha256
    noted = "none" if a.note_pin is None else ("match" if h.hexdigest() == a.note_pin else "differs")
    Path(a.status).write_text("STREAM_%s bytes=%d secs=%d sha256=%s pin=%s\n" % ("OK" if ok else "MISMATCH", pos, time.time() - t0, h.hexdigest(), noted))
    return 0 if ok else 3


if __name__ == "__main__":
    sys.exit(main())
