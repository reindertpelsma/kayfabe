#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Propose an update of ci/vast-broker/bandwidth.json from the bytes real runs actually moved (stdlib only; never writes the file).

STATUS: DESIGN-ONLY, 2026-11-16 (docs/design/V3_WORKER_PROPIMAGES.md, "Bandwidth"). GPU-free tests: scripts/ci/test_bandwidth_calibrate.py.

The payload records, per stage, the rx/tx bytes of the box's default interface in its result file (`net_bytes`, kf-result.json).
Give this script one or more of those files and the instruction the runs were started with:

    python3 -B -I scripts/ci/bandwidth_calibrate.py --instruction instruction.json run1/kf-result.json run2/kf-result.json

It prints a diff of the table (and warnings), nothing else changes. Editing = run it, review the diff, apply it (`... | patch -p0`
or edit by hand), run `bandwidth_check.py`, commit. What it proposes:
  - the entry of a stage marked `calibrate: true` becomes (mean actual download of the stage) minus (the stage's other entries), rounded
    up to 10 MB, basis `measured`, today's date, a source naming the files; a negative remainder becomes 0 and is reported;
  - the stage's upload becomes the mean actual upload, rounded up to 10 KB;
  - a stage with no `calibrate` entry whose actual download is more than 25 percent away from the table is only REPORTED.
The repo clone happens before the first stage and is not attributable to one: it is not calibrated here.
"""

import argparse
import datetime
import difflib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import bandwidth_check as check  # noqa: E402

DOWN_ROUND = 10_000_000
UP_ROUND = 10_000
REPORT_RATIO = 0.25


def read_runs(paths):
    """{stage: [(rx, tx), ...]} over the result files; a file without `net_bytes` is an error naming it."""
    stages, errors = {}, []
    for path in paths:
        try:
            doc = json.loads(Path(path).read_text())
            rows = doc["net_bytes"]["stages"]
        except (OSError, ValueError, KeyError, TypeError):
            errors.append("NO_NET_BYTES:" + str(path))
            continue
        for name, row in rows.items():
            stages.setdefault(name, []).append((row["rx"], row["tx"]))
    return stages, errors


def _up(n, step):
    return -(-int(n) // step) * step


def propose(data, runs, items, sources, today):
    """(new table, [report line]). `items` are the table's own selection for the runs' instruction (bandwidth_check.explain)."""
    new = json.loads(json.dumps(data))
    notes = []
    for name, samples in sorted(runs.items()):
        stage = new["stages"].get(name)
        if stage is None:
            notes.append("UNKNOWN_STAGE:" + name)
            continue
        rx = sum(s[0] for s in samples) / len(samples)
        tx = sum(s[1] for s in samples) / len(samples)
        mine = [i for i in items if i["stage"] == name]
        others = sum(i["down"] for i in mine if i["id"] not in {e["id"] for e in stage["entries"] if e.get("calibrate")})
        target = next((e for e in stage["entries"] if e.get("calibrate")), None)
        source = "calibrated from %d run(s): %s; stage rx mean %d, min %d, max %d" % (
            len(samples), ", ".join(sources), rx, min(s[0] for s in samples), max(s[0] for s in samples))
        if target is not None:
            rest = rx - others
            if rest < 0:
                notes.append("NEGATIVE_REMAINDER:%s: actual %d is below the other entries %d; %s set to 0" % (name, rx, others, target["id"]))
                rest = 0
            target.update(down_bytes=_up(rest, DOWN_ROUND), basis="measured", date=today.isoformat(), source=source,
                          note="calibrated: the stage's actual download minus its other entries")
            target.pop("size_from", None)
        else:
            table = sum(i["down"] for i in mine)
            if table and abs(rx - table) / table > REPORT_RATIO:
                notes.append("NO_CALIBRATE_ENTRY:%s: actual rx %d against the table's %d (more than %d percent apart); edit its entries by hand"
                             % (name, rx, table, REPORT_RATIO * 100))
        stage["upload"].update(up_bytes=_up(tx, UP_ROUND), basis="measured", date=today.isoformat(), source=source,
                               note="calibrated: the stage's actual upload")
    return new, notes


def dump(data):
    return json.dumps(data, indent=1) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("results", nargs="+", help="kf-result.json files of one or more runs (with net_bytes)")
    parser.add_argument("--instruction", required=True, help="the instruction file the runs were started with")
    parser.add_argument("--tier", default=None)
    parser.add_argument("--file", default=str(check.DATA))
    parser.add_argument("--today", default=None, help="YYYY-MM-DD (default: today, UTC)")
    args = parser.parse_args(argv)
    today = datetime.date.fromisoformat(args.today) if args.today else datetime.datetime.now(datetime.timezone.utc).date()
    data = check.load(args.file)
    runs, errors = read_runs(args.results)
    for error in errors:
        print(error, file=sys.stderr)
    if not runs:
        print("no run had net_bytes: nothing to calibrate", file=sys.stderr)
        return 1
    props, matrix = json.loads(check.PROPS.read_text()), json.loads(check.MATRIX.read_text())
    instruction = json.loads(Path(args.instruction).read_text())
    items = check.explain(instruction, args.tier, data, props, matrix, only=list(runs))
    new, notes = propose(data, runs, items, [Path(p).parent.name + "/" + Path(p).name for p in args.results], today)
    for note in notes:
        print("# " + note)
    diff = list(difflib.unified_diff(dump(data).splitlines(True), dump(new).splitlines(True), "bandwidth.json", "bandwidth.json (proposed)"))
    sys.stdout.writelines(diff)
    if not diff:
        print("# no change")
    return 0


if __name__ == "__main__":
    sys.exit(main())
