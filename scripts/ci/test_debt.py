"""Regression checks for the migration-debt gate itself; no GPU or cargo needed."""

import json
from pathlib import Path
import tempfile
import unittest

from debt import clippy, compare, identity


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
