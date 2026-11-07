#!/usr/bin/env python3
"""Mutation checks of the box run report (vast_run_report.py): every mutant needs two killers.

Each mutant is one exact-once text replacement in a private copy of vast_run_report.py; the tests of
test_vast_run_report.py run against the copy and the failing test names are the killers. Stdlib only,
GPU-free. The Worker side has its own mutants (ci/vast-broker/report-mutants.mjs).

    python3 scripts/ci/run_report_mutants.py [--jobs N] [substring-of-mutant-name]
"""

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / "vast_run_report.py").read_text()

# (name, old text (exactly once), new text)
MUTANTS = [
    (
        "apps-invented",
        "            for name, (res, _secs, note) in c.apps.items()",
        '            for name, (res, _secs, note) in list(c.apps.items()) + [("invented", ("PASS", None, ""))]',
    ),
    (
        "not-run-without-reason",
        'verdict, reason = "NOT_RUN", reason_idle',
        'verdict, reason = "NOT_RUN", None',
    ),
    ("failed-subrun-is-pass", "    if failed:\n", "    if False:\n"),
    (
        "partial-subrun-is-pass",
        'verdict, reason = "UNATTRIBUTED", "incomplete: "',
        'verdict, reason = "PASS", "incomplete: "',
    ),
    (
        "ratio-inverted",
        'row["ref_ratio"] = value / ref if ref else None',
        'row["ref_ratio"] = ref / value if value else None',
    ),
    ("guest-without-reference", 'if mode == "guest" and name in c.bench["host"]:', "if False:"),
    ('bandwidth-factor', '("h2d_ms", "h2d_GBps", 2)', '("h2d_ms", "h2d_GBps", 1)'),
    ("bandwidth-for-small-sizes", "                if n >= 1024:", "                if n >= 1:"),
    ("probe-repeat-overwritten", 'name = parts[1] if n == 1 else "%s#%d" % (parts[1], n)', "name = parts[1]"),
    ("probe-failure-ignored", 'ok = d.get("rc") == "0" and d.get("timeout") == "0"', "ok = True"),
    ("notrun-is-pass", 'if verdict in ("NOTRUN", "SKIP"):\n        return "SKIP"', 'if verdict in ("NOTRUN", "SKIP"):\n        return "PASS"'),
    ("died-is-pass", '    return "FAIL"  # FAIL, TIMEOUT, VMM_DIED, CRASH: the arm did not pass', '    return "PASS"'),
    (
        "thin-arm-uses-wall-seconds",
        'ms = whole_ms(d.get("client_ms")) or seconds_ms(d.get("secs"))',
        'ms = seconds_ms(d.get("secs"))',
    ),
    (
        "stage-without-summary-is-pass",
        'stages.append({"name": name, "result": "UNATTRIBUTED", "rc": rc, "duration_ms": duration})',
        'stages.append({"name": name, "result": "PASS", "rc": rc, "duration_ms": duration})',
    ),
    ("timeout-stage-is-pass", '"TIMEOUT": "FAIL"', '"TIMEOUT": "PASS"'),
    ("provider-fault-is-fail", '"PROVIDER_FAULT": "RETRY"', '"PROVIDER_FAULT": "FAIL"'),
    ("document-unbounded", "        while size() > limit and doc[key]:", "        while False:"),
    ("apps-rows-of-another-os", 'os=linux result=(PASS|FAIL|NOT_RUN)"', 'os=\\S+ result=(PASS|FAIL|NOT_RUN)"'),
    ("app-note-unbounded", 'note": note[:200]}', 'note": note}'),
    ("app-not-run-is-a-pass-test", '"SKIP" if m[2] == "NOT_RUN" else m[2]', "m[2]"),
    (
        "apps-suite-outside-the-linux-subrun",
        '    linux_group = LINUX_SUITES + tuple(r["name"] for r in rows if r["name"].startswith("apps"))',
        "    linux_group = LINUX_SUITES",
    ),
    ("apps-uncapped", '][: LIMITS["apps"]],', "],"),
    (
        "partial-hidden-by-count",
        'all(r["pass"] > 0 for r in mine) and set(stages_hint) <= {r["name"] for r in mine}',
        'all(r["pass"] > 0 for r in mine) and len(mine) >= len(stages_hint)',
    ),
    ("tests-uncapped", '        "tests": tests[: LIMITS["tests"]],', '        "tests": tests,'),
    ("perf-uncapped", '        "perf": perf[: LIMITS["perf"]],', '        "perf": perf,'),
    (
        "sent-without-the-flag",
        'if env.get("KF_RUN_REPORT") != "1" and not dry:',
        "if False:",
    ),
    (
        "refusal-retried",
        "if status not in (None, 409, 429, 500, 502, 503, 504):",
        "if False:",
    ),
    (
        "existing-report-not-sent",
        'if status == 409 and state == "REPORT_EXISTS":',
        "if False:",
    ),
    ("wrong-url", 'url = "%s/run-report/%s" % (env["KF_BROKER_URL"], env["KF_RUN_ID"])', 'url = "%s/report/%s" % (env["KF_BROKER_URL"], env["KF_RUN_ID"])'),
    ("redirect-followed", "            _NoRedirect(),\n", ""),
    ("proxy-used", "            urllib.request.ProxyHandler({}),\n", ""),
    (
        "builder-error-raised",
        "    except Exception as error:  # noqa: BLE001  (a builder bug must never touch the run)",
        "    except ZeroDivisionError as error:",
    ),
    (
        "windows-retry-as-fail",
        'subruns[2]["verdict"] = c.windows["gate"]',
        'subruns[2]["verdict"] = "FAIL"',
    ),
    (
        "unknown-linux-driver-without-id",
        'sub_linux = "linux-" + linux_driver if linux_driver else "linux-unknown"',
        'sub_linux = "linux-" + linux_driver if linux_driver else "linux"',
    ),
    (
        "host-driver-unchecked",
        'host_driver = hardware.get("driver_version") if DRIVER.fullmatch(str(hardware.get("driver_version"))) else None',
        'host_driver = hardware.get("driver_version")',
    ),
    (
        "rows-of-the-wrong-subrun",
        'keys = {"host": "host", "linux": sub_linux, "windows": sub_windows}',
        'keys = {"host": "host", "linux": sub_windows, "windows": sub_linux}',
    ),
    (
        "long-lines-parsed",
        "if len(x) <= LINE_MAX]",
        "]",
    ),
    (
        "token-in-the-log-line",
        'say("KF_RUN_REPORT_SENT bytes=%d subruns=%d" % (len(body), len(doc["subruns"])))',
        'say("KF_RUN_REPORT_SENT bytes=%d subruns=%d %s" % (len(body), len(doc["subruns"]), env["KF_RUN_TOKEN"]))',
    ),
    (
        "windows-not-requested-hidden",
        '        "Windows stage not requested for this profile"\n        if c.windows_requested is False\n        else idle',
        "        idle",
    ),
    (
        "duplicate-arm-keeps-first",
        "        self.tests[(key, suite, name)] = {",
        "        self.tests.setdefault((key, suite, name), None) or self.tests.__setitem__((key, suite, name), {",
    ),
]
# the last mutant needs its closing parenthesis too
MUTANTS[-1] = (
    "duplicate-arm-keeps-first",
    '        self.tests[(key, suite, name)] = {"suite": suite, "name": name, "result": result, "ms": ms, "_key": key}',
    '        self.tests.setdefault((key, suite, name), {"suite": suite, "name": name, "result": result, "ms": ms, "_key": key})',
)

# 2026-11-07: the SPECviewperf stage's status rows (docs/design/V3_SPEC_GATE.md): status only, neutral suite name.
MUTANTS += [
    (
        "spec-row-with-extra-fields-accepted",
        'failed=([A-Za-z0-9_,]{1,120}|-)$"\n)\nSPEC_SUITE',
        'failed=([A-Za-z0-9_,]{1,120}|-)"\n)\nSPEC_SUITE',
    ),
    ("spec-not-run-is-a-pass", '"NOT_RUN": "SKIP"}', '"NOT_RUN": "PASS"}'),
    ("spec-row-with-a-time", "SPEC_RESULT[m[2]], None)", "SPEC_RESULT[m[2]], 1000)"),
    (
        "spec-failed-subtests-dropped",
        '                for sub in ([] if m[3] == "-" else m[3].split(","))[:6]:',
        "                for sub in []:",
    ),
    (
        "spec-suite-keeps-the-benchmark-name",
        '                kept["name"] = SPEC_SUITE + kept["name"][len("specviewperf"):]',
        "                pass",
    ),
    (
        "spec-suite-outside-the-windows-subrun",
        'if r["name"].startswith(("windows", SPEC_SUITE))] or ["windows_gate"], rows,',
        'if r["name"].startswith("windows")] or ["windows_gate"], rows,',
    ),
    (
        "spec-retry-stays-a-failure",
        'all(r["name"].startswith(SPEC_SUITE + "_retry_") for r in spec_bad)',
        "False",
    ),
    (
        "spec-retry-hides-a-failure",
        'all(r["name"].startswith(SPEC_SUITE + "_retry_") for r in spec_bad)',
        'any(r["name"].startswith(SPEC_SUITE + "_retry_") for r in spec_bad)',
    ),
]

COLLECT = """import io,json,sys,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_vast_run_report.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run_tests(directory):
    done = subprocess.run(
        [sys.executable, "-c", COLLECT],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=600,
        env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp")},
    )
    if done.returncode != 0:
        raise SystemExit("harness failure: " + done.stderr[-400:])
    return json.loads(done.stdout.splitlines()[-1])


def workdir(parent, name, source):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    ci = root / "scripts" / "ci"
    ci.mkdir(parents=True)
    for f in ("test_vast_run_report.py", "vast_report.py"):
        shutil.copyfile(HERE / f, ci / f)
    (ci / "vast_run_report.py").write_text(source)
    return ci


def one(parent, mutant):
    name, old, new = mutant
    if SOURCE.count(old) != 1:
        return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d" % SOURCE.count(old)}
    result = run_tests(workdir(parent, "m", SOURCE.replace(old, new)))
    killers = sorted(set(result["killers"]))
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-run-report-mutants-") as parent:
        baseline = run_tests(workdir(parent, "base", SOURCE))
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"tests": baseline["tests"], "cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d)" % row, file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
