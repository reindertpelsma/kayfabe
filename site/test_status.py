"""Status section modes (docs/design/V3_PLATFORM_TESTING.md section 9).

FALSIFIER: a local, fork or PR build carries a Worker URL; an invalid origin is accepted; the
fixture can reach a deployment; the workflow passes the variable to a non-candidate build.
"""

import io
import json
import os
import subprocess
import textwrap
from contextlib import redirect_stdout
from pathlib import Path
import re
import tempfile
import unittest

import status_pages
from test_build import builder

SITE = Path(__file__).resolve().parent
FIXTURE = SITE / "status/fixture.json"
WORKER_URL = re.compile(r"api\.kayfabe\.dev|logs\.kayfabe\.dev|kf-status-api|kf-status-logs")


class Origins(unittest.TestCase):
    def test_valid_origins(self):
        for value, expected in [
            ("https://api.kayfabe.dev", "https://api.kayfabe.dev"),
            ("https://api.kayfabe.dev/", "https://api.kayfabe.dev"),
            ("https://kayfabe.dev", "https://kayfabe.dev"),
            ("https://a-b.c.kayfabe.dev", "https://a-b.c.kayfabe.dev"),
        ]:
            self.assertEqual(status_pages.validate_origin(value), expected, value)

    def test_invalid_origins(self):
        for value in [
            None,
            "",
            "http://api.kayfabe.dev",
            "https://api.kayfabe.dev/v1",
            "https://api.kayfabe.dev?x=1",
            "https://api.kayfabe.dev#x",
            "https://user:pw@api.kayfabe.dev",
            "https://user@api.kayfabe.dev",
            "https://api.kayfabe.dev:443",
            "https://api.kayfabe.dev.evil.example",
            "https://evilkayfabe.dev",
            "https://API.kayfabe.dev",
            "https://-x.kayfabe.dev",
            "https://api.kayfabe.dev\n",
            " https://api.kayfabe.dev",
            "javascript:alert(1)//kayfabe.dev",
        ]:
            self.assertIsNone(status_pages.validate_origin(value), repr(value))


class Modes(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="status-test-")
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name)

    def files(self):
        return sorted(p.relative_to(self.out).as_posix() for p in self.out.rglob("*") if p.is_file())

    def test_unconfigured_build_has_one_page_and_no_worker_url(self):
        self.assertEqual(status_pages.write_status(self.out), "off")
        self.assertEqual(self.files(), ["status/index.html"])
        text = (self.out / "status/index.html").read_text()
        self.assertIn("not configured", text)
        self.assertNotRegex(text, WORKER_URL)
        self.assertNotIn("status.js", text)
        self.assertIn('id="kf-footer"', text)

    def test_invalid_base_is_a_warning_and_the_unconfigured_page(self):
        log = io.StringIO()
        with redirect_stdout(log):
            mode = status_pages.write_status(self.out, api_base="https://evil.example")
        self.assertEqual(mode, "off")
        self.assertIn("::warning::", log.getvalue())
        self.assertNotIn("evil.example", (self.out / "status/index.html").read_text())

    def test_live_pages_carry_the_base_only_in_meta(self):
        mode = status_pages.write_status(
            self.out, api_base="https://api.kayfabe.dev", logs_base="https://logs.kayfabe.dev"
        )
        self.assertEqual(mode, "live")
        for page in ["status/index.html", "status/die/index.html", "status/run/index.html",
                     "status/commit/index.html", "status/find/index.html"]:
            text = (self.out / page).read_text()
            self.assertEqual(text.count("https://api.kayfabe.dev"), 1, page)
            self.assertIn('<meta name="kf-status-api" content="https://api.kayfabe.dev" />', text)
            self.assertIn('<meta name="kf-status-logs" content="https://logs.kayfabe.dev" />', text)
            self.assertNotIn("Synthetic", text)
        self.assertFalse((self.out / "status/fixture.json").exists())
        self.assertFalse((self.out / "status/cell").exists())
        self.assertEqual(self.files(), sorted([
            "status/index.html", "status/die/index.html", "status/run/index.html",
            "status/commit/index.html", "status/find/index.html", "status/status.js",
            "status/status.css", "status/coverage.json", "status/gpu_names.json"]))
        self.assertNotRegex((self.out / "status/status.js").read_text(), r"https?://")
        self.assertNotRegex((self.out / "status/coverage.json").read_text(), WORKER_URL)
        self.assertNotRegex((self.out / "status/gpu_names.json").read_text(), r"https?://")

    def test_invalid_logs_base_is_dropped(self):
        status_pages.write_status(
            self.out, api_base="https://api.kayfabe.dev", logs_base="https://logs.evil.example"
        )
        self.assertNotIn("kf-status-logs", (self.out / "status/index.html").read_text())

    def test_fixture_pages_are_labelled_synthetic_and_hold_no_worker_url(self):
        self.assertEqual(status_pages.write_status(self.out, fixture=FIXTURE), "fixture")
        self.assertIn("status/fixture.json", self.files())
        self.assertIn("status/coverage.json", self.files())
        self.assertIn("status/gpu_names.json", self.files())
        self.assertIn("status/find/index.html", self.files())
        for page in self.out.rglob("*.html"):
            text = page.read_text()
            self.assertIn("Synthetic data.", text)
            self.assertNotRegex(text, WORKER_URL)
        self.assertNotRegex((self.out / "status/fixture.json").read_text(), WORKER_URL)
        self.assertNotRegex((self.out / "status/coverage.json").read_text(), WORKER_URL)
        self.assertNotRegex((self.out / "status/gpu_names.json").read_text(), WORKER_URL)

    def test_fixture_and_api_are_exclusive_and_fixture_must_be_synthetic(self):
        with self.assertRaises(ValueError):
            status_pages.write_status(self.out, api_base="https://api.kayfabe.dev", fixture=FIXTURE)
        fake = self.out / "real.json"
        fake.write_text(json.dumps({"responses": {}}))
        with self.assertRaises(ValueError):
            status_pages.write_status(self.out / "o", fixture=fake)

    def test_rebuild_removes_an_earlier_status_section(self):
        status_pages.write_status(self.out, api_base="https://api.kayfabe.dev")
        status_pages.write_status(self.out)
        self.assertEqual(self.files(), ["status/index.html"])


class GpuNames(unittest.TestCase):
    """site/status/gpu_names.json: the hand-written marketing-name to die table of the picker."""

    PREFIX_FAMILY = {"TU": "turing", "GA": "ampere", "AD": "ada", "GB": "blackwell"}
    # Names whose die the Worker's rules (ci/vast-broker/defaults.json dieRules) label differently from
    # the table. Empty since 2026-11-06, when the Turing rules were corrected (GTX 1650 and 1630 are
    # TU117, the RTX 2070 is TU106; INFERRED from public product data, not measured on hardware).
    WORKER_RULE_DISAGREES = set()

    def setUp(self):
        self.data = json.loads((SITE / "status/gpu_names.json").read_text())
        self.rules = json.loads(
            (SITE.parent / "ci/vast-broker/defaults.json").read_text())["dieRules"]

    @staticmethod
    def key(name):
        return re.sub(r"[^a-z0-9]+", " ", name.lower()).strip()

    def test_shape(self):
        self.assertEqual(self.data["schema"], 1)
        for field in ("note", "variants", "laptops", "unmapped"):
            self.assertTrue(self.data[field], field)
        self.assertIn("Laptop", self.data["laptops"])
        names = set()
        for entry in self.data["entries"]:
            self.assertEqual(set(entry) - {"note"}, {"name", "dies"}, entry)
            self.assertRegex(entry["name"], r"^(?:RTX \d{4}|GTX \d{4}|TITAN RTX)(?: (?:Ti|SUPER|D))*$")
            self.assertNotIn(self.key(entry["name"]), names, "duplicate " + entry["name"])
            names.add(self.key(entry["name"]))
            dies = entry["dies"]
            self.assertTrue(1 <= len(dies) <= 4, entry)  # the API takes up to four dies
            self.assertEqual(len(set(dies)), len(dies))
            for die in dies:
                self.assertRegex(die, r"^(?:TU|GA|AD|GB)[0-9]{3}$")
        # The consumer lines the owner named are all present.
        for line in ("GTX 16", "RTX 20", "RTX 30", "RTX 40", "RTX 50"):
            self.assertTrue(any(e["name"].startswith(line) for e in self.data["entries"]), line)
        self.assertNotRegex(json.dumps(self.data), r"(?i)laptop \w*[0-9]{4}|https?://")

    def test_a_multi_die_name_says_why(self):
        by = {e["name"]: e for e in self.data["entries"]}
        self.assertEqual(by["RTX 3060"]["dies"], ["GA106", "GA104"])
        for entry in self.data["entries"]:
            if len(entry["dies"]) > 1:
                self.assertTrue(entry.get("note"), entry["name"] + " lists several dies without saying so")

    def test_dies_agree_with_the_workers_die_rules_except_the_known_cases(self):
        def matches(pattern, name):
            return re.fullmatch(".*".join(re.escape(p) for p in pattern.split("*")), name) is not None

        disagree = set()
        for entry in self.data["entries"]:
            worker = entry["name"].upper().replace(" ", "_")
            for rule in self.rules:
                if any(matches(p, worker) for p in rule["names"]) and rule["die"] not in entry["dies"]:
                    disagree.add(entry["name"])
        self.assertEqual(disagree, self.WORKER_RULE_DISAGREES)

    def test_dies_agree_with_the_boards_named_in_the_evidence_file(self):
        coverage = json.loads((SITE / "status/coverage.json").read_text())
        by = {self.key(e["name"]): e for e in self.data["entries"]}
        checked = 0
        for family in coverage["families"]:
            for die in family["dies"]:
                if not die["board"]:
                    continue
                entry = by.get(self.key(die["board"]))
                self.assertIsNotNone(entry, die["board"])
                self.assertIn(die["id"], entry["dies"], die["board"])
                self.assertEqual(self.PREFIX_FAMILY[die["id"][:2]], family["id"])
                checked += 1
        self.assertGreaterEqual(checked, 9)


class Coverage(unittest.TestCase):
    """The committed evidence file (scripts/site/coverage_from_traces.py builds it from traces/; its
    own tests need the traces and run in the platform CI job)."""

    def setUp(self):
        self.data = json.loads((SITE / "status/coverage.json").read_text())

    def test_shape(self):
        self.assertEqual(self.data["schema"], 1)
        self.assertEqual(set(self.data["sources"]), {"worker_reproduced", "imported_vast"})
        self.assertEqual(self.data["sources"]["worker_reproduced"], "CI Worker run (reproduced)")
        self.assertEqual(self.data["sources"]["imported_vast"],
                         "Direct Vast run, evidence in repo (imported)")
        self.assertIn("Imported evidence never does", self.data["gate_rule"])
        dies = 0
        for family in self.data["families"]:
            for die in family["dies"]:
                dies += 1
                self.assertEqual(set(die["cells"]), {"linux", "windows"}, die["id"])
                for cell in die["cells"].values():
                    if cell["state"] == "covered":
                        self.assertTrue(cell["groups"], die["id"])
                        for g in cell["groups"]:
                            for key in ("source", "revision", "date", "n", "pass", "fail", "evidence"):
                                self.assertIn(key, g, die["id"])
                            self.assertTrue(g["evidence"].startswith("traces/"))
                    else:
                        self.assertEqual(cell["state"], "not_covered")
                        self.assertTrue(cell["reason"], die["id"])
                        self.assertEqual(cell["groups"], [])
        self.assertGreaterEqual(dies, 20)

    def test_holds_no_worker_url_and_no_secret_shape(self):
        text = (SITE / "status/coverage.json").read_text()
        self.assertNotRegex(text, WORKER_URL)
        self.assertNotRegex(text, r"https?://")
        self.assertNotRegex(text, r"(?i)specviewperf|instance_api_key|token")


class BuildPolicy(unittest.TestCase):
    def test_only_deployment_candidates_receive_the_base(self):
        module = builder()
        with redirect_stdout(io.StringIO()):
            self.assertEqual(
                module.status_inputs(False, "https://api.kayfabe.dev", "https://logs.kayfabe.dev", None),
                (None, None),
            )
        self.assertEqual(
            module.status_inputs(True, "https://api.kayfabe.dev", None, None),
            ("https://api.kayfabe.dev", None),
        )
        with self.assertRaisesRegex(ValueError, "fixture"):
            module.status_inputs(True, None, None, FIXTURE)

    def test_fixture_candidate_is_refused_before_planning(self):
        module = builder()
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaisesRegex(ValueError, "fixture"):
                module.build(Path(folder), {}, [], deploy_candidate=True, status_fixture=FIXTURE)
            self.assertFalse((Path(folder) / "site/public").exists())


class Workflow(unittest.TestCase):
    def setUp(self):
        self.workflow = (SITE.parent / ".github/workflows/site.yml").read_text()
        self.top = self.workflow.split("\njobs:\n")[0]
        self.build = self.workflow.split("  build:\n")[1].split("  deploy:\n")[0]

    def selection(self, candidate, api, logs=""):
        """Run the build step's own status selection lines in bash."""
        start = self.build.index("          origin='")
        end = self.build.index("          python3 site/build.py", start)
        lines = textwrap.dedent(self.build[start:end])
        script = 'candidate=()\n' + lines + 'printf "%s\\n" "${candidate[@]}"\n'
        env = {"PATH": os.environ["PATH"], "DEPLOY_CANDIDATE": candidate,
               "STATUS_API_BASE": api, "STATUS_LOGS_BASE": logs}
        run = subprocess.run(["bash", "--noprofile", "--norc", "-euo", "pipefail", "-c", script],
                             env=env, capture_output=True, text=True, check=True)
        return [line for line in run.stdout.splitlines() if line and not line.startswith("::")]

    def test_variables_are_workflow_env_and_the_build_job_names_no_variable(self):
        self.assertIn("STATUS_API_BASE: ${{ vars.STATUS_API_BASE }}", self.top)
        self.assertIn("STATUS_LOGS_BASE: ${{ vars.STATUS_LOGS_BASE }}", self.top)
        self.assertNotIn("vars.", self.build)
        self.assertEqual(self.build.count("--status-api-base"), 1)
        self.assertLess(self.build.index("--portable"), self.build.index("--status-api-base"))
        self.assertNotIn("--status-fixture", self.build)

    def test_only_a_valid_origin_reaches_a_candidate_build(self):
        good = "https://api.kayfabe.dev"
        self.assertEqual(self.selection("true", good), ["--status-api-base", good])
        self.assertEqual(
            self.selection("true", good, "https://logs.kayfabe.dev"),
            ["--status-api-base", good, "--status-logs-base", "https://logs.kayfabe.dev"],
        )
        self.assertEqual(self.selection("true", good, "https://evil.example"),
                         ["--status-api-base", good])
        self.assertEqual(self.selection("false", good), [])
        self.assertEqual(self.selection("true", ""), [])
        for bad in ["http://api.kayfabe.dev", "https://api.kayfabe.dev/x", "https://evil.example",
                    "https://api.kayfabe.dev.evil.example", "https://u:p@api.kayfabe.dev",
                    "https://api.kayfabe.dev:8443", "https://api.kayfabe.dev\nx"]:
            self.assertEqual(self.selection("true", bad), [], repr(bad))

    def test_status_tests_run_in_ci(self):
        self.assertIn("node --test site/test_status_dom.mjs", self.build)
        self.assertIn("-p 'test_status.py'", self.build)


if __name__ == "__main__":
    unittest.main()
