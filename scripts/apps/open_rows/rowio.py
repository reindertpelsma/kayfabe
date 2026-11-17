#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""How the open rows hand their files to a comparison: a directory, or the row's own log (stdlib only; GPU-free tested).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 7).

A row run under run_apps.sh leaves one log, and for a guest run that log is the only thing copied out of the guest. So a row that is
started with --emit prints each result file between markers, and a comparison reads either DIR/<name> or the same text out of the log:

    @@FILE results.tsv sha256=<16 hex> bytes=<n>
    ...the file...
    @@END results.tsv
"""
import hashlib
from pathlib import Path
import re

BEGIN = re.compile(r"^@@FILE (\S+) sha256=([0-9a-f]{16}) bytes=(\d+)$")


def emit(name, text):
    data = text if text.endswith("\n") else text + "\n"
    digest = hashlib.sha256(data.encode()).hexdigest()[:16]
    return "@@FILE %s sha256=%s bytes=%d\n%s@@END %s\n" % (name, digest, len(data.encode()), data, name)


def extract(log_text):
    """-> {name: text} for every complete block whose byte count and digest match. A truncated or edited block is dropped, never repaired."""
    out, cur, buf, want = {}, None, [], None
    for line in log_text.splitlines():
        m = BEGIN.match(line)
        if m and cur is None:
            cur, buf, want = m[1], [], (m[2], int(m[3]))
            continue
        if cur is not None and line == "@@END " + cur:
            body = "".join(l + "\n" for l in buf)
            if len(body.encode()) == want[1] and hashlib.sha256(body.encode()).hexdigest()[:16] == want[0]:
                out[cur] = body
            cur = None
            continue
        if cur is not None:
            buf.append(line)
    return out


def load(path, name):
    """The text of file `name` of a run given as a directory or as a log file; None when it is not there."""
    p = Path(path)
    if p.is_dir():
        f = p / name
        return f.read_text() if f.is_file() else None
    if p.is_file():
        return extract(p.read_text(errors="replace")).get(name)
    return None
