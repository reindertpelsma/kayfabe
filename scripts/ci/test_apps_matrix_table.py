#!/usr/bin/env python3
"""scripts/apps/matrix_table.py, GPU-free: the cause class of a failing app is decided mechanically from
the bare-metal baseline of the same box (CLAUDE.md, "bare metal first").

FALSIFIER (written before the code): a host-PASS app that fails alone in a fresh guest boot is called
anything but KAYFABE_DEFECT; an app that fails on the host is called a kayfabe defect; a batched failure
that passes alone is hidden (it must be ORDER_SENSITIVE and listed); a policy skip, an app with no host
row or no usable guest row becomes a pass or a gate member.
"""

import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("matrix_table", HERE.parent / "apps/matrix_table.py")
mt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mt)


def res(side, app, verdict, secs=3, extra=""):
    return "APPRES side=%s app=%s verdict=%s rc=0 secs=%d quiet=0 note=-%s\n" % (side, app, verdict, secs, extra)


GUEST = " boot=t_b1 guest_xid=%d kf3_lines=2 kf3_refusals=0"


class Classify(unittest.TestCase):
    def cls(self, host, batched, alone):
        h = {"verdict": host} if host else None
        g = {"verdict": batched} if batched else None
        i = {"verdict": alone} if alone else None
        return mt.classify("app", h, g, i)[0]

    def test_host_pass_guest_pass_is_gate(self):
        self.assertEqual(self.cls("PASS", "PASS", None), "GATE")
        self.assertEqual(self.cls("PASS", "PASS", "PASS"), "GATE")

    def test_batched_pass_wins_over_an_alone_failure_only_when_there_is_none(self):
        # an isolated FAIL of an app that passed batched is still a PASS in a shared boot: R2/R3 rule
        self.assertEqual(self.cls("PASS", "PASS", "FAIL"), "GATE")

    def test_host_pass_guest_fails_alone_is_a_kayfabe_defect(self):
        for alone in ("FAIL", "TIMEOUT", "HANG"):
            self.assertEqual(self.cls("PASS", "FAIL", alone), "KAYFABE_DEFECT", alone)

    def test_batched_fail_alone_pass_is_order_sensitive_and_never_hidden(self):
        self.assertEqual(self.cls("PASS", "TIMEOUT", "PASS"), "ORDER_SENSITIVE")

    def test_a_host_failure_is_a_documented_exception_whatever_the_guest_did(self):
        for host in ("FAIL", "TIMEOUT", "NOTRUN"):
            for guest in ("PASS", "FAIL"):
                self.assertEqual(self.cls(host, guest, None), "HOST_LIMIT", (host, guest))

    def test_no_host_row_means_no_baseline(self):
        self.assertEqual(self.cls(None, "FAIL", "FAIL"), "UNATTRIBUTED")

    def test_a_dead_boot_without_a_clean_rerun_is_unattributed(self):
        self.assertEqual(self.cls("PASS", "GUEST_DEAD", None), "UNATTRIBUTED")
        self.assertEqual(self.cls("PASS", "FAIL", "BOOT_FAIL"), "UNATTRIBUTED")
        self.assertEqual(self.cls("PASS", None, None), "UNATTRIBUTED")

    def test_uvm_stays_unsupported_even_with_a_recorded_guest_failure(self):
        for name in mt.KNOWN_UNSUPPORTED:
            cls, final, reason = mt.classify(name, {"verdict": "PASS"}, {"verdict": "FAIL"}, {"verdict": "FAIL"})
            self.assertEqual((cls, final), ("KNOWN_UNSUPPORTED", "-"))
            self.assertIn("unsupported (UVM)", reason)

    def test_uvm_is_not_a_gate_pass_even_if_a_guest_prints_pass(self):
        for name in mt.KNOWN_UNSUPPORTED:
            self.assertEqual(mt.classify(name, {"verdict": "PASS"}, {"verdict": "PASS"}, None)[0], "KNOWN_UNSUPPORTED")

    def test_policy_skip_is_not_a_result(self):
        self.assertEqual(mt.classify("geekbench_gpu", {"verdict": "PASS"}, {"verdict": "PASS"}, None)[0], "POLICY_SKIP")


class Table(unittest.TestCase):
    def table(self, host, guest, isolated=""):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "host.res").write_text(host)
            Path(d, "guest.res").write_text(guest)
            if isolated:
                Path(d, "guest_isolated.res").write_text(isolated)
            Path(d, "nbody.host.log").write_text("=== x\nnbody 123.4 GFLOP/s\nCHECK nbody ok\n")
            return mt.table(d)

    def test_rows_columns_and_key_lines(self):
        rows = self.table(
            res("host", "vectorAdd", "PASS") + res("host", "nbody", "PASS") + res("host", "bad", "FAIL"),
            res("guest", "vectorAdd", "PASS", extra=GUEST % 0)
            + res("guest", "nbody", "FAIL", extra=GUEST % 2)
            + res("guest", "bad", "FAIL", extra=GUEST % 0),
            res("guest", "nbody", "FAIL", extra=GUEST % 2),
        )
        by = {r["app"]: r for r in rows}
        self.assertEqual(by["vectorAdd"]["class"], "GATE")
        self.assertEqual(by["nbody"]["class"], "KAYFABE_DEFECT")
        self.assertEqual(by["nbody"]["guest_xid"], "2")
        self.assertEqual(by["nbody"]["key_host"], "nbody 123.4 GFLOP/s")
        self.assertEqual(by["bad"]["class"], "HOST_LIMIT")
        self.assertEqual(by["geekbench_gpu"]["class"], "POLICY_SKIP")

    def test_all_unsupported_apps_are_visible_even_without_input_rows(self):
        by = {r["app"]: r for r in self.table("", "")}
        for name in mt.KNOWN_UNSUPPORTED:
            self.assertEqual(by[name]["class"], "KNOWN_UNSUPPORTED")
            self.assertIn("unsupported (UVM)", by[name]["reason"])

    def test_unsupported_missing_rows_are_listed_beside_a_supported_app(self):
        by = {r["app"]: r for r in self.table(res("host", "a", "PASS"), res("guest", "a", "PASS"))}
        self.assertTrue(set(mt.KNOWN_UNSUPPORTED) <= set(by))
        self.assertEqual(by["a"]["class"], "GATE")

    def test_unsupported_keeps_the_historical_observed_columns(self):
        by = {r["app"]: r for r in self.table(res("host", "attach_verify", "PASS"), res("guest", "attach_verify", "FAIL"))}
        self.assertEqual(by["attach_verify"]["class"], "KNOWN_UNSUPPORTED")
        self.assertEqual(by["attach_verify"]["guest_batched"], "FAIL")

    def test_the_last_row_of_an_app_wins(self):
        rows = self.table(res("host", "a", "FAIL") + res("host", "a", "PASS"), res("guest", "a", "PASS"))
        self.assertEqual({r["app"]: r["class"] for r in rows}["a"], "GATE")

    def test_the_tsv_has_one_line_per_app_and_no_tabs_in_cells(self):
        text = mt.tsv(self.table(res("host", "a", "PASS"), res("guest", "a", "PASS")))
        lines = text.splitlines()
        self.assertEqual(lines[0].split("\t"), mt.COLUMNS)
        self.assertTrue(all(len(x.split("\t")) == len(mt.COLUMNS) for x in lines))

    def test_the_gate_list_holds_only_gate_apps_in_run_order(self):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "host.res").write_text(res("host", "a", "PASS") + res("host", "b", "PASS") + res("host", "c", "PASS"))
            Path(d, "guest.res").write_text(res("guest", "a", "PASS") + res("guest", "b", "FAIL") + res("guest", "c", "PASS"))
            Path(d, "guest_isolated.res").write_text(res("guest", "b", "FAIL"))
            out = Path(d, "gate.txt")
            self.assertEqual(mt.main([d, "--out", str(Path(d, "t.tsv")), "--gate-list", str(out)]), 0)
            self.assertEqual([line for line in out.read_text().splitlines() if not line.startswith("#")], ["a", "c"])
            self.assertIn("known_unsupported", out.read_text())
            for name in mt.KNOWN_UNSUPPORTED:
                self.assertIn(name + ": unsupported (UVM)", out.read_text())


if __name__ == "__main__":
    unittest.main()
