"""Contract tests for the design-document status checker (GPU-free)."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).with_name("doc_status_check.py")


class DocStatusTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "docs/design").mkdir(parents=True)

    def write(self, name, content):
        path = self.root / "docs/design" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def run_check(self, expected, text=False):
        command = [sys.executable, str(CHECKER), "--root", str(self.root)]
        if not text:
            command += ["--format", "json"]
        run = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(run.returncode, expected, run.stderr or run.stdout)
        return run.stdout if text else json.loads(run.stdout)

    def kinds(self, expected=1):
        return [v["kind"] for v in self.run_check(expected)["violations"]]

    def test_all_five_statuses_are_accepted(self):
        for state in ("LIVE", "SUPERSEDED", "ANSWERED", "DESIGN-ONLY", "RESEARCH"):
            self.write(state + ".md", f"# Title\n\n**STATUS: {state}, 2026-10-22.**\n\nBody\n")
        self.assertEqual(self.run_check(0)["checked"], 5)

    def test_status_may_precede_title(self):
        self.write("a.md", "STATUS: RESEARCH, 2026-10-22.\n\n# Title\n\nBody\n")
        self.assertEqual(self.run_check(0)["violations"], [])

    def test_blockquoted_heading_status_is_accepted(self):
        self.write("a.md", "# Title\n\n> ### STATUS — 2026-10-22 / **LIVE**\n> Details.\n\nBody\n")
        self.run_check(0)

    def test_parenthesized_date_status_is_accepted(self):
        self.write("a.md", "# Title\n\n**STATUS (updated 2026-10-22): LIVE — code only.**\n")
        self.run_check(0)

    def test_multiline_status_paragraph_is_accepted(self):
        self.write("a.md", "# Title\n\n**STATUS: LIVE (code),\n2026-10-22.**\n\nBody\n")
        self.run_check(0)

    def test_missing_status_is_reported(self):
        self.write("a.md", "# Title\n\nAn undated proposal.\n")
        self.assertIn("missing-status", self.kinds())

    def test_later_status_cannot_cover_opening_prose(self):
        self.write("a.md", "# Title\n\nClaim.\n\nSTATUS: LIVE, 2026-10-22.\n")
        self.assertIn("misplaced-status", self.kinds())

    def test_section_heading_before_status_is_reported(self):
        self.write("a.md", "# Title\n\n## Implementation\n\nSTATUS: LIVE, 2026-10-22.\n")
        self.assertIn("misplaced-status", self.kinds())

    def test_prose_before_status_in_same_paragraph_is_reported(self):
        self.write("a.md", "# Title\n\nCorrection first.\nSTATUS: LIVE, 2026-10-22.\n")
        self.assertIn("misplaced-status", self.kinds())

    def test_yaml_front_matter_before_status_is_accepted(self):
        self.write("a.md", "---\ntitle: Example\n---\n\nSTATUS: LIVE, 2026-10-22.\n")
        self.run_check(0)

    def test_yaml_front_matter_then_title_preserves_status_line_number(self):
        self.write("a.md", "---\ntitle: Example\n---\n\n# Title\n\nSTATUS: LIVE.\n")
        result = self.run_check(1)
        self.assertEqual([v["kind"] for v in result["violations"]], ["missing-date"])
        self.assertEqual(result["violations"][0]["line"], 7)

    def test_status_in_fenced_code_is_not_metadata(self):
        self.write("a.md", "# Title\n\n```\nSTATUS: LIVE, 2026-10-22.\n```\n")
        self.assertIn("missing-status", self.kinds())

    def test_status_in_html_comment_is_not_metadata(self):
        self.write("a.md", "# Title\n\n<!-- STATUS: LIVE, 2026-10-22. -->\n\nBody\n")
        self.assertIn("missing-status", self.kinds())

    def test_date_elsewhere_cannot_cover_undated_status(self):
        self.write("a.md", "# 2026-10-22\n\nSTATUS: LIVE.\n\nMeasured 2026-10-22.\n")
        self.assertIn("missing-date", self.kinds())

    def test_invalid_calendar_date_is_reported(self):
        self.write("a.md", "# Title\n\nSTATUS: LIVE, 2026-02-30.\n")
        self.assertIn("invalid-date", self.kinds())

    def test_nonstandard_status_is_reported(self):
        self.write("a.md", "# Title\n\nSTATUS: BUILT, 2026-10-22.\n")
        self.assertIn("invalid-status", self.kinds())

    def test_live_word_in_body_does_not_fix_bad_status(self):
        self.write("a.md", "# Title\n\nSTATUS: BUILT, 2026-10-22.\n\nLIVE code follows.\n")
        self.assertIn("invalid-status", self.kinds())

    def test_status_word_prefix_is_not_a_valid_state(self):
        self.write("a.md", "# Title\n\nSTATUS: LIVELY, 2026-10-22.\n")
        self.assertIn("invalid-status", self.kinds())

    def test_explicit_supersession_needs_notice_in_old_doc(self):
        self.write("old.md", "# Old\n\nSTATUS: LIVE, 2026-10-01.\n")
        self.write("new.md", "# New\n\nSTATUS: LIVE, 2026-10-22. Supersedes `old.md`.\n")
        self.assertIn("missing-supersession-notice", self.kinds())

    def test_unrelated_superseded_section_is_not_a_backreference(self):
        self.write("old.md", "# Old\n\nSTATUS: LIVE, 2026-10-01.\n\nSUPERSEDED by other.md.\n")
        self.write("new.md", "# New\n\nSTATUS: LIVE, 2026-10-22. Supersedes `old.md`.\n")
        self.assertIn("missing-supersession-notice", self.kinds())

    def test_reciprocal_partial_supersession_is_accepted(self):
        self.write(
            "old.md",
            "# Old\n\nSTATUS: LIVE, 2026-10-01.\n\nData SUPERSEDED by `new.md`; policy remains live.\n",
        )
        self.write("new.md", "# New\n\nSTATUS: LIVE, 2026-10-22. Supersedes `old.md`.\n")
        self.run_check(0)

    def test_markdown_supersession_link_is_checked(self):
        self.write("old.md", "# Old\n\nSTATUS: LIVE, 2026-10-01.\n")
        self.write("new.md", "# New\n\nSTATUS: LIVE, 2026-10-22. Supersedes [old](old.md).\n")
        self.assertIn("missing-supersession-notice", self.kinds())

    def test_missing_supersession_target_is_reported(self):
        self.write("new.md", "# New\n\nSTATUS: LIVE, 2026-10-22. Supersedes `absent.md`.\n")
        self.assertIn("missing-supersession-target", self.kinds())

    def test_nested_documents_are_checked_but_evidence_is_not(self):
        self.write("nested/a.md", "# No status\n")
        for name in ("archive/old.md", "traces/old.md"):
            path = self.root / name
            path.parent.mkdir(parents=True)
            path.write_text("# Historical evidence\n")
        result = self.run_check(1)
        self.assertEqual(result["checked"], 1)
        self.assertEqual(result["violations"][0]["path"], "docs/design/nested/a.md")

    def test_text_output_has_actionable_location(self):
        self.write("a.md", "# Title\n\nSTATUS: LIVE.\n")
        self.assertIn("docs/design/a.md:3: missing-date:", self.run_check(1, text=True))

    def test_wrong_root_is_not_a_green_empty_scan(self):
        (self.root / "docs/design").rmdir()
        self.run_check(2, text=True)

    def test_empty_design_directory_is_not_a_green_scan(self):
        self.run_check(2, text=True)


if __name__ == "__main__":
    unittest.main()
