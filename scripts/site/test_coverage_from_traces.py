"""Tests for scripts/site/coverage_from_traces.py (STATUS: LIVE, 2026-11-05).

FALSIFIER: the committed site/status/coverage.json differs from what the traces give; a number on
it is not backed by a README sentence or a parsed log line; an unreadable log becomes a PASS; an
imported row can be read as a gate pass; the file carries a URL, a GPU UUID or an address; the
placeholder trio set puts data on the page.
"""

import io
import json
from contextlib import redirect_stderr
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import coverage_from_traces as cov  # noqa: E402

TRACES = cov.TRACES


class Text(str):
    """README text with line wraps collapsed. assertIn on it fails without printing the file."""

    def __contains__(self, item):
        return " ".join(item.split()) in str(self)


def readme(rel):
    return Text(" ".join((TRACES / rel).read_text().split()))


def cell(data, die, platform):
    for family in data["families"]:
        for entry in family["dies"]:
            if entry["id"] == die:
                return entry["cells"][platform]
    raise KeyError(die)


class Parsers(unittest.TestCase):
    PASS = ('WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 4090" idle_s=300 secs=1 '
            "resets=0 enable_utc=2026-11-04T00:55:24Z")
    FAIL = ('WINDOWS_GATE_FAIL diag=KAYFABE stage=idle why="x" rev=0c515b3b gpu="NVIDIA GeForce RTX 5090" '
            "secs=452")

    def test_windows_pass_fail_retry(self):
        p = cov.parse_windows("noise\n" + self.PASS + "\n")
        self.assertEqual((p["verdict"], p["rev"], p["date"], p["resets"]),
                         ("PASS", "0c515b3b", "2026-11-04", 0))
        f = cov.parse_windows(self.FAIL)
        self.assertEqual((f["verdict"], f["stage"]), ("FAIL", "idle"))
        r = cov.parse_windows(self.PASS.replace("PASS", "RETRY"))
        self.assertEqual(r["verdict"], "RETRY")

    def test_windows_repeats_are_one_line_but_conflicts_and_gaps_are_unknown(self):
        self.assertEqual(cov.parse_windows(self.PASS + "\n" + self.PASS)["verdict"], "PASS")
        self.assertEqual(cov.parse_windows(self.PASS + "\n" + self.FAIL)["verdict"], "UNKNOWN")
        self.assertEqual(cov.parse_windows("nothing here")["verdict"], "UNKNOWN")
        self.assertEqual(cov.parse_windows("WINDOWS_GATE_PASS gpu=none")["verdict"], "UNKNOWN")
        # A name that merely contains the verdict word is not a verdict line.
        self.assertEqual(cov.parse_windows("MY_WINDOWS_GATE_PASS rev=0c515b3b")["verdict"], "UNKNOWN")

    LINUX = """HEAD=0c515b3b322157dfa0dbe8dd611459839c3534b6 CHECKOUT=/x
V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS
KF3_RC=0
BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30
FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30
BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12 all
"""

    def test_linux_bar(self):
        ok = cov.parse_linux_bar(self.LINUX)
        self.assertEqual((ok["verdict"], ok["rev"]), ("PASS", "0c515b3b"))
        self.assertIn("thin guest 30/30", ok["detail"])

    def test_linux_bar_needs_every_line_and_every_arm(self):
        for broken, want in [
            (self.LINUX.replace("FAST_SUITE_PASS=30", "FAST_SUITE_PASS=29"), "UNKNOWN"),
            (self.LINUX.replace("FAST_SUITE_FAIL=0", "FAST_SUITE_FAIL=1").replace(
                "FAST_SUITE_PASS=30", "FAST_SUITE_PASS=29"), "FAIL"),
            (self.LINUX.replace("pass=9 fail=0", "pass=8 fail=1"), "FAIL"),
            (self.LINUX.replace("pass=9 fail=0", "pass=9 fail=1"), "FAIL"),
            (self.LINUX.replace("BARE_SUITE_PASS=30", "BARE_SUITE_PASS=29"), "UNKNOWN"),
            (self.LINUX.replace("BIRTH_CENSUS_OK", "BIRTH_CENSUS_BAD"), "UNKNOWN"),
            (self.LINUX.replace("KF3_RC=0", "KF3_RC=1"), "UNKNOWN"),
            (self.LINUX.replace("KF3_RC=0\n", ""), "UNKNOWN"),
            (self.LINUX.split("\n", 1)[1], "UNKNOWN"),
            ("", "UNKNOWN"),
        ]:
            self.assertEqual(cov.parse_linux_bar(broken)["verdict"], want, broken[:80])

    WORKER = """[untrusted] [box] WINDOWS_GATE_PASS rev=d74f6063 gpu="NVIDIA GeForce RTX 3090" secs=1 enable_utc=2026-11-05T01:09:23Z
[untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=5
[untrusted] [box] stage name=provision rc=0 duration_ms=1 result=PASS
[untrusted] [box] stage name=cuda_host rc=0 duration_ms=1 result=PASS
[untrusted] [box] stage name=merge_bar rc=6 duration_ms=1 result=PASS
[untrusted] [box] stage name=cuda_guest rc=0 duration_ms=1 result=PASS
[untrusted] [box] stage name=windows_gate rc=0 duration_ms=1 result=PASS
[untrusted] [box] suite name=gates pass=9 fail=0 skip=0
[untrusted] [box] suite name=raw_suite pass=30 fail=0 skip=0
[untrusted] [box] suite name=thin_suite pass=30 fail=0 skip=0
"""

    def test_worker_log(self):
        self.assertEqual(cov.parse_worker_log(self.WORKER, "windows")["verdict"], "PASS")
        self.assertEqual(cov.parse_worker_log(self.WORKER, "linux")["verdict"], "PASS")
        bad = self.WORKER.replace("verdict=PASS stages", "verdict=FAIL stages")
        self.assertEqual(cov.parse_worker_log(bad, "windows")["verdict"], "UNKNOWN")
        self.assertEqual(cov.parse_worker_log(bad, "linux")["verdict"], "UNKNOWN")
        bad = self.WORKER.replace("name=windows_gate rc=0 duration_ms=1 result=PASS",
                                  "name=windows_gate rc=1 duration_ms=1 result=FAIL")
        self.assertEqual(cov.parse_worker_log(bad, "windows")["verdict"], "UNKNOWN")
        bad = self.WORKER.replace("name=thin_suite pass=30 fail=0", "name=thin_suite pass=29 fail=1")
        self.assertEqual(cov.parse_worker_log(bad, "linux")["verdict"], "FAIL")

    def test_worker_run_needs_the_runner_verdict(self):
        spec = next(s for s in cov.RUNS if s[0] == cov.WIN and s[2] == "windows")
        self.assertEqual(cov.run_record(spec)["verdict"], "PASS")
        for facts in [{"verdict": "FAIL", "verified": "1", "state": "GRACE"},
                      {"verdict": "PASS", "verified": "0", "state": "GRACE"},
                      {"verdict": None, "verified": None, "state": None}]:
            with mock.patch.object(cov, "worker_facts", return_value=facts):
                self.assertEqual(cov.run_record(spec)["verdict"], "UNKNOWN", facts)

    def test_groups_keep_revisions_and_sources_apart(self):
        recs = [
            dict(die="X", platform="windows", tag="a", set=cov.DC, source=cov.IMPORTED, verdict="PASS",
                 rev="0c515b3b", rev_full=None, date="2026-11-04", detail=None, stage=None, gpu=None,
                 note=None, file="f"),
            dict(die="X", platform="windows", tag="b", set=cov.DC, source=cov.IMPORTED, verdict="FAIL",
                 rev="0c515b3b", rev_full=None, date="2026-11-04", detail=None, stage="idle", gpu=None,
                 note=None, file="f"),
            dict(die="X", platform="windows", tag="c", set=cov.DC, source=cov.IMPORTED, verdict="UNKNOWN",
                 rev="aaaaaaaa", rev_full=None, date="2026-11-04", detail=None, stage=None, gpu=None,
                 note=None, file="f"),
        ]
        mixed = dict(recs[0], tag="w", set=cov.WIN, source=cov.WORKER)
        split = cov.groups_for([recs[0], mixed])
        self.assertEqual([(g["source"], g["n"]) for g in split].count((cov.IMPORTED, 1)), 1)
        self.assertEqual(len(split), 2)
        groups = cov.groups_for(recs)
        self.assertEqual([(g["revision"], g["n"], g["pass"], g["fail"], g["unknown"]) for g in groups],
                         [("0c515b3b", 2, 1, 1, 0), ("aaaaaaaa", 1, 0, 0, 1)])


class Committed(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = cov.build()
        cls.text = cov.serialize(cls.data)

    def test_committed_file_is_fresh(self):
        self.assertEqual(cov.OUTPUT.read_text(), self.text,
                         "run: python3 scripts/site/coverage_from_traces.py --write")

    def test_no_run_is_unclear_and_no_log_failed_to_parse(self):
        for family in self.data["families"]:
            for die in family["dies"]:
                for platform, c in die["cells"].items():
                    for g in c["groups"]:
                        self.assertEqual(g["unknown"], 0, (die["id"], platform, g["tags"]))
                        self.assertEqual(g["n"], g["pass"] + g["fail"] + g["retry"])

    # (die, platform, runs, pass, fail, README, anchors that state the count)
    EXPECT = [
        ("TU106", "windows", 1, 1, 0, f"{cov.DC}/turing/README.md", ["**PASS** (`tu106a1`)"]),
        ("TU116", "windows", 2, 2, 0, f"{cov.DC}/turing/README.md", ["**PASS x2** (`tu116b1`, `tu116b2`)"]),
        ("GA104", "windows", 1, 1, 0, f"{cov.DC}/ampere/README.md", ["H-windows-GA104 not falsified (n=1)"]),
        ("GA106", "windows", 2, 1, 1, f"{cov.DC}/ampere/README.md",
         ["WINDOWS_GATE_FAIL diag=KAYFABE stage=reboot", "H-windows-GA106 not falsified (n=1, dca106b)"]),
        ("AD102", "windows", 2, 2, 0, f"{cov.DC}/ada/README.md", ["**PASS x2** (ada102a, ada102b)"]),
        ("AD104", "windows", 1, 1, 0, f"{cov.DC}/ada/README.md", ["AD104 Windows gate (H-win not falsified, n=1"]),
        ("GB202", "windows", 3, 2, 1, f"{cov.DC}/blackwell/README.md",
         ["**r1 FAIL**, r2 PASS, r3 PASS", "1 FAIL in 3 runs"]),
        ("GB205", "windows", 2, 2, 0, f"{cov.DC}/blackwell/README.md",
         ["GB205 r1 (`gb205/windows_dcgb205/`): `WINDOWS_GATE_PASS", "GB205 r2 (`gb205/windows_dcgb205r2/`): `WINDOWS_GATE_PASS"]),
        ("GB206", "windows", 2, 2, 0, f"{cov.DC}/blackwell/README.md",
         ["GB206 r1 (`gb206/windows_dcgb206/`): `WINDOWS_GATE_PASS", "GB206 r2 (`gb206/windows_dcgb206r2/`): `WINDOWS_GATE_PASS"]),
        ("GA102", "windows", 1, 1, 0, f"{cov.WIN}/README.md", ["WINDOWS_GATE_PASS", "`verdict=PASS`"]),
    ]

    def test_counts_match_the_readme_that_states_them(self):
        for die, platform, n, passed, failed, rel, anchors in self.EXPECT:
            text = readme(rel)
            for a in anchors:
                self.assertTrue(a in text, (die, a))
            head = [g for g in cell(self.data, die, platform)["groups"] if g["headline"]][0]
            self.assertEqual((head["n"], head["pass"], head["fail"]), (n, passed, failed), die)

    def test_linux_cells_state_the_bar_the_readmes_state(self):
        for die in ["TU106", "TU116", "GA104", "GA106", "AD102", "AD103", "AD104", "AD106",
                    "GB202", "GB205", "GB206"]:
            head = [g for g in cell(self.data, die, "linux")["groups"] if g["headline"]][0]
            self.assertEqual((head["n"], head["pass"], head["revision"]), (1, 1, "0c515b3b"), die)
            self.assertIn("v3 gates 9/9", head["detail"])
            self.assertIn("thin guest 30/30", head["detail"])
        # the README tables say 30/30 for these dies
        for rel, die in [("ada/README.md", "AD103"), ("ada/README.md", "AD106"),
                         ("blackwell/README.md", "GB205"), ("turing/README.md", "TU116")]:
            self.assertTrue("30/30" in readme(f"{cov.DC}/{rel}"), die)

    def test_headline_revision_date_and_set(self):
        for fam in self.data["families"]:
            for die in fam["dies"]:
                for platform, c in die["cells"].items():
                    for g in c["groups"]:
                        self.assertRegex(g["revision"], r"^[0-9a-f]{8}$")
                        self.assertRegex(g["date"], r"^2026-11-0[45]$")
                        self.assertTrue((cov.ROOT / g["evidence"]).is_dir(), g["evidence"])
                        if g["revision_full"]:
                            self.assertTrue(g["revision_full"].startswith(g["revision"]))
        ga102 = cell(self.data, "GA102", "windows")["groups"][0]
        self.assertEqual((ga102["revision"], ga102["date"], ga102["source"], ga102["run"]["path"]),
                         ("d74f6063", "2026-11-05", cov.WORKER, "actions/runs/41936606326"))
        for rel, key in [(f"{cov.DC}/ada/README.md", "0c515b3b3"), (f"{cov.WIN}/README.md", "d74f6063"),
                         (f"{cov.WIN}/README.md", "run 41936606326"),
                         (f"{cov.WIN}/README.md", "2026-11-05"), (f"{cov.DC}/ada/README.md", "2026-11-04")]:
            self.assertTrue(key in readme(rel), (rel, key))

    def test_run_dates_agree_with_the_log_when_it_has_one(self):
        for spec in cov.RUNS:
            rec = cov.run_record(spec)
            self.assertEqual(rec["date"], cov.SETS[spec[0]]["date"], spec[1])

    def test_worker_source_is_only_the_worker_set(self):
        for fam in self.data["families"]:
            for die in fam["dies"]:
                for c in die["cells"].values():
                    for g in c["groups"]:
                        self.assertEqual(g["source"] == cov.WORKER, g["evidence"].endswith(cov.WIN), die["id"])
                        if g["source"] == cov.IMPORTED:
                            self.assertNotIn("run", g)
        self.assertEqual(set(self.data["sources"]), {cov.WORKER, cov.IMPORTED})
        self.assertIn("Only runs reproduced by the CI Worker count", self.data["gate_rule"])

    def test_boards_are_named_in_the_readme(self):
        for fid, _name, dies in cov.FAMILIES:
            for die, sku in dies:
                if sku:
                    rel = cov.DIE_README.get(die, cov.FAMILY_README[fid])
                    self.assertTrue(sku in readme(rel), die)

    def test_uncovered_dies_carry_the_readmes_reason(self):
        fam_of = {die: fid for fid, _n, dies in cov.FAMILIES for die, _s in dies}
        for die, (reason, anchor) in {**cov.UNCOVERED, **cov.LINUX_ONLY_DIES}.items():
            self.assertTrue(anchor in readme(cov.FAMILY_README[fam_of[die]]), die)
            self.assertTrue(reason)
        for die in cov.UNCOVERED:
            for platform in ("linux", "windows"):
                c = cell(self.data, die, platform)
                self.assertEqual((c["state"], c["groups"]), ("not_covered", []), die)
                self.assertTrue(c["reason"])
        self.assertEqual(cell(self.data, "AD103", "windows")["state"], "not_covered")
        self.assertEqual(cell(self.data, "GB203", "windows")["see"], "traces/blackwell_win_20261103")
        self.assertTrue((cov.ROOT / "traces/blackwell_win_20261103").is_dir())

    def test_no_url_address_uuid_or_score(self):
        self.assertNotRegex(self.text, r"https?://")
        self.assertNotRegex(self.text, r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        self.assertNotRegex(self.text, r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b")
        self.assertNotRegex(self.text, r"(?i)specviewperf|serial|\bmac\b|api\.kayfabe|instance_api_key")
        self.assertNotIn("trio", self.text.lower())

    def test_inputs_hash_the_readmes_and_logs(self):
        for rel, digest in self.data["inputs"].items():
            self.assertEqual(cov.sha256(rel), digest)
        for spec in cov.RUNS:
            self.assertIn(spec[3], self.data["inputs"])

    def test_trio_is_a_placeholder_without_data(self):
        self.assertEqual(cov.PENDING_SETS, ["trio_vast_20261105"])
        self.assertNotIn("trio_vast_20261105", cov.SETS)
        with tempfile.TemporaryDirectory() as folder:
            fake = Path(folder)
            (fake / "trio_vast_20261105").mkdir()
            with mock.patch.object(cov, "TRACES", fake):
                self.assertEqual(len(cov.warnings()), 1)
                self.assertIn("NOT imported", cov.warnings()[0])
        self.assertEqual(cov.warnings() == [], not (TRACES / "trio_vast_20261105").exists())


class Cli(unittest.TestCase):
    def test_check_passes_on_the_committed_file(self):
        self.assertEqual(cov.main(["--check"]), 0)

    def test_check_fails_on_a_stale_file(self):
        with tempfile.TemporaryDirectory() as folder:
            stale = Path(folder) / "coverage.json"
            stale.write_text(json.dumps({"schema": 0}))
            with mock.patch.object(cov, "OUTPUT", stale), redirect_stderr(io.StringIO()):
                self.assertEqual(cov.main(["--check"]), 1)
            with mock.patch.object(cov, "OUTPUT", Path(folder) / "missing.json"), redirect_stderr(io.StringIO()):
                self.assertEqual(cov.main(["--check"]), 1)


if __name__ == "__main__":
    unittest.main()
