#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tests for scripts/ci/caller_contracts.py (stdlib only, no network)."""

import os
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import caller_contracts as cc  # noqa: E402

RS = "crates/x/src/a.rs"
C = "cuda/walk/k.cu"
LEDGER_HEAD = "# comment\n"


def files(**kw):
    return [(p.replace("__", "/"), t.encode()) for p, t in kw.items()]


def run(path, text, ledger):
    return cc.check([(path, text.encode())], LEDGER_HEAD + ledger)


RUST_OK = "/// CALLER-CONTRACT: x is valid.\n/// # Errors\n#[must_use]\npub(crate) fn foo(x: u32) {}\n"


class Tests(unittest.TestCase):
    def test_definition_vs_citation(self):
        text = (
            "    /// CALLER-CONTRACT: real one.\n    fn a() {}\n"
            "    // registration exists (CALLER-CONTRACT: above); text\n"
            "    // ordering is the CALLER-CONTRACT: above.\n"
            "    fn b() {}\n"
        )
        defs = list(cc.definitions(RS, text))
        self.assertEqual(defs, [(1, "a")])

    def test_comment_forms(self):
        for pre in ("///", "//!", "//", "/*", " *", "#"):
            with self.subTest(pre=pre):
                self.assertEqual(len(list(cc.definitions("f.h", f"{pre} CALLER-CONTRACT: z\nint f(void) {{\n"))), 1)

    def test_item_rust_fn_struct_impl(self):
        self.assertEqual(list(cc.definitions(RS, RUST_OK)), [(1, "foo")])
        self.assertEqual(list(cc.definitions(RS, "/// CALLER-CONTRACT: a\npub struct Bar {\n"))[0][1], "Bar")
        self.assertEqual(list(cc.definitions(RS, "/// CALLER-CONTRACT: a\nunsafe impl Send for Bar {}\n"))[0][1], "impl Send for Bar")

    def test_item_c_function(self):
        t = "/* CALLER-CONTRACT: a\n * more */\n__device__ static inline uint32_t kf_foo(uint32_t a)\n{\n"
        self.assertEqual(list(cc.definitions(C, t)), [(1, "kf_foo")])

    def test_item_define(self):
        self.assertEqual(list(cc.definitions("broker/a.h", "/* CALLER-CONTRACT: a */\n#define KF_MAX 4\n"))[0][1], "KF_MAX")

    def test_c_marker_inside_body_has_no_item(self):
        t = "void f(void)\n{\n    /* CALLER-CONTRACT: a */\n    if (x) {}\n}\nvoid g(void) {}\n"
        self.assertEqual(list(cc.definitions(C, t)), [(3, None)])

    def test_clean(self):
        self.assertEqual(run(RS, RUST_OK, f"{RS}\tfoo\t1\treason\n"), [])

    def test_missing_row_fails(self):
        e = run(RS, RUST_OK, "")
        self.assertEqual(len(e), 1)
        self.assertIn(f"{RS}:1", e[0])
        self.assertIn("no row", e[0])

    def test_stale_row_fails(self):
        e = run(RS, RUST_OK, f"{RS}\tfoo\t1\tr\n{RS}\tgone\t1\tr\n")
        self.assertEqual(len(e), 1)
        self.assertIn("stale", e[0])

    def test_count_mismatch_fails(self):
        e = run(RS, RUST_OK, f"{RS}\tfoo\t2\tr\n")
        self.assertEqual(len(e), 1)
        self.assertIn("says 2", e[0])
        two = RUST_OK.replace("/// # Errors", "/// CALLER-CONTRACT: y.\n/// # Errors")
        self.assertEqual(run(RS, two, f"{RS}\tfoo\t2\tr\n"), [])
        self.assertTrue(run(RS, two, f"{RS}\tfoo\t1\tr\n"))

    def test_empty_reason_fails(self):
        e = run(RS, RUST_OK, f"{RS}\tfoo\t1\t \n")
        self.assertEqual(len(e), 1)
        self.assertIn("empty reason", e[0])

    def test_malformed_row_fails(self):
        self.assertTrue(run(RS, RUST_OK, f"{RS}\tfoo\tx\tr\n"))

    def test_no_item_within_window_fails(self):
        t = "/// CALLER-CONTRACT: a\n" + "// filler\n" * 45 + "fn late() {}\n"
        e = run(RS, t, "")
        self.assertEqual(len(e), 1)
        self.assertIn("no item", e[0])
        self.assertIn(f"{RS}:1", e[0])

    def test_citation_only_file_needs_no_row(self):
        self.assertEqual(run(RS, "// see (CALLER-CONTRACT: above)\nfn a() {}\n", ""), [])

    def test_scope(self):
        self.assertTrue(cc.in_scope("crates/kf-cuda/x.rs"))
        self.assertTrue(cc.in_scope("qemu/hw/misc/kf3/a.c"))
        self.assertFalse(cc.in_scope("archive/crates/a.rs"))
        self.assertFalse(cc.in_scope("third_party/x/crates/a.rs"))
        self.assertFalse(cc.in_scope("scripts/ci/caller_contracts.py"))

    def test_only_tracked_files_scanned(self):
        with tempfile.TemporaryDirectory() as d:
            def git(*a):
                subprocess.run(["git", "-C", d, *a], check=True, capture_output=True)
            git("init", "-q")
            os.makedirs(os.path.join(d, "crates/x"))
            for n in ("tracked.rs", "untracked.rs"):
                with open(os.path.join(d, "crates/x", n), "w") as f:
                    f.write(RUST_OK)
            git("add", "crates/x/tracked.rs")
            got = [p for p, _ in cc.files_worktree(cc.Path(d))]
            self.assertEqual(got, ["crates/x/tracked.rs"])

    def real(self):
        return list(cc.files_worktree(cc.ROOT)), (cc.ROOT / cc.LEDGER).read_text()

    def test_real_tree_extra_row_is_stale(self):
        f, led = self.real()
        e = cc.check(f, led + "crates/kf-cuda/src/driver_unsafe.rs\tnot_there\t1\tr\n")
        self.assertEqual(len(e), 1)
        self.assertIn("stale", e[0])

    def test_real_tree_wrong_count_fails(self):
        f, led = self.real()
        first = [l for l in led.splitlines() if l and not l.startswith("#")][0]
        bad = first.replace("\t1\t", "\t3\t", 1)
        e = cc.check(f, led.replace(first, bad))
        self.assertEqual(len(e), 1)
        self.assertIn("says 3", e[0])

    def test_real_tree_missing_row_fails(self):
        f, led = self.real()
        first = [l for l in led.splitlines() if l and not l.startswith("#")][0]
        e = cc.check(f, led.replace(first + "\n", ""))
        self.assertEqual(len(e), 1)
        self.assertIn("no row", e[0])

    def test_real_ledger_empty_reason_fails(self):
        f, led = self.real()
        first = [l for l in led.splitlines() if l and not l.startswith("#")][0]
        e = cc.check(f, led.replace(first, "\t".join(first.split("\t")[:3]) + "\t"))
        self.assertEqual(len(e), 1)
        self.assertIn("empty reason", e[0])

    def test_marker_at_end_of_file_has_no_item(self):
        e = run(RS, "fn a() {}\n/// CALLER-CONTRACT: dangling\n", "")
        self.assertEqual(len(e), 1)
        self.assertIn("no item", e[0])

    def test_window_edge(self):
        near = "/// CALLER-CONTRACT: a\n" + "// f\n" * 38 + "fn ok() {}\n"  # item on line 40
        far = "/// CALLER-CONTRACT: a\n" + "// f\n" * 40 + "fn no() {}\n"
        self.assertEqual(run(RS, near, f"{RS}\tok\t1\tr\n"), [])
        self.assertTrue(any("no item" in x for x in run(RS, far, "")))

    def test_c_marker_in_body_fails_check(self):
        t = "void f(void)\n{\n    /* CALLER-CONTRACT: a */\n    if (x) {}\n}\nvoid g(void) {}\n"
        e = run(C, t, "")
        self.assertEqual(len(e), 1)
        self.assertIn(f"{C}:3", e[0])
        self.assertIn("no item", e[0])

    def test_untracked_marker_does_not_fail(self):
        with tempfile.TemporaryDirectory() as d:
            def git(*a):
                subprocess.run(["git", "-C", d, *a], check=True, capture_output=True)
            git("init", "-q")
            os.makedirs(os.path.join(d, "crates/x"))
            with open(os.path.join(d, "crates/x/u.rs"), "w") as f:
                f.write(RUST_OK)
            self.assertEqual(cc.check(cc.files_worktree(cc.Path(d)), LEDGER_HEAD), [])

    def test_real_tree_passes(self):
        e = cc.check(cc.files_worktree(cc.ROOT), (cc.ROOT / cc.LEDGER).read_text())
        self.assertEqual(e, [])

    def test_real_ledger_has_reasons_and_header(self):
        text = (cc.ROOT / cc.LEDGER).read_text()
        self.assertTrue(text.startswith("#"))
        rows, errs = cc.read_ledger(text)
        self.assertEqual(errs, [])
        self.assertTrue(rows)


if __name__ == "__main__":
    unittest.main()
