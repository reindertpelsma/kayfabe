"""2026-11-17: kfml inside the offline guest (scripts/ci/kfml_guest.py), GPU-free, with a fake guest transport. Nothing here boots a guest.

FALSIFIER (written before the code): the flavour is chosen from the host instead of the guest; a box with no pinned flavour is a FAIL or copies
anything; the install is not offline and hashed; a wheelhouse that fails its manifest check reaches pip; a guest that derives another flavour
than the host shipped installs the wrong wheels silently; the datasets are fetched from the network; the guest's rows are not read back or
a FAIL row passes; a secret travels to the guest; a die or a driver is named in the code.
"""
import io
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import kfml_guest as kg
import kfml_stage as ks
import vast_report
from test_kfml_stage import Args, ml_row

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SHA = "a" * 40
SMI = "NVIDIA Test Card, 555.55.01, 12282, 8.9\n"
HEAD = "| NVIDIA-SMI 555.55.01   Driver Version: 555.55.01   CUDA Version: 12.6 |\n"
WH_DOC = {"flavors": ["cu126"], "files": 40, "total_bytes": 4_000_000_000, "build_s": 90.0}


class FakeGuest:
    """A transport that answers by command content; records everything the stage sends."""

    def __init__(self, smi=SMI, head=HEAD, py="3.12\n", have="gcc=1\npython_h=1\nvenv=1\nglibc=glibc 2.39\ndisk_kb=%d\n" % (200 * 2**20), rows=None,
                 verify_rc=0, setup_rc=0, setup_out="", fetch_rc=0, run_rc=0, smi_rc=0):
        self.cmds, self.puts = [], []
        self.smi, self.head, self.py, self.have, self.smi_rc = smi, head, py, have, smi_rc
        self.rows = [ml_row()] if rows is None else rows
        self.verify_rc, self.setup_rc, self.setup_out, self.fetch_rc, self.run_rc = verify_rc, setup_rc, setup_out, fetch_rc, run_rc

    def run(self, cmd, timeout):
        self.cmds.append(cmd)
        if cmd.startswith("nvidia-smi --query"):
            return self.smi_rc, self.smi
        if cmd.startswith("nvidia-smi |"):
            return 0, self.head
        if "sys.version_info" in cmd and "echo" not in cmd:
            return 0, self.py
        if "echo gcc=" in cmd:
            return 0, self.have
        if "wheelhouse --out" in cmd and "--verify" in cmd:
            return self.verify_rc, "verify said so\n"
        if " setup " in cmd:
            return self.setup_rc, self.setup_out
        if "fetch-data" in cmd:
            return self.fetch_rc, ""
        if " run --tier" in cmd:
            return self.run_rc, ""
        if cmd.startswith("cat ") and "rows.json" in cmd:
            return 0, json.dumps(self.rows)
        return 0, ""

    def put_tree(self, local, remote, timeout):
        self.puts.append((str(local), remote))
        return {"bytes": 1000 if remote.endswith("src") else 4_000_000_000, "s": 2.0 if remote.endswith("src") else 40.0}


class FakeHost:
    """subprocess.run stand-in for the host side (kfml wheelhouse, kfml list)."""

    def __init__(self, rc=0, doc=None, reason=None):
        self.calls, self.rc, self.doc, self.reason = [], rc, WH_DOC if doc is None else doc, reason

    def __call__(self, cmd, **kw):
        self.calls.append(cmd)

        class P:
            returncode, stdout, stderr = 0, "", ""
        p = P()
        if "wheelhouse" in cmd:
            p.returncode = self.rc
            p.stdout = json.dumps({"status": "NOT_RUN", "reason": self.reason}) + "\n" if self.rc == 3 else json.dumps(self.doc) + "\n"
        elif "list" in cmd:
            p.stdout = json.dumps({"tests": [{"name": "nanogpt_compile"}, {"name": "stress_alloc"}]})
        return p


class Flow(unittest.TestCase):
    def go(self, guest=None, host=None, tier="mid"):
        guest, host = guest or FakeGuest(), host or FakeHost()
        with tempfile.TemporaryDirectory() as d:
            doc = kg.run_guest(Args(d, tier=tier), guest, host_py="hostpy", run=host)
        return doc, guest, host

    def test_green_flow_order_and_commands(self):
        doc, g, h = self.go()
        self.assertEqual(doc["verdict"], "PASS")
        self.assertEqual(doc["where"], "guest")
        self.assertEqual([s["step"] for s in doc["steps"]], ["probe", "wheelhouse", "copy", "verify", "setup", "fetch-data", "run"])
        self.assertEqual([p[1] for p in g.puts], ["/opt/kfml/src", "/opt/kfml/wheelhouse"])
        setup = next(c for c in g.cmds if " setup " in c)
        self.assertIn("--wheelhouse /opt/kfml/wheelhouse", setup)
        self.assertIn("--nvcc-if-needed", setup)
        self.assertNotIn("--flavor", setup)  # the guest derives it from its own nvidia-smi
        self.assertIn("fetch-data --from /opt/kfml/wheelhouse/data", next(c for c in g.cmds if "fetch-data" in c))
        for c in g.cmds:
            if " run --tier" in c or " setup " in c:
                self.assertIn("KF_ML_GUEST=1", c)
        self.assertIn("--tier mid", next(c for c in g.cmds if " run --tier" in c))
        self.assertNotIn("--allow-unhashed", setup)
        vast_report.validate_kfml(doc)

    def test_setup_is_an_offline_wheelhouse_install_of_the_hashed_lock(self):
        _, g, _ = self.go()
        setup = next(c for c in g.cmds if " setup " in c)
        self.assertIn("--wheelhouse", setup)
        self.assertNotIn("--allow-unhashed", setup)
        self.assertNotIn("--installer", setup)

    def test_guest_data_comes_from_the_wheelhouse_not_the_network(self):
        _, g, _ = self.go()
        fetch = next(c for c in g.cmds if "fetch-data" in c)
        self.assertIn("--from", fetch)
        self.assertIn("/wheelhouse/data", fetch)
        self.assertNotRegex(fetch, r"https?://")

    def test_the_flavour_comes_from_the_guests_facts_not_the_hosts(self):
        _, _, h = self.go(FakeGuest(smi="NVIDIA Other, 600.1, 24576, 12.0\n", head="CUDA Version: 13.0 |\n", py="3.10\n"))
        cmd = next(c for c in h.calls if "wheelhouse" in c)
        self.assertEqual(cmd[cmd.index("--flavor-for") + 1], "13.0:12.0")
        self.assertEqual(cmd[cmd.index("--guest-python") + 1], "3.10")
        self.assertEqual(cmd[cmd.index("--guest-glibc") + 1], "2.39")
        self.assertIn("--with-data", cmd)
        self.assertIn("auto", cmd)
        self.assertFalse([a for a in cmd if re.fullmatch(r"cu1\d\d", a)])

    def test_wheelhouse_command_carries_the_guest_facts_of_the_default_fake(self):
        _, _, h = self.go()
        cmd = next(c for c in h.calls if "wheelhouse" in c)
        self.assertEqual(cmd[cmd.index("--flavor-for") + 1], "12.6:8.9")
        self.assertEqual(cmd[cmd.index("--guest-python") + 1], "3.12")

    NO_FLAVOUR = "no pinned flavour can run on this box: cu128: driver supports CUDA 12.6 < 12.8"

    def test_no_flavour_fits_is_a_not_run_verdict_with_the_reason(self):
        doc, _, _ = self.go(host=FakeHost(rc=3, reason=self.NO_FLAVOUR))
        self.assertEqual(doc["verdict"], "NOT_RUN")
        self.assertIn("no pinned flavour", doc["reason"])
        vast_report.validate_kfml(doc)

    def test_no_flavour_fits_still_says_which_guest_it_was(self):
        doc, _, _ = self.go(host=FakeHost(rc=3, reason=self.NO_FLAVOUR))
        self.assertEqual(doc["facts"]["driver_cuda"], "12.6")
        self.assertEqual(doc["facts"]["compute_cap"], "8.9")

    def test_no_flavour_fits_copies_and_installs_nothing(self):
        doc, g, _ = self.go(host=FakeHost(rc=3, reason=self.NO_FLAVOUR))
        self.assertEqual(g.puts, [])
        self.assertFalse([c for c in g.cmds if " setup " in c or " run --tier" in c])

    def test_missing_guest_pieces_are_named_fails_before_any_download(self):
        for kw, word in (({"have": "gcc=\npython_h=1\nvenv=1\nglibc=glibc 2.39\ndisk_kb=999999999\n"}, "C++ compiler"),
                         ({"have": "gcc=1\npython_h=\nvenv=1\nglibc=glibc 2.39\ndisk_kb=999999999\n"}, "python3-dev"),
                         ({"have": "gcc=1\npython_h=1\nvenv=\nglibc=glibc 2.39\ndisk_kb=999999999\n"}, "python3-venv"),
                         ({"head": "no cuda line"}, "CUDA Version"), ({"py": "oops"}, "python3 not usable")):
            doc, g, h = self.go(FakeGuest(**kw))
            self.assertEqual(doc["verdict"], "FAIL", kw)
            self.assertIn(word, doc["reason"])
            self.assertFalse([c for c in h.calls if "wheelhouse" in c])
            self.assertEqual(g.puts, [])

    def test_unreadable_nvidia_smi_output_stops_before_the_host_does_any_work(self):
        doc, g, h = self.go(FakeGuest(smi="garbage"))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("not understood", doc["reason"])
        self.assertEqual((h.calls, g.puts), ([], []))

    def test_no_nvidia_smi_in_the_guest_is_a_fail(self):
        doc, _, h = self.go(FakeGuest(smi_rc=127))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("nvidia-smi", doc["reason"])
        self.assertEqual(h.calls, [])

    def test_a_guest_disk_that_cannot_hold_it_is_refused_before_copying(self):
        doc, g, _ = self.go(FakeGuest(have="gcc=1\npython_h=1\nvenv=1\nglibc=glibc 2.39\ndisk_kb=%d\n" % (5 * 2**20)))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("guest-disk", doc["reason"])
        self.assertEqual(g.puts, [])

    def test_the_disk_need_scales_with_the_wheelhouse(self):
        have = "gcc=1\npython_h=1\nvenv=1\nglibc=glibc 2.39\ndisk_kb=%d\n" % (12 * 2**20)  # 12 GiB free
        small = dict(WH_DOC, total_bytes=1_000_000_000)
        doc, _, _ = self.go(FakeGuest(have=have), FakeHost(doc=small))
        self.assertEqual(doc["verdict"], "PASS")
        doc, _, _ = self.go(FakeGuest(have=have), FakeHost(doc=dict(WH_DOC, total_bytes=4_000_000_000)))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("guest-disk", doc["reason"])

    def test_host_wheelhouse_failure_is_a_fail(self):
        doc, g, _ = self.go(host=FakeHost(rc=1))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("host wheelhouse failed", doc["reason"])
        self.assertEqual(g.puts, [])

    def test_a_copy_that_fails_the_manifest_check_never_reaches_pip(self):
        doc, g, _ = self.go(FakeGuest(verify_rc=1))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("verify failed", doc["reason"])
        self.assertFalse([c for c in g.cmds if " setup " in c])

    def test_a_guest_that_derives_another_flavour_is_a_named_fail(self):
        doc, _, _ = self.go(FakeGuest(setup_rc=1, setup_out="error: the wheelhouse has no flavour cu128 (has cu126)\n"))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("wheelhouse-flavour-mismatch", doc["reason"])

    def test_the_mismatch_reason_names_what_the_host_shipped(self):
        doc, _, _ = self.go(FakeGuest(setup_rc=1, setup_out="error: the wheelhouse has no flavour cu128 (has cu126)\n"))
        self.assertIn("the host shipped cu126", doc["reason"])

    def test_guest_setup_exit_3_is_not_run(self):
        doc, g, _ = self.go(FakeGuest(setup_rc=3))
        self.assertEqual(doc["verdict"], "NOT_RUN")
        self.assertFalse([c for c in g.cmds if " run --tier" in c])

    def test_guest_setup_exit_3_names_the_reason_and_validates(self):
        doc, _, _ = self.go(FakeGuest(setup_rc=3))
        self.assertIn("no pinned flavour", doc["reason"])
        self.assertTrue(all(r["status"] == "NOT_RUN" and r["reason"] for r in doc["rows"]))
        vast_report.validate_kfml(doc)

    def test_other_setup_failure_is_a_fail(self):
        doc, _, _ = self.go(FakeGuest(setup_rc=1, setup_out="pip exploded\n"))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("guest setup failed", doc["reason"])

    def test_failed_offline_data_is_a_fail_and_nothing_runs(self):
        doc, g, _ = self.go(FakeGuest(fetch_rc=1))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("fetch-data failed", doc["reason"])
        self.assertFalse([c for c in g.cmds if " run --tier" in c])

    def test_failed_offline_data_step_is_recorded_with_its_rc(self):
        doc, _, _ = self.go(FakeGuest(fetch_rc=7))
        self.assertEqual(doc["steps"][-1], dict(doc["steps"][-1], step="fetch-data", rc=7))

    def test_every_kfml_command_in_the_guest_carries_guest_mode(self):
        _, g, _ = self.go()
        kf = [c for c in g.cmds if "-m kfml" in c]
        self.assertGreaterEqual(len(kf), 4)
        for c in kf:
            self.assertIn("KF_ML_GUEST=1", c)

    def test_a_fail_row_fails_the_stage(self):
        doc, _, _ = self.go(FakeGuest(rows=[ml_row(), ml_row("b", "FAIL", "boom")], run_rc=1))
        self.assertEqual(doc["verdict"], "FAIL")

    def test_the_no_rows_record_validates_and_says_why(self):
        doc, _, _ = self.go(FakeGuest(rows=[]))
        vast_report.validate_kfml(doc)

    def test_pass_and_not_run_rows_pass_the_stage(self):
        doc, _, _ = self.go(FakeGuest(rows=[ml_row("a", "NOT_RUN", "cc"), ml_row("b")]))
        self.assertEqual(doc["verdict"], "PASS")
        self.assertEqual([r["status"] for r in doc["rows"]], ["NOT_RUN", "PASS"])

    def test_no_rows_is_a_fail(self):
        doc, _, _ = self.go(FakeGuest(rows=[]))
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("no rows", doc["reason"])

    def test_a_crashing_run_cannot_pass(self):
        doc, _, _ = self.go(FakeGuest(run_rc=137))
        self.assertEqual(doc["verdict"], "FAIL")

    def test_a_run_that_timed_out_cannot_pass(self):
        doc, _, _ = self.go(FakeGuest(run_rc=124))
        self.assertEqual(doc["verdict"], "FAIL")

    def test_the_verify_reason_carries_the_guests_output(self):
        doc, _, _ = self.go(FakeGuest(verify_rc=1))
        self.assertIn("verify said so", doc["reason"])

    def test_transfer_record_has_the_sizes_and_times(self):
        doc, _, _ = self.go()
        t = doc["transfer"]
        self.assertEqual(t["wheelhouse_bytes"], 4_000_000_000)
        self.assertEqual(t["copy_bytes"], 4_000_001_000)
        self.assertEqual(t["copy_s"], 42.0)
        self.assertEqual(t["wheelhouse_build_s"], 90.0)
        self.assertIn("install_s", t)

    def test_no_secret_goes_to_the_guest(self):
        with patch.dict(os.environ, {"VAST_API_TOKEN": "hunter2", "AWS_SECRET_ACCESS_KEY": "hunter3"}):
            _, g, h = self.go()
        sent = " ".join(g.cmds) + " ".join(str(x) for p in g.puts for x in p) + " ".join(" ".join(c) for c in h.calls)
        self.assertNotIn("hunter", sent)
        self.assertNotRegex(sent, r"TOKEN|SECRET|PASSWORD|API_KEY")

    def test_no_die_or_driver_is_named_in_the_module(self):
        text = (HERE / "kfml_guest.py").read_text()
        self.assertIsNone(re.search(r"RTX|GeForce|Tesla|Blackwell|\b[0-9]{3}\.[0-9]{2}\b|sm_[0-9]|cu1[0-9]{2}", text))


class Facts(unittest.TestCase):
    def test_parse(self):
        f = kg.parse_facts(SMI, HEAD, "3.12\n", {"gcc": "1", "python_h": "1", "venv": "1", "glibc": "glibc 2.35"}, "1048576")
        self.assertEqual(f["glibc"], "2.35")
        self.assertEqual((f["gpu_name"], f["driver"], f["cc"], f["driver_cuda"], f["python"], f["vram_gb"]),
                         ("NVIDIA Test Card", "555.55.01", (8, 9), "12.6", "3.12", 11.99))
        self.assertEqual(f["disk_free_bytes"], 1048576 * 1024)
        self.assertEqual(kg.fact_problems(f), [])

    def test_a_guest_whose_glibc_is_unreadable_is_a_named_fail(self):
        f = kg.parse_facts(SMI, HEAD, "3.12\n", {"gcc": "1", "python_h": "1", "venv": "1", "glibc": "musl"}, "1048576")
        self.assertIn("guest-glibc", " ".join(kg.fact_problems(f)))

    def test_unreadable_output_is_none_not_a_guess(self):
        f = kg.parse_facts("", "", "", {}, "x")
        self.assertIsNone(f["cc"])
        self.assertIsNone(f["driver_cuda"])
        self.assertIsNone(f["disk_free_bytes"])
        self.assertGreaterEqual(len(kg.fact_problems(f)), 5)


class Transport(unittest.TestCase):
    class P:
        def __init__(self, rc=0, stdout=None):
            self.returncode, self.stdout, self.waited = rc, stdout or io.BytesIO(), 0

        def wait(self, timeout=None):
            self.waited += 1
            return self.returncode

        def kill(self):
            pass

    def test_put_tree_is_tar_into_ssh_tar_and_counts_bytes(self):
        procs, argvs = [], []

        def popen(argv, **kw):
            argvs.append((argv, kw))
            p = self.P()
            procs.append(p)
            return p
        tr = kg.SshTransport(["gssh", "-x"], popen=popen)
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "a").write_bytes(b"x" * 10)
            (Path(d) / "sub").mkdir()
            (Path(d) / "sub" / "b").write_bytes(b"y" * 5)
            r = tr.put_tree(d, "/opt/kfml/wheelhouse", 60)
        self.assertEqual(r["bytes"], 15)
        self.assertEqual(argvs[0][0][:3], ["tar", "-C", d])
        recv = argvs[1][0]
        self.assertEqual(recv[:2], ["gssh", "-x"])
        self.assertIn("tar -C /opt/kfml/wheelhouse -xf -", recv[2])

    def test_put_tree_ssh_failure_alone_raises(self):
        seq = iter([self.P(0), self.P(1)])
        tr = kg.SshTransport(["gssh"], popen=lambda a, **k: next(seq))
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(kg.GuestError):
                tr.put_tree(d, "/x", 60)

    def test_put_tree_tar_failure_alone_raises(self):
        seq = iter([self.P(2), self.P(0)])
        tr = kg.SshTransport(["gssh"], popen=lambda a, **k: next(seq))
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(kg.GuestError):
                tr.put_tree(d, "/x", 60)

    def test_run_returns_rc_and_output_and_124_on_timeout(self):
        class R:
            returncode, stdout, stderr = 7, "out", "err"
        tr = kg.SshTransport(["gssh"], run=lambda *a, **k: R())
        self.assertEqual(tr.run("cmd", 5), (7, "outerr"))

        def boom(*a, **k):
            raise subprocess.TimeoutExpired("x", 1)
        self.assertEqual(kg.SshTransport(["gssh"], run=boom).run("cmd", 1)[0], 124)


class Hook(unittest.TestCase):
    def test_the_hook_is_short_linear_glue_over_the_stage_and_the_payloads_transport(self):
        path = HERE / "kfml_guest_hook.sh"
        text = path.read_text()
        self.assertLess(len(text.splitlines()), 40)
        self.assertEqual(subprocess.run(["bash", "-n", str(path)]).returncode, 0)
        self.assertIn("set -euo pipefail", text)
        self.assertIn("--where guest", text)
        self.assertIn("scripts/bench/gssh_nv", text)
        self.assertNotRegex(text, r"TOKEN|SECRET|cu1[0-9]{2}|RTX")
        self.assertTrue(os.access(path, os.X_OK))

    def test_the_hook_refuses_to_run_without_a_sha(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
        p = subprocess.run(["bash", str(HERE / "kfml_guest_hook.sh"), "t1"], env=env, capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("KF_KFML_SHA", p.stderr)

    def test_the_payloads_transport_script_exists(self):
        self.assertTrue((ROOT / "scripts" / "bench" / "gssh_nv").is_file())


class Main(unittest.TestCase):
    def test_where_guest_dispatches_with_the_payloads_own_transport(self):
        seen = {}

        def fake(args, tr):
            seen["ssh"] = tr.ssh
            return ks.failure(SHA, "mid", [], "x")
        with tempfile.TemporaryDirectory() as d, patch.object(kg, "run_guest", fake), patch.dict(os.environ, {}, clear=False):
            os.environ.pop("KF_GUEST_SSH", None)
            out = io.StringIO()
            import contextlib
            with contextlib.redirect_stdout(out):
                rc = ks.main(["run", "--sha", SHA, "--checkout", str(ROOT), "--prefix", d + "/p", "--out", d + "/o.json", "--where", "guest"])
        self.assertEqual(rc, 1)
        self.assertEqual(seen["ssh"], [str(ROOT / "scripts" / "bench" / "gssh_nv")])
        self.assertIn("KF_KFML_RESULT", out.getvalue())

    def test_transfer_keys_are_an_allowlist(self):
        doc = ks.summarize([ml_row()], SHA, "mid", [{"step": "run", "rc": 0, "s": 1.0}], "ci1")
        doc["transfer"] = {"token": 1}
        with self.assertRaises(ValueError):
            vast_report.validate_kfml(doc)

    def test_guest_ssh_flag_overrides(self):
        seen = {}
        with tempfile.TemporaryDirectory() as d, patch.object(kg, "run_guest", lambda a, tr: seen.setdefault("ssh", tr.ssh) and ks.failure(SHA, "mid", [], "x")):
            ks.main(["run", "--sha", SHA, "--checkout", str(ROOT), "--prefix", d + "/p", "--out", d + "/o.json", "--where", "guest", "--guest-ssh", "ssh -p 2222 u@h"])
        self.assertEqual(seen["ssh"], ["ssh", "-p", "2222", "u@h"])

    def test_the_guest_record_validates_in_the_reporter(self):
        doc = ks.summarize([ml_row()], SHA, "mid", [{"step": "probe", "rc": 0, "s": 1.0}, {"step": "wheelhouse", "rc": 0, "s": 90.0},
                                                  {"step": "copy", "rc": 0, "s": 40.0}], "ci1")
        doc["where"], doc["transfer"] = "guest", {"wheelhouse_bytes": 4_000_000_000, "copy_s": 40.0}
        vast_report.validate_kfml(doc)
        for bad in ({"where": "moon"}, {"transfer": {"secret": 1}}, {"transfer": {"copy_s": "40"}}, {"transfer": {"copy_s": -1}}):
            d = dict(doc, **bad)
            with self.assertRaises(ValueError):
                vast_report.validate_kfml(d)


if __name__ == "__main__":
    unittest.main()
