"""The published manual lists every page once and carries no internal evidence label outside the operator pages.

STATUS: LIVE, 2026-11-20 (scripts/ci/doc_manual_check.py).

FALSIFIER (written before the code): a page added to docs/howto/ without a SUMMARY.md entry, an entry for a missing page, a
page listed twice, or a CODE READ / MEASURED / [measured ...] / Verified on: label in the visible text of a Part 1 or Part 2
page or the FAQ passes this file. A label inside an HTML comment, inside a code fence, or on an operator page must pass.
"""

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import doc_manual_check as manual  # noqa: E402

ROOT = HERE.parents[1]


class Repository(unittest.TestCase):
    def test_the_manual_in_the_tree_is_clean(self):
        self.assertEqual(manual.check(ROOT), [])


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-manual-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        (self.tmp / "docs/howto/ops").mkdir(parents=True)
        (self.tmp / "docs/howto/SUMMARY.md").write_text("# Summary\n\n- [A](a.md)\n- [Ops](ops/o.md)\n")
        (self.tmp / "docs/howto/a.md").write_text("# A\n\nPlain text.\n")
        (self.tmp / "docs/howto/ops/o.md").write_text("# O\n\nCODE READ: operators keep labels.\n")
        (self.tmp / "docs/FAQ.md").write_text("# FAQ\n\nPlain.\n")

    def text(self, rel, body):
        (self.tmp / rel).write_text(body)

    def findings(self):
        return "\n".join(manual.check(self.tmp))

    def test_the_fixture_is_clean(self):
        self.assertEqual(manual.check(self.tmp), [])

    def test_a_page_missing_from_the_summary_fails(self):
        self.text("docs/howto/b.md", "# B\n")
        self.assertIn("docs/howto/b.md is not listed", self.findings())

    def test_a_page_in_a_subdirectory_missing_from_the_summary_fails(self):
        self.text("docs/howto/ops/p.md", "# P\n")
        self.assertIn("docs/howto/ops/p.md is not listed", self.findings())

    def test_an_entry_for_a_missing_page_fails(self):
        self.text("docs/howto/SUMMARY.md", "# Summary\n\n- [A](a.md)\n- [Ops](ops/o.md)\n- [Gone](gone.md)\n")
        self.assertIn("lists gone.md, which does not exist", self.findings())

    def test_a_page_listed_twice_fails(self):
        self.text("docs/howto/SUMMARY.md", "# Summary\n\n- [A](a.md)\n- [A again](a.md)\n- [Ops](ops/o.md)\n")
        self.assertIn("lists a.md more than once", self.findings())

    def test_a_label_in_a_user_page_fails(self):
        self.text("docs/howto/a.md", "# A\n\nIt works. MEASURED on a box.\n")
        self.assertIn("a.md:3: internal label 'MEASURED'", self.findings())

    def test_a_bracket_label_and_a_footer_in_a_user_page_fail(self):
        self.text("docs/howto/a.md", "# A\n\nIt works [measured, rev abc].\n\nVerified on: a box.\n")
        found = self.findings()
        self.assertIn("'[measured'", found)
        self.assertIn("'Verified on:'", found)

    def test_a_label_in_the_faq_fails(self):
        self.text("docs/FAQ.md", "# FAQ\n\nCODE READ says so.\n")
        self.assertIn("docs/FAQ.md:3", self.findings())

    def test_a_label_inside_a_comment_or_a_code_fence_passes(self):
        self.text("docs/howto/a.md", "# A\n\nText. <!-- src: MEASURED at abc -->\n\n```\nCODE READ in a command\n```\n")
        self.assertEqual(manual.check(self.tmp), [])

    def test_a_label_on_an_operator_page_passes(self):
        self.assertEqual(manual.check(self.tmp), [])


if __name__ == "__main__":
    unittest.main()
