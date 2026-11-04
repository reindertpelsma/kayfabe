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
        for page in ["status/index.html", "status/cell/index.html", "status/run/index.html",
                     "status/commit/index.html"]:
            text = (self.out / page).read_text()
            self.assertEqual(text.count("https://api.kayfabe.dev"), 1, page)
            self.assertIn('<meta name="kf-status-api" content="https://api.kayfabe.dev" />', text)
            self.assertIn('<meta name="kf-status-logs" content="https://logs.kayfabe.dev" />', text)
            self.assertNotIn("Synthetic", text)
        self.assertFalse((self.out / "status/fixture.json").exists())
        self.assertNotRegex((self.out / "status/status.js").read_text(), r"https?://")

    def test_invalid_logs_base_is_dropped(self):
        status_pages.write_status(
            self.out, api_base="https://api.kayfabe.dev", logs_base="https://logs.evil.example"
        )
        self.assertNotIn("kf-status-logs", (self.out / "status/index.html").read_text())

    def test_fixture_pages_are_labelled_synthetic_and_hold_no_worker_url(self):
        self.assertEqual(status_pages.write_status(self.out, fixture=FIXTURE), "fixture")
        self.assertIn("status/fixture.json", self.files())
        for page in self.out.rglob("*.html"):
            text = page.read_text()
            self.assertIn("Synthetic data.", text)
            self.assertNotRegex(text, WORKER_URL)
        self.assertNotRegex((self.out / "status/fixture.json").read_text(), WORKER_URL)

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
