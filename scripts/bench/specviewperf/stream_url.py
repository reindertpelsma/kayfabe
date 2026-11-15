# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Stream one object of a presigned-URL JSON file to stdout, checking its sha256 on the fly (stdlib only).

STATUS: CODE, 2026-11-12 (branch claude/spec-pc-20261112; PC run, not on the Worker path). The stage's normal input is
the Worker's /bench-data route; this helper serves a run where the disk cannot hold the archive: the bytes go to stdout
(zstd | tarfilter | guest), the hash is computed as they pass, and the exit status says whether they matched the pin.
Exit 0 = whole object read and the sha256 equals the pin; 3 = read to the end and the sha256 differs; 1 = transport failure
after the retries. A resume uses a Range request at the byte already delivered. The URL is never printed or logged.
"""

import argparse
import hashlib
import json
import sys
import time
import urllib.request
from pathlib import Path


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--sha256", required=True)
    ap.add_argument("--status", required=True)
    ap.add_argument("--retries", type=int, default=20)
    a = ap.parse_args(argv)
    url = json.loads(Path(a.url_file).read_text())[a.key]
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
    ok = h.hexdigest() == a.sha256
    Path(a.status).write_text("STREAM_%s bytes=%d secs=%d sha256=%s\n" % ("OK" if ok else "MISMATCH", pos, time.time() - t0, h.hexdigest()))
    return 0 if ok else 3


if __name__ == "__main__":
    sys.exit(main())
