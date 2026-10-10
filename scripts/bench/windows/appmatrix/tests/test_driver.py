# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The driver (winapps.py) end to end against the mock guest: batching, TDR budget, reboot, wedge, hang, VM death,
stage failure, boot failure, phase-2 isolation, resume. No VM, no GPU, no Windows."""
import contextlib
import io
import json
import os
import re
import tempfile
import unittest

import helpers as H
import winapps


def run(apps, scenario=None, extra=(), run_dir=None):
    d = run_dir or tempfile.mkdtemp(prefix="kfwa-t-")
    sc = os.path.join(d, "scen.json")
    json.dump(scenario or {}, open(sc, "w"))
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        rc = winapps.main(["--vm", "mock", "--fast", "--run-dir", d, "--apps", ",".join(apps), "--mock-scenario", sc, *extra])
    return rc, d


def res(d, name="win.res"):
    r = {}
    p = os.path.join(d, name)
    if os.path.exists(p):
        for line in open(p):
            if line.startswith("APPRES "):
                kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
                r[kv["app"]] = kv
    return r


def slog(d):
    return open(os.path.join(d, "session.log")).read()


class Driver(unittest.TestCase):
    def test_all_pass_batched_over_two_guests(self):
        rc, d = run(["nvidia_smi", "deviceQuery_demo", "matmul_t", "vkpeak"], extra=["--per-guest", "2"])
        self.assertEqual(rc, 0)
        r = res(d)
        self.assertEqual({k: v["verdict"] for k, v in r.items()}, {"nvidia_smi": "PASS", "deviceQuery_demo": "PASS", "matmul_t": "PASS", "vkpeak": "PASS"})
        self.assertEqual(sorted({v["boot"] for v in r.values()}), ["g1", "g2"])
        self.assertFalse(os.path.exists(os.path.join(d, "win_isolated.res")))
        for a in r:
            self.assertTrue(os.path.exists(os.path.join(d, "apps", a + ".log")))
            self.assertTrue(os.path.exists(os.path.join(d, "apps", a + ".verdict.json")))
        self.assertIn("EXIT rc=0", slog(d))
        self.assertEqual(json.load(open(os.path.join(d, "matrix.json")))["apps"][0], "nvidia_smi")

    def test_interactive_apps_need_the_signed_in_session(self):
        rc, d = run(["glgears_wgl"])                       # session=interactive: kf_launch refuses unless the keystroke sign-in worked
        self.assertEqual(res(d)["glgears_wgl"]["verdict"], "PASS")

    def test_tdr_budget_moves_the_rest_to_a_fresh_guest(self):
        sc = {"dxprobe_d3d11": {"kind": "tdr", "n": 2}, "dxprobe_d3d12": {"kind": "tdr", "n": 2}}
        rc, d = run(["dxprobe_d3d11", "dxprobe_d3d12", "matmul_t"], sc, ["--per-guest", "5", "--max-tdr", "3"])
        r = res(d)
        self.assertEqual([r[a]["verdict"] for a in ("dxprobe_d3d11", "dxprobe_d3d12", "matmul_t")], ["PASS"] * 3)
        self.assertEqual((r["dxprobe_d3d11"]["guest_tdr"], r["dxprobe_d3d12"]["guest_tdr"]), ("2", "2"))
        self.assertEqual((r["dxprobe_d3d11"]["boot"], r["dxprobe_d3d12"]["boot"], r["matmul_t"]["boot"]), ("g1", "g1", "g2"))
        self.assertIn("tdr-budget:4>=3", slog(d))

    def test_reboot_is_a_fail_then_alone_in_a_fresh_guest_it_passes(self):
        sc = {"vkpeak": {"kind": "reboot", "once": True}}
        rc, d = run(["nvidia_smi", "vkpeak", "matmul_t"], sc, ["--per-guest", "3"])
        r, ri = res(d), res(d, "win_isolated.res")
        self.assertEqual(r["vkpeak"]["verdict"], "FAIL")
        self.assertIn("rebooted", r["vkpeak"]["note"])
        self.assertEqual(r["matmul_t"]["verdict"], "PASS")
        self.assertNotEqual(r["matmul_t"]["boot"], r["vkpeak"]["boot"])         # the app after the reboot ran in a fresh guest
        self.assertEqual(ri["vkpeak"]["verdict"], "PASS")                       # the re-run alone is the final verdict
        self.assertTrue(ri["vkpeak"]["boot"].startswith("i"))
        self.assertIn("guest-rebooted", slog(d))

    def test_wedged_agent_is_detected_and_the_run_continues(self):
        sc = {"scan_t": {"kind": "wedge", "once": True}}
        rc, d = run(["scan_t", "matmul_t"], sc, ["--per-guest", "2"])
        r = res(d)
        self.assertEqual(r["scan_t"]["verdict"], "FAIL")
        self.assertRegex(r["scan_t"]["note"], "agent-lost|unresponsive")
        self.assertEqual(r["matmul_t"]["verdict"], "PASS")
        self.assertNotEqual(r["scan_t"]["boot"], r["matmul_t"]["boot"])
        self.assertEqual(res(d, "win_isolated.res")["scan_t"]["verdict"], "PASS")

    def test_hang_with_a_healthy_guest_is_a_timeout(self):
        rc, d = run(["llama_cuda_bench"], {"llama_cuda_bench": {"kind": "hang"}}, ["--per-guest", "1"])
        self.assertEqual(res(d)["llama_cuda_bench"]["verdict"], "TIMEOUT")

    def test_vm_death_mid_app(self):
        rc, d = run(["nvidia_smi", "matmul_t"], {"nvidia_smi": {"kind": "qemu_exit", "once": True}}, ["--per-guest", "2"])
        r = res(d)
        self.assertEqual(r["nvidia_smi"]["verdict"], "FAIL")
        self.assertEqual(r["matmul_t"]["verdict"], "PASS")

    def test_stage_failure_is_notrun_not_a_gpu_result(self):
        rc, d = run(["clpeak_ocl", "matmul_t"], {"_stage_fail": ["clpeak_ocl"]})
        r = res(d)
        self.assertEqual(r["clpeak_ocl"]["verdict"], "NOTRUN")
        self.assertIn("stage-failed:clpeak_ocl", r["clpeak_ocl"]["note"])
        self.assertEqual(r["matmul_t"]["verdict"], "PASS")
        self.assertFalse(os.path.exists(os.path.join(d, "win_isolated.res")))      # NOTRUN is not re-run

    def test_no_proof_and_wrong_adapter_are_failures(self):
        rc, d = run(["edge_webgl", "oceanFFT_demo"], {"edge_webgl": "nogpu", "oceanFFT_demo": "warp"}, ["--no-isolate"])
        r = res(d)
        self.assertIn("no-gpu-proof", r["edge_webgl"]["note"])
        self.assertIn("non-NVIDIA-adapter", r["oceanFFT_demo"]["note"])

    def test_guests_that_never_come_up_give_boot_fail_then_abort(self):
        rc, d = run(["nvidia_smi", "matmul_t", "scan_t", "sort_t"], {"_boot_fail": 99}, ["--per-guest", "1"])
        self.assertEqual(rc, 3)
        r = res(d)
        self.assertTrue(all(v["verdict"] == "BOOT_FAIL" for v in r.values()))
        self.assertIn("MATRIX_ABORT", slog(d))

    def test_one_dead_guest_then_recovery(self):
        rc, d = run(["nvidia_smi", "matmul_t"], {"_boot_fail": 1}, ["--per-guest", "2"])
        self.assertEqual(rc, 0)
        r = res(d)
        self.assertEqual(r["nvidia_smi"]["verdict"], "BOOT_FAIL")        # the first app of the failed boot, as in the Linux harness
        self.assertEqual(r["matmul_t"]["verdict"], "PASS")

    def test_resume_skips_what_is_done(self):
        rc, d = run(["nvidia_smi"])
        self.assertEqual(res(d)["nvidia_smi"]["verdict"], "PASS")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            winapps.main(["--vm", "mock", "--fast", "--run-dir", d, "--apps", "nvidia_smi,matmul_t", "--resume"])
        lines = [l for l in open(os.path.join(d, "win.res")) if l.startswith("APPRES")]
        self.assertEqual(sorted(re.search(r"app=(\S+)", l).group(1) for l in lines), ["matmul_t", "nvidia_smi"])   # nvidia_smi was not run twice

    def test_password_never_appears_in_the_run_dir(self):
        rc, d = run(["nvidia_smi"])
        for root, _, files in os.walk(d):
            for f in files:
                if f.endswith((".log", ".json", ".res")):
                    self.assertNotIn("mockpw", open(os.path.join(root, f), errors="replace").read(), f)

    def test_list_mode(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = winapps.main(["--run-dir", tempfile.mkdtemp(), "--list", "--tier", "1"])
        self.assertEqual(rc, 0)
        self.assertIn("nvidia_smi", out.getvalue())
        self.assertNotIn("vkcube", out.getvalue())          # tier 2


class Helpers(unittest.TestCase):
    def test_keys_for(self):
        self.assertEqual(winapps.keys_for("ab1"), [(None, "a"), (None, "b"), (None, "1")])
        self.assertEqual(winapps.keys_for("A"), [("shift", "a")])
        with self.assertRaises(ValueError):
            winapps.keys_for("é")

    def test_parse_marker(self):
        self.assertEqual(winapps.parse_marker('junk\nKFSETUP {"ok": true}\n', "KFSETUP"), {"ok": True})
        self.assertIsNone(winapps.parse_marker("nothing", "KFSETUP"))
        self.assertIsNone(winapps.parse_marker("KFSETUP {broken", "KFSETUP"))

    def test_closure(self):
        self.assertEqual(winapps.closure(["b", "a"], {"b": ["a"], "a": []}), ["a", "b"])

    def test_health_ok(self):
        ok = {"display": [{"problem": "CM_PROB_NONE"}], "smi_ok": True}
        self.assertEqual(winapps.GuestSession.health_ok(ok), (True, ""))
        self.assertEqual(winapps.GuestSession.health_ok({"display": [{"problem": "CM_PROB_FAILED_START"}], "smi_ok": True})[1], "display-problem:CM_PROB_FAILED_START")
        self.assertEqual(winapps.GuestSession.health_ok({"unreachable": True})[1], "agent-unreachable")
        self.assertEqual(winapps.GuestSession.health_ok({"display": [], "smi_ok": True})[1], "no-nvidia-display-device")


if __name__ == "__main__":
    unittest.main()
