#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The decisions of the SPECviewperf gate stage, as pure functions (stdlib only, GPU-free).

STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_SPEC_GATE.md). GPU-free tests only
(scripts/ci/test_vast_spec_lane.py, scripts/ci/spec_stage_mutants.py); no box has run the stage.

scripts/bench/box/specviewperf_stage.sh does the orchestration (fetch, copy, boots, polling); every
RULE lives here so that tests and mutants can reach it without a box:

  lanes     which viewsets a lane runs (lanes.json is the single source; the payload reads it too)
  timeout   the time guard: skip a viewset, run it with its full budget, or run it with a budget cut
            by the time left (`cut`: running out of it is a RETRY, never a verdict on the guest)
  grade     one viewset's verdict from the CLI's status lines, its exit line and the reset counters
  decide    the verdict of the stage from the rows and the same-box host reference
  scrub     the status-only extract of a CLI log (nothing that is not a status line is kept)

Verdict rules (owner, 2026-11-01: THE GATE IS THE GUEST PASSING). PASS needs every viewset of the lane
to Complete with every subtest Completed, exit 0, zero guest resets, the host reference green after
the run and no new host Xid. A viewset that fails, is refused by SPEC's own validation, hangs past its
budget or resets the guest is FAIL (kayfabe's, unless the same-box host reference was red or kf3
refused a BAR1 size). Missing media, a transport fault, a boot that never came, or the time running out
is RETRY: never a pass, never an excuse. Scores are never read: this module sees status lines only.
"""

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TABLE = HERE / "lanes.json"

RETRY_CLASSES = ("NOT_BUILT", "MEDIA", "PROVIDER", "TRANSPORT", "TIME", "HOSTREF", "BAR1_EXCEPTION", "OPERATOR", "USAGE")
ROW_RESULTS = ("PASS", "FAIL", "NOT_RUN")
FAIL_REASONS = ("RESET", "TIMEOUT", "REFUSED", "SUBTEST", "NOTDONE", "RC", "DISPLAY", "GUEST")
NOT_RUN_REASONS = ("TIME", "GATHER", "NO_TDR", "BOOT", "SETUP", "LAUNCH")
WORKLOAD = re.compile(r"[a-z][a-z_]*-[0-9]{2}")
SUBTEST = re.compile(r"[A-Za-z0-9_]{1,48}")


def table(path=TABLE):
    return json.loads(Path(path).read_text())


def lane_ids(lane, tbl=None):
    """The viewsets of a lane in run order. `both` is lane A then lane B. Anything else is an error."""
    tbl = tbl or table()
    if lane == "both":
        return list(tbl["lanes"]["lanea"]) + list(tbl["lanes"]["laneb"])
    if lane in tbl["lanes"]:
        return list(tbl["lanes"][lane])
    raise ValueError("unknown lane")


def lane_native_s(lane, tbl=None):
    tbl = tbl or table()
    return sum(tbl["viewsets"][w]["native_s"] for w in lane_ids(lane, tbl))


def whole_lane_s(lane, tbl=None):
    """What a lane needs from its start: setup plus the native walls with the margin, plus the tail."""
    tbl = tbl or table()
    return int(tbl["setup_s"] + tbl["factor"] * lane_native_s(lane, tbl) + tbl["tail_reserve_s"])


def need_s(workload, tbl=None):
    """The time a viewset needs left before it may start: its native wall with the margin."""
    tbl = tbl or table()
    return int(tbl["factor"] * tbl["viewsets"][workload]["native_s"])


def timeout_for(workload, remaining_s, tbl=None):
    """The time guard. Returns ("skip", 0) when the viewset must not be started (less time left than
    its native wall with the margin plus the tail reserve), else (mode, seconds): "full" is the
    viewset's own budget (running past it is a FAIL, a hang), "cut" is the time left minus the tail
    reserve when that is shorter than the budget (running past it is a RETRY, class TIME)."""
    tbl = tbl or table()
    usable = int(remaining_s) - tbl["tail_reserve_s"]
    budget = tbl["viewsets"][workload]["budget_s"]
    if usable < need_s(workload, tbl):
        return "skip", 0
    if usable < budget:
        return "cut", usable
    return "full", budget


# ---------------------------------------------------------------------------------------------
# the CLI's status lines
# ---------------------------------------------------------------------------------------------
STAMP = r"(?:\[[0-9|:-]+\] )?"
SUB_LINE = re.compile(
    r"^" + STAMP + r"\[workload: ([a-z][a-z_]*-[0-9]{2}) / subtest: (\S+)\] (Started|Completed|Failed|Incomplete|Skipped)\b"
)
WL_LINE = re.compile(r"^" + STAMP + r"\[Workload:\s*([a-z][a-z_]*-[0-9]{2})\]\s*(Starting|Completed|Failed|Incomplete|Skipped)\b")
LISTED = re.compile(r"<id=[0-9]+>\s+(\S+)")
REFUSED = re.compile(r"Requirement:|does not meet|[Ss]ufficient|not supported|No graphics card")
GATHER = re.compile(r"Unable to gather system details")
DISPLAY = re.compile(r"does not support 4K or Full HD|No compatible resolution")
STATUS_LINE = re.compile(
    r"^" + STAMP + r"\[[Ww]orkload: [a-z][a-z_]*-[0-9]{2}(?: / subtest: [A-Za-z0-9_]{1,48})?\] "
    r"(?:Starting|Started|Completed|Failed|Incomplete|Skipped)(?: -- check logs for errors)?\s*$"
)
LISTING_LINE = re.compile(r"^\s*\S?\s*<id=[0-9]+> [A-Za-z0-9_]{1,48}\s*$")
NOTE_LINE = re.compile(
    r"^(?:Error|Warning): (?:Unable to gather system details|System does not support 4K or Full HD display resolution"
    r"|No graphics card supports 4K or Full HD display resolution)[^\n]{0,160}$|"
    r"^\s*Requirement: [A-Za-z0-9_ ]{1,64}$"
)


def parse_cli(text, workload):
    """What the CLI said about one viewset (status words only)."""
    last = {}
    order = []
    wl = None
    started = False
    listed = []
    for raw in text.splitlines():
        line = raw.rstrip("\r")
        m = SUB_LINE.match(line)
        if m and m.group(1) == workload:
            if m.group(2) not in last:
                order.append(m.group(2))
            last[m.group(2)] = m.group(3)
            continue
        m = WL_LINE.match(line)
        if m and m.group(1) == workload:
            if m.group(2) == "Starting":
                started = True
            else:
                wl = m.group(2)
            continue
        m = LISTED.search(line)
        if m:
            listed.append(m.group(1))
    return {
        "workload_status": wl,
        "started": started,
        "subtests": [(n, last[n]) for n in order],
        "listed": len(set(listed)),
        "gather_failed": bool(GATHER.search(text)),
        "refused": bool(REFUSED.search(text)),
        "display": bool(DISPLAY.search(text)),
    }


def tdr_total(text):
    """The reset counter total of `gate_state.ps1 -Mode tdr` (None when the line is missing)."""
    m = re.search(r"^GATE_TDR .* total=([0-9]+)\s*$", text or "", re.M)
    return int(m.group(1)) if m else None


def end_rc(text):
    m = re.search(r"^SPECRUN_END workload=\S+ rc=(-?[0-9]+)\b", text or "", re.M)
    return int(m.group(1)) if m else None


def grade_viewset(workload, cli_text, end_text, tdr_text, timeout=None):
    """One viewset's row: {name, result, reason, done, total, failed}. `timeout` is None, "full" (the
    viewset did not end inside its own budget), "cut" (it did not end inside the time that was left),
    "dead" (the guest went away during the run) or "silent" (it stopped answering): the last two are the
    guest's failure; "launch" (the guest script said it could not start the run) is the harness's."""
    cli = parse_cli(cli_text or "", workload)
    done = sum(1 for _, s in cli["subtests"] if s == "Completed")
    total = max(len(cli["subtests"]), cli["listed"])
    failed = [n for n, s in cli["subtests"] if s != "Completed" and SUBTEST.fullmatch(n)]
    row = {"name": workload, "result": "FAIL", "reason": "NOTDONE", "done": done, "total": total, "failed": failed[:6]}
    if timeout in ("dead", "silent"):
        row["reason"] = "GUEST"
        return row
    if timeout == "launch":
        row.update(result="NOT_RUN", reason="LAUNCH")
        return row
    if timeout == "cut":
        row.update(result="NOT_RUN", reason="TIME")
        return row
    resets = tdr_total(tdr_text)
    if timeout == "full":
        row["reason"] = "TIMEOUT"
        return row
    if not cli["started"] and cli["gather_failed"] and not cli["subtests"]:
        row.update(result="NOT_RUN", reason="GATHER")
        return row
    if resets is not None and resets > 0:
        row["reason"] = "RESET"
        return row
    if cli["display"] and not cli["subtests"]:
        row["reason"] = "DISPLAY"
        return row
    if cli["refused"] and not cli["subtests"]:
        row["reason"] = "REFUSED"
        return row
    if cli["subtests"] and any(s != "Completed" for _, s in cli["subtests"]):
        row["reason"] = "SUBTEST"
        return row
    rc = end_rc(end_text)
    if rc is None and resets is None:
        row.update(result="NOT_RUN", reason="NO_TDR")
        return row
    if cli["workload_status"] == "Completed" and total >= 1 and done == total and rc == 0 and resets == 0:
        row.update(result="PASS", reason="-")
        return row
    if resets is None:
        row.update(result="NOT_RUN", reason="NO_TDR")
        return row
    if cli["workload_status"] == "Completed" and total >= 1 and done == total and rc != 0:
        row["reason"] = "RC"
        return row
    return row


def row_line(row):
    failed = ",".join(row["failed"]) if row["failed"] else "-"
    return "SPEC_ROW name=%s result=%s subtests=%d/%d reason=%s failed=%s" % (
        row["name"], row["result"], row["done"], row["total"], row["reason"], failed)


ROW_PATTERN = re.compile(
    r"^SPEC_ROW name=([a-z][a-z_]*-[0-9]{2}) result=(PASS|FAIL|NOT_RUN) subtests=([0-9]{1,3})/([0-9]{1,3})"
    r" reason=([A-Z_]{1,16}|-) failed=([A-Za-z0-9_,]{1,120}|-)$"
)


def parse_row(line):
    m = ROW_PATTERN.match(line)
    if not m:
        return None
    return {"name": m[1], "result": m[2], "done": int(m[3]), "total": int(m[4]), "reason": m[5],
            "failed": [] if m[6] == "-" else m[6].split(",")}


def decide(rows, lane, hostref_before, hostref_after, bar1=False, xid_delta=0, abort=None, tbl=None):
    """The verdict of the stage: (kind, class) with kind PASS|FAIL|RETRY and class a RETRY class, "KAYFABE"
    for a FAIL or "NONE" for a PASS. `rows` are parsed SPEC_ROW dicts (the last row of a name counts);
    `abort` names the RETRY class of a setup step that failed (media, transport, ...)."""
    expected = lane_ids(lane, tbl)
    last = {}
    for r in rows:
        if r["name"] in expected:
            last[r["name"]] = r
    failed = [n for n in expected if last.get(n, {}).get("result") == "FAIL"]
    passed = [n for n in expected if last.get(n, {}).get("result") == "PASS"]
    if failed:
        if bar1:
            return "RETRY", "BAR1_EXCEPTION"
        if hostref_before != "green" or hostref_after != "green":
            return "RETRY", "HOSTREF"
        return "FAIL", "KAYFABE"
    if len(passed) == len(expected) and not abort:
        if hostref_after != "green":
            return "RETRY", "HOSTREF"
        if xid_delta:
            return "FAIL", "KAYFABE"
        return "PASS", "NONE"
    if abort in RETRY_CLASSES:
        return "RETRY", abort
    for n in expected:
        reason = last.get(n, {}).get("reason")
        if n not in passed:
            return "RETRY", {"TIME": "TIME", "NO_TDR": "TRANSPORT", "GUEST": "TRANSPORT"}.get(reason, "OPERATOR")
    return "RETRY", "OPERATOR"


def scrub_cli(text):
    """The status-only extract of a CLI log: only the lines of the status vocabulary survive, so that
    nothing the vendor tool prints besides statuses (a score, a path, a model name) reaches a file that
    leaves the guest."""
    keep = []
    for raw in text.splitlines():
        line = raw.rstrip("\r")
        if len(line) <= 240 and (STATUS_LINE.match(line) or LISTING_LINE.match(line) or NOTE_LINE.match(line)):
            keep.append(line)
    return "\n".join(keep) + ("\n" if keep else "")


def tdr_line(text):
    m = re.search(r"^GATE_TDR .*$", text or "", re.M)
    return m.group(0) if m else ""


# ---------------------------------------------------------------------------------------------
# command line (used by specviewperf_stage.sh)
# ---------------------------------------------------------------------------------------------
def main(argv=None):
    ap = argparse.ArgumentParser(prog="lane.py")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("ids", "native", "whole", "min-free-gb", "settle", "tail", "gather-wait", "max-boots", "release"):
        p = sub.add_parser(name)
        if name not in ("settle", "tail", "gather-wait", "max-boots", "release"):
            p.add_argument("lane")
    p = sub.add_parser("vars")
    p.add_argument("lane")
    p = sub.add_parser("timeout")
    p.add_argument("workload")
    p.add_argument("remaining_s", type=int)
    p = sub.add_parser("grade")
    p.add_argument("workload")
    p.add_argument("--cli", required=True)
    p.add_argument("--end", required=True)
    p.add_argument("--tdr", required=True)
    p.add_argument("--timeout", choices=("full", "cut", "dead", "silent", "launch"))
    p.add_argument("--scrub-out")
    p = sub.add_parser("decide")
    p.add_argument("lane")
    p.add_argument("--rows", required=True)
    p.add_argument("--before", required=True)
    p.add_argument("--after", required=True)
    p.add_argument("--bar1", type=int, default=0)
    p.add_argument("--xid", type=int, default=0)
    p.add_argument("--abort", default="")
    p = sub.add_parser("scrub")
    p.add_argument("path")
    a = ap.parse_args(argv)
    tbl = table()

    def text(path):
        try:
            return Path(path).read_text(errors="replace")
        except OSError:
            return ""

    try:
        if a.cmd == "ids":
            print(",".join(lane_ids(a.lane, tbl)))
        elif a.cmd == "native":
            print(lane_native_s(a.lane, tbl))
        elif a.cmd == "whole":
            print(whole_lane_s(a.lane, tbl))
        elif a.cmd == "min-free-gb":
            print(tbl["min_free_gb"][a.lane])
        elif a.cmd == "settle":
            print(tbl["settle_s"])
        elif a.cmd == "tail":
            print(tbl["tail_reserve_s"])
        elif a.cmd == "gather-wait":
            print(tbl["gather_retry_wait_s"])
        elif a.cmd == "max-boots":
            print(tbl["max_guest_boots"])
        elif a.cmd == "release":
            print(tbl["release"])
        elif a.cmd == "vars":
            print("SETTLE_S=%d" % tbl["settle_s"])
            print("TAIL_S=%d" % tbl["tail_reserve_s"])
            print("GATHER_WAIT_S=%d" % tbl["gather_retry_wait_s"])
            print("MAX_BOOTS=%d" % tbl["max_guest_boots"])
            print("NEED_GB=%d" % tbl["min_free_gb"][a.lane])
            print("RELEASE=%s" % tbl["release"])
            print("IDS=%s" % ",".join(lane_ids(a.lane, tbl)))
        elif a.cmd == "timeout":
            mode, secs = timeout_for(a.workload, a.remaining_s, tbl)
            print(mode, secs)
        elif a.cmd == "grade":
            row = grade_viewset(a.workload, text(a.cli), text(a.end), text(a.tdr), a.timeout)
            if a.scrub_out:
                Path(a.scrub_out).write_text(scrub_cli(text(a.cli)))
            print(row_line(row))
            return {"PASS": 0, "FAIL": 1}.get(row["result"], 3)
        elif a.cmd == "decide":
            rows = [r for r in (parse_row(s.strip()) for s in text(a.rows).splitlines()) if r]
            kind, cls = decide(rows, a.lane, a.before, a.after, bool(a.bar1), a.xid, a.abort or None, tbl)
            print(kind, cls)
        elif a.cmd == "scrub":
            sys.stdout.write(scrub_cli(text(a.path)))
    except (KeyError, ValueError):
        print("lane.py: bad argument", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
