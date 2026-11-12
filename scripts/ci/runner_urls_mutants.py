#!/usr/bin/env python3
"""Mutation checks for the runner's public URLs, stage lines and unbudgeted verdict (2026-11-12).

Every mutant needs two killers. Target: ci/vast-broker/runner.py (`public_urls`, `Runner.announce`,
`stage_events`, `show_overview`, the final verdict line), driven by
ci/vast-broker/test/runner_urls_fixture.py and judged by ci/vast-broker/test/runner-urls.test.js
(KF_RUNNER_PATH). Stdlib and node only; GPU-free.
Falsifier: baseline red, an anchor absent or duplicated, or a mutant with fewer than two killers.

    python3 -B scripts/ci/runner_urls_mutants.py [substring-of-mutant-name]
"""

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BROKER = ROOT / "ci/vast-broker"
RUNNER = BROKER / "runner.py"

RUNID = "        or not RUN_ID.fullmatch(run)\n"
LEGTYPE = "        or type(leg) is not int\n"
LEGRANGE = "        or not 0 <= leg <= 31\n"
START = "            self.announce(0)\n"
VERDICT = "        self.emit(self.verdict_line(result))\n"
OVERVIEW = "        self.show_overview(result)\n"
EMIT_LOG = '        self.emit("LOG_URL leg=" + str(leg) + " " + urls[0] + "\\n")\n'
EMIT_STATUS = '        self.emit("STATUS_URL leg=" + str(leg) + " " + urls[1] + "\\n")\n'
EMIT_OVERVIEW = '        self.emit("\\n".join(out) + "\\n")\n'
ANCHOR = "(?=[ \\t\\r\\n]|$)"
SUMMARY = (
    "                *[\n"
    '                    ("Log URL leg " + str(leg), urls[0])\n'
    "                    for leg, urls in self.announced.items()\n"
    "                ],\n"
)

# (name, old text (exactly once), new text)
MUTANTS = [
    # --- URL not printed at the start -----------------------------------------------------------
    ("start-not-announced", START, ""),
    ("announce-never-emits-log", EMIT_LOG, "        pass\n"),
    ("announce-never-emits-status", EMIT_STATUS, "        pass\n"),
    (
        "announce-twice",
        "        if urls is None or leg in self.announced:\n",
        "        if urls is None:\n",
    ),
    # --- URL built from an unsanitised id -------------------------------------------------------
    ("run-id-unchecked", RUNID, "        or False\n"),
    (
        "run-id-loose",
        'RUN_ID = re.compile(r"[1-9][0-9]{0,19}-[1-9][0-9]{0,19}")',
        'RUN_ID = re.compile(r".*", re.S)',
    ),
    ("leg-type-unchecked", LEGTYPE, "        or False\n"),
    ("leg-range-unchecked", LEGRANGE, "        or False\n"),
    ("leg-bool-accepted", LEGTYPE, "        or not isinstance(leg, int)\n"),
    (
        "logs-base-http",
        'LOGS_BASE = "https://logs.kayfabe.dev"',
        'LOGS_BASE = "http://logs.kayfabe.dev"',
    ),
    (
        "status-base-moved",
        'STATUS_BASE = "https://kayfabe.dev/status/run/?id="',
        'STATUS_BASE = "https://kayfabe.dev/status/run/"',
    ),
    # --- verdict / URLs lost under the budget ---------------------------------------------------
    ("verdict-dropped", VERDICT, ""),
    ("verdict-through-budget", VERDICT, "        self.show(self.verdict_line(result))\n"),
    ("overview-dropped", OVERVIEW, ""),
    ("overview-through-budget", EMIT_OVERVIEW, '        self.show("\\n".join(out) + "\\n")\n'),
    # --- stage lines ----------------------------------------------------------------------------
    ("stage-name-unanchored", ANCHOR, ""),
    ("stage-not-marked-box-reported", '            line += " (box-reported)"\n', ""),
    ("stage-lines-uncapped", " or len(self.stage_lines) >= STAGE_LINES", ""),
    ("summary-urls-dropped", SUMMARY, ""),
]


def killers(runner):
    env = dict(os.environ, KF_RUNNER_PATH=str(runner))
    p = subprocess.run(
        ["node", "--test", "--test-reporter=tap", "test/runner-urls.test.js"],
        cwd=BROKER,
        env=env,
        capture_output=True,
        text=True,
        timeout=900,
    )
    return re.findall(r"^not ok \d+ - (.*)$", p.stdout, re.M)


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    source = RUNNER.read_text()
    rows, bad = [], 0
    with tempfile.TemporaryDirectory(prefix="kf-runner-urls-") as directory:
        work = Path(directory) / "base-runner.py"
        work.write_text(source)
        baseline = killers(work)
        if baseline:
            print("BASELINE RED: %s" % baseline, file=sys.stderr)
            return 1
        for name, old, new in MUTANTS:
            if only not in name:
                continue
            assert source.count(old) == 1, (name, source.count(old))
            work = Path(directory) / ("mut-" + name + "-runner.py")
            work.write_text(source.replace(old, new))
            found = killers(work)
            rows.append({"mutant": name, "killers": found, "count": len(found)})
            print("%s: %d killers" % (name, len(found)), file=sys.stderr, flush=True)
            bad += len(found) < 2
    print(json.dumps({"cases": rows}, indent=2))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
