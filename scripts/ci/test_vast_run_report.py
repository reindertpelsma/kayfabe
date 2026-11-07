"""GPU-free tests of the box run report (vast_run_report.py).

FALSIFIER: a report that invents a number or a row no output of the payload produced (an apps row,
a perf row without a BSUM or BENCH_*_MS line, a test row without an arm line); that puts a
SPECviewperf datum, a run token or any secret in a row; that exceeds 256 KiB; that attaches a row
to the wrong subrun or to none; that hides a subrun that did not run (it must be NOT_RUN with a
reason); that sends without KF_RUN_REPORT=1, sends the token anywhere but the Authorization header
of the fixed URL, raises, changes the result file or prints the token.
"""

import json
import os
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vast_run_report as rr  # noqa: E402

RUN = "4242-7"
TOKEN = ".".join(["eyJhbGciOiJIUzI1NiJ9", "eyJydW4iOiI0MjQyIn0", ("Zk9_-Qw" * 7)[:43]])
URL = "https://broker.example/pfx-9f3a7c1d9f3a7c1d9f3a7c1d"
ENV = {
    "KF_PROTOCOL": "7",
    "KF_BROKER_URL": URL,
    "KF_RUN_ID": RUN,
    "KF_RUN_TOKEN": TOKEN,
    "KF_NONCE": "ab" * 16,
    "KF_SHA": "1234abcd" * 5,
    "KF_REPO": "reindertpelsma/kayfabe",
    "KF_RUNNER_NONCE": "cd" * 32,
    "KF_SOFT_DEADLINE": "1900000000",
    "KF_HEARTBEAT_SECONDS": "30",
    "KF_GRACE_FAIL": "600",
    "KF_RUN_REPORT": "1",
}
BSUM = (
    "BSUM N=%d iters=20 first_ms=0.9 med_ms=%s p90_ms=0.04 min_ms=0.03 gflops=%s "
    "batch_per_launch_ms=0.012 batch_gflops=%s h2d_ms=%s d2h_med_ms=%s submit_med_ms=0.01 sync_med_ms=0.02"
)


def log_lines(windows=True, host="580.159.04", failing=None):
    lines = [
        "KF_PAYLOAD_TIER windows=%d source=worker tier=middle windows_runs=1 stages=none" % windows,
        "KF_PAYLOAD_STAGE_START name=cuda_host",
        "CL_ROW mode=host rung=cup2 rep=1 verdict=PASS rc=0 wall_ms=1234 graded=[x]",
        "CL_ROW mode=host rung=cup8bench rep=1 verdict=PASS rc=0 wall_ms=22000 graded=[x]",
        "CL_BENCH mode=host rep=1 BENCH_INIT_MS=61.5",
        "CL_BENCH mode=host rep=1 " + BSUM % (16, "0.031", "0.26", "0.68", "0.02", "0.03"),
        "CL_BENCH mode=host rep=1 " + BSUM % (2048, "7.0", "2400.5", "2600.0", "3.2", "3.4"),
        "KF_PAYLOAD_STAGE_END name=cuda_host rc=0 duration_ms=40000",
        "KF_PAYLOAD_STAGE_START name=merge_bar",
        "BARE_CELL_ARM arm=timer verdict=PASS secs=1 ms=300 rc=0 pci_dev=0x2786 drv=%s kmod=open" % host,
        "BARE_CELL_ARM arm=engines verdict=FAIL secs=2 ms=410 rc=1 pci_dev=0x2786 drv=%s kmod=open" % host,
        "FAST_CELL_ARM arm=timer verdict=PASS secs=12 client_ms=9500",
        "FAST_CELL_ARM arm=engines verdict=NOTRUN secs=- client_ms=?",
        "FAST_CELL_ARM arm=doorbell-census verdict=VMM_DIED secs=6 client_ms=?",
        "KF_PAYLOAD_STAGE_END name=merge_bar rc=0 duration_ms=1900000",
        "KF_PAYLOAD_STAGE_START name=cuda_guest",
        "CL_ROW mode=guest rung=cup8bench rep=1 verdict=PASS boot_rc=0 graded=[x] BENCH_measure_WALL_S=33.4 trapped=1 ledger=[]",
        "CL_BENCH mode=guest rep=1 GUEST_BENCH_INIT_MS=80.0",
        "CL_BENCH mode=guest rep=1 GUEST_" + BSUM % (2048, "7.5", "2300.0", "2500.0", "3.4", "3.6"),
        "KF_PAYLOAD_STAGE_END name=cuda_guest rc=0 duration_ms=300000",
    ]
    if windows:
        lines += [
            "KF_PAYLOAD_STAGE_START name=windows_gate",
            "GATE_PROBE_RESULT cup8 GATE_PROBE exe=cup8 arg= rc=0 secs=12 timeout=0",
            "GATE_PROBE_RESULT d3d11 GATE_PROBE exe=kf_dxprobe arg= rc=0 secs=20 timeout=0",
            "GATE_PROBE_RESULT cup8 GATE_PROBE exe=cup8 arg= rc=0 secs=11 timeout=0",
            'WINDOWS_GATE_PASS rev=1234abcd gpu="RTX 4070" idle_s=300 hostref=green/green secs=3600 '
            "install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=1",
            "WINDOWS_GATE_TIMING total=3662s preflight=9s install=1852s boot1=155s",
            "KF_PAYLOAD_STAGE_END name=windows_gate rc=0 duration_ms=3700000",
        ]
    lines += [
        "stage name=provision rc=0 duration_ms=900000 result=PASS",
        "stage name=cuda_host rc=0 duration_ms=40000 result=PASS",
        "stage name=merge_bar rc=0 duration_ms=1900000 result=PASS",
        "stage name=cuda_guest rc=0 duration_ms=300000 result=TIMEOUT",
        "stage name=windows_gate result=NOT_RUN",
        "SPECVIEWPERF_STAGE_PASS",
        "viewperf maya-06 score=123.4",
    ]
    return lines


def result(windows=True, verdict="PASS", rows=None, claims=None):
    suites = rows or [
        ("provision", 1, 0, 0), ("gates", 9, 0, 0), ("kf3_build", 1, 0, 0), ("raw_suite", 30, 0, 0),
        ("thin_suite", 30, 0, 0), ("birth_census", 1, 0, 0), ("cuda_host", 4, 0, 0), ("cuda_fat", 4, 0, 0),
    ] + ([("windows_gate", 1, 0, 0)] if windows else [])
    return {
        "verdict": verdict,
        "suites": [dict(name=n, **{"pass": p, "fail": f, "skip": s}) for n, p, f, s in suites],
        "hardware": {"gpu_name": "NVIDIA GeForce RTX 4070", "compute_cap": 890, "driver_version": "580.159.04"},
        "host_claims": claims
        if claims is not None
        else dict(
            linux_driver="580.159.03",
            linux_os_version="ubuntu-24.04",
            **({"windows_driver": "580.88", "windows_os_version": "11-ltsc-2024-26100"} if windows else {}),
        ),
    }


class Build(unittest.TestCase):
    def build(self, lines, res):
        return rr.build_report(lines, res)

    def setUp(self):
        self.doc = self.build(log_lines(), result())

    def by_sub(self):
        return {s["sub"]: s for s in self.doc["subruns"]}

    def test_the_host_session_has_a_subrun_per_environment(self):
        subs = self.by_sub()
        self.assertEqual(list(subs), ["host", "linux-580.159.03", "windows-580.88"])
        host, linux, windows = subs["host"], subs["linux-580.159.03"], subs["windows-580.88"]
        self.assertEqual((host["kind"], host["os"], host["guest_driver"]), ("host_bare", "host", None))
        self.assertEqual((linux["kind"], linux["os"], linux["guest_driver"]), ("linux_guest", "linux", "580.159.03"))
        self.assertEqual((windows["kind"], windows["os"], windows["guest_driver"]), ("windows_guest", "windows", "580.88"))
        for s in subs.values():
            self.assertEqual(s["host_driver"], "580.159.04")
            self.assertEqual(s["verdict"], "PASS")
        self.assertEqual(host["counts"], {"pass": 44, "fail": 0, "skip": 0})
        self.assertEqual(linux["counts"], {"pass": 35, "fail": 0, "skip": 0})
        self.assertEqual(windows["counts"], {"pass": 1, "fail": 0, "skip": 0})
        self.assertEqual(host["log_stages"], ["cuda_host", "merge_bar"])
        self.assertEqual(windows["log_stages"], ["windows_gate", "windows_apps", "windows_gate_2"])
        self.assertEqual(self.doc["guest_drivers"], {"linux": "580.159.03", "windows": "580.88"})

    def test_every_row_carries_its_subrun_and_only_known_subruns(self):
        ids = set(self.by_sub())
        for key in ("tests", "perf", "apps"):
            for row in self.doc[key]:
                self.assertIn(row["sub"], ids)
        rows = {(t["sub"], t["suite"], t["name"]): t for t in self.doc["tests"]}
        self.assertEqual(rows[("host", "raw_suite", "timer")]["ms"], 300)
        self.assertEqual(rows[("host", "raw_suite", "engines")]["result"], "FAIL")
        self.assertEqual(rows[("host", "cuda_host", "cup2")]["ms"], 1234)
        self.assertEqual(rows[("linux-580.159.03", "thin_suite", "timer")]["ms"], 9500)
        self.assertEqual(rows[("linux-580.159.03", "thin_suite", "engines")]["result"], "SKIP")
        self.assertEqual(rows[("linux-580.159.03", "thin_suite", "doorbell-census")]["result"], "FAIL")
        self.assertEqual(rows[("linux-580.159.03", "thin_suite", "doorbell-census")]["ms"], 6000)
        self.assertEqual(rows[("linux-580.159.03", "cuda_fat", "cup8bench")]["ms"], 33400)
        self.assertEqual(rows[("windows-580.88", "windows_gate", "cup8")]["ms"], 12000)
        self.assertEqual(rows[("windows-580.88", "windows_gate", "cup8#2")]["ms"], 11000)
        self.assertEqual(rows[("windows-580.88", "windows_gate", "d3d11")]["result"], "PASS")

    def test_perf_rows_come_only_from_bench_lines_and_the_guest_has_a_host_reference(self):
        perf = {(p["sub"], p["name"]): p for p in self.doc["perf"]}
        host = perf[("host", "cup8bench N=2048 gflops")]
        self.assertEqual((host["value"], host["unit"], host["higher_is_better"]), (2400.5, "GFLOPS", True))
        self.assertIsNone(host["ref_value"])
        guest = perf[("linux-580.159.03", "cup8bench N=2048 gflops")]
        self.assertEqual(guest["ref_value"], 2400.5)
        self.assertAlmostEqual(guest["ref_ratio"], 2300.0 / 2400.5)
        self.assertEqual(perf[("host", "cup8bench N=2048 med_ms")]["higher_is_better"], False)
        self.assertEqual(perf[("host", "cup8bench init_ms")]["value"], 61.5)
        self.assertEqual(perf[("linux-580.159.03", "cup8bench init_ms")]["value"], 80.0)
        # Derived the way scripts/bench/perf_summarize.py derives it: 2 matrices over h2d_ms.
        gb = perf[("host", "cup8bench N=2048 h2d_GBps")]
        self.assertAlmostEqual(gb["value"], 2 * 2048 * 2048 * 4 / (3.2 / 1000) / 1e9)
        self.assertEqual(gb["unit"], "GB/s")
        self.assertNotIn(("host", "cup8bench N=16 h2d_GBps"), perf, "small sizes are not a bandwidth measure")
        self.assertEqual(self.doc["apps"], [], "the application matrix is not a payload stage yet")
        for p in self.doc["perf"]:
            self.assertTrue(p["suite"] in ("cuda_host", "cuda_fat"))
            self.assertIsNone(p["ref_value"]) if p["sub"] == "host" else None

    def test_windows_facts_come_from_the_gate_lines(self):
        self.assertEqual(
            self.doc["windows"],
            {"gate": "PASS", "install_s": 1852, "probes": ["cup8", "d3d11"], "resets": 0, "live_dumps_193": 1},
        )

    def test_stage_rows_map_the_payload_results(self):
        stages = {s["name"]: s for s in self.doc["stages"]}
        self.assertEqual(stages["provision"], {"name": "provision", "result": "PASS", "rc": 0, "duration_ms": 900000})
        self.assertEqual(stages["cuda_guest"]["result"], "FAIL", "TIMEOUT is a failure of the stage")
        self.assertEqual(stages["windows_gate"]["result"], "NOT_RUN")
        # Without a summary line the stage is UNATTRIBUTED, never a guessed PASS.
        doc = self.build([x for x in log_lines() if not x.startswith("stage name=cuda_host")], result())
        self.assertEqual({s["name"]: s for s in doc["stages"]}["cuda_host"]["result"], "UNATTRIBUTED")

    def test_suites_are_the_graded_rows_of_the_result_file(self):
        self.assertEqual(
            {s["name"]: (s["pass"], s["fail"], s["skip"]) for s in self.doc["suites"]}["raw_suite"], (30, 0, 0)
        )
        self.assertEqual(len(self.doc["suites"]), 9)

    def test_nothing_about_specviewperf_and_no_secret_shape_is_ever_a_row(self):
        text = json.dumps(self.doc)
        self.assertNotRegex(text, r"(?i)viewperf|maya")
        for key in ("stages", "suites", "tests", "perf", "apps"):
            self.assertNotIn("score", json.dumps(self.doc[key]))

    def test_a_subrun_that_did_not_run_is_not_run_with_a_reason(self):
        doc = self.build(log_lines(windows=False), result(windows=False))
        subs = {s["os"]: s for s in doc["subruns"]}
        self.assertEqual(subs["windows"]["verdict"], "NOT_RUN")
        self.assertEqual(subs["windows"]["reason"], "Windows stage not requested for this profile")
        self.assertEqual(subs["windows"]["sub"], "windows")
        self.assertIsNone(subs["windows"]["guest_driver"])
        self.assertEqual(doc["windows"]["gate"], None)
        self.assertEqual(subs["linux"]["verdict"], "PASS")

    def test_a_failure_stops_the_payload_and_the_later_subruns_are_not_run(self):
        rows = [("provision", 1, 0, 0), ("gates", 3, 1, 0)]
        doc = self.build(
            ["KF_PAYLOAD_TIER windows=1 source=worker tier=middle windows_runs=1 stages=none"],
            result(verdict="FAIL", rows=rows),
        )
        subs = {s["os"]: s for s in doc["subruns"]}
        self.assertEqual(subs["host"]["verdict"], "FAIL")
        self.assertEqual(subs["host"]["counts"], {"pass": 3, "fail": 1, "skip": 0})
        self.assertEqual(subs["linux"]["verdict"], "NOT_RUN")
        self.assertEqual(subs["linux"]["reason"], "not reached: gates")
        self.assertEqual(subs["windows"]["verdict"], "NOT_RUN")
        self.assertEqual(doc["tests"], [])

    def test_a_windows_retry_is_a_retry_not_a_failure_of_the_guest(self):
        rows = [("provision", 1, 0, 0), ("gates", 9, 0, 0), ("kf3_build", 1, 0, 0), ("raw_suite", 30, 0, 0),
                ("thin_suite", 30, 0, 0), ("birth_census", 1, 0, 0), ("cuda_host", 4, 0, 0),
                ("cuda_fat", 4, 0, 0), ("windows_retry_provider", 0, 1, 0)]
        lines = [x for x in log_lines() if not x.startswith("WINDOWS_GATE_PASS")] + [
            "WINDOWS_GATE_RETRY action=rerun-on-a-fresh-box class=PROVIDER stage=boot1 why=x rev=1234abcd"
        ]
        doc = self.build(lines, result(verdict="FAIL", rows=rows))
        windows = {s["os"]: s for s in doc["subruns"]}["windows"]
        self.assertEqual(windows["verdict"], "RETRY")
        self.assertEqual(doc["windows"]["gate"], "RETRY")

    def test_a_malformed_host_driver_is_dropped_from_every_subrun(self):
        res = result()
        res["hardware"]["driver_version"] = "580; rm -rf /"
        self.assertTrue(all(s["host_driver"] is None for s in self.build(log_lines(), res)["subruns"]))

    APPS = [
        "APP_ROW name=blender os=linux result=PASS secs=41 guest_xid=0 note=ok",
        "APP_ROW name=ollama os=linux result=FAIL secs=90 guest_xid=- note=FAIL exit 1 with spaces in the note",
        "APP_ROW name=vlc os=linux result=NOT_RUN secs=- guest_xid=- note=no result row (budget or boot)",
    ]

    def test_apps_stage_rows_become_apps_and_tests_of_the_linux_subrun(self):
        doc = self.build(log_lines() + self.APPS, result())
        sub = "linux-580.159.03"
        self.assertEqual(
            doc["apps"],
            [
                {"sub": sub, "name": "blender", "os": "linux", "result": "PASS", "note": "ok"},
                {"sub": sub, "name": "ollama", "os": "linux", "result": "FAIL", "note": "FAIL exit 1 with spaces in the note"},
                {"sub": sub, "name": "vlc", "os": "linux", "result": "NOT_RUN", "note": "no result row (budget or boot)"},
            ],
        )
        rows = {t["name"]: t for t in doc["tests"] if t["suite"] == "apps_linux"}
        self.assertEqual({k: (v["result"], v["ms"], v["sub"]) for k, v in rows.items()}, {
            "blender": ("PASS", 41000, sub), "ollama": ("FAIL", 90000, sub), "vlc": ("SKIP", None, sub)})

    def test_no_apps_stage_means_no_apps_rows(self):
        doc = self.build(log_lines(), result())
        self.assertEqual(doc["apps"], [])
        self.assertFalse([t for t in doc["tests"] if t["suite"] == "apps_linux"])

    def test_a_failed_apps_stage_fails_the_linux_subrun_and_a_retry_does_too(self):
        base = [("provision", 1, 0, 0), ("gates", 9, 0, 0), ("kf3_build", 1, 0, 0), ("raw_suite", 30, 0, 0),
                ("thin_suite", 30, 0, 0), ("birth_census", 1, 0, 0), ("cuda_host", 4, 0, 0), ("cuda_fat", 4, 0, 0)]
        for name in ("apps", "apps_retry_time"):
            doc = self.build(log_lines(windows=False), result(windows=False, verdict="FAIL", rows=base + [(name, 0, 1, 0)]))
            linux = {s["os"]: s for s in doc["subruns"]}["linux"]
            self.assertEqual(linux["verdict"], "FAIL", name)
            self.assertIn(name, linux["stages"])
            self.assertEqual(linux["counts"]["fail"], 1)
        ok = self.build(log_lines(windows=False), result(windows=False, rows=base + [("apps", 12, 0, 0)]))
        linux = {s["os"]: s for s in ok["subruns"]}["linux"]
        self.assertEqual((linux["verdict"], linux["counts"]["pass"]), ("PASS", 47))
        self.assertIn("apps", linux["log_stages"])

    def test_hostile_or_foreign_app_rows_are_ignored_and_notes_are_bounded(self):
        lines = [
            "APP_ROW name=ok os=linux result=PASS secs=1 guest_xid=0 note=" + "n" * 500,
            "APP_ROW name=win os=windows result=PASS secs=1 guest_xid=0 note=x",
            "APP_ROW name=bad-name! os=linux result=PASS secs=1",
            "APP_ROW name=weird os=linux result=MAYBE secs=1",
            "APP_ROW name=" + "a" * 60 + " os=linux result=PASS",
        ]
        doc = self.build(lines, result())
        self.assertEqual([a["name"] for a in doc["apps"]], ["ok"])
        self.assertEqual(len(doc["apps"][0]["note"]), 200)

    def test_the_apps_array_is_capped(self):
        lines = ["APP_ROW name=a%d os=linux result=PASS secs=1" % i for i in range(650)]
        doc = self.build(lines, result())
        self.assertEqual(len(doc["apps"]), 500)

    def test_a_missing_thin_suite_is_not_hidden_by_an_apps_row(self):
        rows = [("provision", 1, 0, 0), ("birth_census", 1, 0, 0), ("cuda_fat", 4, 0, 0), ("apps", 3, 0, 0)]
        doc = self.build(log_lines(windows=False), result(windows=False, verdict="FAIL", rows=rows))
        linux = {s["os"]: s for s in doc["subruns"]}["linux"]
        self.assertEqual(linux["verdict"], "UNATTRIBUTED")
        self.assertIn("thin_suite", linux["reason"])

    def test_an_unknown_linux_driver_gets_an_honest_id(self):
        doc = self.build(log_lines(), result(claims={}))
        self.assertEqual([s["sub"] for s in doc["subruns"]], ["host", "linux-unknown", "windows"])
        self.assertEqual(doc["guest_drivers"], {"linux": None, "windows": None})

    def test_a_missing_or_empty_input_gives_a_report_not_an_exception(self):
        doc = self.build([], {})
        self.assertEqual(doc["schema"], 1)
        self.assertTrue(all(s["verdict"] == "NOT_RUN" and s["reason"] for s in doc["subruns"]))
        self.assertEqual((doc["tests"], doc["perf"], doc["apps"]), ([], [], []))

    def test_hostile_lines_are_skipped_not_trusted(self):
        hostile = [
            "FAST_CELL_ARM arm=" + "x" * 9000,
            "FAST_CELL_ARM arm=a verdict=PASS secs=nan client_ms=1e99",
            "CL_BENCH mode=host rep=1 " + BSUM % (2048, "inf", "nan", "1e999", "-3", "x"),
            "BARE_CELL_ARM arm=b verdict=PASS ms=999999999999",
            "GATE_PROBE_RESULT Bad-Name GATE_PROBE rc=0 secs=1 timeout=0",
            "stage name=evil result=PASS rc=zzz duration_ms=-5",
        ]
        doc = self.build(hostile, result())
        text = json.dumps(doc)
        self.assertNotIn("Infinity", text)
        self.assertNotIn("NaN", text)
        for p in doc["perf"]:
            self.assertEqual(p["value"], p["value"])
            self.assertLess(abs(p["value"]), 1e15)
        names = [t["name"] for t in doc["tests"]]
        self.assertIn("a", names)
        self.assertNotIn("Bad-Name", names)
        self.assertEqual([t["ms"] for t in doc["tests"] if t["name"] == "b"], [None])

    def test_the_document_is_bounded_to_256_KiB(self):
        lines = log_lines()
        lines += ["FAST_CELL_ARM arm=arm-%05d-%s verdict=PASS secs=1 client_ms=1" % (i, "n" * 100) for i in range(6000)]
        doc = self.build(lines, result())
        self.assertLessEqual(len(json.dumps(doc, separators=(",", ":")).encode()), rr.MAX_BYTES)
        self.assertLessEqual(len(doc["tests"]), rr.LIMITS["tests"])

    def test_duplicate_arms_keep_the_last_row(self):
        doc = self.build(
            ["FAST_CELL_ARM arm=timer verdict=FAIL secs=1 client_ms=5", "FAST_CELL_ARM arm=timer verdict=PASS secs=1 client_ms=7"],
            result(),
        )
        rows = [t for t in doc["tests"] if t["name"] == "timer"]
        self.assertEqual([(t["result"], t["ms"]) for t in rows], [("PASS", 7)])


class WindowsApps(unittest.TestCase):
    """Falsifier: missing/harness rows pass, Windows overwrites Linux, or a gate PASS hides apps FAIL."""

    def line(self, name="vectorAdd", cls="pass", verdict="PASS", secs="2", note="-"):
        return (f"WINAPPS_APP app={name} class={cls} verdict={verdict} batched={verdict} alone=- "
                f"secs={secs} tdr=0 wer=0 gsp=0 note={note}")

    def build(self, lines, suite="windows_apps", failed=0):
        res = result(verdict="FAIL" if failed else "PASS")
        res["suites"].append(dict(name=suite, **{"pass": 0 if failed else 1, "fail": failed, "skip": 0}))
        return rr.build_report(log_lines() + lines, res)

    def test_windows_app_is_reported_with_its_subrun_and_measured_time(self):
        doc = self.build([self.line()])
        self.assertEqual(doc["apps"], [dict(sub="windows-580.88", name="vectorAdd", os="windows", result="PASS", note="-")])
        row = next(t for t in doc["tests"] if t["suite"] == "apps_windows")
        self.assertEqual((row["sub"], row["result"], row["ms"]), ("windows-580.88", "PASS", 2000))
        self.assertIn("windows_apps", doc["subruns"][2]["log_stages"])

    def test_same_name_in_linux_and_windows_keeps_both_rows(self):
        doc = self.build(["APP_ROW name=vectorAdd os=linux result=FAIL secs=1", self.line()])
        self.assertEqual([(a["os"], a["result"]) for a in doc["apps"]], [("linux", "FAIL"), ("windows", "PASS")])

    def test_missing_and_harness_rows_are_not_run_never_pass(self):
        for cls, verdict in (("not_run", "-"), ("harness", "NOTRUN"), ("pass", "FAIL")):
            doc = self.build([self.line(cls=cls, verdict=verdict, secs="-")])
            self.assertEqual(doc["apps"][0]["result"], "NOT_RUN")
            row = next(t for t in doc["tests"] if t["suite"] == "apps_windows")
            self.assertEqual((row["result"], row["ms"]), ("SKIP", None))

    def test_windows_class_and_verdict_mapping_is_fail_closed(self):
        for cls, verdict, want in (("pass", "PASS", "PASS"), ("pass", "PASS*", "PASS"),
                                   ("guest_fail", "FAIL", "FAIL"), ("guest_fail", "TIMEOUT", "FAIL"),
                                   ("boot_fail", "BOOT_FAIL", "FAIL"), ("harness", "NOTRUN", "NOT_RUN"),
                                   ("not_run", "-", "NOT_RUN"), ("pass", "FAIL", "NOT_RUN")):
            self.assertEqual(rr.windows_app_result(cls, verdict), want)

    def test_fail_and_timeout_rows_fail_with_bounded_notes(self):
        for cls, verdict in (("guest_fail", "FAIL"), ("guest_fail", "TIMEOUT"), ("boot_fail", "BOOT_FAIL")):
            doc = self.build([self.line(cls=cls, verdict=verdict, note="n" * 400)], failed=1)
            self.assertEqual(doc["apps"][0]["result"], "FAIL")
            self.assertEqual(len(doc["apps"][0]["note"]), 200)
            self.assertEqual(next(t for t in doc["tests"] if t["suite"] == "apps_windows")["result"], "FAIL")

    def test_duplicate_windows_app_keeps_the_final_row(self):
        doc = self.build([self.line(), self.line(cls="guest_fail", verdict="FAIL")], failed=1)
        self.assertEqual(len(doc["apps"]), 1)
        self.assertEqual(doc["apps"][0]["result"], "FAIL")

    def test_a_gate_pass_does_not_hide_a_windows_apps_suite_failure(self):
        for suite in ("windows_apps", "windows_apps_retry_time", "windows_apps_retry_media"):
            doc = self.build([], suite=suite, failed=1)
            self.assertEqual(doc["subruns"][2]["verdict"], "FAIL")
            self.assertIn(suite, doc["subruns"][2]["stages"])
            self.assertIn(dict(name=suite, **{"pass": 0, "fail": 1, "skip": 0}), doc["suites"])

    def test_no_app_rows_are_invented_by_a_windows_apps_suite(self):
        self.assertEqual(self.build([])["apps"], [])

    def test_bad_names_and_incomplete_windows_rows_are_ignored(self):
        doc = self.build([self.line(name="../bad"), "WINAPPS_APP app=ok class=pass verdict=PASS"])
        self.assertEqual(doc["apps"], [])

    def test_windows_apps_share_the_array_cap_with_linux(self):
        lines = [f"APP_ROW name=l{i} os=linux result=PASS" for i in range(300)]
        lines += [self.line(name=f"w{i}") for i in range(300)]
        self.assertEqual(len(self.build(lines)["apps"]), 500)

    def test_windows_notes_are_bounded_even_for_passing_apps(self):
        self.assertEqual(len(self.build([self.line(note="n" * 400)])["apps"][0]["note"]), 200)

    def test_recorded_full_run_reports_58_pass_9_fail_12_not_run(self):
        # This is a replay of recorded output, not a new hardware qualification.
        trace = Path(__file__).resolve().parents[2] / "traces/winapps_20261107/wa1f_winapps.log"
        if not trace.exists():
            self.skipTest("trace not copied into the isolated mutation fixture")
        doc = self.build(trace.read_text().splitlines(), failed=1)
        from collections import Counter
        self.assertEqual(Counter(a["result"] for a in doc["apps"]), {"PASS": 58, "FAIL": 9, "NOT_RUN": 12})
        self.assertEqual(doc["subruns"][2]["verdict"], "FAIL")


class FakeTransport:
    def __init__(self, *statuses):
        self.statuses = list(statuses)
        self.calls = []

    def post(self, url, body):
        self.calls.append((url, body))
        status, doc = self.statuses.pop(0) if self.statuses else (200, {"state": "STORED"})
        return status, doc


class BuildPosted(Build):
    """The same facts, read from the document that run() actually POSTS (a second angle on every rule)."""

    def build(self, lines, res):
        with tempfile.TemporaryDirectory() as d:
            log, result_path = Path(d, "log"), Path(d, "result.json")
            log.write_text("\n".join(lines) + "\n")
            result_path.write_text(json.dumps(res))
            transport = FakeTransport()
            import io
            from contextlib import redirect_stdout

            with redirect_stdout(io.StringIO()), unittest.mock.patch.object(rr.time, "sleep", lambda s: None):
                rr.run(["--log", str(log), "--result", str(result_path)], dict(ENV), transport=transport)
            return json.loads(transport.calls[0][1])


class Run(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.log = Path(self.dir.name, "log")
        self.log.write_text("\n".join(log_lines()) + "\n")
        self.result = Path(self.dir.name, "result.json")
        self.result.write_text(json.dumps(result()))
        patcher = unittest.mock.patch.object(rr.time, "sleep", lambda s: None)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self):
        self.dir.cleanup()

    def go(self, transport, env=None, args=()):
        import io
        from contextlib import redirect_stdout

        out = io.StringIO()
        with redirect_stdout(out):
            rc = rr.run(
                ["--log", str(self.log), "--result", str(self.result), *args],
                dict(ENV if env is None else env),
                transport=transport,
            )
        return rc, out.getvalue()

    def test_it_posts_the_report_to_the_fixed_url_and_prints_one_line(self):
        t = FakeTransport()
        rc, out = self.go(t)
        self.assertEqual(rc, 0)
        self.assertEqual([c[0] for c in t.calls], [URL + "/run-report/" + RUN])
        doc = json.loads(t.calls[0][1])
        self.assertEqual(doc["schema"], 1)
        self.assertRegex(out, r"^KF_RUN_REPORT_SENT bytes=[0-9]+ subruns=3\n$")
        self.assertNotIn(TOKEN, out)

    def test_it_sends_nothing_unless_the_worker_asked(self):
        t = FakeTransport()
        env = dict(ENV)
        del env["KF_RUN_REPORT"]
        rc, out = self.go(t, env)
        self.assertEqual((rc, t.calls, out), (0, [], "KF_RUN_REPORT_SKIPPED reason=not_enabled\n"))
        env["KF_RUN_REPORT"] = "2"
        self.assertEqual(self.go(t, env)[1], "KF_RUN_REPORT_SKIPPED reason=not_enabled\n")

    def test_retries_503_and_429_but_not_a_refusal(self):
        t = FakeTransport((503, None), (429, None))
        rc, out = self.go(t)
        self.assertEqual(len(t.calls), 3)
        self.assertIn("KF_RUN_REPORT_SENT", out)
        t = FakeTransport((404, {"state": "REPORTS_DISABLED"}))
        rc, out = self.go(t)
        self.assertEqual((rc, len(t.calls)), (0, 1))
        self.assertEqual(out, "KF_RUN_REPORT_SKIPPED reason=http_404\n")
        t = FakeTransport((None, None), (None, None), (None, None))
        self.assertIn("reason=http_none", self.go(t)[1])

    def test_an_existing_report_counts_as_sent(self):
        exists = (409, {"state": "REPORT_EXISTS"})
        t = FakeTransport(exists, exists, exists)
        self.assertIn("KF_RUN_REPORT_SENT", self.go(t)[1])
        self.assertEqual(len(t.calls), 1)
        t = FakeTransport((409, {"state": "BUSY"}), (200, {"state": "STORED"}))
        self.assertIn("KF_RUN_REPORT_SENT", self.go(t)[1])
        self.assertEqual(len(t.calls), 2)

    def test_it_never_raises_whatever_the_inputs(self):
        self.log.write_bytes(b"\xff\xfe\x00 garbage\n" * 10)
        self.result.write_text("{not json")
        rc, out = self.go(FakeTransport())
        self.assertEqual(rc, 0)
        self.assertRegex(out, r"^KF_RUN_REPORT_SENT")
        rc, out = self.go(FakeTransport(), args=("--log", "/nonexistent/x"))
        self.assertEqual(rc, 0)
        bad = dict(ENV, KF_RUN_TOKEN="not a token")
        self.assertEqual(self.go(FakeTransport(), bad)[1], "KF_RUN_REPORT_SKIPPED reason=environment_contract\n")

        class Boom:
            def post(self, *a):
                raise RuntimeError("boom with " + TOKEN)

        rc, out = self.go(Boom())
        self.assertEqual(rc, 0)
        self.assertEqual(out, "KF_RUN_REPORT_SKIPPED reason=error:RuntimeError\n")

    def test_dry_run_builds_without_the_environment(self):
        rc, out = self.go(None, env={}, args=("--dry-run",))
        self.assertEqual(rc, 0)
        self.assertRegex(out, r"^KF_RUN_REPORT_BUILT bytes=[0-9]+ subruns=3 tests=[0-9]+ perf=[0-9]+\n$")

    def test_the_result_file_is_not_touched(self):
        before = self.result.read_bytes()
        self.go(FakeTransport())
        self.assertEqual(self.result.read_bytes(), before)

    def test_the_token_goes_only_in_the_authorization_header(self):
        t = rr.Transport(TOKEN, access={"KF_ACCESS_ID": "id.access", "KF_ACCESS_SECRET": "s" * 20})
        self.assertEqual(t.access, {"CF-Access-Client-Id": "id.access", "CF-Access-Client-Secret": "s" * 20})
        proxies = [h for h in t.opener.handlers if type(h).__name__ == "ProxyHandler"]
        self.assertEqual(proxies, [], "an explicit empty ProxyHandler: no environment proxy is used")
        self.assertEqual(rr.Transport(TOKEN, access={"KF_ACCESS_ID": "x"}).access, {})
        redirect = [h for h in t.opener.handlers if type(h).__name__ == "_NoRedirect"]
        self.assertTrue(redirect)
        self.assertIsNone(redirect[0].redirect_request(None))


class Edges(unittest.TestCase):
    """Single rules, each from its own angle."""

    def build(self, lines, res):
        return rr.build_report(lines, res)

    def test_a_failed_row_makes_its_subrun_fail_and_a_partial_one_unattributed(self):
        rows = [("provision", 1, 0, 0), ("gates", 9, 0, 0), ("kf3_build", 1, 0, 0), ("raw_suite", 29, 1, 0)]
        doc = self.build(log_lines(), result(verdict="FAIL", rows=rows))
        host = {s["os"]: s for s in doc["subruns"]}["host"]
        self.assertEqual((host["verdict"], host["counts"]["fail"]), ("FAIL", 1))
        partial = [("provision", 1, 0, 0), ("thin_suite", 30, 0, 0)]
        doc = self.build(log_lines(), result(verdict="FAIL", rows=partial))
        linux = {s["os"]: s for s in doc["subruns"]}["linux"]
        self.assertEqual(linux["verdict"], "UNATTRIBUTED")
        self.assertEqual(linux["reason"], "incomplete: birth_census,cuda_fat")

    def test_a_probe_that_failed_or_timed_out_is_a_failed_row(self):
        lines = [
            "GATE_PROBE_RESULT cup8 GATE_PROBE exe=cup8 arg= rc=1 secs=5 timeout=0",
            "GATE_PROBE_RESULT d3d11 GATE_PROBE exe=x arg= rc=0 secs=60 timeout=1",
            "GATE_PROBE_RESULT ok GATE_PROBE exe=x arg= rc=0 secs=2 timeout=0",
        ]
        doc = self.build(lines, result())
        rows = {t["name"]: t["result"] for t in doc["tests"]}
        self.assertEqual(rows, {"cup8": "FAIL", "d3d11": "FAIL", "ok": "PASS"})

    def test_a_repeated_probe_keeps_both_rows(self):
        lines = ["GATE_PROBE_RESULT cup8 GATE_PROBE exe=c rc=0 secs=1 timeout=0"] * 3
        names = [t["name"] for t in self.build(lines, result())["tests"]]
        self.assertEqual(names, ["cup8", "cup8#2", "cup8#3"])

    def test_provider_fault_stages_are_retry_and_nothing_is_guessed(self):
        lines = ["stage name=provision rc=1 duration_ms=5 result=PROVIDER_FAULT", "stage name=x rc=0 duration_ms=1 result=WEIRD"]
        stages = {s["name"]: s["result"] for s in self.build(lines, result())["stages"]}
        self.assertEqual(stages, {"provision": "RETRY", "x": "UNATTRIBUTED"})

    def test_the_caps_hold_for_each_array(self):
        tests = ["FAST_CELL_ARM arm=a%d verdict=PASS secs=1 client_ms=1" % i for i in range(2500)]
        doc = self.build(tests, result())
        self.assertLessEqual(len(doc["tests"]), 2000)
        bench = []
        for n in range(1, 400):
            bench.append("CL_BENCH mode=host rep=1 " + BSUM % (n, "1.0", "2.0", "3.0", "4.0", "5.0"))
        doc = self.build(bench, result())
        self.assertEqual(len(doc["perf"]), 200)
        stages = ["stage name=s%d rc=0 duration_ms=1 result=PASS" % i for i in range(100)]
        self.assertEqual(len(self.build(stages, result())["stages"]), 64)

    def test_the_document_stays_inside_the_bound_even_when_every_array_is_full(self):
        tests = ["FAST_CELL_ARM arm=%s%04d verdict=PASS secs=1 client_ms=1" % ("n" * 100, i) for i in range(2500)]
        doc = self.build(tests, result())
        self.assertLessEqual(len(json.dumps(doc, separators=(",", ":")).encode()), rr.MAX_BYTES)
        self.assertGreater(len(doc["tests"]), 100)

    def test_lines_over_the_limit_are_not_parsed(self):
        lines = ["FAST_CELL_ARM arm=%s verdict=PASS secs=1 client_ms=1" % ("x" * 9000), "FAST_CELL_ARM arm=ok verdict=PASS secs=1 client_ms=1"]
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write("\n".join(lines))
        try:
            parsed = rr.lines_of(f.name)
        finally:
            os.unlink(f.name)
        self.assertEqual(len(parsed), 1)
        self.assertTrue(all(len(x) <= rr.LINE_MAX for x in parsed))
        self.assertEqual([t["name"] for t in self.build(parsed, result())["tests"]], ["ok"])

    def test_a_weird_host_driver_is_dropped(self):
        res = result()
        res["hardware"]["driver_version"] = "580; rm -rf /"
        doc = self.build([], res)
        self.assertTrue(all(s["host_driver"] is None for s in doc["subruns"]))

    def test_a_windows_retry_is_a_retry(self):
        lines = ["KF_PAYLOAD_TIER windows=1 source=worker tier=middle windows_runs=1 stages=none",
                 "WINDOWS_GATE_RETRY action=rerun-on-a-fresh-box class=PROVIDER stage=boot1 why=x rev=1"]
        rows = [("provision", 1, 0, 0), ("windows_retry_provider", 0, 1, 0)]
        doc = self.build(lines, result(verdict="FAIL", rows=rows))
        self.assertEqual({s["os"]: s["verdict"] for s in doc["subruns"]}["windows"], "RETRY")

    def test_the_idle_reason_names_the_failing_suite_or_the_profile(self):
        rows = [("provision", 1, 0, 0), ("gates", 0, 1, 0)]
        doc = self.build(["KF_PAYLOAD_TIER windows=0 source=worker tier=middle windows_runs=1 stages=none"], result(verdict="FAIL", rows=rows))
        reasons = {s["os"]: s["reason"] for s in doc["subruns"]}
        self.assertEqual(reasons["linux"], "not reached: gates")
        self.assertEqual(reasons["windows"], "Windows stage not requested for this profile")
        self.assertEqual(reasons["host"], None)


class EdgesPosted(Edges):
    """The same single rules, read from the document that run() POSTS."""

    build = BuildPosted.build



class Transport2(unittest.TestCase):
    def setUp(self):
        patcher = unittest.mock.patch.object(rr.time, "sleep", lambda s: None)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.log = Path(self.dir.name, "log")
        self.log.write_text("\n".join(log_lines()) + "\n")
        self.result = Path(self.dir.name, "result.json")
        self.result.write_text(json.dumps(result()))

    def go(self, transport, env=None):
        import io
        from contextlib import redirect_stdout

        out = io.StringIO()
        with redirect_stdout(out):
            rc = rr.run(["--log", str(self.log), "--result", str(self.result)], dict(ENV if env is None else env), transport=transport)
        return rc, out.getvalue()

    def test_a_value_other_than_one_never_sends(self):
        for value in ("0", "2", "true", ""):
            t = FakeTransport()
            self.go(t, dict(ENV, KF_RUN_REPORT=value))
            self.assertEqual(t.calls, [], value)

    def test_a_refusal_is_final_a_server_error_is_retried(self):
        for status in (400, 401, 404, 413):
            t = FakeTransport((status, {"state": "X"}))
            self.go(t)
            self.assertEqual(len(t.calls), 1, status)
        for status in (500, 502, 503, 504, 429):
            t = FakeTransport((status, None), (200, {"state": "STORED"}))
            self.go(t)
            self.assertEqual(len(t.calls), 2, status)

    def test_report_exists_stops_at_once(self):
        t = FakeTransport((409, {"state": "REPORT_EXISTS"}), (409, {"state": "REPORT_EXISTS"}), (409, {"state": "REPORT_EXISTS"}))
        rc, out = self.go(t)
        self.assertEqual(len(t.calls), 1)
        self.assertIn("KF_RUN_REPORT_SENT", out)
        t = FakeTransport((409, {"state": "BUSY"}), (409, {"state": "BUSY"}), (409, {"state": "BUSY"}))
        rc, out = self.go(t)
        self.assertEqual(len(t.calls), 3)
        self.assertIn("reason=http_409", out)

    def test_the_url_is_the_run_report_route_of_this_rental(self):
        t = FakeTransport()
        self.go(t)
        self.assertEqual(t.calls[0][0], URL + "/run-report/" + RUN)
        self.assertTrue(t.calls[0][0].startswith("https://"))

    def test_a_builder_error_is_a_skipped_report_not_an_exception(self):
        for error in (ValueError("x"), KeyError("k"), ZeroDivisionError(), RuntimeError(TOKEN)):
            with unittest.mock.patch.object(rr, "build_report", side_effect=error):
                rc, out = self.go(FakeTransport())
            self.assertEqual(rc, 0)
            self.assertEqual(out, "KF_RUN_REPORT_SKIPPED reason=build_error:%s\n" % type(error).__name__)
            self.assertNotIn(TOKEN, out)

    def test_a_builder_error_inside_the_parsers_is_caught_too(self):
        with unittest.mock.patch.object(rr, "suite_rows", side_effect=OSError("disk")):
            rc, out = self.go(FakeTransport())
        self.assertEqual((rc, out), (0, "KF_RUN_REPORT_SKIPPED reason=build_error:OSError\n"))

    def test_a_long_line_in_the_log_file_never_becomes_a_row(self):
        self.log.write_text("FAST_CELL_ARM arm=%s verdict=PASS secs=1 client_ms=1\nFAST_CELL_ARM arm=ok verdict=PASS secs=1 client_ms=1\n" % ("x" * 9000))
        t = FakeTransport()
        self.go(t)
        doc = json.loads(t.calls[0][1])
        self.assertEqual([x["name"] for x in doc["tests"]], ["ok"])

    def test_a_proxy_in_the_environment_receives_nothing(self):
        import http.server
        import threading

        seen = {"proxy": 0, "target": 0}

        def server(key):
            class H(http.server.BaseHTTPRequestHandler):
                def log_message(self, *a):
                    pass

                def do_POST(self):
                    seen[key] += 1
                    self.rfile.read(int(self.headers.get("Content-Length") or 0))
                    self.send_response(200)
                    self.send_header("Content-Length", "2")
                    self.end_headers()
                    self.wfile.write(b"{}")

                do_GET = do_POST

            s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
            threading.Thread(target=s.serve_forever, daemon=True).start()
            return s

        proxy, target = server("proxy"), server("target")
        try:
            with unittest.mock.patch.dict(os.environ, {"http_proxy": "http://127.0.0.1:%d" % proxy.server_address[1]}):
                t = rr.Transport(TOKEN)
                t.post("http://127.0.0.1:%d/x" % target.server_address[1], b"{}")
        finally:
            for s in (proxy, target):
                s.shutdown()
                s.server_close()
        self.assertEqual(seen, {"proxy": 0, "target": 1})

    def test_no_environment_proxy_is_used(self):
        with unittest.mock.patch.dict(os.environ, {"https_proxy": "http://127.0.0.1:9", "HTTPS_PROXY": "http://127.0.0.1:9"}):
            t = rr.Transport(TOKEN)
        proxies = [h for h in t.opener.handlers if type(h).__name__ == "ProxyHandler"]
        self.assertEqual(proxies, [])

    def test_a_redirect_is_never_followed(self):
        import http.server
        import threading

        hits = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                hits.append(("POST", self.path, self.headers.get("Authorization")))
                self.rfile.read(int(self.headers.get("Content-Length") or 0))
                if self.path == "/first":
                    self.send_response(302)
                    self.send_header("Location", "/stolen")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.send_header("Content-Length", "2")
                    self.end_headers()
                    self.wfile.write(b"{}")

            do_GET = do_POST

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            t = rr.Transport(TOKEN)
            status, doc = t.post("http://127.0.0.1:%d/first" % server.server_address[1], b"{}")
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual(status, 302)
        self.assertEqual([h[1] for h in hits], ["/first"])

    def test_the_token_is_in_no_outcome_line(self):
        for transport in (FakeTransport(), FakeTransport((404, None)), FakeTransport((503, None), (503, None), (503, None))):
            rc, out = self.go(transport)
            self.assertNotIn(TOKEN, out)
            self.assertNotIn("Bearer", out)


class SpecStatusRows(unittest.TestCase):
    """The SPECviewperf stage (disabled by default) reports status only: a test row per viewset and per failed
    subtest under the neutral suite `spec_viewsets`, no time, no number, in the Windows subrun."""

    ROWS = [
        "SPEC_ROW name=energy-04 result=PASS subtests=6/6 reason=- failed=-",
        "SPEC_ROW name=creo-04 result=FAIL subtests=4/6 reason=SUBTEST failed=03_ThreeLoftJetsDarkMirror,06_LoftJets",
        "SPEC_ROW name=enscape-01 result=NOT_RUN subtests=0/0 reason=TIME failed=-",
    ]
    BASE = [("provision", 1, 0, 0), ("gates", 9, 0, 0), ("kf3_build", 1, 0, 0), ("raw_suite", 30, 0, 0),
            ("thin_suite", 30, 0, 0), ("birth_census", 1, 0, 0), ("cuda_host", 4, 0, 0), ("cuda_fat", 4, 0, 0),
            ("windows_gate", 1, 0, 0)]

    def build(self, lines, rows=None, verdict="PASS"):
        return rr.build_report(log_lines() + lines, result(rows=(rows if rows is not None else self.BASE), verdict=verdict))

    def spec_tests(self, doc):
        return {t["name"]: t for t in doc["tests"] if t["suite"] == "spec_viewsets"}

    def windows(self, doc):
        return {s["os"]: s for s in doc["subruns"]}["windows"]

    def test_windows_apps_and_spec_status_share_a_subrun_without_sharing_timing(self):
        app = WindowsApps().line()
        doc = self.build([app, self.ROWS[0]], rows=self.BASE + [("windows_apps", 1, 0, 0), ("specviewperf", 1, 0, 0)])
        windows_app = next(t for t in doc["tests"] if t["suite"] == "apps_windows")
        spec = self.spec_tests(doc)["energy-04"]
        self.assertEqual(windows_app["sub"], spec["sub"])
        self.assertEqual(windows_app["ms"], 2000)
        self.assertIsNone(spec["ms"])
        self.assertEqual(doc["apps"][0]["os"], "windows")

    def test_spec_retry_cannot_hide_a_windows_apps_guest_failure(self):
        doc = self.build([WindowsApps().line(cls="guest_fail", verdict="FAIL")],
                         rows=self.BASE + [("windows_apps", 0, 1, 0), ("specviewperf_retry_time", 0, 1, 0)],
                         verdict="FAIL")
        self.assertEqual(self.windows(doc)["verdict"], "FAIL")

    def test_spec_rows_become_status_tests_of_the_windows_subrun(self):
        doc = self.build(self.ROWS)
        got = self.spec_tests(doc)
        self.assertEqual(
            {k: v["result"] for k, v in got.items()},
            {"energy-04": "PASS", "creo-04": "FAIL", "enscape-01": "SKIP",
             "creo-04/03_ThreeLoftJetsDarkMirror": "FAIL", "creo-04/06_LoftJets": "FAIL"},
        )
        self.assertEqual({v["sub"] for v in got.values()}, {"windows-580.88"})

    def test_a_viewset_that_did_not_run_is_a_skip_never_a_pass(self):
        got = self.spec_tests(self.build(self.ROWS))
        self.assertEqual(got["enscape-01"]["result"], "SKIP")
        self.assertNotEqual(got["enscape-01"]["result"], "PASS")

    def test_the_rows_carry_no_time_and_no_number(self):
        doc = self.build(self.ROWS)
        for t in self.spec_tests(doc).values():
            self.assertIsNone(t["ms"])
            self.assertEqual(set(t), {"sub", "suite", "name", "result", "ms"})

    def test_a_viewset_row_has_no_duration_so_no_timing_of_a_benchmark_is_published(self):
        got = self.spec_tests(self.build(self.ROWS))
        viewsets = {k: v for k, v in got.items() if "/" not in k}
        self.assertEqual(sorted(viewsets), ["creo-04", "energy-04", "enscape-01"])
        self.assertEqual({v["ms"] for v in viewsets.values()}, {None})

    def test_the_failed_subtests_are_named_as_failing_rows(self):
        got = self.spec_tests(self.build(self.ROWS))
        self.assertEqual(got["creo-04/06_LoftJets"]["result"], "FAIL")
        self.assertNotIn("energy-04/01_A", got)

    def test_no_spec_rows_means_no_spec_tests(self):
        self.assertEqual(self.spec_tests(self.build([])), {})

    def test_a_row_with_extra_fields_is_not_a_row(self):
        doc = self.build(["SPEC_ROW name=energy-04 result=PASS subtests=6/6 reason=- failed=- score=99.5"])
        self.assertEqual(self.spec_tests(doc), {})
        self.assertNotIn("99.5", json.dumps(doc))

    def test_hostile_or_foreign_rows_are_ignored(self):
        lines = [
            "SPEC_ROW name=Bad_Name-01 result=PASS subtests=1/1 reason=- failed=-",
            "SPEC_ROW name=energy-04 result=MAYBE subtests=1/1 reason=- failed=-",
            "SPEC_ROW name=energy-04 result=PASS subtests=x/1 reason=- failed=-",
            "SPEC_ROW name=energy-4 result=PASS subtests=1/1 reason=- failed=-",
            "SPEC_ROW name=energy-04 result=PASS subtests=1/1 reason=- failed=a b",
            "SPEC_ROW name=energy-04 result=PASS subtests=1/1 reason=lower failed=-",
        ]
        self.assertEqual(self.spec_tests(self.build(lines)), {})

    def test_a_row_of_the_stage_is_never_a_number_a_benchmark_measured(self):
        text = json.dumps(self.build(self.ROWS))
        self.assertNotRegex(text, r"(?i)viewperf|score|fps|composite")

    def test_the_suite_rows_of_the_result_file_are_renamed_without_the_benchmark_name(self):
        rows = self.BASE + [("specviewperf", 4, 0, 0), ("specviewperf_lanea", 4, 0, 0)]
        doc = self.build(self.ROWS[:1], rows)
        names = [r["name"] for r in doc["suites"]]
        self.assertIn("spec_viewsets", names)
        self.assertIn("spec_viewsets_lanea", names)
        self.assertNotIn("specviewperf", names)
        self.assertNotRegex(json.dumps(doc), r"(?i)viewperf")

    def test_the_spec_suite_rows_belong_to_the_windows_subrun(self):
        rows = self.BASE + [("specviewperf", 4, 0, 0)]
        doc = self.build(self.ROWS[:1], rows)
        self.assertIn("spec_viewsets", self.windows(doc)["stages"])
        self.assertEqual(self.windows(doc)["verdict"], "PASS")
        self.assertEqual(self.windows(doc)["counts"]["pass"], 5)

    def test_a_failed_spec_stage_fails_the_windows_subrun(self):
        rows = self.BASE + [("specviewperf", 0, 1, 0)]
        doc = self.build(self.ROWS, rows, verdict="FAIL")
        self.assertEqual(self.windows(doc)["verdict"], "FAIL")
        self.assertEqual(self.windows(doc)["counts"]["fail"], 1)

    def test_a_spec_lane_that_could_not_finish_is_a_retry_not_a_guest_failure(self):
        for cls in ("time", "media", "transport"):
            with self.subTest(cls):
                rows = self.BASE + [("specviewperf_retry_" + cls, 0, 1, 0)]
                doc = self.build(self.ROWS, rows, verdict="FAIL")
                self.assertEqual(self.windows(doc)["verdict"], "RETRY")

    def test_a_lone_time_retry_row_is_a_retry_and_no_failure_is_counted_as_a_viewset(self):
        rows = self.BASE + [("specviewperf_retry_time", 0, 1, 0), ("specviewperf_passed", 2, 0, 0)]
        doc = self.build(self.ROWS[:1], rows, verdict="FAIL")
        self.assertEqual(self.windows(doc)["verdict"], "RETRY")

    def test_a_retry_next_to_a_failed_viewset_row_stays_a_failure(self):
        rows = self.BASE + [("specviewperf", 0, 1, 0), ("specviewperf_retry_time", 0, 1, 0)]
        doc = self.build(self.ROWS, rows, verdict="FAIL")
        self.assertEqual(self.windows(doc)["verdict"], "FAIL")

    def test_a_retry_next_to_a_failed_windows_gate_stays_a_failure(self):
        rows = self.BASE[:-1] + [("windows_gate", 0, 1, 0), ("specviewperf_retry_time", 0, 1, 0)]
        doc = self.build(self.ROWS, rows, verdict="FAIL")
        self.assertEqual(self.windows(doc)["verdict"], "FAIL")


if __name__ == "__main__":
    unittest.main()
