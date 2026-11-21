#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/windows/apps_b/ui_table.py (2026-11-20): the table of UI text lines about missing online add-ons, from synthetic
fixtures only (no real data): ordering, de-duplication, severity, the nearest heading as the feature, the scrub, the bound and empty input.

STATUS: LIVE, 2026-11-20."""
import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "bench/windows/apps_b"))
sys.path.insert(0, str(HERE.parent / "bench/windows"))
import ui_table as U  # noqa: E402

RUN = "0123456789abcdef0123456789abcdef"
HEAD = "| app | feature | text | severity |\n|---|---|---|---|\n"


class T(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.root = Path(self.td.name)
        self.addCleanup(self.td.cleanup)

    def put(self, rel, lines, name="uitext_hits.txt"):
        d = self.root / rel
        d.mkdir(parents=True, exist_ok=True)
        (d / name).write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        return d

    def table(self, *dirs, **kw):
        return U.render(U.collect(U.find_files([self.root / d if d else self.root for d in (dirs or ("",))])), **kw)

    def rows(self, *dirs):
        return self.table(*dirs).splitlines()[2:]

    def test_the_header_is_the_requested_one(self):
        self.assertEqual(self.table(), HEAD)

    def test_empty_input_gives_the_header_only(self):
        self.assertEqual(self.table(), HEAD)
        self.put("out/boot1/bridge/" + RUN, [])
        self.assertEqual(self.table(), HEAD)
        self.put("out/boot1/audition/" + RUN, ["", "   "])
        self.assertEqual(self.table(), HEAD)
        self.assertEqual(U.render(U.collect([])), HEAD)
        self.assertEqual(U.collect(U.find_files([self.root / "missing"])), [])

    def test_ordering_is_severity_then_app_then_appearance(self):
        self.put("out/boot1/painter/" + RUN, ["Text | Library is empty | a", "Text | Requires an internet connection | b", "Text | Free tips | c",
                                              "Button | Download models | d", "Text | Not available offline | e"])
        self.put("out/boot2/bridge/" + RUN, ["Text | Sign in required | x", "Text | Install the plug-in | y", "Text | Cloud storage | z"])
        got = [(r.split(" | ")[0][2:], r.split(" | ")[-1][:-2]) for r in self.rows("out")]
        self.assertEqual(got, [("bridge", "high"), ("painter", "high"), ("painter", "high"), ("bridge", "medium"), ("painter", "medium"),
                               ("painter", "medium"), ("bridge", "low"), ("painter", "low")])
        texts = [r.split(" | ")[2] for r in self.rows("out")]
        self.assertEqual(texts[:4], ["Sign in required", "Requires an internet connection", "Not available offline", "Install the plug-in"])
        self.assertEqual(texts[4:6], ["Library is empty", "Download models"])
        self.assertEqual(texts[6:], ["Cloud storage", "Free tips"])

    def test_severity_words(self):
        for text in ("This requires internet", "No Internet Connection", "Feature not available", "Currently UNAVAILABLE", "Sign in required to continue",
                     "Trial expired", "Your trial ends soon", "Trial days left", "You are offline"):
            self.assertEqual(U.severity(text), "high", text)
        for text in ("Download now", "Install", "Neural model", "Add-on manager", "Plug-in", "Update available", "Content packs", "Library"):
            self.assertEqual(U.severity(text), "medium", text)
        for text in ("Sign in", "Account", "Cloud", "Free", "Get started", "Connect", ""):
            self.assertEqual(U.severity(text), "low", text)

    def test_dedupe_per_app_ignores_case_and_keeps_other_apps(self):
        self.put("out/boot1/audition/" + RUN, ["Text | Download models | a", "Button | download MODELS | b", "Text | Download models | c"])
        self.put("evidence/audition", ["Text | Download models | a"])
        self.put("evidence/bridge", ["Text | Download models | a"])
        rows = self.rows("out", "evidence")
        self.assertEqual([r.split(" | ")[0][2:] for r in rows], ["audition", "bridge"])
        self.assertEqual(len(rows), 2)

    def test_both_layouts_name_the_app(self):
        self.assertEqual(U.app_of(self.root / "out/boot3/dreamweaver" / RUN / "uitext_hits.txt"), "dreamweaver")
        self.assertEqual(U.app_of(self.root / "evidence/incopy/uitext_hits.txt"), "incopy")
        self.put("out/boot1/dreamweaver/" + RUN, ["Text | Install | a"])
        self.put("evidence/incopy", ["Text | Install | a"])
        self.assertEqual([r.split(" | ")[0][2:] for r in self.rows("out", "evidence")], ["dreamweaver", "incopy"])

    def test_the_scrub_covers_an_email_a_key_and_the_signed_in_line(self):
        self.put("evidence/painter", ["Text | Download for jane.doe@example.com | a", "Text | Install key ABCD-1234-EFGH-5678 | b",
                                      "Text | Signed in as Jane Doe, online | c", "Text | Download 0123456789abcdef0123 | d"])
        out = self.table("evidence")
        for raw in ("jane", "example.com", "ABCD", "1234-EFGH", "Jane Doe", "0123456789abcdef"):
            self.assertNotIn(raw, out)
        self.assertEqual(out.count("<scrubbed>"), 4)
        self.assertIn("Download for <scrubbed>", out)
        self.assertIn("Install key <scrubbed>", out)

    def test_the_table_text_is_scrubbed_even_when_the_file_was_not(self):
        rows = [("painter", "-", "mail me at x@y.org", "low")]
        self.assertNotIn("x@y.org", U.render(rows))

    def test_the_feature_is_the_nearest_preceding_heading_else_a_dash(self):
        self.put("evidence/painter", ["Window | Adobe Substance 3D Painter | main", "Group | Neural Filters | g", "Text | Plain label | t",
                                      "Button | Download models | b", "TabItem | Library | tab", "Text | Content is online only | c",
                                      "Text | Other | o", "Text | Update available | u"], name="uitext.txt")
        self.put("evidence/painter", ["Button | Download models | b", "Text | Content is online only | c", "Text | Update available | u"])
        got = {r.split(" | ")[2]: r.split(" | ")[1] for r in self.rows("evidence")}
        self.assertEqual(got, {"Download models": "Neural Filters", "Content is online only": "Library", "Update available": "Library"})

    def test_no_heading_before_the_text_gives_a_dash_and_a_heading_line_is_not_its_own_feature(self):
        self.put("evidence/painter", ["Text | Download models | b", "Group | Library | g", "Text | Plain | p"], name="uitext.txt")
        self.put("evidence/painter", ["Text | Download models | b", "Group | Library | g"])
        got = {r.split(" | ")[2]: r.split(" | ")[1] for r in self.rows("evidence")}
        self.assertEqual(got, {"Download models": "-", "Library": "-"})

    def test_a_title_like_automation_id_makes_a_heading(self):
        self.put("evidence/painter", ["Text | Neural panel | PanelTitle", "Text | Download models | b"], name="uitext.txt")
        self.put("evidence/painter", ["Text | Download models | b"])
        self.assertEqual(self.rows("evidence")[0].split(" | ")[1], "Neural panel")

    def test_without_a_context_file_the_hit_file_is_the_context(self):
        self.put("evidence/painter", ["Group | Library | g", "Text | Download models | b"])
        self.assertEqual([r.split(" | ")[1] for r in self.rows("evidence")], ["-", "Library"])

    def test_the_table_is_bounded_and_says_how_many_rows_were_cut(self):
        self.put("evidence/painter", ["Text | Download item %03d | a" % i for i in range(260)])
        out = self.table("evidence")
        self.assertEqual(len([r for r in out.splitlines() if r.startswith("| painter")]), 200)
        self.assertIn("(60 more rows not shown)", out)
        self.assertEqual(len([r for r in self.table("evidence", max_rows=5).splitlines() if r.startswith("| painter")]), 5)

    def test_high_severity_rows_survive_the_bound(self):
        self.put("evidence/painter", ["Text | Free item %03d | a" % i for i in range(260)] + ["Text | Requires internet | z"])
        out = self.table("evidence")
        self.assertEqual(self.rows("evidence")[0], "| painter | - | Requires internet | high |")
        self.assertEqual(len([r for r in out.splitlines() if r.startswith("| painter")]), 200)

    def test_the_same_text_in_the_out_tree_and_its_evidence_copy_is_one_row(self):
        self.put("out/boot1/bridge/" + RUN, ["Text | Download models | a"])
        self.put("evidence/bridge", ["Text | Download models | a"])
        self.assertEqual(len(self.rows("out", "evidence")), 1)
        self.assertEqual(len(self.rows("evidence", "out")), 1)

    def test_the_app_and_feature_cells_are_scrubbed_too(self):
        out = U.render([("a@b.co", "k@l.org", "x", "low")])
        self.assertNotIn("@", out)
        self.assertEqual(out.count("<scrubbed>"), 2)

    def test_a_pipe_in_the_text_does_not_break_the_table(self):
        self.put("evidence/painter", ["Text | Download a|b models | x"])
        row = self.rows("evidence")[0]
        self.assertIn("Download a\\|b models", row)
        self.assertEqual(row.replace("\\|", "").count("|"), 5)

    def test_oversized_and_linked_files_are_ignored(self):
        d = self.put("evidence/painter", ["Text | Download models | a"])
        (d / "uitext_hits.txt").write_bytes(b"Text | Download models | a\n" * 20000)
        self.assertEqual(self.table("evidence"), HEAD)
        other = self.put("evidence/bridge", ["Text | Install | a"])
        real = self.put("elsewhere", ["Text | Install | a"])
        (self.root / "evidence" / "linked").mkdir()
        os.symlink(real / "uitext_hits.txt", self.root / "evidence/linked/uitext_hits.txt")
        self.assertEqual([r.split(" | ")[0][2:] for r in self.rows("evidence")], ["bridge"])

    def test_parse_line(self):
        self.assertEqual(U.parse_line("Button | Download | dl\n"), ("Button", "Download", "dl"))
        self.assertEqual(U.parse_line("Text | a | b | c"), ("Text", "a | b", "c"))
        self.assertEqual(U.parse_line("Text | name | "), ("Text", "name", ""))
        self.assertEqual(U.parse_line("Text | name |"), ("Text", "name", ""))
        self.assertEqual(U.parse_line("just text"), ("", "just text", ""))

    def test_main_prints_the_table(self):
        self.put("evidence/painter", ["Text | Requires internet | a"])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(U.main([str(self.root / "evidence")]), 0)
        self.assertEqual(buf.getvalue(), HEAD + "| painter | - | Requires internet | high |\n")


if __name__ == "__main__":
    unittest.main()
