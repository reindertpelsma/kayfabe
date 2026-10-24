"""Bounded enable-to-idle evidence verdict; no rental or hardware operations."""

import argparse
import datetime as dt
import json
import math
from pathlib import Path

DRIVER = "32.0.15.8088"
FATAL_DUMPS = {"0x141", "0x113"}
KNOWN_DUMPS = FATAL_DUMPS | {"0x193", "0x1a8", "0x1b8"}
SYSTEM_PROVIDERS = {
    1001: {"Microsoft-Windows-WER-SystemErrorReporting", "BugCheck"},
    41: {"Microsoft-Windows-Kernel-Power"},
    6008: {"EventLog"},
    4101: {"Display"},
}


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def number(value):
    require(type(value) in (float, int) and math.isfinite(value), "invalid number")
    return value


def utc(value):
    require(isinstance(value, str), "missing UTC")
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(parsed.utcoffset() == dt.timedelta(0), "UTC clock must have zero offset")
    return parsed


def read_jsonl(text):
    require(0 < len(text) <= 8_000_000 and text.endswith("\n"), "partial/oversize JSONL")
    rows = [json.loads(line) for line in text.splitlines()]
    require(len(rows) <= 10000, "too many records")
    return rows


def evaluate(rows, readiness_seconds=120):
    """PASS is only the 60-second regression; it is never full acceptance."""
    try:
        return _evaluate(rows, readiness_seconds)
    except (ValueError, TypeError, KeyError, IndexError, OverflowError) as error:
        return {"verdict": "FAIL", "reason": str(error), "acceptance": "UNRUN"}


def _evaluate(rows, readiness_seconds):
    require(number(readiness_seconds) > 0, "invalid inherited readiness deadline")
    require(isinstance(rows, list) and 4 <= len(rows) <= 10000, "incomplete stream")
    start = rows[0]
    require(start["kind"] == "start" and start["code"] == 22, "start must be disabled")
    cursors = start["cursors"]
    require(set(cursors) == {"System", "Application"}, "missing baseline cursors")
    require(all(type(v) is int and v >= 0 for v in cursors.values()), "invalid cursors")
    require(rows[1]["kind"] == "enable", "enable marker must precede invocation")
    enable = number(rows[1]["mono"])
    baseline = {dump["id"] for dump in start["dumps"]}
    boot = utc(start["boot"])
    initial_utc = utc(start["utc"])
    initial_mono = number(start["mono"])
    previous = initial_mono
    first_code0 = None
    for index, row in enumerate(rows):
        require(isinstance(row, dict) and row["complete"] is True, "partial query")
        require(utc(row["boot"]) == boot, "boot identity changed")
        mono = number(row["mono"])
        delta = (utc(row["utc"]) - initial_utc).total_seconds()
        require(abs(delta - (mono - initial_mono)) <= 2, "UTC/monotonic disagreement")
        require(index == 0 or 0 <= mono - previous <= 10, "clock rollback/coverage gap")
        previous = mono
        expected_kind = "start" if index == 0 else "enable" if index == 1 else "sample"
        if index == len(rows) - 1:
            expected_kind = "end"
        require(row["kind"] == expected_kind, "missing end or repeated enable")
        require(row["driver"] == DRIVER and type(row["code"]) is int, "driver/code")
        require(
            isinstance(row["events"], list) and len(row["events"]) <= 4096,
            "events missing/overflow",
        )
        for event in row["events"]:
            channel, provider = event["channel"], event["provider"]
            require(channel in cursors, "wrong event channel")
            require(
                type(event["record_id"]) is int and event["record_id"] > cursors[channel],
                "event cursor reset/stale",
            )
            require(type(event["id"]) is int, "invalid event id")
            if channel == "Application":
                require(
                    provider == "Windows Error Reporting" and event["id"] == 1001,
                    "wrong Application provider/id",
                )
            else:
                require(
                    provider in SYSTEM_PROVIDERS.get(event["id"], set()), "wrong System provider/id"
                )
                require(False, "System bugcheck/reboot/TDR since enable")
        require(
            isinstance(row["dumps"], list) and len(row["dumps"]) <= 4096, "dumps missing/overflow"
        )
        for dump in row["dumps"]:
            require(isinstance(dump["id"], str) and bool(dump["id"]), "invalid dump id")
            require(dump["code"] in KNOWN_DUMPS, "unclassified dump: cannot pass")
            if index > 0 and dump["id"] not in baseline:
                require(dump["code"] not in FATAL_DUMPS, "new engine timeout/fatal dump")
        if index >= 2:
            if first_code0 is None and row["code"] == 0:
                first_code0 = mono
                require(first_code0 - enable <= readiness_seconds, "late Code0")
            if first_code0 is not None:
                require(row["code"] == 0, "Code0 lost")
    require(first_code0 is not None, "no Code0")
    require(previous - first_code0 >= 60, "less than 60 seconds after first Code0")
    return {
        "verdict": "REGRESSION_PASS",
        "acceptance": "UNRUN",
        "observed_seconds": previous - first_code0,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--readiness-seconds", required=True, type=float)
    args = parser.parse_args()
    try:
        result = evaluate(read_jsonl(args.evidence.read_text()), args.readiness_seconds)
    except (OSError, ValueError) as error:
        result = {"verdict": "FAIL", "reason": str(error), "acceptance": "UNRUN"}
    print(json.dumps(result, sort_keys=True))
    return 0 if result["verdict"] == "REGRESSION_PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
