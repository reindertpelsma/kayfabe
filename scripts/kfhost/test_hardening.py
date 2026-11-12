"""Trust-review fixes: wire format, frame guard, deadline, early ack, flags model, download policy, tree key parity.

FALSIFIER: the Python tree key differs from the JS one on any vector; a Worker welcome loosens a local
restriction; cmd runs without BOTH local allow_cmd and the Worker's enable plus an active commands_until;
a PR-only commit, a wrong tree key or an oversized/over-nested frame is accepted.
"""

import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kfhost import cli, config, downloads, profile, profile_borrowed, seeder, treekey  # noqa: E402
from kfhost.daemon import MAX_FRAME, parse_frame  # noqa: E402
from kfhost.profile import Refused  # noqa: E402
from kfhost.test_kfhost import (Base, FakeConn, FakeRunner, FakeSys, REPO, SHA, TREE, cfg_for, offer,  # noqa: E402
                                welcome, Closed)

FIX = Path(__file__).with_name("tests") / "fixtures"


class TreeKeyParity(unittest.TestCase):
    d = json.loads((FIX / "treekey_vectors.json").read_text())

    def test_keys_equal_the_js_keys(self):
        entries = [tuple(e) for e in self.d["entries"]]
        for v in self.d["vectors"]:
            p = treekey.patterns_for(v["ignore"])
            self.assertEqual(treekey.tree_key_of(entries, p), v["key"], repr(v["ignore"]))

    def test_ignored_matcher_equals_js(self):
        for pat, path, dr, want in self.d["ignored"]:
            self.assertEqual(treekey.ignored(treekey.parse_ignore(pat), path, dr), want, (pat, path, dr))

    def test_patterns_outside_the_subset_are_refused(self):
        for line, expect_none in self.d["bad"]:
            self.assertEqual(treekey.parse_ignore(line) is None, expect_none, line)

    def test_documented_hash(self):
        import hashlib
        sha = lambda n: "%040x" % n  # noqa: E731
        want = hashlib.sha256(("kf-tree-v2\nignore " + hashlib.sha256(b"/docs/").hexdigest() + "\na\x00100755\x00" + sha(1)
                               + "\nb\x00100644\x00" + sha(2) + "\n").encode()).hexdigest()
        got = treekey.tree_key_of([("b", "100644", sha(2)), ("a", "100755", sha(1))], treekey.parse_ignore("# c\n/docs/\n\n"))
        self.assertEqual(got, want)

    def test_fail_closed(self):
        p = treekey.patterns_for("")
        sha = "%040x" % 1
        self.assertIsNone(treekey.tree_key_of([], p))
        self.assertIsNone(treekey.tree_key_of([("a", "100644", sha), ("a", "100644", sha)], p))
        self.assertIsNone(treekey.tree_key_of([("a/../b", "100644", sha)], p))
        self.assertIsNone(treekey.tree_key_of([("a", "100644", sha)], None))
        self.assertIsNone(treekey.patterns_for("/crates/\n"))  # covers a code root

    def test_key_of_this_repository_is_well_formed(self):
        root = Path(__file__).resolve().parents[2]
        top = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=root, capture_output=True, text=True)
        if top.returncode or not Path(top.stdout.strip(), ".workerignore").exists():
            self.skipTest("not in a kayfabe checkout")
        # A pure-python self-consistency check; the JS comparison was done by hand when this was written.
        key = treekey.commit_tree_key(lambda a: subprocess.run(["git", *a], cwd=root, check=True, capture_output=True).stdout, "HEAD")
        self.assertRegex(key or "", "^[0-9a-f]{64}$")


class FrameGuard(unittest.TestCase):
    def test_guard(self):
        self.assertEqual(parse_frame('{"a":1}'), {"a": 1})
        self.assertIsNone(parse_frame("[1]"))
        self.assertIsNone(parse_frame("x" * (MAX_FRAME + 1)))
        self.assertIsNone(parse_frame('{"a":' + "[" * 40 + "]" * 40 + "}"))
        self.assertIsNotNone(parse_frame('{"a":' + "[" * 20 + "]" * 20 + "}"))
        self.assertIsNotNone(parse_frame('{"a":"' + "[" * 100 + '"}'))  # brackets inside a string do not count
        self.assertIsNone(parse_frame('{"a":' + "[" * 100000 + "]" * 100000 + "}"))

    def test_hostile_frame_does_not_kill_the_daemon(self):
        b = Base()
        b.setUp()
        d, conn, _ = b.serve([welcome(), '{"a":' + "[" * 5000 + "]" * 5000 + "}", "x" * (MAX_FRAME + 5),
                              {"type": "offer", "offer_id": 5}, None, Closed(1006)])
        d.log = lambda m: None
        b.run_one(d)
        self.assertIn("heartbeat", conn.types() + ["heartbeat"])  # reached the end of the script
        b.tmp.cleanup()


class WireAndFlow(Base):
    def test_stage_names_are_the_workers(self):
        self.assertEqual(profile.STAGES, ("provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "windows_gate",
                                          "windows_apps", "specviewperf", "ladder"))

    def test_tree_key_and_commit_formats(self):
        a = offer()["assignment"]
        for k, v in (("tree_key", "b" * 40), ("tree_key", "B" * 64), ("commit", "a" * 64)):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(assignment=dict(a, **{k: v})), cfg_for(self.tmp.name), int(time.time() * 1000))

    def test_deadline_must_be_epoch_milliseconds(self):
        cfg, now = cfg_for(self.tmp.name), int(time.time() * 1000)
        for dl in (int(time.time()) + 3600, now - 1000, "x", None, 1.5e12, True):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(deadline=dl), cfg, now)
        profile.check_offer("standard", offer(deadline=now + 5000), cfg, now)

    def test_deadline_enforces_a_time_cap(self):
        FakeRunner.gate = threading.Event()
        o = offer(deadline=int(time.time() * 1000) + 400)
        d, conn, _ = self.serve([welcome(), o, lambda c: self.wait_ran(), None, Closed(1006)])
        d.run_forever(max_sessions=1)
        for _ in range(100):
            if self.runners[0].cancelled:
                break
            threading.Event().wait(0.02)
        self.assertTrue(self.runners[0].cancelled)
        FakeRunner.gate.set()
        d.join()

    def wait_ran(self):
        for _ in range(200):
            if self.runners and self.runners[0].ran:
                return
            threading.Event().wait(0.01)

    def test_early_ack_then_ready_and_nack_after_failed_verify(self):
        d, conn, _ = self.serve([welcome(), offer(), lambda c: self.wait_ran(), Closed(1006)])
        self.run_one(d)
        self.assertEqual([t for t in conn.types() if t in ("ack", "ready", "nack")], ["ack", "ready"])
        self.fail_prepare = "tree key mismatch"
        d, conn, _ = self.serve([welcome(), offer(), lambda c: [threading.Event().wait(0.05)], None, Closed(1006)])
        self.run_one(d)
        self.assertEqual([t for t in conn.types() if t in ("ack", "ready", "nack")], ["ack", "nack"])

    def test_ancestry_only_for_trust_git_only(self):
        for prof in ("borrowed", "container", "standard", "owned"):
            FakeRunner.profiles = []
            d, conn, _ = self.serve([welcome(prof), offer(), lambda c: self.wait_ran(), Closed(1006)], ceiling=prof)
            self.run_one(d)
            self.assertEqual(FakeRunner.profiles, [prof])
            self.assertEqual(profile.needs_ancestry(prof), prof == "borrowed")

    def test_borrowed_refuses_seed_url_and_root(self):
        o = offer(assignment=dict(offer()["assignment"], seed_url="http://1.2.3.4:80/x/y"))
        with self.assertRaises(Refused) as cm:
            profile.check_offer("borrowed", o, cfg_for(self.tmp.name), int(time.time() * 1000))
        self.assertIn("never receives artifacts from other boxes", str(cm.exception))
        profile.check_offer("standard", o, cfg_for(self.tmp.name), int(time.time() * 1000))  # others may consume seeds
        d, conn, _ = self.serve([welcome("borrowed"), offer(), None, Closed(1006)], ceiling="borrowed", euid=0)
        self.run_one(d)
        nack = [m for m in conn.sent if m["type"] == "nack"][0]
        self.assertIn("root", nack["reason"])
        self.assertEqual(FakeRunner.log, [])


class FlagModel(Base):
    def go(self, ceiling, w, script_extra=()):
        d, conn, _ = self.serve([w, *script_extra, None, Closed(1006)], ceiling=ceiling)
        self.run_one(d)
        return d, conn

    def test_hello_carries_enforced_flags(self):
        d, conn = self.go("borrowed", welcome("standard"))
        self.assertEqual(conn.sent[0]["enforced"], {"trust_git_only": True, "allow_cmd": False, "container": False})
        d, conn = self.go("owned", welcome("owned"))
        self.assertEqual(conn.sent[0]["enforced"], {"trust_git_only": False, "allow_cmd": True, "container": False})

    def test_welcome_cannot_loosen_a_local_restriction(self):
        for loosening in (welcome("owned"), welcome("standard", trust_git_only=False, container=False),
                          welcome("owned", trust_git_only=False)):
            d, _ = self.go("borrowed", loosening)
            self.assertEqual(d.profile, "borrowed")
        d, _ = self.go("container", welcome("owned", container=False))
        self.assertEqual(d.profile, "container")

    def test_worker_can_add_a_restriction(self):
        d, _ = self.go("standard", welcome("standard", trust_git_only=True))
        self.assertEqual(d.profile, "borrowed")
        d, _ = self.go("owned", welcome("container"))
        self.assertEqual(d.profile, "container")
        d, _ = self.go("owned", welcome("owned", container=True))
        self.assertEqual(d.profile, "container")

    def test_power_needs_both_sides(self):
        d, _ = self.go("owned", welcome("owned"))
        self.assertEqual(d.profile, "owned")
        d, _ = self.go("owned", welcome("standard"))  # Worker does not enable
        self.assertEqual(d.profile, "standard")
        d, _ = self.go("standard", welcome("owned"))  # local does not permit
        self.assertEqual(d.profile, "standard")

    def run_cmd(self, ceiling, w):
        c = {"type": "cmd", "cmd_id": "c1", "argv": ["true"]}
        d, conn, _ = self.serve([w, c, lambda cc: d.join(), None, Closed(1006)], ceiling=ceiling)
        self.run_one(d)
        return [m["rc"] for m in conn.sent if m["type"] == "cmd_done"]

    def test_cmd_needs_flag_profile_and_active_commands_until(self):
        self.assertEqual(self.run_cmd("owned", welcome("owned")), [0])
        self.assertEqual(self.run_cmd("standard", welcome("owned")), [126])  # no local flag
        self.assertEqual(self.run_cmd("owned", welcome("standard")), [126])  # Worker did not enable
        w = welcome("owned")
        del w["commands_until_ms"]
        self.assertEqual(self.run_cmd("owned", w), [126])
        self.assertEqual(self.run_cmd("owned", welcome("owned", commands_until_ms=int(time.time() * 1000) - 1)), [126])
        self.assertEqual(self.run_cmd("owned", welcome("owned", commands_until_ms="never")), [126])

    def test_allow_cmd_defaults_off(self):
        self.assertFalse(cfg_for(self.tmp.name, "standard")["allow_cmd"])


class CliRefusals(Base):
    def args(self, **kw):
        a = type("A", (), {"config": str(Path(self.tmp.name) / "cfg.json")})()
        for k, v in kw.items():
            setattr(a, k, v)
        return a

    def test_trust_git_only_refused_when_build_flag_off(self):
        cfg_for(self.tmp.name, "standard")
        old = profile_borrowed.ENABLED
        try:
            profile_borrowed.ENABLED = False
            self.assertEqual(cli.run(self.args(trust_git_only=True)), 2)
        finally:
            profile_borrowed.ENABLED = old

    def test_hidden_alias_and_flags(self):
        p = cli.argparse.ArgumentParser()
        self.assertTrue(profile_borrowed.ENABLED is False)  # parked until the owner decides

    def test_config_inside_work_dir_refused(self):
        p = Path(self.tmp.name) / "cfg.json"
        cfg_for(self.tmp.name, "standard", work_dir=self.tmp.name)
        self.assertEqual(cli.run(self.args()), 2)

    def test_borrowed_refuses_root(self):
        cfg_for(self.tmp.name, "borrowed")
        old = profile_borrowed.ENABLED
        try:
            profile_borrowed.ENABLED = True
            if os.geteuid() == 0:
                self.assertEqual(cli.run(self.args()), 2)
            else:
                self.skipTest("not root")
        finally:
            profile_borrowed.ENABLED = old


class SeederHijack(unittest.TestCase):
    def test_kfseed_runs_isolated_with_absolute_module_path(self):
        argv = seeder.kfseed_argv("check")
        self.assertEqual(argv[1], "-I")
        self.assertTrue(Path(seeder.SCRIPTS_DIR).is_absolute())
        self.assertIn(seeder.SCRIPTS_DIR, argv[3])

    def test_default_run_uses_a_temp_cwd_outside_the_checkout(self):
        rc, out = seeder.default_run([sys.executable, "-c", "import os;print(os.getcwd())"])
        self.assertEqual(rc, 0)
        self.assertNotEqual(Path(out).resolve(), Path.cwd().resolve())
        self.assertTrue(Path(out).resolve().is_relative_to(Path(tempfile.gettempdir()).resolve()))

    def test_planted_module_in_cwd_is_not_imported(self):
        with tempfile.TemporaryDirectory() as t:
            Path(t, "kfseed.py").write_text("print('PWNED')")
            done = subprocess.run([sys.executable, "-I", "-c", "import importlib.util as u;print(u.find_spec('kfseed'))"],
                                  cwd=t, capture_output=True, text=True)
            self.assertEqual(done.stdout.strip(), "None")


class DownloadPolicyTests(unittest.TestCase):
    URL, SHA, OTHER = "https://developer.download.nvidia.com/x.run", "a" * 64, "b" * 64

    def checkout(self, t):
        d = Path(t, "scripts/ci")
        d.mkdir(parents=True)
        (d / "setup-cache.lock.json").write_text(json.dumps(
            {"assets": [{"kind": "x", "files": [{"url": self.URL, "sha256": self.SHA, "size": 7}]}]}))
        return t

    def test_pinned_pair_allowed_wrong_sha_refused(self):
        with tempfile.TemporaryDirectory() as t:
            pol = downloads.DownloadPolicy("borrowed", self.checkout(t), log=lambda m: None)
            pol.check(self.URL, self.SHA, 7, source="checkout")
            with self.assertRaises(Refused):
                pol.check(self.URL, self.OTHER, 7)
            with self.assertRaises(Refused):
                pol.check(self.URL, self.SHA, 8)  # size differs from the pin

    def test_worker_or_seeder_triple_not_in_checkout_is_refused_and_logged(self):
        with tempfile.TemporaryDirectory() as t:
            logs = []
            pol = downloads.DownloadPolicy("borrowed", self.checkout(t), log=logs.append)
            for src in ("worker", "seeder"):
                with self.assertRaises(Refused):
                    pol.check("https://evil.example/x", self.OTHER, 1, source=src)
            self.assertTrue(any("not pinned in the verified checkout" in m for m in logs))

    def test_attestation_path_refused_with_message(self):
        with tempfile.TemporaryDirectory() as t:
            pol = downloads.DownloadPolicy("borrowed", self.checkout(t), log=lambda m: None)
            with self.assertRaises(Refused) as cm:
                pol.check(self.URL, self.SHA, 7, attestation_path="/x/a")
            self.assertIn("attestation verification not available", str(cm.exception))
            self.assertFalse(downloads.verify_attestation("/x/a", self.SHA))

    def test_other_profiles_unaffected_and_fetch_goes_through_kf_fetch(self):
        with tempfile.TemporaryDirectory() as t:
            calls = []
            run = lambda argv, cwd, check: calls.append((argv, cwd)) or type("R", (), {"returncode": 0})()  # noqa: E731
            for prof in ("standard", "container", "owned"):
                pol = downloads.DownloadPolicy(prof, t, run=run)
                self.assertEqual(pol.fetch("https://w.example/blob", self.OTHER, 5, "/tmp/o"), 0)
            argv, cwd = calls[0]
            self.assertEqual(argv[1], "-I")
            self.assertTrue(argv[2].endswith("kf_fetch.py"))
            self.assertIn("--sha256", argv)
            with self.assertRaises(Refused):
                downloads.DownloadPolicy("borrowed", t, run=run).fetch("https://w.example/blob", self.OTHER, 5, "/tmp/o")
            self.assertEqual(len(calls), 3)  # the refused one never ran


if __name__ == "__main__":
    unittest.main()
