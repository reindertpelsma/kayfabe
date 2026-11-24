"""Regression checks for the migration-debt gate itself; no GPU or cargo needed."""

import json
from pathlib import Path
import tempfile
import unittest

from debt import clippy, compare, identity, logs


class DebtTests(unittest.TestCase):
    def test_removed_item_does_not_buy_an_unrelated_new_item(self):
        self.assertEqual(compare({"new": "text"}, {"old": "text"}), ["new"])

    def test_path_category_and_text_are_all_bound(self):
        known = identity("a.rs", "kind", "old text")
        for other in (identity("b.rs", "kind", "old text"),
                      identity("a.rs", "other", "old text"),
                      identity("a.rs", "kind", "new text")):
            self.assertNotEqual(known, other)
        self.assertEqual(known, identity("a.rs", "kind", "old\n  text"))

    def test_full_paragraph_not_just_display_excerpt_is_bound(self):
        prefix = "unchanged excerpt " * 30
        self.assertNotEqual(identity("a.rs", "claim", prefix + "old ending"),
                            identity("a.rs", "claim", prefix + "new ending"))

    def test_missing_or_failed_clippy_completion_is_not_green(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "log"
            for content in ("", json.dumps({"reason": "build-finished", "success": False})):
                path.write_text(content)
                with self.assertRaises(ValueError):
                    clippy(path)

    def test_errors_cannot_be_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "log"
            path.write_text(json.dumps({"reason": "compiler-message", "message": {
                "level": "error", "rendered": "compile failed"}}))
            with self.assertRaisesRegex(ValueError, "compile failed"):
                clippy(path)

    def test_successful_empty_diagnostics_are_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "log"
            path.write_text(json.dumps({"reason": "build-finished", "success": True}))
            self.assertEqual(clippy(path), {})


if __name__ == "__main__":
    unittest.main()


class LogRatchetTests(unittest.TestCase):
    """The `logs` kind: every plain `eprintln!` of a product crate is pinned; a new one fails."""

    def tree(self, files):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        root = Path(d.name)
        for rel, text in files.items():
            p = root / "crates" / "kf-x" / "src" / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        return root

    def sites(self, files):
        return logs(self.tree(files), ("kf-x",))

    BASE = 'fn serve() {\n    eprintln!("refused {x}");\n}\n'

    def test_a_new_plain_site_is_new_debt(self):
        old = self.sites({"lib.rs": self.BASE})
        new = self.sites({"lib.rs": self.BASE + 'fn other() {\n    eprintln!("refused {x}");\n}\n'})
        self.assertEqual(len(compare(new, old)), 1)

    def test_a_duplicate_site_in_the_same_fn_is_new_debt(self):
        old = self.sites({"lib.rs": self.BASE})
        dup = self.sites({"lib.rs": self.BASE.replace("}\n", '    eprintln!("refused {x}");\n}\n')})
        self.assertEqual(len(compare(dup, old)), 1)

    def test_rate_limited_and_test_only_sites_are_not_debt(self):
        found = self.sites({
            "lib.rs": "#[cfg(test)]\nmod fuzz;\nfn a() {\n    kf_arch::rate_eprintln!(\"x\");\n}\n"
                      "#[cfg(test)]\nmod tests {\n    fn t() { eprintln!(\"t\"); }\n}\n",
            "fuzz.rs": 'fn f() { eprintln!("f"); }\n',
            "chan_tests.rs": 'fn g() { eprintln!("g"); }\n',
        })
        self.assertEqual(found, {})

    def test_the_format_string_is_bound_across_a_rustfmt_break(self):
        a = self.sites({"lib.rs": 'fn s() {\n    eprintln!(\n        "one {x}",\n    );\n}\n'})
        b = self.sites({"lib.rs": 'fn s() {\n    eprintln!(\n        "two {x}",\n    );\n}\n'})
        self.assertEqual(len(compare(b, a)), 1)
