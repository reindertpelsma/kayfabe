"""Composition falsifiers: mode isolation and exercised portable CI coverage."""

import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_build import builder


class Modes(unittest.TestCase):
    def test_api_rejects_portable_candidate_before_reading_or_removing_output(self):
        module = builder()
        with tempfile.TemporaryDirectory() as folder:
            repo = Path(folder)
            output = repo / "site/public"
            output.mkdir(parents=True)
            sentinel = output / "index.html"
            sentinel.write_bytes(b"existing output")
            with patch.object(
                module,
                "version_plan",
                side_effect=AssertionError("mode guard must precede planning"),
            ) as plan:
                with self.assertRaisesRegex(ValueError, "portable.*deploy"):
                    module.build(repo, {}, [], portable=True, deploy_candidate=True)
                plan.assert_not_called()
            self.assertEqual(sentinel.read_bytes(), b"existing output")

    def test_publication_cli_cannot_combine_portable_and_candidate(self):
        module = builder()
        context = {
            "GITHUB_ACTIONS": "true",
            "GITHUB_REPOSITORY": "reindertpelsma/kayfabe",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_REF": "refs/heads/master",
        }
        with patch.dict(os.environ, context, clear=True), patch.object(
            sys, "argv", ["build.py", "--tools-dir", "/unused", "--portable", "--deploy-candidate"]
        ), patch.object(module, "tools", return_value={}), patch.object(
            module, "version_plan", side_effect=AssertionError("CLI must reject conflicting modes")
        ):
            with self.assertRaisesRegex(ValueError, "portable.*deploy"):
                module.main()


class WorkflowModes(unittest.TestCase):
    def setUp(self):
        workflow = Path(__file__).resolve().parents[1] / ".github/workflows/site.yml"
        self.build = workflow.read_text().split("  build:\n")[1].split("  deploy:\n")[0]

    def test_portable_integration_uses_real_tools_in_ci(self):
        self.assertIn('SITE_PORTABLE_TOOLS="$RUNNER_TEMP/site-tools"', self.build)
        self.assertIn("-p 'test_portable.py'", self.build)
        self.assertIn("-p 'test_modes.py'", self.build)

    def test_portable_crawl_precedes_a_fresh_pages_build_and_upload(self):
        self.assertIn("--portable", self.build)
        portable = self.build.index("--portable")
        crawl = self.build.index("python3 site/check_portable.py", portable)
        pages = self.build.index("python3 site/build.py", crawl)
        strict = self.build.index("python3 site/test_output.py", pages)
        upload = self.build.index("actions/upload-pages-artifact@", strict)
        self.assertLess(portable, crawl)
        self.assertLess(crawl, pages)
        self.assertLess(pages, strict)
        self.assertLess(strict, upload)


if __name__ == "__main__":
    unittest.main()
