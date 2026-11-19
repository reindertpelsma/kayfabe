"""Rootless harness fixes (2026-11-15), GPU-free: the shared GPU lock, per-invocation git trust, kernel-log offsets.

Found by the manual release-gate rehearsal (traces/release_manual_rehearsal_20261115): the lock file was created
by root and then refused to the suite user (fs.protected_regular), git said `dubious ownership` on a root-owned
checkout (boot_rc=2), and 8 of 15 Xid lines were kept with no stage attached.
FALSIFIER: a second user cannot open or contend for the lock, git gets `safe.directory=*` or a parent path, or a
line logged inside a stage is attributed to another stage.
"""

import errno
import fcntl
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts/lib"))
sys.path.insert(0, str(HERE))
import kernel_log  # noqa: E402
import kf_git  # noqa: E402
import kf_lock  # noqa: E402


def other_user_opener(owner_of):
    """os.open seam for fs.protected_regular: O_CREAT on a file the 'caller' does not own is EACCES."""

    def opener(path, flags, mode=0o777):
        if flags & os.O_CREAT and os.path.exists(path) and owner_of(path) != "bob":
            raise PermissionError(errno.EACCES, "protected_regular", path)
        return os.open(path, flags, mode)

    return opener


class SharedLock(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.path = str(self.tmp / "gpu.lock")

    def test_first_user_creates_it_world_writable(self):
        os.close(kf_lock.open_lock(self.path))
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o666)

    def test_second_user_opens_a_file_the_first_created_where_open_a_is_refused(self):
        os.close(kf_lock.open_lock(self.path))  # alice (root in the rehearsal) created it
        bob = other_user_opener(lambda p: "alice")
        with self.assertRaises(PermissionError):  # the old `open(path, "a")` / `exec 9>path`
            bob(self.path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o666)
        fd = kf_lock.open_lock(self.path, opener=bob)  # the fix
        os.close(fd)

    def test_two_users_exclude_each_other_on_the_one_path(self):
        alice = kf_lock.acquire(self.path)
        bob = other_user_opener(lambda p: "alice")
        with self.assertRaises(BlockingIOError):
            kf_lock.acquire(self.path, blocking=False, opener=bob)
        os.close(alice)
        os.close(kf_lock.acquire(self.path, blocking=False, opener=bob))

    def test_lost_creation_race_is_not_an_error(self):
        open(self.path, "w").close()
        os.close(kf_lock.open_lock(self.path, opener=other_user_opener(lambda p: "alice")))

    def test_refuses_symlink_and_directory(self):
        target = self.tmp / "t"
        target.write_text("")
        link = self.tmp / "link"
        link.symlink_to(target)
        with self.assertRaises(OSError):
            kf_lock.open_lock(str(link))
        with self.assertRaises(OSError):
            kf_lock.open_lock(str(self.tmp))

    def bash(self, body):
        script = '. "%s/scripts/lib/kf_lock.sh"\n%s' % (ROOT, body)
        return subprocess.run(["bash", "-c", script], capture_output=True, text=True)

    def test_shell_helper_creates_0666_and_contends(self):
        r = self.bash('kf_lock_open "%s" 9 && flock -n 9 && echo got' % self.path)
        self.assertIn("got", r.stdout, r.stderr)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o666)
        held = os.open(self.path, os.O_RDONLY)
        fcntl.flock(held, fcntl.LOCK_EX)
        r = self.bash('kf_lock_open "%s" 9 && flock -n 9 && echo got' % self.path)
        self.assertNotIn("got", r.stdout)
        os.close(held)

    def test_shell_helper_opens_readonly_file_it_cannot_create(self):
        open(self.path, "w").close()
        os.chmod(self.path, 0o444)
        r = self.bash('kf_lock_open "%s" 9 && flock -n 9 && echo got' % self.path)
        self.assertIn("got", r.stdout, r.stderr)

    def test_shell_helper_refuses_symlink(self):
        (self.tmp / "t").write_text("")
        (self.tmp / "l").symlink_to(self.tmp / "t")
        r = self.bash('kf_lock_open "%s/l" 9; echo rc=$?' % self.tmp)
        self.assertIn("rc=1", r.stdout)

    def test_no_writer_still_creates_the_lock_with_exec_redirect(self):
        bad = []
        for path in ROOT.glob("scripts/**/*"):
            if not path.is_file() or "host_tools" in path.parts or path.suffix not in (".sh", ".py"):
                continue
            if path.name in ("test_harness_rootless.py", "kf_lock.sh", "kf_lock.py"):
                continue
            for n, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
                code = line.split("#", 1)[0]
                if "kayfabe-fastguest.lock" in code and ("exec 9>" in code or "exec 8>" in code or "open(" in code):
                    bad.append("%s:%d" % (path.relative_to(ROOT), n))
        self.assertEqual(bad, [])


    def test_lib_source_line_does_not_unscope_a_file_wide_shellcheck_directive(self):
        """A `# shellcheck disable=` before the first command is file-wide; a command inserted above it makes it
        cover one line only (win_vm.sh SC2054 on next's stable, 2026-11-15). The sourced helper goes below it."""
        bad = []
        for path in ROOT.glob("scripts/**/*.sh"):
            lines = path.read_text(errors="replace").splitlines()
            src = [i for i, l in enumerate(lines) if l.startswith('. "$(dirname "${BASH_SOURCE[0]}")/') and "/lib/kf_" in l]
            if not src:
                continue
            first = min(src)
            commands = [i for i, l in enumerate(lines[1:], 1) if l.strip() and not l.lstrip().startswith("#")]
            if commands and commands[0] != first:
                continue
            for i, l in enumerate(lines):
                if i > first and l.startswith("# shellcheck disable=") and i < (commands[1] if len(commands) > 1 else i + 1):
                    bad.append("%s:%d" % (path.relative_to(ROOT), i + 1))
        self.assertEqual(bad, [])


class GitTrust(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.repo = self.tmp / "repo"
        self.repo.mkdir()

    def test_exact_argument_for_exact_path(self):
        self.assertEqual(
            kf_git.safe_directory_args(self.repo), ["-c", "safe.directory=" + os.path.realpath(self.repo)]
        )
        self.assertEqual(
            kf_git.git_command(self.repo, "rev-parse", "HEAD"),
            ["git", "-c", "safe.directory=" + os.path.realpath(self.repo), "-C", os.path.realpath(self.repo), "rev-parse", "HEAD"],
        )

    def test_refuses_star_patterns_root_and_missing(self):
        for bad in ("*", "", "/", str(self.repo) + "/*", str(self.tmp / "nope"), str(self.repo) + "\n/etc"):
            with self.assertRaises(ValueError, msg=repr(bad)):
                kf_git.safe_directory_args(bad)

    def test_symlink_resolves_to_the_real_directory_not_the_link(self):
        link = self.tmp / "link"
        link.symlink_to(self.repo)
        self.assertEqual(kf_git.safe_directory_args(link)[1], "safe.directory=" + os.path.realpath(self.repo))

    def fake_git(self):
        bindir = self.tmp / "bin"
        bindir.mkdir()
        git = bindir / "git"
        git.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$KF_ARGS"\n')
        git.chmod(0o755)
        return {**os.environ, "PATH": "%s:%s" % (bindir, os.environ["PATH"]), "KF_ARGS": str(self.tmp / "args")}

    def sh(self, directory, env):
        script = '. "%s/scripts/lib/kf_git.sh"\nkf_git "$1" rev-parse HEAD' % ROOT
        return subprocess.run(["bash", "-c", script, "x", directory], env=env, capture_output=True, text=True)

    def test_shell_helper_passes_exactly_that_path(self):
        env = self.fake_git()
        r = self.sh(str(self.repo), env)
        self.assertEqual(r.returncode, 0, r.stderr)
        real = os.path.realpath(self.repo)
        self.assertEqual((self.tmp / "args").read_text().split("\n")[:-1],
                         ["-c", "safe.directory=" + real, "-C", real, "rev-parse", "HEAD"])

    def test_shell_helper_refuses_star_root_and_missing(self):
        env = self.fake_git()
        starred = self.tmp / "a*b"
        starred.mkdir()
        for d in (str(starred), "/", str(self.tmp / "nope")):
            r = self.sh(d, env)
            self.assertEqual(r.returncode, 2, d)
            self.assertFalse((self.tmp / "args").exists(), d)

    def test_fastguest_scripts_use_the_helper_not_bare_git(self):
        for name in ("run_fast_guest.sh", "fast_suite.sh", "bare_metal_suite.sh"):
            text = (ROOT / "scripts/fastguest" / name).read_text()
            self.assertIn("kf_git ", text, name)
            self.assertNotRegex(text, r"\bgit -C \"\$KF3_REPO\"", name)


class KernelLogOffsets(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.log = kernel_log.log_path(self.d)

    def append(self, *lines):
        with open(self.log, "a") as f:
            for line in lines:
                f.write(line + "\n")

    def mark(self, stage, edge):
        return kernel_log.mark(self.d, stage, edge, settle=0, clock=lambda: 1)

    def test_xid_is_attributed_to_the_stage_it_fell_in(self):
        xid = "NVRM: Xid (PCI:0000:01:00): 31, pid=1, name=qemu-system-x86"
        self.append("boot 1", "boot 2", xid + " old")  # before the run window: history
        self.mark("merge_check", "start")
        self.append("t1", xid + " a", "t2")
        self.mark("merge_check", "end")
        self.append(xid + " between")
        self.mark("windows_gate", "start")
        self.append("t3", "t4", xid + " b", xid + " c")
        self.mark("windows_gate", "end")
        self.append(xid + " after")
        got = [(n, s) for n, s, _ in kernel_log.matching(self.d)]
        self.assertEqual(
            got,
            [(3, "outside-stages"), (5, "merge_check"), (7, "outside-stages"), (10, "windows_gate"),
             (11, "windows_gate"), (12, "outside-stages")],
        )
        self.assertEqual(len(got), 6)  # every line kept, none dropped by tailing

    def test_open_stage_runs_to_the_end_and_offsets_file_is_tsv(self):
        self.append("a")
        self.mark("s", "start")
        self.append("b", "c")
        self.assertEqual(kernel_log.attribute(self.d, 3), "s")
        self.assertEqual(kernel_log.attribute(self.d, 1), "outside-stages")
        self.assertEqual(kernel_log.offsets_path(self.d).read_text(), "s\tstart\t1\t1\n")

    def test_bad_edge_rejected(self):
        with self.assertRaises(ValueError):
            kernel_log.mark(self.d, "s", "middle", settle=0)

    def test_follower_copies_every_line_of_its_source(self):
        src = [sys.executable, "-c", "print('l1'); print('NVRM: Xid x'); print('l3')"]
        self.assertEqual(kernel_log.follow(self.d, src), 0)
        self.assertEqual(self.log.read_text().splitlines(), ["l1", "NVRM: Xid x", "l3"])

    def test_start_fails_loudly_on_an_unreadable_log(self):
        with self.assertRaises(RuntimeError):
            kernel_log.start(self.d, [sys.executable, "-c", "import sys; sys.exit(1)"])

    def test_execute_stages_marks_edges(self):
        import vast_payload

        calls = []
        old = vast_payload.kernel_log_mark
        vast_payload.kernel_log_mark = lambda name, edge: calls.append((name, edge))
        try:
            result = {"verdict": "FAIL", "suites": []}
            dest = self.d / "r.json"
            stage = dict(name="x", command=["true"], output=self.d / "o", progress=[], grade=lambda rc: [])
            vast_payload.execute_stages([stage], 1 << 62, result, dest, runner=lambda *a: 0)
        finally:
            vast_payload.kernel_log_mark = old
        self.assertEqual(calls, [("x", "start"), ("x", "end")])


if __name__ == "__main__":
    unittest.main()
