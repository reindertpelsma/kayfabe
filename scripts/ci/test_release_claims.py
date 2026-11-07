#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tests for scripts/ci/release_claims.py (stdlib unittest, no network, no GPU).

Run: python3 -m unittest scripts/ci/test_release_claims.py
The real ledger is also validated against the real README, so a README edit without a
ledger update fails here as well as in the validator.
"""

import datetime
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import release_claims as rc  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
TODAY = datetime.date(2026, 11, 7)

README = """# title

**STATUS: X.**

A claim line one.

| h1 | h2 |
|---|---|
| row a | ok |
| row b | ok |

See [doc](docs/a.md#section-one) and [web](https://example.org/).
"""


def row(**kw):
    base = {
        "claim_id": "c-one", "source": "README.md:5", "src_hash": "-", "claim": "x", "kind": "auto",
        "verification": "run it", "check_ref": "scripts/x.sh", "where": "CI GPU-free",
        "status": "never-verified", "evidence": "-", "rev": "-", "date": "-", "action": "keep", "note": "-",
    }
    base.update(kw)
    return base


def tsv(rows):
    lines = ["\t".join(rc.COLUMNS)]
    for r in rows:
        lines.append("\t".join(r[c] for c in rc.COLUMNS))
    return "\n".join(lines) + "\n"


class Tree:
    def __enter__(self):
        self.d = tempfile.TemporaryDirectory()
        self.root = Path(self.d.name)
        (self.root / "scripts").mkdir()
        (self.root / "scripts/x.sh").write_text("#!/bin/sh\n")
        (self.root / "docs").mkdir()
        (self.root / "docs/a.md").write_text("# Section One\n")
        (self.root / "ev.md").write_text("evidence\n")
        return self

    def __exit__(self, *a):
        self.d.cleanup()


def covering_rows():
    """One row per README claim line of the fixture, auto and unverified."""
    out = []
    for i, (n, _t, h) in enumerate(rc.readme_claim_lines(README)):
        out.append(row(claim_id=f"c-{i}", source=f"README.md:{n}", src_hash=h))
    return out


def run(rows, readme=README, doc="", root=None, check_git=False):
    with Tree() as t:
        return rc.validate(rows, readme, root or t.root, doc, check_git, TODAY)


class ReadmeLines(unittest.TestCase):
    def test_claim_lines_skip_headings_blank_header_and_separator(self):
        got = [n for n, _t, _h in rc.readme_claim_lines(README)]
        self.assertEqual(got, [3, 5, 9, 10, 12])

    def test_status_line_is_a_claim(self):
        texts = [t for _n, t, _h in rc.readme_claim_lines(README)]
        self.assertTrue(any("STATUS" in t for t in texts))

    def test_hash_ignores_whitespace_only(self):
        self.assertEqual(rc.line_hash("a  b"), rc.line_hash(" a b "))
        self.assertNotEqual(rc.line_hash("a b"), rc.line_hash("a c"))

    def test_table_header_exempt_but_rows_not(self):
        texts = [t for _n, t, _h in rc.readme_claim_lines(README)]
        self.assertFalse(any("h1" in t for t in texts))
        self.assertTrue(any("row a" in t for t in texts))


class Parse(unittest.TestCase):
    def test_roundtrip(self):
        rows, errs = rc.parse_ledger(tsv([row()]))
        self.assertEqual(errs, [])
        self.assertEqual(rows[0]["claim_id"], "c-one")

    def test_bad_header_rejected(self):
        _rows, errs = rc.parse_ledger("a\tb\n1\t2\n")
        self.assertTrue(errs)

    def test_wrong_column_count_rejected(self):
        text = tsv([row()]) + "only\ttwo\n"
        _rows, errs = rc.parse_ledger(text)
        self.assertTrue(any("columns" in e for e in errs))

    def test_empty_file(self):
        _rows, errs = rc.parse_ledger("")
        self.assertTrue(errs)


class Coverage(unittest.TestCase):
    def test_full_cover_is_clean(self):
        errs, _w = run(covering_rows())
        self.assertEqual(errs, [])

    def test_readme_edit_without_ledger_update_fails(self):
        edited = README.replace("A claim line one.", "A claim line one, edited.")
        errs, _w = run(covering_rows(), readme=edited)
        self.assertTrue(any("not covered" in e for e in errs))
        self.assertTrue(any("matches no README claim line" in e for e in errs))

    def test_uncovered_line_fails(self):
        errs, _w = run(covering_rows()[:-1])
        self.assertTrue(any("not covered" in e for e in errs))

    def test_stale_row_fails(self):
        rows = covering_rows()
        rows.append(row(claim_id="stale", source="README.md:5", src_hash="deadbeef00"))
        errs, _w = run(rows)
        self.assertTrue(any("matches no README claim line" in e for e in errs))

    def test_moved_line_number_fails_and_renumber_fixes(self):
        rows = covering_rows()
        rows[0]["source"] = "README.md:99"
        errs, _w = run(rows)
        self.assertTrue(any("renumber" in e for e in errs))
        fixed = rc.renumber(tsv(rows), README)
        parsed, _ = rc.parse_ledger(fixed)
        errs2, _w2 = run(parsed)
        self.assertEqual(errs2, [])

    def test_non_readme_row_needs_dash_hash_and_existing_file(self):
        rows = covering_rows() + [row(claim_id="other", source="docs/a.md:1", src_hash="abc")]
        errs, _w = run(rows)
        self.assertTrue(any("only README rows carry" in e for e in errs))
        rows[-1].update(src_hash="-", source="docs/missing.md:1")
        errs, _w = run(rows)
        self.assertTrue(any("does not exist" in e for e in errs))
        rows[-1].update(source="docs/a.md:50")
        errs, _w = run(rows)
        self.assertTrue(any("has 1 lines" in e for e in errs))
        rows[-1].update(source="docs/a.md:1")
        errs, _w = run(rows)
        self.assertEqual(errs, [])


class RowRules(unittest.TestCase):
    def one(self, **kw):
        rows = covering_rows()
        rows[0].update(kw)
        return run(rows)

    def test_duplicate_id(self):
        rows = covering_rows()
        rows[1]["claim_id"] = rows[0]["claim_id"]
        errs, _w = run(rows)
        self.assertTrue(any("duplicate" in e for e in errs))

    def test_slug(self):
        errs, _w = self.one(claim_id="Bad Id")
        self.assertTrue(any("slug" in e for e in errs))

    def test_vocabulary(self):
        for col, val in (("kind", "x"), ("where", "x"), ("status", "x"), ("action", "x")):
            errs, _w = self.one(**{col: val})
            self.assertTrue(any(col in e for e in errs), col)

    def test_verified_needs_rev_date_evidence(self):
        for missing in ("rev", "date", "evidence"):
            kw = dict(status="verified", rev="0c515b3b3", date="2026-11-04", evidence="ev.md")
            kw[missing] = "-"
            errs, _w = self.one(**kw)
            self.assertTrue(any("needs" in e for e in errs), missing)
        errs, _w = self.one(status="verified", rev="0c515b3b3", date="2026-11-04", evidence="ev.md")
        self.assertEqual(errs, [])

    def test_bad_rev_and_date(self):
        errs, _w = self.one(status="verified", rev="zz", date="2026-11-04", evidence="ev.md")
        self.assertTrue(any("not a hex revision" in e for e in errs))
        errs, _w = self.one(status="verified", rev="0c515b3b3", date="04-11-2026", evidence="ev.md")
        self.assertTrue(any("YYYY-MM-DD" in e for e in errs))
        errs, _w = self.one(status="verified", rev="0c515b3b3", date="2026-12-31", evidence="ev.md")
        self.assertTrue(any("future" in e for e in errs))
        errs, _w = self.one(status="verified", rev="0c515b3b3", date="2026-02-30", evidence="ev.md")
        self.assertTrue(any("calendar" in e for e in errs))

    def test_rev_list_accepts_comma_and_semicolon(self):
        errs, _w = self.one(status="verified", rev="0c515b3b3,3aa2a75a6;eb9f2aa3", date="2026-11-04", evidence="ev.md")
        self.assertEqual(errs, [])

    def test_evidence_pointer_must_exist_and_stay_inside(self):
        for ev in ("nope.md", "../outside", "/etc/passwd"):
            errs, _w = self.one(status="outdated", rev="0c515b3b3", date="2026-11-04", evidence=ev)
            self.assertTrue(any("evidence" in e for e in errs), ev)

    def test_git_pointer_syntax(self):
        errs, _w = self.one(status="outdated", rev="0c515b3b3", date="2026-11-04", evidence="git:zz:path")
        self.assertTrue(any("malformed git pointer" in e for e in errs))
        errs, _w = self.one(status="outdated", rev="0c515b3b3", date="2026-11-04", evidence="git:b746e1b60:a/b.md")
        self.assertEqual(errs, [])

    def test_contradicted_needs_evidence_and_note(self):
        errs, _w = self.one(status="contradicted")
        self.assertTrue(any("contradicting evidence" in e for e in errs))
        self.assertTrue(any("needs a note" in e for e in errs))
        errs, _w = self.one(status="contradicted", evidence="ev.md", note="why")
        self.assertEqual(errs, [])

    def test_wording_rules(self):
        base = dict(kind="wording", check_ref="-", where="not verifiable: reword")
        errs, _w = self.one(**base)
        self.assertEqual(errs, [])
        errs, _w = self.one(**{**base, "status": "verified", "rev": "0c515b3b3", "date": "2026-11-04", "evidence": "ev.md"})
        self.assertTrue(any("cannot be verified" in e for e in errs))
        errs, _w = self.one(**{**base, "where": "CI GPU-free"})
        self.assertTrue(any("wording row must have where" in e for e in errs))
        errs, _w = self.one(**{**base, "check_ref": "scripts/x.sh"})
        self.assertTrue(any("no check_ref" in e for e in errs))

    def test_only_wording_may_be_unverifiable(self):
        errs, _w = self.one(kind="auto", where="not verifiable: reword")
        self.assertTrue(any("only a wording row" in e for e in errs))

    def test_auto_rules(self):
        errs, _w = self.one(check_ref="-")
        self.assertTrue(any("needs check_ref" in e for e in errs))
        errs, _w = self.one(check_ref="scripts/missing.sh")
        self.assertTrue(any("does not exist" in e for e in errs))
        errs, _w = self.one(check_ref="NEW:some-stage", action="add-test")
        self.assertEqual(errs, [])
        errs, _w = self.one(check_ref="NEW:some-stage", action="keep")
        self.assertTrue(any("add-test" in e for e in errs))
        errs, _w = self.one(check_ref="NEW:Bad Slug", action="add-test")
        self.assertTrue(any("NEW:<slug>" in e for e in errs))
        errs, _w = self.one(where="owner hardware")
        self.assertTrue(any("owner hardware" in e for e in errs))

    def test_manual_rules(self):
        base = dict(kind="manual", where="owner hardware", check_ref="MC-1")
        errs, _w = run(self._with(base), doc="### MC-1 Something\n")
        self.assertEqual(errs, [])
        errs, _w = run(self._with(base), doc="### MC-2 Other\n")
        self.assertTrue(any("not a heading" in e for e in errs))
        errs, _w = run(self._with({**base, "check_ref": "scripts/x.sh"}), doc="### MC-1 x\n")
        self.assertTrue(any("MC-n" in e for e in errs))
        errs, _w = run(self._with({**base, "where": "CI GPU-free"}), doc="### MC-1 x\n")
        self.assertTrue(any("owner hardware or release large" in e for e in errs))
        errs, _w = run(self._with({**base, "check_ref": "-"}), doc="### MC-1 x\n")
        self.assertTrue(any("needs a checklist id" in e for e in errs))

    def _with(self, kw):
        rows = covering_rows()
        rows[0].update(kw)
        return rows


class Links(unittest.TestCase):
    def test_links_and_anchors(self):
        with Tree() as t:
            self.assertEqual(rc.readme_link_errors(README, t.root), [])
            bad = README.replace("docs/a.md#section-one", "docs/a.md#nope")
            self.assertTrue(any("anchor" in e for e in rc.readme_link_errors(bad, t.root)))
            bad = README.replace("docs/a.md", "docs/gone.md")
            self.assertTrue(any("does not exist" in e for e in rc.readme_link_errors(bad, t.root)))

    def test_gh_slug(self):
        self.assertEqual(rc.gh_slug("What the isolation is made of"), "what-the-isolation-is-made-of")
        self.assertEqual(rc.gh_slug("`Code` and *more*"), "code-and-more")


class ChecklistAndRelease(unittest.TestCase):
    def rows(self):
        return [
            row(claim_id="ok", status="verified", rev="0c515b3b3", date="2026-11-04", evidence="ev.md"),
            row(claim_id="stale", status="verified", rev="3aa2a75a6", date="2026-11-05", evidence="ev.md"),
            row(claim_id="todo", status="never-verified"),
            row(claim_id="fix", status="outdated", rev="0c515b3b3", date="2026-11-04", evidence="ev.md"),
            row(claim_id="bad", status="contradicted", evidence="ev.md", note="n"),
        ]

    def test_classify(self):
        r = self.rows()
        got = [rc.classify(x, "0c515b3b3abcdef") for x in r]
        self.assertEqual(got, ["ok", "stale", "todo", "fix", "fix"])

    def test_rev_match_requires_seven_digits_and_prefix(self):
        self.assertTrue(rc.rev_matches(["0c515b3b3"], "0c515b3b3aaaa"))
        self.assertTrue(rc.rev_matches(["0c515b3b3aaaa"], "0c515b3b3"))
        self.assertFalse(rc.rev_matches(["0c515b3b3"], "0c515b3c"))
        self.assertFalse(rc.rev_matches(["0c515"], "0c515b3b3"))

    def test_checklist_lists_only_pending_and_groups_by_stage(self):
        text = rc.checklist(self.rows(), "0c515b3b3")
        self.assertNotIn("[STALE] ok", text)
        for needle in ("stale", "todo", "fix", "bad"):
            self.assertIn(f"] {needle} ", text)
        self.assertIn("== CI GPU-free (4)", text)
        self.assertIn("ok at this revision: 1", text)

    def test_summary(self):
        text = rc.summary(self.rows())
        self.assertIn("claims=5", text)
        self.assertIn("verified=2", text)


class CliGate(unittest.TestCase):
    """The same rules through the command line, which is what a CI job runs."""

    def cli(self, rows, readme=README, doc="### MC-1 Something\n"):
        with Tree() as t:
            (t.root / "README.md").write_text(readme)
            (t.root / "docs/RELEASE_CLAIMS.tsv").write_text(tsv(rows))
            (t.root / "docs/RELEASE_CLAIMS.md").write_text(doc)
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = rc.main(["validate", "--root", str(t.root)])
            return code, buf.getvalue()

    def edit(self, **kw):
        rows = covering_rows()
        rows[0].update(kw)
        return rows

    def test_clean_exit_zero(self):
        code, out = self.cli(covering_rows())
        self.assertEqual(code, 0, out)

    def test_verified_without_date_exits_one(self):
        code, out = self.cli(self.edit(status="verified", rev="0c515b3b3", date="-", evidence="ev.md"))
        self.assertEqual(code, 1)
        self.assertIn("needs the date", out)

    def test_verified_without_revision_exits_one(self):
        code, out = self.cli(self.edit(status="verified", rev="-", date="2026-11-04", evidence="ev.md"))
        self.assertEqual(code, 1)
        self.assertIn("needs the revision", out)

    def test_verified_without_evidence_exits_one(self):
        code, out = self.cli(self.edit(status="verified", rev="0c515b3b3", date="2026-11-04"))
        self.assertEqual(code, 1)
        self.assertIn("needs an evidence pointer", out)

    def test_duplicate_id_exits_one(self):
        rows = covering_rows()
        rows[1]["claim_id"] = rows[0]["claim_id"]
        code, out = self.cli(rows)
        self.assertEqual(code, 1)
        self.assertIn("duplicate", out)

    def test_wording_verified_exits_one(self):
        code, out = self.cli(self.edit(kind="wording", check_ref="-", where="not verifiable: reword",
                                       status="verified", rev="0c515b3b3", date="2026-11-04", evidence="ev.md"))
        self.assertEqual(code, 1)
        self.assertIn("cannot be verified", out)

    def test_missing_evidence_file_exits_one(self):
        code, out = self.cli(self.edit(status="outdated", rev="0c515b3b3", date="2026-11-04", evidence="gone.md"))
        self.assertEqual(code, 1)
        self.assertIn("does not exist", out)

    def test_missing_evidence_file_in_second_pointer_exits_one(self):
        code, out = self.cli(self.edit(status="outdated", rev="0c515b3b3", date="2026-11-04", evidence="ev.md;gone2.md"))
        self.assertEqual(code, 1)
        self.assertIn("gone2.md", out)

    def test_future_date_exits_one(self):
        far = (datetime.date.today() + datetime.timedelta(days=400)).isoformat()
        code, out = self.cli(self.edit(status="outdated", rev="0c515b3b3", date=far, evidence="ev.md"))
        self.assertEqual(code, 1)
        self.assertIn("future", out)

    def test_future_date_one_day_ahead_exits_one(self):
        day = (datetime.date.today() + datetime.timedelta(days=1)).isoformat()
        code, out = self.cli(self.edit(status="outdated", rev="0c515b3b3", date=day, evidence="ev.md"))
        self.assertEqual(code, 1)
        self.assertIn("future", out)

    def test_broken_readme_link_exits_one(self):
        code, out = self.cli(covering_rows(), readme=README.replace("docs/a.md", "docs/gone.md"))
        self.assertEqual(code, 1)
        self.assertTrue("link target" in out or "not covered" in out)

    def test_broken_readme_link_alone_is_reported(self):
        edited = README.replace("docs/a.md", "docs/gone.md")
        rows = []
        for i, (n, _t, h) in enumerate(rc.readme_claim_lines(edited)):
            rows.append(row(claim_id=f"c-{i}", source=f"README.md:{n}", src_hash=h))
        code, out = self.cli(rows, readme=edited)
        self.assertEqual(code, 1)
        self.assertIn("link target", out)

    def test_contradicted_without_note_exits_one(self):
        code, out = self.cli(self.edit(status="contradicted", evidence="ev.md", note="-"))
        self.assertEqual(code, 1)
        self.assertIn("needs a note", out)

    def test_auto_on_owner_hardware_exits_one(self):
        code, out = self.cli(self.edit(where="owner hardware"))
        self.assertEqual(code, 1)
        self.assertIn("owner hardware", out)

    def test_new_check_kept_exits_one(self):
        code, out = self.cli(self.edit(check_ref="NEW:some-stage", action="keep"))
        self.assertEqual(code, 1)
        self.assertIn("add-test", out)

    def test_moved_line_exits_one(self):
        code, out = self.cli(self.edit(source="README.md:99"))
        self.assertEqual(code, 1)
        self.assertIn("renumber", out)

    def test_undefined_checklist_exits_one(self):
        rows = self.edit(kind="manual", where="owner hardware", check_ref="MC-7")
        code, out = self.cli(rows, doc="### MC-1 Something\n")
        self.assertEqual(code, 1)
        self.assertIn("not a heading", out)

    def test_defined_checklist_exits_zero(self):
        rows = self.edit(kind="manual", where="owner hardware", check_ref="MC-1")
        code, out = self.cli(rows)
        self.assertEqual(code, 0, out)

    def test_release_command_requires_all_rows_at_revision(self):
        with Tree() as t:
            rows = covering_rows()
            for r in rows:
                r.update(status="verified", rev="0c515b3b3", date="2026-11-04", evidence="ev.md")
            (t.root / "README.md").write_text(README)
            (t.root / "docs/RELEASE_CLAIMS.tsv").write_text(tsv(rows))
            buf = io.StringIO()
            with redirect_stdout(buf):
                ok = rc.main(["release", "--rev", "0c515b3b3ff", "--root", str(t.root)])
                bad = rc.main(["release", "--rev", "1111111", "--root", str(t.root)])
            self.assertEqual(ok, 0, buf.getvalue())
            self.assertEqual(bad, 1)


class RealLedger(unittest.TestCase):
    """The committed ledger against the committed README."""

    @classmethod
    def setUpClass(cls):
        cls.readme = (REPO / "README.md").read_text()
        cls.rows, cls.perr = rc.parse_ledger((REPO / "docs/RELEASE_CLAIMS.tsv").read_text())
        cls.doc = (REPO / "docs/RELEASE_CLAIMS.md").read_text()

    def test_parses(self):
        self.assertEqual(self.perr, [])
        self.assertGreater(len(self.rows), 50)

    def test_validates_against_readme(self):
        errs, _w = rc.validate(self.rows, self.readme, REPO, self.doc)
        self.assertEqual(errs, [])

    def test_readme_links_resolve(self):
        self.assertEqual(rc.readme_link_errors(self.readme, REPO), [])

    def test_every_manual_checklist_is_defined(self):
        used = {r["check_ref"] for r in self.rows if r["kind"] == "manual"}
        self.assertLessEqual(used, rc.manual_ids(self.doc))

    def test_cli(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = rc.main(["validate"])
        self.assertEqual(code, 0, buf.getvalue())
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = rc.main(["release", "--rev", "0000000"])
        self.assertEqual(code, 1)
        self.assertIn("NOT READY", buf.getvalue())
        self.assertEqual(rc.main(["checklist"]), 2)


if __name__ == "__main__":
    unittest.main()
