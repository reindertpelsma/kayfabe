"""FALSIFIER: the style guard accepts long prose or rejects readable short copy."""

import unittest
from unittest.mock import patch

import claims_guard
import test_build


class ReadabilityGuard(unittest.TestCase):
    def html(self, text):
        return "<main><section><h1>Try it</h1><p>" + text + "</p></section></main>"

    def test_average_boundary_is_inclusive(self):
        text = " ".join(["word"] * 20) + "."
        self.assertEqual(claims_guard.page_review(self.html(text))["errors"], [])
        result = claims_guard.page_review(self.html(" ".join(["word"] * 21) + "."))
        self.assertTrue(any("average" in e for e in result["errors"]))

    def test_one_long_sentence_cannot_hide_in_short_copy(self):
        text = " ".join(["word"] * 36) + ". Go. Try. Run. Stop."
        result = claims_guard.page_review(self.html(text))
        self.assertTrue(any("35" in e for e in result["errors"]))

    def test_long_table_prose_is_checked_too(self):
        text = " ".join(["word"] * 36) + ". Go. Try. Run. Stop."
        result = claims_guard.page_review(
            "<main><table><tr><td>" + text + "</td></tr></table></main>"
        )
        self.assertTrue(any("35" in e for e in result["errors"]))

    def test_section_average_cannot_hide_in_short_sections(self):
        html = self.html(" ".join(["word"] * 25) + ".")
        html = html.replace("</main>", "<section><h2>Links</h2><p>Go.</p></section></main>")
        result = claims_guard.page_review(html)
        self.assertTrue(any("Try it" in e and "average" in e for e in result["errors"]))

    def test_comments_scripts_hidden_text_and_nav_are_not_prose(self):
        bad = " ".join(["word"] * 80) + "."
        html = "<nav>" + bad + "</nav>" + self.html("Build it from source.")
        html = html.replace(
            "</main>",
            "<!-- " + bad + " --><script>" + bad + "</script>" "<p hidden>" + bad + "</p></main>",
        )
        self.assertEqual(claims_guard.page_review(html)["errors"], [])

    def test_sentence_split_keeps_versions_and_contractions(self):
        stats = claims_guard.reading_stats(["NVIDIA's driver is 580.88. It works."])
        self.assertEqual(stats["sentences"], 2)
        self.assertEqual(stats["longest_sentence_words"], 4)

    def test_flesch_and_depth_are_advisory(self):
        text = "Virtualization interoperability initialization."
        html = self.html(text + " One. Two. Three.")
        result = claims_guard.page_review(html)
        self.assertEqual(result["errors"], [])
        self.assertTrue(result["warnings"])

    def test_no_word_count_gate(self):
        html = self.html("Run it. " * 600)
        self.assertEqual(claims_guard.page_review(html)["errors"], [])
        self.assertEqual(claims_guard.page_review(self.html("Run."))["errors"], [])

    def test_honesty_and_hero_rules_stay_hard(self):
        for text in ["123.4 fps", "production path", "seamless", "Fast\u2014easy"]:
            self.assertTrue(claims_guard.page_review(self.html(text))["errors"], text)
        html = self.html("Open /dev/kvm.").replace("<section>", '<section class="hero">')
        self.assertTrue(claims_guard.page_review(html)["errors"])

    def test_metadata_calls_copy_review(self):
        import tempfile
        from pathlib import Path

        module = test_build.builder()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ["index.html", "works-today/index.html"]:
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(
                    "<html><head><title>Test</title></head><body>"
                    + self.html("Try it.")
                    + "</body></html>"
                )
            with patch.object(module, "check_public_copy", return_value=None) as review:
                module.metadata(root, {"versions": [], "latest": "master"})
            checked = [str(call.args[1]) for call in review.call_args_list]
            self.assertIn("index.html", checked)
            self.assertIn("works-today/index.html", checked)

    def test_metadata_rejects_long_subpage_copy(self):
        import tempfile
        from pathlib import Path

        module = test_build.builder()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            p = root / "works-today/index.html"
            p.parent.mkdir(parents=True)
            p.write_text(
                "<html><head><title>Test</title></head><body>"
                + self.html(" ".join(["word"] * 40) + ".")
                + "</body></html>"
            )
            with self.assertRaisesRegex(ValueError, "sentence"):
                module.metadata(root, {"versions": [], "latest": "master"})

    def test_build_checks_landing_and_main_site_subpages(self):
        module = test_build.builder()
        with patch.object(module.claims_guard, "enforce_page", side_effect=ValueError("style")):
            with self.assertRaisesRegex(ValueError, "style"):
                module.check_public_copy("<main><p>Hi.</p></main>", "index.html")
            with self.assertRaisesRegex(ValueError, "style"):
                module.check_public_copy("<main><p>Hi.</p></main>", "works-today/index.html")
        with patch.object(module.claims_guard, "enforce_page", side_effect=ValueError("style")):
            self.assertIsNone(
                module.check_public_copy("<p>Long manual.</p>", "docs/master/FAQ.html")
            )


if __name__ == "__main__":
    unittest.main()
