#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box-side structured run report (schema 1) for the Vast CI broker (stdlib only).

Run by scripts/ci/vast_onstart.sh after the payload has ended and BEFORE the result is reported,
only when the Worker enabled it (KF_RUN_REPORT=1; an older Worker has no such route and never sets
it). Builds report.json from output the repository ALREADY produces and POSTs it to
POST <KF_BROKER_URL>/run-report/<run> with the run token (docs/BOX_PROTOCOL.md, "Run report").

Sources (nothing is invented; a field with no source is left empty):
  - /workspace/kf-result.json   graded suite rows (`suite name=X pass= fail= skip=`), the hardware
                                (host driver from nvidia-smi) and the host claims (the Linux guest
                                driver from the guest.qcow2.STAGED receipt as PR #271 reads it,
                                the Windows harness driver constant)
  - the payload log             KF_PAYLOAD_TIER, `stage name=X rc= duration_ms= result=` summary
                                lines, FAST_CELL_ARM / BARE_CELL_ARM / CL_ROW arm rows, the
                                cup8bench BSUM and BENCH_*_MS lines (the same lines
                                scripts/bench/perf_summarize.py reads), WINDOWS_GATE_* and
                                GATE_PROBE_RESULT lines
Host session (one rental) = several SUBRUNS: `host` (bare-metal arms, host driver),
`linux-<guest driver>` (thin suite and the guest CUDA ladder) and `windows-<driver>`
(windows_gate). Every test, perf and app row carries its `sub`. A subrun that did not run is
NOT_RUN with a reason. Subrun `duration_ms` is the SUM OF THE MEASURED TEST TIMES in the subrun
(the payload prints no wall-clock timestamps); the Worker, not this script, finds the position of
each subrun in the published log from the stage markers.
The report is a CLAIM: the Worker re-validates and bounds every field, sets the run's identity
itself, and nothing here can change a verdict. SPECviewperf SCORES are never included; since 2026-11-07 the SPECviewperf
stage (scripts/bench/box/specviewperf_stage.sh, disabled by default) contributes status only: one test row per viewset
(PASS, FAIL or SKIP for a viewset that did not run) and one FAIL row per failed subtest, under the neutral suite name
`spec_viewsets` (the Worker drops anything that names the benchmark, ci/vast-broker/src/runreport.js), with no timing
and no number; its graded suite rows are renamed to the same prefix. They belong to the Windows subrun. The helper
never raises, never changes the result file and prints one outcome line.
"""

import json
import os
import re
import ssl
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import app_policy
import vast_report as contract  # noqa: E402  (the one definition of the box environment contract)

MAX_BYTES = 262144  # the Worker's cap (ci/vast-broker/src/runreport.js REPORT_MAX_BYTES)
LIMITS = {"tests": 2000, "apps": 500, "perf": 200, "stages": 64, "suites": 20, "subruns": 64}
LINE_MAX = 8192  # longer lines are not parsed
LOG_MAX = 32 * 1024 * 1024
HOST_SUITES = ("cuda_host", "kvm_tests", "gates", "kf3_build", "raw_suite")
LINUX_SUITES = ("thin_suite", "birth_census", "cuda_fat")
APP_ROW = re.compile(
    r"^APP_ROW name=([A-Za-z0-9_]{1,48}) os=linux result=(PASS|FAIL|NOT_RUN)"
    r"(?: secs=(\S+))?(?: guest_xid=\S+)?(?: note=(.*))?$"
)
WINAPP_ROW = re.compile(
    r"^WINAPPS_APP app=([A-Za-z0-9_]{1,48}) class=(pass|guest_fail|boot_fail|harness|not_run)"
    r" verdict=(PASS\*?|FAIL|TIMEOUT|BOOT_FAIL|NOTRUN|-) batched=\S+ alone=\S+"
    r" secs=(\S+) tdr=\S+ wer=\S+ gsp=\S+ note=(.*)$"
)


def windows_app_result(cls, verdict):
    # Only the exact guest-pass pair can pass. Missing, harness and inconsistent rows cannot.
    if cls == "pass" and verdict in ("PASS", "PASS*"):
        return "PASS"
    if (cls == "guest_fail" and verdict in ("FAIL", "TIMEOUT")) or (cls == "boot_fail" and verdict == "BOOT_FAIL"):
        return "FAIL"
    return "NOT_RUN"



# The SPECviewperf stage's status rows (scripts/bench/box/specviewperf_stage.sh; the wrapper lets only these through):
# a viewset, its result and the names of its failed subtests. Nothing here is a number a benchmark measured.
SPEC_ROW = re.compile(
    r"^SPEC_ROW name=([a-z][a-z_]*-[0-9]{2}) result=(PASS|FAIL|NOT_RUN) subtests=[0-9]{1,3}/[0-9]{1,3}"
    r" reason=(?:[A-Z_]{1,16}|-) failed=([A-Za-z0-9_,]{1,120}|-)$"
)
SPEC_SUITE = "spec_viewsets"  # the report's name for the stage's suite rows ("specviewperf" in the result file)
SPEC_RESULT = {"PASS": "PASS", "FAIL": "FAIL", "NOT_RUN": "SKIP"}
# The stage markers whose position in the published log delimits a subrun (the Worker finds them).
LOG_STAGES = {
    "host": ["cuda_host", "merge_bar"],
    "linux": ["merge_bar", "cuda_guest", "apps"],
    "windows": ["windows_gate", "windows_apps", "windows_gate_2"],
}
DRIVER = re.compile(r"[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?")
KV = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)=(\S+)")
STAGE_RESULT = {"PASS": "PASS", "FAIL": "FAIL", "TIMEOUT": "FAIL", "PROVIDER_FAULT": "RETRY"}
BENCH_UNITS = {
    "gflops": ("GFLOPS", True),
    "batch_gflops": ("GFLOPS", True),
    "med_ms": ("ms", False),
    "first_ms": ("ms", False),
    "p90_ms": ("ms", False),
    "min_ms": ("ms", False),
    "batch_per_launch_ms": ("ms", False),
    "h2d_ms": ("ms", False),
    "d2h_med_ms": ("ms", False),
    "submit_med_ms": ("ms", False),
    "sync_med_ms": ("ms", False),
}


def kvs(line):
    return dict(KV.findall(line))


def number(text):
    try:
        value = float(text)
    except (TypeError, ValueError):
        return None
    return value if value == value and abs(value) < 1e15 else None


def whole_ms(text):
    value = number(text)
    return int(value) if value is not None and 0 <= value <= 7 * 86400000 else None


def seconds_ms(text):
    value = number(text)
    return int(value * 1000) if value is not None and 0 <= value <= 7 * 86400 else None


def lines_of(path):
    """The log's lines (bounded), longer lines skipped."""
    try:
        with open(path, "rb") as handle:
            data = handle.read(LOG_MAX + 1)[:LOG_MAX]
    except OSError:
        return []
    return [x for x in data.decode("utf-8", "replace").splitlines() if len(x) <= LINE_MAX]


def read_result(path):
    try:
        with open(path, "rb") as handle:
            doc = json.loads(handle.read(1 << 20).decode("utf-8"))
    except (OSError, ValueError):
        return {}
    return doc if isinstance(doc, dict) else {}


def arm_result(verdict):
    if verdict == "PASS":
        return "PASS"
    if verdict in ("NOTRUN", "SKIP"):
        return "SKIP"
    return "FAIL"  # FAIL, TIMEOUT, VMM_DIED, CRASH: the arm did not pass


class Collector:
    def __init__(self):
        self.tests = {}  # (sub key, suite, name) -> row, last one wins
        self.apps = {}  # app name -> (result, secs text, note), last one wins
        self.windows_apps = {}  # same shape, separate namespace from Linux
        self.bench = {"host": {}, "guest": {}}
        self.stage_rows = {}
        self.stage_end = {}
        self.windows_requested = None
        self.windows = {"gate": None, "install_s": None, "probes": [], "resets": None, "live_dumps_193": None, "known_live_dumps_193": None,
                        "live_dumps_193_ratchet": None, "live_dumps_193_die": None, "live_dumps_193_max": None,
                        "live_dumps_193_entries": None, "live_dumps_193_unresolved_entries": None, "live_dumps_193_unit": None}
        self.probe_seen = {}

    def test(self, key, suite, name, result, ms):
        self.tests[(key, suite, name)] = {"suite": suite, "name": name, "result": result, "ms": ms, "_key": key}

    def feed(self, line):
        head = line.split(" ", 1)[0]
        if head == "FAST_CELL_ARM":
            d = kvs(line)
            ms = whole_ms(d.get("client_ms")) or seconds_ms(d.get("secs"))
            if "arm" in d and "verdict" in d:
                self.test("linux", "thin_suite", d["arm"], arm_result(d["verdict"]), ms)
        elif head == "BARE_CELL_ARM":
            d = kvs(line)
            ms = whole_ms(d.get("ms")) or seconds_ms(d.get("secs"))
            if "arm" in d and "verdict" in d:
                self.test("host", "raw_suite", d["arm"], arm_result(d["verdict"]), ms)
        elif head == "CL_ROW":
            d = kvs(line)
            rung = d.get("rung")
            if d.get("mode") == "host" and rung:
                self.test("host", "cuda_host", rung, arm_result(d.get("verdict", "")), whole_ms(d.get("wall_ms")))
            elif d.get("mode") == "guest" and rung:
                wall = d.get("CUP8_WALL_S") or d.get("BENCH_measure_WALL_S")
                self.test("linux", "cuda_fat", rung, arm_result(d.get("verdict", "")), seconds_ms(wall))
        elif head == "CL_BENCH":
            self.bench_line(line)
        elif head == "APP_ROW":
            # The apps stage (scripts/bench/box/apps_gate.sh): one row per gate app, in the Linux guest.
            m = APP_ROW.match(line)
            if m:
                self.apps[m[1]] = (m[2], m[3], (m[4] or "").strip())
                self.test("linux", "apps_linux", m[1], "SKIP" if m[2] == "NOT_RUN" else m[2], seconds_ms(m[3]))
        elif head == "WINAPPS_APP":
            m = WINAPP_ROW.fullmatch(line)
            if m:
                res = windows_app_result(m[2], m[3])
                self.windows_apps[m[1]] = (res, m[4], m[5].strip())
                self.test("windows", "apps_windows", m[1], "SKIP" if res == "NOT_RUN" else res, seconds_ms(m[4]))

        elif head == "SPEC_ROW":
            # The SPECviewperf stage: one row per viewset and one FAIL row per failed subtest, status only, no time.
            m = SPEC_ROW.match(line)
            if m:
                self.test("windows", SPEC_SUITE, m[1], SPEC_RESULT[m[2]], None)
                for sub in ([] if m[3] == "-" else m[3].split(","))[:6]:
                    self.test("windows", SPEC_SUITE, "%s/%s" % (m[1], sub), "FAIL", None)
        elif head == "GATE_LKE_COUNTS":
            m = re.fullmatch(r"GATE_LKE_COUNTS reports=([0-9]{1,10}) entries=([0-9]{1,10}|-) unresolved=([0-9]{1,10}|-) unit=distinct_reports", line)
            if m:
                self.windows["known_live_dumps_193"] = self.windows["live_dumps_193"] = int(m[1])
                self.windows["live_dumps_193_entries"] = None if m[2] == "-" else int(m[2])
                self.windows["live_dumps_193_unresolved_entries"] = None if m[3] == "-" else int(m[3])
                self.windows["live_dumps_193_unit"] = "distinct_reports"
        elif head == "GATE_LKE_RATCHET":
            m = re.fullmatch(r"GATE_LKE_RATCHET die=([A-Z]{2}[0-9]{3}|unknown) count=([0-9]{1,10}) max=([0-9]{1,10}|-) verdict=(PASS|WARN|FAIL|UNKNOWN)", line)
            if m and self.windows["live_dumps_193_ratchet"] is None:
                self.windows["known_live_dumps_193"] = self.windows["live_dumps_193"] = int(m[2])
                self.windows["live_dumps_193_ratchet"] = m[4]
                self.windows["live_dumps_193_die"] = m[1]
                self.windows["live_dumps_193_max"] = None if m[3] == "-" else int(m[3])
        elif head == "GATE_PROBE_RESULT":
            parts = line.split(" ", 2)
            if len(parts) == 3 and re.fullmatch(r"[a-z][a-z0-9_]{0,23}", parts[1]):
                d = kvs(parts[2])
                n = self.probe_seen[parts[1]] = self.probe_seen.get(parts[1], 0) + 1
                name = parts[1] if n == 1 else "%s#%d" % (parts[1], n)
                ok = d.get("rc") == "0" and d.get("timeout") == "0"
                self.test("windows", "windows_gate", name, "PASS" if ok else "FAIL", seconds_ms(d.get("secs")))
        elif head.startswith("WINDOWS_GATE_"):
            self.gate_line(line)
        elif head == "KF_PAYLOAD_TIER":
            flag = kvs(line).get("windows")
            self.windows_requested = flag == "1" if flag in ("0", "1") else None
        elif head == "KF_PAYLOAD_STAGE_END":
            d = kvs(line)
            if "name" in d:
                self.stage_end[d["name"]] = (int_or_none(d.get("rc")), whole_ms(d.get("duration_ms")))
        elif head == "stage":
            d = kvs(line)
            if "name" in d and "result" in d:
                self.stage_rows[d["name"]] = (d["result"], int_or_none(d.get("rc")), whole_ms(d.get("duration_ms")))

    def gate_line(self, line):
        m = re.match(r"WINDOWS_GATE_(PASS|FAIL|RETRY)\b", line)
        if m and self.windows["gate"] is None:
            self.windows["gate"] = m[1]
            d = kvs(line)
            if m[1] == "PASS":
                probes = [p for p in d.get("probes", "").split(",") if re.fullmatch(r"[a-z][a-z0-9_]{0,23}", p)]
                self.windows["probes"] = [p for p in probes if not re.fullmatch(r"x[0-9]+", p)][:8]
                self.windows["resets"] = int_or_none(d.get("resets"))
                dumps = d.get("known_live_dumps_193")
                if "live_dumps_193_entries" in d:
                    self.windows["live_dumps_193_entries"] = int_or_none(d["live_dumps_193_entries"])
                    self.windows["live_dumps_193_unit"] = "distinct_reports"
                if self.windows["known_live_dumps_193"] is None:
                    self.windows["known_live_dumps_193"] = self.windows["live_dumps_193"] = int_or_none(dumps) if dumps and dumps != "?" else None
        elif line.startswith("WINDOWS_GATE_TIMING") and self.windows["install_s"] is None:
            m = re.search(r"\binstall=([0-9]+)s", line)
            if m:
                self.windows["install_s"] = int(m[1])

    def bench_line(self, line):
        d = kvs(line)
        mode = d.get("mode")
        if mode not in ("host", "guest"):
            return
        prefix = "GUEST_" if mode == "guest" else ""
        store = self.bench[mode]
        if (prefix + "BSUM") in line.split():
            size = d.get("N")
            if size and size.isdigit():
                for field, (unit, up) in BENCH_UNITS.items():
                    value = number(d.get(field))
                    if value is not None:
                        store["cup8bench N=%s %s" % (size, field)] = (value, unit, up)
                n = int(size)
                if n >= 1024:
                    mat = n * n * 4
                    for field, tag, factor in (("h2d_ms", "h2d_GBps", 2), ("d2h_med_ms", "d2h_GBps", 1)):
                        ms = number(d.get(field))
                        if ms and ms > 0:
                            store["cup8bench N=%s %s" % (size, tag)] = (factor * mat / (ms / 1000.0) / 1e9, "GB/s", True)
        for field in ("BENCH_INIT_MS", "BENCH_CTX_MS", "BENCH_MODULE_MS"):
            value = number(d.get(prefix + field))
            if value is not None:
                store["cup8bench " + field[6:].lower()] = (value, "ms", False)


def int_or_none(text):
    try:
        value = int(text)
    except (TypeError, ValueError):
        return None
    return value if -255 <= value <= 7 * 86400000 else None


def suite_rows(result):
    rows = []
    for row in result.get("suites", []) if isinstance(result.get("suites"), list) else []:
        if (
            isinstance(row, dict)
            and isinstance(row.get("name"), str)
            and contract.SUITE_NAME.fullmatch(row["name"])
            and all(contract._count(row.get(k)) for k in ("pass", "fail", "skip"))
        ):
            kept = {k: row[k] for k in ("name", "pass", "fail", "skip")}
            if kept["name"].startswith("specviewperf"):
                # the Worker drops names that carry the benchmark's name: the report says `spec_viewsets`
                kept["name"] = SPEC_SUITE + kept["name"][len("specviewperf"):]
            if contract.SUITE_NAME.fullmatch(kept["name"]):
                rows.append(kept)
    return rows[: LIMITS["suites"]]


def subrun(sub, kind, os_name, group, rows, host_driver, guest_driver, tests, overall, stages_hint, reason_idle):
    mine = [r for r in rows if r["name"] in group]
    counts = {k: sum(r[k] for r in mine) for k in ("pass", "fail", "skip")}
    names = [r["name"] for r in mine] or list(stages_hint)
    spent = sum(t["ms"] or 0 for t in tests)
    failed = any(r["fail"] > 0 for r in mine)
    if failed:
        verdict, reason = "FAIL", None
    elif not mine:
        verdict, reason = "NOT_RUN", reason_idle
    elif overall == "PASS" or (
        all(r["pass"] > 0 for r in mine) and set(stages_hint) <= {r["name"] for r in mine}
    ):
        verdict, reason = "PASS", None
    else:
        verdict, reason = "UNATTRIBUTED", "incomplete: " + ",".join(sorted(set(stages_hint) - {r["name"] for r in mine}))[:150]
    return {
        "sub": sub,
        "kind": kind,
        "os": os_name,
        "guest_driver": guest_driver,
        "host_driver": host_driver,
        "verdict": verdict,
        "reason": reason,
        "stages": names[:16],
        "log_stages": LOG_STAGES["host" if os_name == "host" else os_name],
        "started_at": None,
        "ended_at": None,
        "duration_ms": spent if spent else None,
        "counts": counts,
    }


def build_report(lines, result, run_id=None):
    """The report document (schema 1) from the payload log lines and the result file."""
    c = Collector()
    for line in lines:
        c.feed(line)
    app_policy.apply(c)
    policy = app_policy.rows()
    policy_names = {os: {p["name"] for p in policy if p["os"] == os} for os in ("linux", "windows")}
    rows = suite_rows(result)
    hardware = result.get("hardware") if isinstance(result.get("hardware"), dict) else {}
    claims = contract.clean_claims(result.get("host_claims"))
    host_driver = hardware.get("driver_version") if DRIVER.fullmatch(str(hardware.get("driver_version"))) else None
    linux_driver = claims.get("linux_driver")
    windows_driver = claims.get("windows_driver")
    overall = result.get("verdict")
    failing = [r["name"] for r in rows if r["fail"] > 0]
    idle = "not reached: " + failing[0] if failing else "stage not reached"
    windows_idle = (
        "Windows stage not requested for this profile"
        if c.windows_requested is False
        else idle
    )
    sub_linux = "linux-" + linux_driver if linux_driver else "linux-unknown"
    sub_windows = "windows-" + windows_driver if windows_driver else "windows"
    keys = {"host": "host", "linux": sub_linux, "windows": sub_windows}
    mine = {k: [t for t in c.tests.values() if t["_key"] == k] for k in keys}
    linux_group = LINUX_SUITES + tuple(r["name"] for r in rows if r["name"].startswith("apps"))
    subruns = [
        subrun("host", "host_bare", "host", HOST_SUITES, rows, host_driver, None, mine["host"], overall,
               ("cuda_host", "gates", "kf3_build", "raw_suite"), idle),
        subrun(sub_linux, "linux_guest", "linux", linux_group, rows, host_driver, linux_driver, mine["linux"],
               overall, ("thin_suite", "birth_census", "cuda_fat"), idle),
        subrun(sub_windows, "windows_guest", "windows",
               [r["name"] for r in rows if r["name"].startswith(("windows", SPEC_SUITE))] or ["windows_gate"], rows,
               host_driver, windows_driver, mine["windows"], overall, ("windows_gate",), windows_idle),
    ]
    if subruns[2]["verdict"] == "NOT_RUN":
        subruns[2]["stages"] = ["windows_gate"]
    if c.windows_requested and c.windows["gate"] in ("FAIL", "RETRY"):
        subruns[2]["verdict"] = c.windows["gate"]
        subruns[2]["reason"] = None
    # A SPECviewperf lane that could not finish (media, transport, time, a host reference: rows `<stage>_retry_<class>`)
    # is a RETRY of the Windows subrun, not a failure of the guest; any other failing row of the subrun stays a FAIL.
    spec_bad = [r for r in rows if r["fail"] > 0 and r["name"].startswith(("windows", SPEC_SUITE))]
    if spec_bad and subruns[2]["verdict"] == "FAIL" and all(r["name"].startswith(SPEC_SUITE + "_retry_") for r in spec_bad):
        subruns[2]["verdict"] = "RETRY"
    tests = []
    for key, sub_id in keys.items():
        for t in mine[key]:
            tests.append({"sub": sub_id, "suite": t["suite"], "name": t["name"], "result": t["result"], "ms": t["ms"]})
    perf = []
    for mode, key in (("host", "host"), ("guest", "linux")):
        for name, (value, unit, up) in c.bench[mode].items():
            row = {"sub": keys[key], "name": name, "value": value, "unit": unit, "higher_is_better": up,
                   "suite": "cuda_host" if mode == "host" else "cuda_fat", "ref_value": None, "ref_ratio": None}
            if mode == "guest" and name in c.bench["host"]:
                ref = c.bench["host"][name][0]
                row["ref_value"] = ref
                row["ref_ratio"] = value / ref if ref else None
            perf.append(row)
    stages = []
    for name, (res, rc, duration) in c.stage_rows.items():
        stages.append({"name": name, "result": STAGE_RESULT.get(res, "NOT_RUN" if res == "NOT_RUN" else "UNATTRIBUTED"),
                       "rc": rc, "duration_ms": duration})
    for name, (rc, duration) in c.stage_end.items():
        if name not in c.stage_rows:
            stages.append({"name": name, "result": "UNATTRIBUTED", "rc": rc, "duration_ms": duration})
    doc = {
        "schema": 1,
        "guest_drivers": {"linux": linux_driver, "windows": windows_driver},
        "stages": stages[: LIMITS["stages"]],
        "suites": rows,
        "subruns": subruns,
        "tests": tests[: LIMITS["tests"]],
        "apps": ([
            {"sub": sub_linux if p["os"] == "linux" else sub_windows, "name": p["name"], "os": p["os"],
             "result": "NOT_RUN", "note": (c.apps if p["os"] == "linux" else c.windows_apps)[p["name"]][2][:200]}
            for p in policy
        ] + [
            {"sub": sub_linux, "name": name, "os": "linux", "result": res, "note": note[:200]}
            for name, (res, _secs, note) in c.apps.items()
            if name not in policy_names["linux"]
        ] + [
            {"sub": sub_windows, "name": name, "os": "windows", "result": res, "note": note[:200]}
            for name, (res, _secs, note) in c.windows_apps.items()
            if name not in policy_names["windows"]
        ])[: LIMITS["apps"]],  # policy entries always present; observed supported apps follow
        "perf": perf[: LIMITS["perf"]],
        "windows": c.windows,
    }
    return bound(doc)


def bound(doc, limit=MAX_BYTES):
    """Shrink the largest arrays (newest rows are kept last) until the JSON fits."""
    def size():
        return len(json.dumps(doc, separators=(",", ":")).encode())
    for key in ("tests", "perf", "apps"):
        while size() > limit and doc[key]:
            doc[key] = doc[key][: len(doc[key]) // 2]
    return doc


# --- upload ----------------------------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Transport:
    """HTTPS POST to one fixed URL; no proxy, no redirect; the token only in a header."""

    def __init__(self, token, access=None, timeout=20.0):
        self.token, self.timeout = token, timeout
        ident, secret = (access or {}).get("KF_ACCESS_ID"), (access or {}).get("KF_ACCESS_SECRET")
        self.access = (
            {"CF-Access-Client-Id": ident, "CF-Access-Client-Secret": secret} if ident and secret else {}
        )
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            _NoRedirect(),
            urllib.request.HTTPSHandler(context=ssl.create_default_context()),
        )

    def post(self, url, body):
        headers = {
            "Content-Type": "application/json",
            "Authorization": "Bearer " + self.token,
            "User-Agent": contract.USER_AGENT,
            **self.access,
        }
        request = urllib.request.Request(url, data=body, headers=headers, method="POST")
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                status, raw = response.status, response.read(65536)
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read(65536)
            error.close()
        except (urllib.error.URLError, OSError, ValueError):
            return None, None
        try:
            return status, json.loads(raw)
        except ValueError:
            return status, None


def say(text):
    print(text, flush=True)


def run(argv, env, transport=None, clock=time.monotonic):
    """The whole step. Never raises; prints one outcome line; returns 0."""
    log, result_path, dry = "/workspace/kf-ci-output.log", contract.RESULT_PATH, False
    budget = 25.0
    args = list(argv)
    while args:
        flag = args.pop(0)
        if flag == "--log" and args:
            log = args.pop(0)
        elif flag == "--result" and args:
            result_path = args.pop(0)
        elif flag == "--budget" and args:
            budget = float(args.pop(0))
        elif flag == "--dry-run":
            dry = True
    if env.get("KF_RUN_REPORT") != "1" and not dry:
        say("KF_RUN_REPORT_SKIPPED reason=not_enabled")
        return 0
    deadline = clock() + budget
    try:
        doc = build_report(lines_of(log), read_result(result_path))
        body = json.dumps(doc, separators=(",", ":")).encode()
    except Exception as error:  # noqa: BLE001  (a builder bug must never touch the run)
        say("KF_RUN_REPORT_SKIPPED reason=build_error:%s" % type(error).__name__)
        return 0
    if len(body) > MAX_BYTES:
        say("KF_RUN_REPORT_SKIPPED reason=too_large")
        return 0
    if dry:
        say("KF_RUN_REPORT_BUILT bytes=%d subruns=%d tests=%d perf=%d" % (
            len(body), len(doc["subruns"]), len(doc["tests"]), len(doc["perf"])))
        return 0
    try:
        if contract.validate_env(env):
            say("KF_RUN_REPORT_SKIPPED reason=environment_contract")
            return 0
        url = "%s/run-report/%s" % (env["KF_BROKER_URL"], env["KF_RUN_ID"])
        transport = transport or Transport(env["KF_RUN_TOKEN"], access=env)
        reason = "upload_failed"
        for attempt in range(3):
            if deadline - clock() < 3:
                reason = "time"
                break
            status, answer = transport.post(url, body)
            state = answer.get("state") if isinstance(answer, dict) else None
            if status == 200:
                say("KF_RUN_REPORT_SENT bytes=%d subruns=%d" % (len(body), len(doc["subruns"])))
                return 0
            if status == 409 and state == "REPORT_EXISTS":
                say("KF_RUN_REPORT_SENT bytes=%d already" % len(body))
                return 0
            reason = "http_%s" % (status if status is not None else "none")
            if status not in (None, 409, 429, 500, 502, 503, 504):
                break
            time.sleep(min(2.0 * (attempt + 1), max(0.0, deadline - clock() - 3)))
        say("KF_RUN_REPORT_SKIPPED reason=%s" % reason)
    except Exception as error:  # noqa: BLE001
        say("KF_RUN_REPORT_SKIPPED reason=error:%s" % type(error).__name__)
    return 0


def main(argv=None, env=None):
    return run(sys.argv[1:] if argv is None else argv, dict(os.environ if env is None else env))


if __name__ == "__main__":
    sys.exit(main())
