"""Idle exit (injectable clock) and the bootstrap script (local git fixture, fakes). GPU-free, no network.

FALSIFIER: the daemon exits while a job or cmd runs, or never exits when idle; the clock does not restart after a
job; bootstrap runs with a wrong commit, changes anything on --dry-run, writes a config that is not 0600, or a
secret appears in argv or output.
"""

import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kfhost import bootstrap  # noqa: E402
from kfhost.test_kfhost import Base, FakeRunner, Closed, offer, welcome  # noqa: E402

TOKEN, SECRET = "box-token-SECRET-1234", "access-SECRET-5678"


class IdleExit(Base):
    def serve_idle(self, script, minutes, **kw):
        d, conn, tr = self.serve(script, exit_idle_s=minutes * 60, **kw)
        d.log = self.logs.append
        return d, conn, tr

    def setUp(self):
        super().setUp()
        self.logs = []

    def test_exits_zero_after_idle_minutes_with_log_line(self):
        d, conn, _ = self.serve_idle([welcome()] + [None] * 400, 2)
        self.assertEqual(d.run_forever(), 0)
        self.assertGreaterEqual(self.clock.t, 120)
        self.assertLess(self.clock.t, 125)
        self.assertTrue(any("idle for 2 minutes" in m for m in self.logs))

    def test_default_off_never_exits_idle(self):
        d, conn, _ = self.serve([welcome()] + [None] * 400 + [Closed(4401)])
        self.assertEqual(d.run_forever(), 3)  # ended by the script, not by idleness

    def test_job_resets_the_clock_and_blocks_exit(self):
        FakeRunner.gate = threading.Event()

        def finish(c):
            FakeRunner.gate.set()
            d.join()

        # 90 s idle, a job runs 'for' 200 s of fake time (busy: no exit), then the clock restarts at its end
        script = [welcome()] + [None] * 90 + [offer(), lambda c: [threading.Event().wait(0.1)]] + [None] * 200 + [finish]
        script += [None] * 400
        d, conn, _ = self.serve_idle(script, 2)
        self.assertEqual(d.run_forever(), 0)
        ended_at = d.idle_wall
        self.assertGreater(self.clock.t, 90 + 200 + 119)  # a full idle period AFTER the job ended
        self.assertTrue(any(m["type"] == "ready" for m in conn.sent))

    def test_heartbeat_reports_idle_since(self):
        d, conn, _ = self.serve_idle([welcome()] + [None] * 40 + [Closed(4401)], 60)
        d.run_forever()
        beats = [m for m in conn.sent if m["type"] == "heartbeat"]
        self.assertTrue(beats and isinstance(beats[0]["idle_since"], int))

    def test_running_cmd_blocks_exit(self):
        c = {"type": "cmd", "cmd_id": "00000000000000c1", "argv": [sys.executable, "-c", "import time;time.sleep(0.5)"]}
        d, conn, _ = self.serve_idle([welcome(), c] + [None] * 130 + [lambda x: d.join(), None, None], 2)
        d.run_forever()
        done = [m for m in conn.sent if m["type"] == "cmd_done"]
        self.assertEqual([m["rc"] for m in done], [0])  # not killed by the idle exit

    def test_cli_rejects_non_positive(self):
        from kfhost import cli
        self.assertEqual(cli.main(["run", "--config", "/nonexistent", "--exit-when-idle", "0"]), 2)


def git(cwd, *a):
    return subprocess.run(["git", "-c", "user.email=t@t", "-c", "user.name=t", *a], cwd=cwd, check=True,
                          capture_output=True, text=True).stdout.strip()


class Bootstrap(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        t = Path(self.tmp.name)
        self.src = t / "src"
        (self.src / "scripts" / "kfhost").mkdir(parents=True)
        git(self.src, "init", "-q", "-b", "master")
        git(self.src, "config", "uploadpack.allowAnySHA1InWant", "true")
        (self.src / "scripts" / "kfhost" / "x.py").write_text("1")
        git(self.src, "add", ".")
        git(self.src, "commit", "-qm", "one")
        self.sha = git(self.src, "rev-parse", "HEAD")
        self.tok, self.sec = t / "tok", t / "sec"
        for f, v in ((self.tok, TOKEN), (self.sec, SECRET)):
            f.write_text(v + "\n")
            os.chmod(f, 0o600)
        self.work = t / "box"
        self.calls, self.execs, self.out = [], [], []

    def go(self, extra=(), commit=None, **kw):
        argv = ["--repo", str(self.src), "--commit", commit or self.sha, "--worker-url", "https://w.example",
                "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                "--access-secret-file", str(self.sec), "--work-dir", str(self.work), *extra]
        run = lambda argv_, cwd, check: self.calls.append(list(argv_)) or type("R", (), {"returncode": 0})()  # noqa: E731
        return bootstrap.main(argv, run=run, execv=lambda p, a: self.execs.append(list(a)), out=self.out.append, **kw)

    def test_happy_path_pinned_checkout_config_0600_and_exec(self):
        code = self.run_with_url()
        self.assertEqual(code, 0, "\n".join(self.out))
        cfgp = self.work / "secrets" / "config.json"
        self.assertEqual(stat.S_IMODE(cfgp.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(cfgp.parent.stat().st_mode), 0o700)
        jobs_dir = Path(json.loads(cfgp.read_text())["work_dir"])
        self.assertNotIn(cfgp.parent, [jobs_dir, *jobs_dir.parents[:1]])  # not the jobs' cwd, not its parent dir
        self.assertNotEqual(cfgp.parent, jobs_dir.parent)
        cfg = json.loads(cfgp.read_text())
        self.assertEqual((cfg["box_token"], cfg["access_secret"]), (TOKEN, SECRET))
        self.assertEqual(git(self.work / "checkout", "rev-parse", "HEAD"), self.sha)
        self.assertEqual(self.calls[0][1:5], ["-m", "kfhost", "setup", "--venv"])
        self.assertEqual(self.execs[0][1:5], ["-m", "kfhost", "run", "--config"])
        self.assertEqual(self.execs[0][-2:], ["--exit-when-idle", "50"])

    def run_with_url(self):
        # route the https URL to the local fixture through a fake git that rewrites the remote
        real = bootstrap.default_git
        src = str(self.src)

        def fake_git(args, cwd):
            args = [src if x == "https://example.invalid/r" else x for x in args]
            return real(args, cwd)

        argv = ["--repo", "https://example.invalid/r", "--commit", self.sha, "--worker-url", "https://w.example",
                "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                "--access-secret-file", str(self.sec), "--work-dir", str(self.work), "--", "--exit-when-idle", "50"]
        run = lambda argv_, cwd, check: self.calls.append(list(argv_)) or type("R", (), {"returncode": 0})()  # noqa: E731
        self.cwd = os.getcwd()
        try:
            return bootstrap.main(argv, run=run, execv=lambda p, a: self.execs.append(list(a)), out=self.out.append, git=fake_git)
        finally:
            os.chdir(self.cwd)

    def test_wrong_commit_refused_and_nothing_started(self):
        real = bootstrap.default_git

        def fake_git(args, cwd):
            args = [str(self.src) if x == "https://example.invalid/r" else x for x in args]
            if args[0] == "rev-parse":
                return "f" * 40  # a checkout that is not the pinned commit
            return real(args, cwd)

        argv = ["--repo", "https://example.invalid/r", "--commit", self.sha, "--worker-url", "https://w.example",
                "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                "--access-secret-file", str(self.sec), "--work-dir", str(self.work)]
        rc = bootstrap.main(argv, run=lambda *a, **k: self.calls.append(a), execv=lambda *a: self.execs.append(a),
                            out=self.out.append, git=fake_git)
        self.assertEqual(rc, 2)
        self.assertEqual((self.calls, self.execs), ([], []))
        self.assertFalse((self.work / "secrets" / "config.json").exists())

    def test_unknown_commit_and_bad_format_refused(self):
        self.assertEqual(self.go(commit="abc"), 2)
        self.assertEqual(self.go(commit="1" * 40), 2)  # not https
        self.assertEqual((self.calls, self.execs), ([], []))

    def test_dry_run_changes_nothing_and_prints_plan(self):
        before = sorted(os.listdir(self.tmp.name))
        rc = bootstrap.main(["--repo", "https://example.invalid/r", "--commit", self.sha, "--worker-url", "https://w.example",
                             "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                             "--access-secret-file", str(self.sec), "--work-dir", str(self.work), "--dry-run"],
                            run=lambda *a, **k: self.calls.append(a), execv=lambda *a: self.execs.append(a), out=self.out.append)
        self.assertEqual(rc, 0)
        self.assertEqual(sorted(os.listdir(self.tmp.name)), before)
        self.assertFalse(self.work.exists())
        self.assertEqual((self.calls, self.execs), ([], []))
        self.assertTrue(any("will, inside" in l for l in self.out))

    def test_secrets_never_in_argv_or_output(self):
        self.run_with_url()
        everything = json.dumps(self.calls + self.execs) + "\n".join(self.out)
        self.assertNotIn(TOKEN, everything)
        self.assertNotIn(SECRET, everything)

    def test_secrets_from_stdin_and_loose_file_refused(self):
        os.chmod(self.tok, 0o644)
        self.assertEqual(self.go_url(), 2)
        self.assertEqual(bootstrap.read_secret("-", io.StringIO(" tok-from-stdin \n")), "tok-from-stdin")

    def go_url(self):
        return bootstrap.main(["--repo", "https://example.invalid/r", "--commit", self.sha, "--worker-url", "https://w.example",
                               "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                               "--access-secret-file", str(self.sec), "--work-dir", str(self.work)],
                              run=lambda *a, **k: None, execv=lambda *a: None, out=self.out.append)

    def test_setup_failure_stops_before_config_and_exec(self):
        real = bootstrap.default_git
        fake = lambda args, cwd: real([str(self.src) if x == "https://example.invalid/r" else x for x in args], cwd)  # noqa: E731
        rc = bootstrap.main(["--repo", "https://example.invalid/r", "--commit", self.sha, "--worker-url", "https://w.example",
                             "--box-id", "box1", "--token-file", str(self.tok), "--access-id", "aid",
                             "--access-secret-file", str(self.sec), "--work-dir", str(self.work)],
                            run=lambda argv, cwd, check: type("R", (), {"returncode": 1})(),
                            execv=lambda *a: self.execs.append(a), out=self.out.append, git=fake)
        self.assertEqual(rc, 2)
        self.assertFalse((self.work / "secrets" / "config.json").exists())
        self.assertEqual(self.execs, [])


if __name__ == "__main__":
    unittest.main()
