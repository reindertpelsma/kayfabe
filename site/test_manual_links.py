"""Repository-correct manual links must also resolve in the flattened book."""

from pathlib import Path
import re
import tempfile
import unittest
from urllib.parse import unquote, urlsplit

from test_build import builder


class ManualLinks(unittest.TestCase):
    # FALSIFIER: a valid source link becomes a missing chapter, or loses its query/fragment.
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="manual-links-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.source = self.root / "source"
        self.book = self.root / "book"
        howto = self.source / "docs/howto"
        howto.mkdir(parents=True)
        (howto / "SUMMARY.md").write_text("# Summary\n- [Start](index.md)\n")
        (howto / "index.md").write_text("# Start\n")
        (howto / "windows.md").write_text("# Windows\n")
        (self.source / "docs/FAQ.md").write_text("# FAQ\n")

    def check_link(self, source, output, target, fragment="", query=""):
        original = "[Destination](" + target + ")\n"
        page = self.source / source
        page.write_text(original)
        self.assertTrue((page.parent / unquote(urlsplit(target).path)).is_file())
        builder().manual_source(self.source, self.book)
        rendered = (self.book / output).read_text()
        link = urlsplit(re.search(r"\]\(([^)]+)\)", rendered)[1])
        self.assertTrue((self.book / output).parent.joinpath(unquote(link.path)).is_file())
        self.assertEqual(link.fragment, fragment)
        self.assertEqual(link.query, query)
        self.assertEqual(page.read_text(), original, "rewrite only the publication copy")

    def test_faq_to_howto(self):
        self.check_link("docs/FAQ.md", "FAQ.md", "howto/windows.md")

    def test_faq_to_howto_keeps_query_and_fragment(self):
        self.check_link(
            "docs/FAQ.md",
            "FAQ.md",
            "howto/windows.md?view=plain#setup",
            "setup",
            "view=plain",
        )

    def test_howto_to_faq(self):
        self.check_link("docs/howto/windows.md", "windows.md", "../FAQ.md")

    def test_howto_to_faq_keeps_fragment(self):
        self.check_link("docs/howto/windows.md", "windows.md", "../FAQ.md#drivers", "drivers")

    def test_howto_to_faq_keeps_query(self):
        self.check_link("docs/howto/windows.md", "windows.md", "../FAQ.md?plain=1", query="plain=1")

    def test_external_and_local_anchor_are_unchanged(self):
        text = "[Web](https://example.invalid/howto/windows.md)\n[Here](#drivers)\n"
        (self.source / "docs/FAQ.md").write_text(text)
        builder().manual_source(self.source, self.book)
        self.assertEqual((self.book / "FAQ.md").read_text(), text)

    def test_evidence_comment_is_unchanged(self):
        text = "<!-- src: docs/howto/windows.md -->\n# FAQ\n"
        (self.source / "docs/FAQ.md").write_text(text)
        builder().manual_source(self.source, self.book)
        self.assertEqual((self.book / "FAQ.md").read_text(), text)

    def test_inline_code_is_unchanged(self):
        text = "`[Example](howto/windows.md)`\n"
        (self.source / "docs/FAQ.md").write_text(text)
        builder().manual_source(self.source, self.book)
        self.assertEqual((self.book / "FAQ.md").read_text(), text)

    def test_fenced_example_is_unchanged(self):
        text = "```markdown\n[Example](howto/windows.md)\n```\n"
        (self.source / "docs/FAQ.md").write_text(text)
        builder().manual_source(self.source, self.book)
        self.assertEqual((self.book / "FAQ.md").read_text(), text)

    def check_unpublished(self, target):
        design = self.source / "docs/design"
        design.mkdir()
        (design / "index.md").write_text("# Internal\n")
        text = "[Internal](" + target + ")\n"
        (self.source / "docs/FAQ.md").write_text(text)
        builder().manual_source(self.source, self.book)
        self.assertEqual((self.book / "FAQ.md").read_text(), text)
        self.assertFalse((self.book / "design").exists())

    def test_internal_source_is_not_made_into_a_chapter(self):
        self.check_unpublished("design/index.md")

    def test_other_internal_relative_path_is_preserved_for_rejection(self):
        self.check_unpublished("./design/index.md#details")
