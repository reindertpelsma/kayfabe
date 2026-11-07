# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sshguest.py: the Qga-compatible transport of a win_vm.sh guest, against fake ssh/scp programs (no VM)."""
import base64
import os
import stat
import tempfile
import unittest

import helpers as H  # noqa: F401
import qga as qgamod
import sshguest

FAKE_SSH = r'''#!/usr/bin/env python3
import base64, os, sys
a = sys.argv[1:]
log = os.environ["FAKE_LOG"]
if "-O" in a:                                   # ssh -O exit
    open(log, "a").write("EXIT-MASTER\n"); sys.exit(0)
cmd = a[-1]
mode = open(os.environ["FAKE_MODE"]).read().strip() if os.path.exists(os.environ["FAKE_MODE"]) else "ok"
if mode == "down":
    sys.stderr.write("ssh: connect to host 127.0.0.1 port 2224: Connection refused\n"); sys.exit(255)
if mode == "hang":
    import time; time.sleep(9)
if cmd.startswith("powershell -NoProfile -NonInteractive -EncodedCommand "):
    script = base64.b64decode(cmd.rsplit(" ", 1)[1]).decode("utf-16-le")
else:
    script = cmd
open(log, "a").write("SCRIPT " + script.replace("\n", " | ") + "\n")
if script.strip() == "echo KF_SSH_UP":
    print("KF_SSH_UP"); sys.exit(0)
if "Test-Path" in script:
    print("True" if "present.txt" in script else "False"); sys.exit(0 if "present.txt" in script else 1)
if "exit $LASTEXITCODE" in script and "kf_fail.ps1" in script:
    print("KFSTAGE {}"); sys.exit(7)
print("OUT " + script.splitlines()[0][:60]); sys.exit(0)
'''
FAKE_SCP = r'''#!/usr/bin/env python3
import os, shutil, sys
a = [x for x in sys.argv[1:]]
src, dst = a[-2], a[-1]
open(os.environ["FAKE_LOG"], "a").write(f"SCP {src} -> {dst}\n")
if os.path.exists(os.environ["FAKE_MODE"]) and open(os.environ["FAKE_MODE"]).read().strip() == "scpfail":
    sys.stderr.write("scp: Connection closed\n"); sys.exit(255)
store = os.environ["FAKE_STORE"]
if ":" in src and not os.path.exists(src):                # from the guest
    name = src.split(":", 1)[1]
    p = os.path.join(store, name.replace("/", "_"))
    if not os.path.exists(p):
        sys.stderr.write(f"scp: {name}: No such file or directory\n"); sys.exit(1)
    shutil.copyfile(p, dst); sys.exit(0)
name = dst.split(":", 1)[1]
shutil.copyfile(src, os.path.join(store, name.replace("/", "_")))
'''


class SshGuest(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(self.d, ignore_errors=True))
        for name, body in (("ssh", FAKE_SSH), ("scp", FAKE_SCP)):
            p = os.path.join(self.d, name)
            open(p, "w").write(body)
            os.chmod(p, os.stat(p).st_mode | stat.S_IXUSR)
        self.log = os.path.join(self.d, "log")
        self.store = os.path.join(self.d, "store")
        os.makedirs(self.store)
        self.mode = os.path.join(self.d, "mode")
        os.environ.update(FAKE_LOG=self.log, FAKE_MODE=self.mode, FAKE_STORE=self.store)
        self.g = sshguest.SshGuest(2224, "/nonexistent/key", io_timeout=5.0, ssh=os.path.join(self.d, "ssh"), scp=os.path.join(self.d, "scp"),
                                   workdir=os.path.join(self.d, "w"))
        os.makedirs(self.g.dir)
        self.addCleanup(self.g.cleanup)

    def logtxt(self):
        return open(self.log).read() if os.path.exists(self.log) else ""

    def test_quoting_is_exact_and_an_empty_argument_survives(self):
        self.assertEqual(sshguest.ps_quote("a b"), "'a b'")
        self.assertEqual(sshguest.ps_quote("it's"), "'it''s'")
        self.assertEqual(sshguest.ps_quote(""), "'\"\"'")
        self.assertEqual(sshguest.ps_quote(r"C:\kf\x.ps1"), r"'C:\kf\x.ps1'")

    def test_ping(self):
        self.assertTrue(self.g.ping(5))
        open(self.mode, "w").write("down")
        self.assertFalse(self.g.ping(5))

    def test_powershell_runs_the_script_with_file_semantics_and_returns_the_exit_code(self):
        r = self.g.powershell(r"C:\kf\kf_fail.ps1", ["-Pkg", "x", ""], timeout=10)
        self.assertEqual(r.exitcode, 7)
        self.assertIn("KFSTAGE", r.text())
        t = self.logtxt()
        self.assertIn("-File", t)
        self.assertIn("'C:\\kf\\kf_fail.ps1' '-Pkg' 'x' '\"\"'", t)       # an empty argument is not dropped
        self.assertIn("exit $LASTEXITCODE", t)

    def test_powershell_command_is_run_in_the_default_shell(self):
        r = self.g.exec(sshguest.PS_EXE, ["-NoProfile", "-Command", "Get-Date"], timeout=10)
        self.assertEqual(r.exitcode, 0)
        self.assertIn("Get-Date", self.logtxt())

    def test_a_dead_transport_is_a_qga_error_not_a_result(self):
        open(self.mode, "w").write("down")
        with self.assertRaises(qgamod.QgaError):
            self.g.exec(r"C:\Windows\System32\net.exe", ["user"], timeout=5)

    def test_a_hung_ssh_is_a_timeout_result(self):
        open(self.mode, "w").write("hang")
        r = self.g.exec(r"C:\x.exe", [], timeout=1)
        self.assertTrue(r.timed_out)
        self.assertIsNone(r.exitcode)

    def test_file_roundtrip_and_paths(self):
        self.g.file_write(r"C:\kf\apps\a.ps1", b"hello\r\n")
        self.assertIn("SCP", self.logtxt())
        self.assertIn("kf@127.0.0.1:C:/kf/apps/a.ps1", self.logtxt())      # backslashes become slashes: scp wants them
        self.assertEqual(self.g.file_read(r"C:\kf\apps\a.ps1"), b"hello\r\n")

    def test_a_missing_file_is_a_command_error_and_a_broken_copy_is_a_qga_error(self):
        with self.assertRaises(qgamod.QgaCommandError):
            self.g.file_read(r"C:\kf\results\nope.json")
        open(self.mode, "w").write("scpfail")
        with self.assertRaises(qgamod.QgaError):
            self.g.file_write(r"C:\kf\x", b"x")

    def test_file_read_elides_the_middle_of_a_huge_log(self):
        self.g.file_write(r"C:\kf\logs\big.log", b"H" * 100 + b"M" * 1000 + b"T" * 100)
        out = self.g.file_read(r"C:\kf\logs\big.log", head=100, tail=100)
        self.assertTrue(out.startswith(b"H" * 100) and out.endswith(b"T" * 100))
        self.assertIn(b"middle of the log elided", out)
        self.assertNotIn(b"M" * 10, out)

    def test_file_exists(self):
        self.assertTrue(self.g.file_exists(r"C:\kf\results\present.txt"))
        self.assertFalse(self.g.file_exists(r"C:\kf\results\absent.txt"))

    def test_no_password_is_ever_an_argument_of_this_transport(self):
        code = open(sshguest.__file__).read().split("import base64")[1].lower()     # the docstring talks about it; the code must not
        self.assertNotIn("kf_guest_pw", code)
        self.assertNotIn("password", code)


if __name__ == "__main__":
    unittest.main()
