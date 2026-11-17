# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kfopen.py: the command line, NOT_RUN decisions and the row dispatch with fakes (no Windows, no network)."""
import argparse
import contextlib
import io
import json
import os
import tempfile
import unittest
from unittest import mock

import helpers as H
import cts_results as R
import kfopen as K
import kfopen_build as B
import kfopen_pins as P
from kfopen_common import Result, Trust

PINS = P.load_pins()
UAV_OK = "".join("    UAV[%d] = 0x%x\n" % (i, 4 + i) for i in range(8)) + "\n Execution complete\n"


def ctx_for(tmp, runner=None, trust=None):
    return B.Ctx(PINS, root=tmp, runner=runner or H.FakeRunner(), fetcher=H.FakeFetcher(), trust=trust or Trust(), log=lambda *a: None)


def run_main(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = K.main(argv)
    return rc, out.getvalue(), err.getvalue()


class Tmp(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory(prefix="kfopen-cli-")
        self.tmp = self._t.name
        self.addCleanup(self._t.cleanup)


class Commands(Tmp):
    def test_check_is_clean(self):
        self.assertEqual([], K.check_all())
        rc, out, _ = run_main(["check"])
        self.assertEqual(0, rc)
        self.assertIn("OPEN_ROWS_OK", out)

    def test_list_names_every_row(self):
        rc, out, _ = run_main(["list"])
        self.assertEqual(0, rc)
        for rid in PINS["rows"]:
            self.assertIn(rid, out)

    def test_plan_prints_json_with_anchors(self):
        rc, out, _ = run_main(["plan", "dxsamples_work_graphs"])
        self.assertEqual(0, rc)
        self.assertIn("nuget-signature", {s["anchor"] for s in json.loads(out)})

    def test_unknown_row_is_refused_with_exit_2(self):
        rc, _, err = run_main(["run", "nope", "--root", self.tmp])
        self.assertEqual(2, rc)
        self.assertIn("unknown row", err)

    def test_bad_trust_flag_is_refused_with_exit_2(self):
        rc, _, err = run_main(["prepare", "vkglcts_vk", "--root", self.tmp, "--source-commit-override", "cts=short"])
        self.assertEqual(2, rc)
        self.assertIn("REFUSED", err)

    def test_diff_command_exit_codes(self):
        def write(name, mapping):
            p = os.path.join(self.tmp, name)
            open(p, "w").write(R.format_results({"row": "r"}, mapping))
            return p
        n = write("n.tsv", {"a.b.c": "Pass", "a.b.d": "Pass"})
        self.assertEqual(0, run_main(["diff", "--native", n, "--guest", write("g1.tsv", {"a.b.c": "Pass", "a.b.d": "Pass"})])[0])
        self.assertEqual(1, run_main(["diff", "--native", n, "--guest", write("g2.tsv", {"a.b.c": "Pass", "a.b.d": "Crash"})])[0])
        self.assertEqual(3, run_main(["diff", "--native", n, "--guest", write("g3.tsv", {"a.b.c": "Pass"})])[0])


class NotRun(Tmp):
    def test_multiadapter_is_notrun(self):
        rc, out, _ = run_main(["run", "dxsamples_multiadapter", "--root", self.tmp])
        self.assertEqual(3, rc)
        self.assertIn("KFNOTRUN dxsamples_multiadapter reason=needs-two-gpus", out)

    def test_multiadapter_names_reason(self):
        why = K.preflight(ctx_for(self.tmp), "dxsamples_multiadapter", dry_run=True)
        self.assertIn("D3D12HeterogeneousMultiadapter", why)
        self.assertIn("D3D12LinkedGpus", why)

    def test_not_windows_is_notrun(self):
        with mock.patch.object(K.sys, "platform", "linux"):
            self.assertIn("not-windows", K.preflight(ctx_for(self.tmp), "vkglcts_gl46"))

    def test_offline_without_tree_is_notrun(self):
        with mock.patch.object(K.sys, "platform", "win32"), mock.patch.object(K, "can_reach", lambda *a, **k: False):
            why = K.preflight(ctx_for(self.tmp), "vkglcts_gl46")
        self.assertIn("offline-guest-without-build-tree", why)

    def test_offline_prepare_is_notrun(self):
        with mock.patch.object(K.sys, "platform", "win32"), mock.patch.object(K, "can_reach", lambda *a, **k: False):
            rc, out, _ = run_main(["prepare", "dxsamples_nbody_async", "--root", self.tmp])
        self.assertEqual(3, rc)
        self.assertIn("KFNOTRUN dxsamples_nbody_async reason=offline-guest-without-build-tree", out)

    def test_a_prepared_tree_is_enough_when_offline(self):
        os.makedirs(os.path.join(self.tmp, "build"))
        with mock.patch.object(K.sys, "platform", "win32"), mock.patch.object(K, "can_reach", lambda *a, **k: False):
            self.assertIsNone(K.preflight(ctx_for(self.tmp), "vkglcts_gl46"))

    def test_prebuilt_directory_is_trusted_and_needs_no_network(self):
        with mock.patch.object(K.sys, "platform", "win32"), mock.patch.object(K, "can_reach", lambda *a, **k: False):
            self.assertIsNone(K.preflight(ctx_for(self.tmp, trust=Trust(prebuilt=self.tmp)), "vkglcts_gl46"))

    def test_prebuilt_records_the_hash_and_builds_nothing(self):
        exe = os.path.join(self.tmp, "external", "openglcts", "modules", "Release", "glcts.exe")
        os.makedirs(os.path.dirname(exe))
        open(exe, "wb").write(b"my glcts")
        r = H.FakeRunner()
        ctx = ctx_for(self.tmp, runner=r, trust=Trust(prebuilt=self.tmp))
        paths = K.prepare(ctx, "vkglcts_gl46")
        self.assertEqual(exe, paths["exe"])
        self.assertEqual([], r.calls)
        rec = ctx.receipt["builds"]["prebuilt:glcts.exe"]
        self.assertEqual("trusted-by-user", rec["verified"])
        self.assertEqual(64, len(rec["sha256"]))

    def test_prebuilt_without_the_binary_is_refused(self):
        from kfopen_common import OpenRowError
        with self.assertRaises(OpenRowError):
            K.prepare(ctx_for(self.tmp, trust=Trust(prebuilt=self.tmp)), "vkglcts_vk")


class Dispatch(Tmp):
    def args(self, **kw):
        base = dict(out=os.path.join(self.tmp, "out"), control=None, role="guest", allow_different_gpu=False, smoke=False, share=False)
        base.update(kw)
        return argparse.Namespace(**base)

    def run_row(self, rid, runner, **kw):
        ctx = ctx_for(self.tmp, runner=runner)
        with mock.patch.object(K, "preflight", lambda *a, **k: None), mock.patch.object(K, "prepare", lambda c, r: {"exe": "x.exe", "cwd": self.tmp}):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = K.run_row(ctx, rid, self.args(**kw))
        return rc, buf.getvalue()

    def test_work_graphs_pass_prints_the_verdict_line_the_row_expects(self):
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(0, UAV_OK)))
        self.assertEqual(0, rc)
        self.assertIn("KFOPEN VERDICT PASS row=dxsamples_work_graphs uav=8", out)

    def test_work_graph_notrun_exit_3(self):
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(-1, "Device does not report support for work graphs.")))
        self.assertEqual(3, rc)
        self.assertIn("KFNOTRUN dxsamples_work_graphs reason=work-graphs-tier-not-supported", out)

    def test_a_graph_that_wrote_nothing_is_a_fail_end_to_end(self):
        zero = "".join("    UAV[%d] = 0x0\n" % i for i in range(8)) + "Execution complete\n"
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(0, zero)))
        self.assertEqual(1, rc)
        self.assertIn("CHECK uav_written FAIL", out)

    def test_a_sample_that_cannot_be_started_is_a_failed_check(self):
        ctx = ctx_for(self.tmp)
        with mock.patch.object(K, "preflight", lambda *a, **k: None), mock.patch.object(K, "prepare", lambda c, r: {"exe": os.path.join(self.tmp, "no-such.exe"), "cwd": self.tmp}):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = K.run_row(ctx, "dxsamples_nbody_async", self.args())
        self.assertEqual(1, rc)
        self.assertIn("CHECK sample_started_window FAIL could not start", buf.getvalue())
        self.assertIn("KFOPEN VERDICT FAIL row=dxsamples_nbody_async", buf.getvalue())

    def test_work_graph_failure_is_fail_with_check_lines(self):
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(0, "Execution complete\n")))
        self.assertEqual(1, rc)
        self.assertIn("CHECK uav_lines FAIL", out)

    def test_work_graph_control_is_compared(self):
        ctl = os.path.join(self.tmp, "c.json")
        json.dump({"uav": [[i, 9 + i] for i in range(8)]}, open(ctl, "w"))
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(0, UAV_OK)), control=ctl)
        self.assertEqual(1, rc)
        self.assertIn("CHECK equals_control FAIL", out)

    def test_native_role_writes_the_control_file(self):
        rc, out = self.run_row("dxsamples_work_graphs", H.FakeRunner(default=Result(0, UAV_OK)), role="native")
        doc = json.load(open(os.path.join(self.tmp, "out", "dxsamples_work_graphs.native.json")))
        self.assertEqual(8, len(doc["uav"]))

    def test_prepare_failure_is_a_failed_check_not_a_crash(self):
        from kfopen_common import OpenRowError

        def boom(c, r):
            raise OpenRowError("BUILD_FAILED sample nbody rc=1")
        ctx = ctx_for(self.tmp)
        with mock.patch.object(K, "preflight", lambda *a, **k: None), mock.patch.object(K, "prepare", boom):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = K.run_row(ctx, "vkglcts_gl46", self.args())
        self.assertEqual(1, rc)
        self.assertIn("CHECK prepare_vkglcts_gl46 FAIL BUILD_FAILED", buf.getvalue())

    def test_soak_row_runs_the_binary_with_the_seconds_of_the_row(self):
        ok = "KFGL renderer NVIDIA X\nKFGL RESULT OK iters=1/1\n"

        def script(argv, cwd):
            return Result(0, "[]") if argv[0] == "powershell.exe" else Result(0, ok, 605.0)
        r = H.FakeRunner([(lambda a: True, script)])
        rc, out = self.run_row("vkglcts_gl_dual_soak", r)
        self.assertEqual(0, rc)
        self.assertEqual(["x.exe", "--seconds", "600"], r.argvs()[0])


if __name__ == "__main__":
    unittest.main()
