"""Give selected QEMU-log lines a time window, from the RPC observer's byte offsets.

The QEMU log carries no timestamps. rpc_timing.jsonl rows carry `observed_utc` and `file_offset_after`
(bytes of the log consumed when the row was observed), so a line at byte offset X happened between the
last row with offset <= X and the first row with offset > X. Observation time, not generation time.
Usage: keyline_times.py QEMU.log rpc_timing.jsonl OUT.jsonl PATTERN [PATTERN ...]
"""
import json
import sys
from bisect import bisect_right
from pathlib import Path


def key_lines(log: Path, rows: list, patterns: list) -> list:
    offs = [r["file_offset_after"] for r in rows]
    out = []
    pos = 0
    with log.open("rb") as f:
        for n, line in enumerate(f, 1):
            if any(p in line for p in patterns):
                i = bisect_right(offs, pos)  # first row observed after this line began
                before = rows[i - 1] if i > 0 else None
                after = rows[i] if i < len(rows) else None
                out.append({
                    "line": n,
                    "offset": pos,
                    "after_ordinal": before["ordinal"] if before else None,
                    "not_before_utc": before["observed_utc"] if before else None,
                    "not_after_utc": after["observed_utc"] if after else None,
                    "text": line[:300].decode("utf-8", "replace").rstrip("\n"),
                })
            pos += len(line)
    return out


def main(argv):
    log, rt, dest = Path(argv[1]), Path(argv[2]), Path(argv[3])
    rows = [json.loads(l) for l in rt.read_text().splitlines() if l.strip()]
    patterns = [a.encode() for a in argv[4:]]
    with dest.open("w") as o:
        for r in key_lines(log, rows, patterns):
            o.write(json.dumps(r) + "\n")


if __name__ == "__main__":
    main(sys.argv)
