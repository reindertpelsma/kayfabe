#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""attempts.py RUNS_DIR: one row per Premiere attempt from the pp_run.ps1 results (`runs/<run>/<attempt>.out`, the JSON that pp.py run printed)
-> RUNS_DIR/attempts.tsv, and each attempt's progress log -> RUNS_DIR/<run>/<attempt>_progress.txt. Outcome classes as PREREGISTRATION.md."""
import json
import sys
from pathlib import Path

COLS = ("run", "attempt", "mode", "keys", "start_utc", "sequence_step_utc", "done_utc", "process_gone_utc", "gone_after_sequence_s",
        "alive_after", "responding_after", "outcome", "summary", "keys_sent")


def outcome(r, prog):
    # the first learn attempts never opened the project (a script error, or Premiere's "file path does not exist" dialog for a command-line
    # path); only the openDocument-and-return form counts
    if r.get("mode") == "learn" and not any("returning so the project can load" in p for p in prog):
        return "OTHER"
    if r.get("error"):
        return "OTHER"
    if r.get("process_gone_utc") and r.get("sequence_step_utc"):
        return "CRASH"
    if r.get("hang"):
        return "HANG"
    if r.get("done") and r.get("alive_after") and r.get("responding_after"):
        return "PASS"
    return "OTHER"


def main(root):
    root = Path(root)
    rows = []
    for out in sorted(root.glob("*/*.out")):
        text = out.read_text(errors="replace")
        i = text.find("{")
        if i < 0:
            continue
        try:
            r = json.loads(text[i:text.rfind("}") + 1])
        except ValueError:
            continue
        run = out.parent.name
        prog = r.pop("progress", []) or []
        prog = [p.get("value", "") if isinstance(p, dict) else str(p) for p in prog]
        (out.parent / ("%s_progress.txt" % out.stem)).write_text("\n".join(prog) + "\n")
        r["outcome"] = outcome(r, prog)
        r["run"], r["attempt"] = run, out.stem
        r["keys"] = "yes" if r.get("keys_mode") else "no"
        r["keys_sent"] = " ".join(k.replace(" ", "@") for k in r.get("keys", []) or [])
        rows.append("\t".join(str(r.get(c, "")) for c in COLS))
    (root / "attempts.tsv").write_text("\t".join(COLS) + "\n" + "\n".join(rows) + "\n")
    print("\n".join(rows))


if __name__ == "__main__":
    main(sys.argv[1])
