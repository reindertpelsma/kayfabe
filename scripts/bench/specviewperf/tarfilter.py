#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Keep one lane's viewsets of the SPECviewperf tree in a tar stream (stdlib only, GPU-free).

STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_SPEC_GATE.md). GPU-free tests only
(scripts/ci/test_vast_spec_lane.py); the stream form was used by hand on 2026-11-05
(traces/specviewperf_vast_20261105/README.md, "SPEC tree into the guest").

    zstd -d -c --long=31 specviewperf-15.0.0.tar.zst | tarfilter.py --lane lanea --manifest FILE \
        | <guest tar -xf - -C C:/>

A member is DROPPED when a path component names a viewset directory `<id>-<x.y.z>` of a viewset that
is not in the lane (for example `Program Files/SPECviewperf 15/viewsets/blender-01-1.0.1/...` for lane
B). Everything else is kept, so the CLI, its libraries and the redistributable runtime stay. The input
was verified against its sha256 pin before it is streamed (fetch_bench_data.sh), so the filter is not a
trust boundary; it still fails closed on what a tree of ours never holds: an absolute path, a `..`
component, a link, a device or a FIFO (exit 3, class MEDIA for the stage).

The manifest (written when the stream ends) has one line per kept viewset directory,
`VIEWSET <dir> files=<n> bytes=<n>`, which the stage compares with what the guest holds after the copy,
and one `KEPT members=<n> bytes=<n> dropped=<n>` line. No score is in a tree (the tree is the tool).
"""

import argparse
import re
import sys
import tarfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lane as lane_mod  # noqa: E402

DIRNAME = re.compile(r"^([a-z][a-z_]*-[0-9]{2})-[0-9]+\.[0-9]+\.[0-9]+$")


class Refused(Exception):
    pass


def viewset_of(name):
    """(viewset id, directory name) of the first component of `name` that is a viewset directory."""
    for part in name.split("/"):
        m = DIRNAME.match(part)
        if m:
            return m.group(1), part
    return None, None


def keep(name, wanted):
    """True when the member stays in the stream."""
    vid, _ = viewset_of(name)
    return vid is None or vid in wanted


def check_name(name):
    parts = name.split("/")
    if not name or name.startswith("/") or ".." in parts or "\\" in name or "\x00" in name or re.match(r"^[A-Za-z]:", name):
        raise Refused("path")


class Tail:
    """Reads `source` and remembers its last 1024 bytes: a complete tar ends with two zero blocks."""

    def __init__(self, source):
        self.source, self.tail = source, b""

    def read(self, n=-1):
        data = self.source.read(n)
        self.tail = (self.tail + data)[-1024:]
        return data


def filter_stream(source, sink, wanted, manifest):
    """Copy the wanted members of the tar stream `source` to `sink`; return the manifest lines. A stream that
    does not end with the tar end-of-archive blocks (cut short) is an error: tarfile alone reads it as complete."""
    stats = {}
    kept = dropped = nbytes = 0
    source = Tail(source)
    with tarfile.open(fileobj=source, mode="r|") as tin, tarfile.open(fileobj=sink, mode="w|", format=tarfile.GNU_FORMAT) as tout:
        for member in tin:
            check_name(member.name)
            if not (member.isfile() or member.isdir()):
                raise Refused("member type")
            if not keep(member.name, wanted):
                dropped += 1
                continue
            vid, dirname = viewset_of(member.name)
            if member.isfile():
                tout.addfile(member, tin.extractfile(member))
                nbytes += member.size
                if vid is not None:
                    f, b = stats.get(dirname, (0, 0))
                    stats[dirname] = (f + 1, b + member.size)
            else:
                tout.addfile(member)
                if vid is not None:
                    stats.setdefault(dirname, (0, 0))
            kept += 1
        while source.read(1 << 20):
            pass
    if source.tail != b"\0" * 1024:
        raise tarfile.ReadError("the stream ends without the end-of-archive blocks")
    lines = ["VIEWSET %s files=%d bytes=%d" % (d, f, b) for d, (f, b) in sorted(stats.items())]
    lines.append("KEPT members=%d bytes=%d dropped=%d" % (kept, nbytes, dropped))
    Path(manifest).write_text("\n".join(lines) + "\n")
    return lines


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--lane", required=True)
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--table", default=str(lane_mod.TABLE))
    a = ap.parse_args(argv)
    try:
        wanted = set(lane_mod.lane_ids(a.lane, lane_mod.table(a.table)))
    except (ValueError, KeyError, OSError):
        print("tarfilter: unknown lane", file=sys.stderr)
        return 2
    try:
        filter_stream(sys.stdin.buffer, sys.stdout.buffer, wanted, a.manifest)
    except Refused as e:
        print("tarfilter: refused member (%s)" % e, file=sys.stderr)
        return 3
    except (tarfile.TarError, OSError) as e:
        print("tarfilter: stream error (%s)" % type(e).__name__, file=sys.stderr)
        return 4
    return 0


if __name__ == "__main__":
    sys.exit(main())
