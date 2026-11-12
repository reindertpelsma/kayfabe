"""Hostile fake Worker against the git-only (--trust-git-only, profile borrowed) daemon, with a REAL local git
repository and the real JobRunner/fetch path (only `execute`, reporting and the report POST are recording stubs).

FALSIFIER: any case below reaches JobRunner.execute (something ran), a result is posted for it, or a
tree_key_mismatch appears anywhere but the run-report call (it must never be in a result).
"""

import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kfhost import config, downloads, fetch, jobs, profile  # noqa: E402
from kfhost.profile import Refused  # noqa: E402
from kfhost.test_kfhost import Base, Closed, REPO, offer, welcome  # noqa: E402
from kfhost.treekey import commit_tree_key  # noqa: E402

GOOD_SEED = {"url": "http://93.184.216.34:40022/ab12cd34/kf3.tar", "fallback_url": None, "sha256": "d" * 64, "size": 10,
             "name": "kf3.tar"}


def git(cwd, *args):
    env = {"PATH": "/usr/bin:/bin", "HOME": str(cwd), "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@e", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@e"}
    return subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True).stdout.strip()


class RecordingRunner(jobs.JobRunner):
    executed = []
    reports = []

    def __init__(self, cfg, caps):
        super().__init__(cfg, caps, reporting=self.Rep)
        self.hardware_fn = lambda: {"gpu_name": "G", "compute_cap": 890, "driver_version": "580.1"}
        self.report_fn = lambda cfg_, offer_, lines, result, mismatch: RecordingRunner.reports.append((result, mismatch))

    class Rep:
        posted = []

        def __init__(self, cfg, offer_):
            pass

        def progress(self, *a):
            pass

        def feed(self, *a):
            pass

        def finish(self, result, note):
            RecordingRunner.Rep.posted.append(result)

    def execute(self, ctx, checkout, rep, scratch):
        RecordingRunner.executed.append(ctx.offer["offer_id"])
        return 0


class HostileWorker(Base):
    @classmethod
    def setUpClass(cls):
        cls.repo = tempfile.mkdtemp(prefix="hostile-repo-")
        git(cls.repo, "init", "-q", "-b", "master")
        for n in ("one", "two"):
            (Path(cls.repo) / (n + ".txt")).write_text(n)
            git(cls.repo, "add", ".")
            git(cls.repo, "commit", "-qm", n)
        cls.c1 = git(cls.repo, "rev-parse", "HEAD~1")
        cls.c2 = git(cls.repo, "rev-parse", "HEAD")
        git(cls.repo, "checkout", "-q", "-b", "evil", cls.c1)
        (Path(cls.repo) / "evil.txt").write_text("evil")
        git(cls.repo, "add", ".")
        git(cls.repo, "commit", "-qm", "evil")
        cls.evil = git(cls.repo, "rev-parse", "HEAD")
        git(cls.repo, "checkout", "-q", "master")
        cls.key = lambda cls, c: commit_tree_key(lambda a: subprocess.run(["git", *a], cwd=cls.repo, capture_output=True, check=True).stdout, c)

    @classmethod
    def tearDownClass(cls):
        import shutil
        shutil.rmtree(cls.repo, ignore_errors=True)

    def setUp(self):
        super().setUp()
        RecordingRunner.executed, RecordingRunner.reports, RecordingRunner.Rep.posted = [], [], []
        self.fetched = []
        real = fetch._git

        def to_local(args, cwd, timeout, raw=False, ok=(0,)):
            if args[:3] == ["remote", "add", "origin"]:
                self.fetched.append(args[3])
                args = ["remote", "add", "origin", self.repo]  # the allowlisted https URL is served by the local repo
            return real(args, cwd, timeout, raw, ok)

        fetch._git = to_local
        self.addCleanup(setattr, fetch, "_git", real)
        self.factory = lambda cfg, caps: RecordingRunner(cfg, caps)

    def attempt(self, **assignment_over):
        a = {"repo": REPO, "commit": self.c2, "tree_key": self.key(self.c2), "stages": ["merge_bar"]}
        a.update(assignment_over)
        o = offer(assignment=a, run={"nonce": "c" * 32, "runner_nonce": "d" * 64, "sha": a["commit"]})
        d, conn, _ = self.serve([welcome("borrowed"), o, lambda c: [time.sleep(0.05) for _ in range(100) if not {"ready", "nack"} & set(c.types())],
                                 None, Closed(4401)], ceiling="borrowed", euid=1000)
        d.log = lambda m: None
        d.run_forever(max_sessions=1)
        d.join()
        return conn

    def assert_refused_clean(self, conn, why):
        types = conn.types()
        self.assertIn("nack", types, why)
        self.assertNotIn("ready", types, why)
        self.assertEqual(RecordingRunner.executed, [], why)  # nothing ran
        self.assertEqual(RecordingRunner.Rep.posted, [], why)  # no result posted
        self.assertEqual(RecordingRunner.reports, [], why)  # no run-report either: nothing ran
        for r in RecordingRunner.Rep.posted:
            self.assertNotIn("tree_key_mismatch", r)

    def test_positive_control_good_commit_runs(self):
        conn = self.attempt()
        self.assertEqual(conn.types().count("ready"), 1, conn.types())
        self.assertEqual(len(RecordingRunner.executed), 1)
        (result, mismatch), = RecordingRunner.reports
        self.assertFalse(mismatch)
        self.assertEqual(sorted(result), ["hardware", "nonce", "runner_nonce", "sha", "suites", "verdict"])

    def test_commit_not_an_ancestor_of_master_or_next(self):
        conn = self.attempt(commit=self.evil, tree_key=self.key(self.evil))
        self.assert_refused_clean(conn, "non-ancestor commit")

    def test_wrong_tree_key_is_refused_not_run(self):
        conn = self.attempt(tree_key="e" * 64)
        self.assert_refused_clean(conn, "wrong tree key")
        self.assertTrue(any("tree key mismatch" in m.get("reason", "") for m in conn.sent if m["type"] == "nack"))

    def test_repo_not_in_allowlist_never_fetches(self):
        conn = self.attempt(repo="https://github.com/evil/kayfabe")
        self.assert_refused_clean(conn, "other repo")
        self.assertEqual(self.fetched, [])  # no git command touched the network
        self.assertNotIn("ack", conn.types())

    def test_seed_with_private_ip_never_fetches(self):
        for url in ("http://10.0.0.5:40022/ab12cd34/kf3.tar", "http://192.168.1.2:40022/ab12cd34/kf3.tar",
                    "http://127.0.0.1:40022/ab12cd34/kf3.tar", "http://169.254.169.254:80/ab12cd34/kf3.tar"):
            RecordingRunner.executed.clear()
            self.fetched.clear()
            conn = self.attempt(seed={**GOOD_SEED, "url": url})
            self.assert_refused_clean(conn, url)
            self.assertEqual(self.fetched, [], url)
        for prof in ("standard", "owned", "container"):  # refused by shape for every profile, not only borrowed
            a = {"repo": REPO, "commit": "a" * 40, "tree_key": "b" * 64, "seed": {**GOOD_SEED, "url": "http://10.0.0.5:1/ab12cd34/kf3.tar"}}
            with self.assertRaises(Refused):
                profile.check_offer(prof, offer(assignment=a), self.cfg_std(), int(time.time() * 1000))

    def cfg_std(self):
        from kfhost.test_kfhost import cfg_for
        return cfg_for(self.tmp.name, "standard")

    def test_any_seed_is_refused_under_git_only_even_a_well_formed_public_one(self):
        conn = self.attempt(seed=GOOD_SEED)
        self.assert_refused_clean(conn, "seed under borrowed")
        self.assertTrue(any("artifacts from other boxes" in m.get("reason", "") for m in conn.sent if m["type"] == "nack"))

    def test_artifact_with_wrong_sha256_is_never_accepted(self):
        calls = []
        pol = downloads.DownloadPolicy("borrowed", self.repo, log=lambda m: None, run=lambda *a, **k: calls.append(a))
        with self.assertRaises(Refused):
            pol.fetch(GOOD_SEED["url"], "0" * 64, 10, Path(self.tmp.name) / "x", source="worker")
        self.assertEqual(calls, [])  # kf_fetch was never invoked
        # a standard-profile consume of a bad artifact leaves no file and no seed path (kf_fetch exits non-zero)
        cfg = self.cfg_std()
        r = RecordingRunner(cfg, {})
        ctx = jobs.JobContext(offer(assignment={"repo": REPO, "commit": self.c2, "tree_key": "b" * 64, "seed": GOOD_SEED}), "standard", cfg, None,
                              lambda: False)
        ctx.log = lambda m: None
        ctx.downloads = downloads.DownloadPolicy("standard", self.repo, log=lambda m: None,
                                                 run=lambda argv, **k: type("R", (), {"returncode": 5})())
        r.consume_seed(ctx, self.tmp.name)
        self.assertFalse(hasattr(ctx, "seed_path"))

    def test_tree_key_mismatch_reaches_only_the_run_report_in_non_strict_profiles(self):
        a = {"repo": REPO, "commit": self.c2, "tree_key": "e" * 64, "stages": ["merge_bar"]}
        o = offer(assignment=a, run={"nonce": "c" * 32, "runner_nonce": "d" * 64, "sha": self.c2})
        d, conn, _ = self.serve([welcome("standard"), o, lambda c: [time.sleep(0.05) for _ in range(100) if not RecordingRunner.reports],
                                 None, Closed(4401)], ceiling="standard")
        d.log = lambda m: None
        d.run_forever(max_sessions=1)
        d.join()
        (result, mismatch), = RecordingRunner.reports
        self.assertTrue(mismatch)
        self.assertNotIn("tree_key_mismatch", result)
        for r in RecordingRunner.Rep.posted:
            self.assertNotIn("tree_key_mismatch", r)


class EnrolCommandFlags(unittest.TestCase):
    def test_bootstrap_accepts_exactly_the_flags_the_worker_enrol_command_prints(self):
        from kfhost import bootstrap
        # the flag list of the Worker's enrol command (hostbox.js, PR #347): same names, in its order
        args = bootstrap.parser().parse_args(
            ["--repo", REPO, "--commit", "a" * 40, "--worker-url", "https://w.example", "--box-id", "rack-1",
             "--token-file", "kfhost.token", "--access-id", "id", "--access-secret-file", "accessfile"])
        self.assertEqual((args.repo, args.access_id, args.access_secret_file), (REPO, "id", "accessfile"))


if __name__ == "__main__":
    unittest.main()
