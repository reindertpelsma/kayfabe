#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/kfroot.py, scripts/lib/kfroot.sh and the rootless wiring of the Windows harness.

STATUS: LIVE, 2026-11-15. OWNER_RULINGS section AR item 6. No box, no GPU, no real sudo: the launcher and the wrapper are
run under a FAKE sudo (a Python script that reads the generated sudoers file and behaves as sudo does for it: refuses a
command that is not whitelisted, refuses -E without SETENV, resets PATH unless !secure_path, resets the environment
without -E, refuses to prompt under -n, logs in sudo's own format). The real sudo was exercised by hand in a sandbox
once (docs/design/V3_KFROOT_ROOTLESS.md, "Measured"); this file is what keeps the pieces honest afterwards.

    KF_KFROOT_TREE=<dir>   aim the tests at a mutated tree (scripts/ci/kfroot.py, scripts/lib/kfroot.sh,
                           scripts/bench/box/windows_*.sh, scripts/bench/windows/win_vm.sh); scripts/ci/kfroot_mutants.py
"""

import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
TREE = Path(os.environ.get("KF_KFROOT_TREE") or ROOT)
KFROOT_PY = TREE / "scripts/ci/kfroot.py"
LIB = TREE / "scripts/lib/kfroot.sh"
GATE = TREE / "scripts/bench/box/windows_gate.sh"
APPS = TREE / "scripts/bench/box/windows_apps.sh"
WINVM = TREE / "scripts/bench/windows/win_vm.sh"

spec = importlib.util.spec_from_file_location("kfroot_under_test", KFROOT_PY)
K = importlib.util.module_from_spec(spec)
spec.loader.exec_module(K)

FAKE_SUDO = r'''#!/usr/bin/env python3
# A sudo that behaves, for the generated sudoers file, as sudo does (see the test module docstring).
import os, re, sys, time
sudoers = open(os.environ["FAKE_SUDOERS"]).read()
user = os.environ["FAKE_SUDO_USER"]
log = os.environ["FAKE_SUDO_LOG"]
args = sys.argv[1:]
flags = []
while args and args[0].startswith("-") and args[0] != "--":
    flags.append(args.pop(0))
def note(failure=""):
    head = f": {user} : " + (failure + " ; " if failure else "") + f"PWD={os.getcwd()} ; USER=root ;"
    # sudo wraps its logfile lines at 80 columns unless loglinelen=0: reproduce that when the sudoers file leaves it on
    command = "COMMAND=" + " ".join(args)
    if "loglinelen=0" in sudoers:
        line = time.strftime("%b %d %H:%M:%S ") + head + " " + command + "\n"
    else:
        line = time.strftime("%b %d %H:%M:%S ") + head + "\n    " + command + "\n"
    open(log, "a").write(line)
def tags(path):
    for m in re.finditer(rf"^{re.escape(user)} ALL=\(root\) (\S+) (\S+)$", sudoers, re.M):
        if m[2] == path:
            return m[1]
    return None
if "-n" not in flags:
    note("a password is required"); sys.stderr.write("sudo: a password is required\n"); sys.exit(1)
if not args:
    sys.exit(1)
tag = tags(args[0])
if tag is None:
    note("command not allowed"); sys.stderr.write("sudo: a password is required\n"); sys.exit(1)
if "NOPASSWD" not in tag:
    note("a password is required"); sys.stderr.write("sudo: a password is required\n"); sys.exit(1)
if "-E" in flags and "SETENV" not in tag:
    note("sorry, you are not allowed to preserve the environment"); sys.exit(1)
env = dict(os.environ) if "-E" in flags else {"HOME": "/root", "PATH": "/usr/bin:/bin"}
if not re.search(rf"^Defaults:{re.escape(user)} !secure_path$", sudoers, re.M):
    env["PATH"] = "/usr/sbin:/usr/bin:/sbin:/bin"
note()
target = os.environ["FAKE_WRAPPER"] if args[0] == "/usr/local/sbin/kfroot-wrapper" else args[0]
os.execve(target, [target] + args[1:], env)
'''

FAKE_VISUDO = "#!/bin/sh\n# accepts a file unless FAKE_VISUDO_REJECT is a substring of it\n" \
    '[ -n "$FAKE_VISUDO_REJECT" ] && grep -q "$FAKE_VISUDO_REJECT" "$3" && { echo "x:1:1: syntax error: unknown setting: \'$FAKE_VISUDO_REJECT\'"; exit 1; }\nexit 0\n'


def write_exe(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)
    return path


class Box:
    """A throwaway filesystem root with the three kfroot files installed and a fake sudo in front of PATH."""

    def __init__(self, tc, user="ci", reject=""):
        self.tmp = Path(tempfile.mkdtemp(prefix="kfroot-test-"))
        tc.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.tmp)]))
        self.user = user
        self.root = self.tmp / "root"
        write_exe(self.tmp / "bin/visudo", FAKE_VISUDO)
        env = {"FAKE_VISUDO_REJECT": reject}
        self.install = K.install(user, str(self.root), str(self.tmp / "bin/visudo"),
                                 run=lambda *a, **k: subprocess.run(*a, **{**k, "env": {**os.environ, **env}}))
        write_exe(self.tmp / "bin/sudo", FAKE_SUDO)
        self.log = self.tmp / "sudo.log"
        self.log.write_text("")
        self.launcher = self.root / "usr/local/bin/kfroot"
        self.wrapper = self.root / "usr/local/sbin/kfroot-wrapper"
        self.sudoers = self.root / "etc/sudoers.d/kfroot"
        self.bin = self.tmp / "bin"

    def env(self, **extra):
        e = {k: v for k, v in os.environ.items() if k != "PATH"}
        e.update({
            "PATH": f"{self.bin}:/usr/bin:/bin", "FAKE_SUDOERS": str(self.sudoers), "FAKE_SUDO_USER": self.user,
            "FAKE_SUDO_LOG": str(self.log), "FAKE_WRAPPER": str(self.wrapper),
        })
        e.update(extra)
        return e

    def kfroot(self, *argv, stdin=None, **extra):
        return subprocess.run([str(self.launcher), *argv], input=stdin, env=self.env(**extra), capture_output=True,
                              text=True, timeout=60)

    def sudo(self, *argv):
        return subprocess.run(["sudo", *argv], env=self.env(), capture_output=True, text=True, timeout=60)


class LauncherAndWrapper(unittest.TestCase):
    def setUp(self):
        self.box = Box(self)

    def test_the_files_are_the_owner_design(self):
        self.assertEqual(self.box.launcher.read_text(),
                         '#!/bin/bash\nexec sudo -n -E /usr/local/sbin/kfroot-wrapper "$@"\n')
        self.assertEqual(self.box.wrapper.read_text(), '#!/bin/bash\nexec "$@"\n')

    def test_the_wrapper_runs_its_argv_and_nothing_else(self):
        p = subprocess.run([str(self.box.wrapper), "printf", "%s|", "a", "b"], capture_output=True, text=True)
        self.assertEqual((p.returncode, p.stdout), (0, "a|b|"))
        # not a shell: no word is evaluated
        marker = self.box.tmp / "pwned"
        subprocess.run([str(self.box.wrapper), "true", f"$(touch {marker})", f"; touch {marker}"], check=True)
        self.assertFalse(marker.exists())

    def test_a_call_runs_through_sudo_and_returns_the_exit_status(self):
        p = self.box.kfroot("sh", "-c", "echo out; echo err >&2; exit 37")
        self.assertEqual((p.returncode, p.stdout, p.stderr), (37, "out\n", "err\n"))

    def test_stdin_is_forwarded(self):
        p = self.box.kfroot("tee", str(self.box.tmp / "f"), stdin="payload\nline 2\n")
        self.assertEqual(p.stdout, "payload\nline 2\n")
        self.assertEqual((self.box.tmp / "f").read_text(), "payload\nline 2\n")

    def test_the_environment_is_forwarded(self):
        p = self.box.kfroot("sh", "-c", 'printf "%s|%s" "$KF3_FLAG" "$DEBIAN_FRONTEND"', KF3_FLAG="7",
                            DEBIAN_FRONTEND="noninteractive")
        self.assertEqual(p.stdout, "7|noninteractive")

    def test_path_is_forwarded_and_a_tool_on_it_is_found(self):
        tool = write_exe(self.box.tmp / "mytools/kf-mytool", "#!/bin/sh\necho mytool-ran\n")
        p = self.box.kfroot("kf-mytool", PATH=f"{tool.parent}:{self.box.bin}:/usr/bin:/bin")
        self.assertEqual((p.returncode, p.stdout.strip()), (0, "mytool-ran"), p.stderr)

    def test_hostile_arguments_arrive_byte_for_byte(self):
        argv = ["a b", "new\nline", "q'uote", '"dq"', "$(touch HACK)", ";rm", "*", "-n", "-E", "", "--", "\\", "tab\there"]
        p = self.box.kfroot("python3", "-c", "import sys,json;print(json.dumps(sys.argv[1:]))", *argv)
        self.assertEqual(json.loads(p.stdout), argv, p.stderr)
        self.assertFalse(Path("HACK").exists())

    def test_a_stray_sudo_is_refused_and_a_wrapper_call_is_not(self):
        stray = self.box.sudo("-n", "/usr/bin/id", "-u")
        self.assertNotEqual(stray.returncode, 0)
        self.assertNotIn("0", stray.stdout.split())
        self.assertEqual(self.box.kfroot("true").returncode, 0)

    def test_the_audit_sees_the_stray_entry_and_only_it(self):
        self.box.kfroot("true")
        self.box.kfroot("printf", "x")
        self.assertEqual(K.audit_check(K.audit_text(str(self.box.log), 0), ("ci",))["violations"], [])
        mark = K.audit_mark(str(self.box.log))
        self.box.sudo("-n", "/usr/bin/id", "-u")
        self.box.kfroot("true")
        result = K.audit_check(K.audit_text(str(self.box.log), mark), ("ci",))
        self.assertEqual(len(result["violations"]), 1)
        self.assertEqual(result["kfroot_calls"], 1)
        self.assertIn("/usr/bin/id", result["violations"][0]["command"])

    def test_without_not_secure_path_the_path_would_not_arrive(self):
        # the fake derives this from the sudoers text: the generated file is what makes PATH survive
        text = self.box.sudoers.read_text().replace("!secure_path", "env_keep+=FOO")
        self.box.sudoers.chmod(0o644)
        self.box.sudoers.write_text(text)
        tool = write_exe(self.box.tmp / "mytools/kf-mytool", "#!/bin/sh\necho mytool-ran\n")
        p = self.box.kfroot("kf-mytool", PATH=f"{tool.parent}:{self.box.bin}:/usr/bin:/bin")
        self.assertNotEqual(p.returncode, 0)

    def test_the_log_has_one_line_per_call_when_the_sudoers_file_says_so(self):
        self.box.kfroot("true")
        self.box.kfroot("printf", "x")
        self.assertEqual(len(self.box.log.read_text().splitlines()), 2)

    def test_a_wrapped_log_is_still_audited_correctly(self):
        # a sudo that wraps its lines (no loglinelen=0): the parser joins continuation lines, nothing is lost or invented
        text = self.box.sudoers.read_text().replace("Defaults:ci loglinelen=0\n", "")
        self.box.sudoers.chmod(0o644)
        self.box.sudoers.write_text(text)
        self.box.kfroot("true")
        self.box.sudo("-n", "/usr/bin/id", "-u")
        self.assertGreater(len(self.box.log.read_text().splitlines()), 2)
        r = K.audit_check(K.audit_text(str(self.box.log), 0), ("ci",))
        self.assertEqual((r["kfroot_calls"], len(r["violations"])), (1, 1), r)

    def test_without_the_setenv_tag_the_environment_would_be_refused(self):
        text = self.box.sudoers.read_text().replace("NOPASSWD:SETENV:", "NOPASSWD:")
        self.box.sudoers.chmod(0o644)
        self.box.sudoers.write_text(text)
        self.assertNotEqual(self.box.kfroot("true").returncode, 0)

    def test_without_nopasswd_a_call_would_prompt_and_fails(self):
        text = self.box.sudoers.read_text().replace("NOPASSWD:SETENV:", "SETENV:")
        self.box.sudoers.chmod(0o644)
        self.box.sudoers.write_text(text)
        self.assertNotEqual(self.box.kfroot("true").returncode, 0)

    def test_a_password_prompt_is_a_failure_not_a_hang(self):
        # the launcher passes -n: the fake refuses to "prompt" without it
        self.assertIn(" -n ", " " + K.LAUNCHER_TEXT.split("exec sudo", 1)[1])
        p = self.box.sudo("/usr/local/sbin/kfroot-wrapper", "true")
        self.assertNotEqual(p.returncode, 0)


class Sudoers(unittest.TestCase):
    def lines(self, **kw):
        return [ln for ln in K.sudoers_text("ci", **kw).splitlines() if ln and not ln.startswith("#")]

    def test_exactly_one_entry_and_it_is_the_wrapper(self):
        entries = [ln for ln in self.lines() if " ALL=" in ln]
        self.assertEqual(entries, ["ci ALL=(root) NOPASSWD:SETENV: /usr/local/sbin/kfroot-wrapper"])

    def test_no_wildcard_no_all_command_no_group(self):
        text = K.sudoers_text("ci")
        entry = [ln for ln in text.splitlines() if " ALL=" in ln][0]
        self.assertNotIn("*", entry)
        self.assertFalse(entry.rstrip().endswith("ALL"))
        self.assertNotIn("%", text)
        self.assertNotIn("(ALL", text)

    def test_the_defaults_are_the_design(self):
        lines = self.lines()
        self.assertIn("Defaults:ci !secure_path", lines)
        self.assertIn("Defaults:ci !use_pty", lines)
        self.assertIn("Defaults:ci logfile=/var/log/kfroot-sudo.log", lines)
        self.assertIn("Defaults:ci loglinelen=0", lines)

    def test_the_sudo_rs_variant_has_no_logfile_lines(self):
        text = K.sudoers_text("ci", logfile=False)
        self.assertNotIn("logfile", text)
        self.assertNotIn("loglinelen", text)
        self.assertIn("!secure_path", text)

    def test_bad_users_are_refused(self):
        for bad in ("root", "", "a b", "ci\nALL ALL=(ALL) ALL", "-x", "Ci", "a;b", "x" * 40, None):
            with self.subTest(bad=bad), self.assertRaises(K.KfrootError):
                K.sudoers_text(bad)

    def test_a_real_visudo_accepts_the_portable_variant(self):
        visudo = next((v for v in ("/usr/sbin/visudo.ws", "/usr/sbin/visudo") if os.path.exists(v)), None)
        if visudo is None:
            self.skipTest("no visudo on this machine")
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "kfroot"
            f.write_text(K.sudoers_text("ci", logfile=False))
            f.chmod(0o440)
            ok, out = K.visudo_check(f, visudo)
            self.assertTrue(ok, out)
            f.chmod(0o640)  # a read-only file is writable only by root: the runner is not
            f.write_text(K.sudoers_text("ci", logfile=False) + "ci ALL=(root BROKEN\n")
            f.chmod(0o440)
            self.assertFalse(K.visudo_check(f, visudo)[0])


class InstallAndVerify(unittest.TestCase):
    MODES = {"/usr/local/bin/kfroot": 0o755, "/usr/local/sbin/kfroot-wrapper": 0o755, "/etc/sudoers.d/kfroot": 0o440}

    def test_the_modes_are_the_design(self):
        box = Box(self)
        for path, mode in self.MODES.items():
            self.assertEqual(stat.S_IMODE((box.root / path.lstrip("/")).stat().st_mode), mode, path)

    def test_a_fresh_install_verifies_clean(self):
        box = Box(self)
        self.assertEqual(K.verify("ci", str(box.root), str(box.bin / "visudo")), [])

    def test_sudoers_and_wrapper_are_not_writable_by_anyone_but_their_owner(self):
        box = Box(self)
        self.assertEqual(stat.S_IMODE(box.sudoers.stat().st_mode) & 0o222, 0)  # read-only even for root: 0440
        for f in (box.sudoers, box.wrapper, box.launcher):
            self.assertEqual(stat.S_IMODE(f.stat().st_mode) & 0o022, 0, f)

    def test_install_is_idempotent(self):
        box = Box(self)
        before = box.sudoers.read_text()
        K.install("ci", str(box.root), str(box.bin / "visudo"))
        self.assertEqual(box.sudoers.read_text(), before)

    def test_a_sudoers_file_that_visudo_refuses_is_never_installed(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        write_exe(tmp / "visudo", "#!/bin/sh\necho 'syntax error'; exit 1\n")
        with self.assertRaises(K.KfrootError) as cm:
            K.install("ci", str(tmp / "root"), str(tmp / "visudo"))
        self.assertIn("visudo_refused", str(cm.exception))
        self.assertFalse((tmp / "root/etc/sudoers.d/kfroot").exists())
        self.assertFalse((tmp / "root/usr/local/bin/kfroot").exists())

    def test_a_missing_visudo_is_a_refusal_not_a_skipped_check(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        with self.assertRaises(K.KfrootError) as cm:
            K.install("ci", str(tmp / "root"), str(tmp / "no-visudo"))
        self.assertIn("visudo_missing", str(cm.exception))
        self.assertFalse((tmp / "root/etc/sudoers.d/kfroot").exists())

    def test_sudo_rs_refusing_logfile_gets_the_portable_file(self):
        box = Box(self, reject="logfile")
        self.assertEqual(box.install["audit_log"], K.AUTH_LOG)
        self.assertNotIn("logfile", box.sudoers.read_text())
        self.assertEqual(K.verify("ci", str(box.root), str(box.bin / "visudo")), [])

    def refusing_visudo(self, message):
        """A visudo that refuses a file naming `logfile` with MESSAGE and accepts the file without it."""
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        write_exe(tmp / "visudo", f"#!/bin/sh\nif grep -q logfile \"$3\"; then echo \"{message}\"; exit 1; fi\nexit 0\n")
        return tmp

    def test_a_refusal_that_does_not_name_logfile_is_final_even_if_the_plain_file_would_pass(self):
        tmp = self.refusing_visudo("x:1:1: unknown setting: 'use_pty'")
        with self.assertRaises(K.KfrootError):
            K.install("ci", str(tmp / "root"), str(tmp / "visudo"))
        self.assertFalse((tmp / "root/etc/sudoers.d/kfroot").exists())

    def test_a_refusal_naming_another_default_is_final_too(self):
        tmp = self.refusing_visudo("x:1:1: unknown setting: 'loglinelen'")
        with self.assertRaises(K.KfrootError):
            K.install("ci", str(tmp / "root"), str(tmp / "visudo"))
        self.assertFalse((tmp / "root/usr/local/bin/kfroot").exists())

    def test_a_refusal_naming_logfile_gets_the_plain_file(self):
        tmp = self.refusing_visudo("x:1:1: unknown setting: 'logfile'")
        r = K.install("ci", str(tmp / "root"), str(tmp / "visudo"))
        self.assertEqual(r["audit_log"], K.AUTH_LOG)
        self.assertTrue((tmp / "root/etc/sudoers.d/kfroot").exists())

    def test_root_cannot_be_the_ci_user(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        write_exe(tmp / "visudo", "#!/bin/sh\nexit 0\n")
        with self.assertRaises(K.KfrootError):
            K.install("root", str(tmp / "root"), str(tmp / "visudo"))
        self.assertFalse((tmp / "root/etc/sudoers.d/kfroot").exists())
        p = subprocess.run([sys.executable, str(KFROOT_PY), "install", "--user", "root", "--root", str(tmp / "root")], capture_output=True, text=True)
        self.assertEqual(p.returncode, 2)
        self.assertIn("KFROOT_REFUSED", p.stderr)

    def verify(self, box):
        return K.verify("ci", str(box.root), str(box.bin / "visudo"))

    def test_verify_names_a_loosened_sudoers_mode(self):
        box = Box(self)
        box.sudoers.chmod(0o666)
        self.assertTrue(any(p.startswith("mode ") for p in self.verify(box)), self.verify(box))

    def test_verify_names_a_loosened_launcher_mode(self):
        box = Box(self)
        box.launcher.chmod(0o777)
        self.assertTrue(any(p.startswith("mode ") and "kfroot" in p for p in self.verify(box)), self.verify(box))

    def test_verify_names_a_tampered_wrapper(self):
        box = Box(self)
        box.wrapper.write_text('#!/bin/bash\nexec "$@" # tampered\n')
        self.assertTrue(any(p.startswith("content ") for p in self.verify(box)), self.verify(box))

    def test_verify_names_a_missing_file(self):
        box = Box(self)
        box.wrapper.unlink()
        self.assertTrue(any(p.startswith("missing ") for p in self.verify(box)), self.verify(box))

    def test_verify_flags_sudo_rs_because_it_ignores_dash_e(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        write_exe(tmp / "sudo", "#!/bin/sh\necho 'sudo-rs 0.2.13'\n")
        self.assertEqual(K.sudo_flavor(str(tmp / "sudo")), "sudo-rs")
        write_exe(tmp / "sudo2", "#!/bin/sh\necho 'Sudo version 1.9.17p2'\n")
        self.assertEqual(K.sudo_flavor(str(tmp / "sudo2")), "sudo")
        self.assertEqual(K.sudo_flavor(str(tmp / "absent")), "missing")


class Bootstrap(unittest.TestCase):
    def plan(self, **kw):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        (tmp / "dev/kvm").write_text("")
        (tmp / "proc/driver/nvidia/gpus/0000:01:00.0").mkdir(parents=True)
        (tmp / "proc/driver/nvidia/gpus/0000:02:00.0").mkdir(parents=True)
        return tmp, K.plan("ci", "/workspace", str(tmp / "dev"), str(tmp / "proc"), **kw)

    def text(self, steps):
        return "\n".join(" ".join(s.argv or []) + (s.text or "") + (s.path or "") for s in steps)

    def test_the_user_is_in_group_kvm(self):
        _, steps = self.plan()
        t = self.text(steps)
        self.assertIn("useradd --create-home --shell /bin/bash --groups kvm ci", t)
        self.assertIn("usermod -aG kvm ci", t)
        self.assertIn("groupadd --system kvm", t)

    def test_dev_kvm_is_group_kvm_and_0660(self):
        tmp, steps = self.plan()
        argvs = [s.argv for s in steps if s.argv]
        self.assertIn(["chgrp", "kvm", str(tmp / "dev/kvm")], argvs)
        self.assertIn(["chmod", "0660", str(tmp / "dev/kvm")], argvs)

    def test_nvidia_nodes_are_created_before_the_drop_and_made_accessible(self):
        tmp, steps = self.plan()
        t = self.text(steps)
        if K.shutil.which("nvidia-modprobe"):
            self.assertIn("nvidia-modprobe -c 0", t)
            self.assertIn("nvidia-modprobe -u -c 0", t)
        self.assertIn("chmod a+rw", t)
        self.assertIn("nvidia-uvm", t)
        self.assertIn("nvidiactl", t)

    def test_memlock_is_unlimited_for_the_user(self):
        _, steps = self.plan()
        limits = [s for s in steps if s.path == K.LIMITS][0]
        self.assertIn("ci - memlock unlimited", limits.text)

    def test_workspace_gpu_lock_and_sudo_log_are_prepared(self):
        _, steps = self.plan()
        t = self.text(steps)
        self.assertIn("install -d -o ci -g ci -m 0755 /workspace /workspace/bench /workspace/winvm /workspace/winapps /workspace/prov", t)
        self.assertIn("chmod 0666 /tmp/kayfabe-fastguest.lock", t)
        self.assertIn("chown root:ci /var/log/kfroot-sudo.log", t)
        self.assertIn("chmod 0640 /var/log/kfroot-sudo.log", t)

    def test_dmesg_is_made_readable(self):
        _, steps = self.plan()
        self.assertIn("kernel.dmesg_restrict=0", self.text(steps))

    def test_the_checkout_is_chowned_when_named(self):
        _, steps = self.plan(chown=["/workspace/kf-ci-src"])
        self.assertIn("chown -R ci:ci /workspace/kf-ci-src", self.text(steps))

    def test_the_sudo_rs_audit_file_gets_an_acl_not_a_logfile(self):
        _, steps = self.plan(audit_log=K.AUTH_LOG)
        t = self.text(steps)
        self.assertIn("setfacl -m u:ci:r /var/log/auth.log", t)
        self.assertNotIn("touch /var/log/kfroot-sudo.log", t)

    def test_a_dry_run_executes_nothing(self):
        calls = []
        out = K.bootstrap("ci", dry_run=True, run=lambda *a, **k: calls.append(a))
        self.assertEqual(calls, [])
        self.assertEqual(out[0]["name"], "kfroot_files")
        self.assertTrue(all(r["rc"] is None for r in out))

    def test_execution_runs_each_step_and_stops_on_a_required_failure(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        ran = []

        def runner(argv, **k):
            ran.append(argv)
            rc = 1 if argv[:2] == ["chmod", "0660"] else 0
            return subprocess.CompletedProcess(argv, rc, "", "")

        real_geteuid = os.geteuid
        os.geteuid = lambda: 0
        self.addCleanup(lambda: setattr(os, "geteuid", real_geteuid))
        (tmp / "dev/kvm").write_text("")
        write_exe(tmp / "visudo", "#!/bin/sh\nexit 0\n")
        with self.assertRaises(K.KfrootError) as cm:
            K.bootstrap("ci", "/workspace", (), False, runner, str(tmp / "root"), str(tmp / "visudo"), str(tmp / "dev"), str(tmp / "proc"))
        self.assertIn("dev_kvm_mode", str(cm.exception))
        names = [a[0] for a in ran]
        self.assertIn("chgrp", names)
        self.assertNotIn("install", names)  # the workspace step comes after the failed required step

    def test_an_optional_failure_is_recorded_and_the_run_continues(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        real_geteuid = os.geteuid
        os.geteuid = lambda: 0
        self.addCleanup(lambda: setattr(os, "geteuid", real_geteuid))
        write_exe(tmp / "visudo", "#!/bin/sh\nexit 0\n")

        def runner(argv, **k):
            return subprocess.CompletedProcess(argv, 1 if "sysctl" in argv else 0, "", "")

        out = K.bootstrap("ci", "/workspace", (), False, runner, str(tmp / "root"), str(tmp / "visudo"), str(tmp / "dev"), str(tmp / "proc"))
        by = {r["name"]: r["rc"] for r in out}
        self.assertEqual(by["dmesg_readable"], 1)
        self.assertEqual(by["gpu_lock"], 0)
        self.assertTrue((tmp / "root/etc/security/limits.d/kfroot.conf").exists())

    def test_install_and_bootstrap_need_root(self):
        if os.geteuid() == 0:
            self.skipTest("this test runs as root")
        for fn in (lambda: K.install("ci"), lambda: K.bootstrap("ci")):
            with self.assertRaises(K.KfrootError):
                fn()


class Drop(unittest.TestCase):
    def test_argv_empties_every_capability_set_and_sets_no_new_privs(self):
        argv = K.drop_argv(1002, 1002, 104)
        self.assertEqual(argv[:1], ["setpriv"])
        for flag in ("--no-new-privs", "--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all"):
            self.assertIn(flag, argv)
        self.assertEqual(argv[argv.index("--reuid") + 1], "1002")
        self.assertEqual(argv[argv.index("--regid") + 1], "1002")
        self.assertEqual(argv[argv.index("--groups") + 1], "104")  # only the /dev/kvm group

    def test_argv_refuses_root_and_junk(self):
        for bad in ((0, 1, 2), (1, 0, 2), (1, 2, 0), (True, 1, 2), ("1", 1, 2), (-1, 1, 2), (None, 1, 2)):
            with self.subTest(bad=bad), self.assertRaises(K.KfrootError):
                K.drop_argv(*bad)

    def test_kfroot_fronts_the_drop_of_a_non_root_caller(self):
        self.assertEqual(K.run_prefix(1002, 1002, 104, True)[0], "kfroot")

    def test_a_root_caller_drops_directly_without_kfroot(self):
        self.assertEqual(K.run_prefix(1002, 1002, 104, False)[0], "prlimit")
        self.assertEqual(K.run_prefix(1002, 1002, 104, False, memlock=False)[0], "setpriv")

    def run_cli_drop(self, euid, fn, *args):
        """Call a kfroot.py command function with a fake identity and a fake effective uid; returns (stdout, exec argv)."""
        import io
        import argparse
        calls = {}
        saved = (K.identity, os.geteuid, os.execvpe, sys.stdout, os.environ.copy())
        K.identity = lambda user, group="kvm": (1002, 1002, 104)
        os.geteuid = lambda: euid
        os.execvpe = lambda f, argv, env: calls.update(argv=argv, env=env)
        sys.stdout = io.StringIO()
        try:
            ns = argparse.Namespace(user="ci", kvm_group="kvm", no_memlock=False, log=args[0] if args else None,
                                    command=["bash", "gate.sh", "tag"])
            fn(ns)
            out = sys.stdout.getvalue()
        finally:
            K.identity, os.geteuid, os.execvpe, sys.stdout = saved[:4]
            os.environ.clear()
            os.environ.update(saved[4])
        return out, calls

    def test_the_drop_argv_command_for_a_non_root_caller_starts_with_kfroot(self):
        out, _ = self.run_cli_drop(1000, K.cmd_drop_argv)
        self.assertTrue(out.startswith("kfroot prlimit "), out)

    def test_the_drop_argv_command_for_root_does_not(self):
        out, _ = self.run_cli_drop(0, K.cmd_drop_argv)
        self.assertTrue(out.startswith("prlimit "), out)

    def test_run_execs_the_prefix_then_the_command_with_require_and_the_log_mark(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "sudo.log"
            log.write_text("12345")
            os.environ.pop("KF_ROOTLESS", None)
            _, calls = self.run_cli_drop(1000, K.cmd_run, str(log))
        self.assertEqual(calls["argv"][:2], ["kfroot", "prlimit"])
        self.assertEqual(calls["argv"][-3:], ["bash", "gate.sh", "tag"])
        self.assertEqual(calls["env"]["KF_ROOTLESS"], "require")
        self.assertEqual(calls["env"]["KF_SUDO_LOG_MARK"], "5")

    def test_run_hands_ld_variables_to_the_command_after_the_drop(self):
        os.environ["LD_LIBRARY_PATH"] = "/opt/cuda/lib64"
        self.addCleanup(lambda: os.environ.pop("LD_LIBRARY_PATH", None))
        _, calls = self.run_cli_drop(1000, K.cmd_run)
        argv = calls["argv"]
        self.assertIn("LD_LIBRARY_PATH=/opt/cuda/lib64", argv)
        self.assertGreater(argv.index("LD_LIBRARY_PATH=/opt/cuda/lib64"), argv.index("--"))

    def test_run_keeps_an_explicit_kf_rootless_mode(self):
        os.environ["KF_ROOTLESS"] = "record"
        self.addCleanup(lambda: os.environ.pop("KF_ROOTLESS", None))
        _, calls = self.run_cli_drop(0, K.cmd_run)
        self.assertEqual(calls["env"]["KF_ROOTLESS"], "record")
        self.assertEqual(calls["argv"][0], "prlimit")

    def test_memlock_is_raised_before_the_drop(self):
        words = K.run_prefix(1002, 1002, 104, True)
        self.assertLess(words.index("--memlock=unlimited:unlimited"), words.index("setpriv"))
        self.assertEqual(words[-1], "--")

    def test_ld_variables_are_carried_past_the_drop_not_through_sudo(self):
        words = K.run_prefix(1002, 1002, 104, True, environ={"LD_LIBRARY_PATH": "/opt/x", "PATH": "/bin", "LD_PRELOAD": "/y.so"})
        self.assertGreater(words.index("env"), words.index("--"))
        self.assertEqual(words[words.index("env") + 1:], ["LD_LIBRARY_PATH=/opt/x", "LD_PRELOAD=/y.so"])
        self.assertNotIn("env", K.run_prefix(1002, 1002, 104, True, environ={"PATH": "/bin"}))

    def test_a_real_drop_produces_the_state_the_assertion_demands(self):
        if os.geteuid() != 0 or not K.shutil.which("setpriv"):
            self.skipTest("needs root and setpriv")
        probe = "import sys; sys.path.insert(0, %r); import kfroot as K; t = K.assert_tree([__import__('os').getpid()], with_tree=False); print(t['violations'])" % str(KFROOT_PY.parent)
        argv = K.drop_argv(65534, 65534, 65534) + ["--", sys.executable, "-I", "-c", probe]
        env = dict(os.environ, PYTHONPATH=str(KFROOT_PY.parent))
        p = subprocess.run(argv, capture_output=True, text=True, env=env)
        if p.returncode != 0 and "cannot" in p.stderr.lower():
            self.skipTest(p.stderr.strip())
        self.assertEqual(p.stdout.strip(), "[]", p.stdout + p.stderr)
        undropped = subprocess.run([sys.executable, "-I", "-c", probe], capture_output=True, text=True, env=env)
        self.assertNotEqual(undropped.stdout.strip(), "[]")


WRAPPED = (
    "Nov 15 12:36:53 : ci : PWD=/tmp ; USER=root ;\n"
    "    COMMAND=/usr/local/sbin/kfroot-wrapper sh -c 'echo \"FOO=$FOO HOME=$HOME\n"
    "    PATH=$PATH USER=$USER\"'\n"
)
FLAT = "Nov 15 12:36:53 : ci : PWD=/tmp ; USER=root ; COMMAND=/usr/local/sbin/kfroot-wrapper apt-get install -y x\n"
SYSLOG = ("2026-11-15T12:36:34.271351+00:00 host sudo:       ci : TTY=pts/0 ; PWD=/tmp ; USER=root ; "
          "COMMAND=/usr/local/sbin/kfroot-wrapper tee -a /etc/x\n")
STRAY = "Nov 15 12:36:53 : ci : a password is required ; PWD=/tmp ; USER=root ; COMMAND=/usr/bin/id -u\n"
NOTALLOWED = "Nov 15 12:36:53 : ci : command not allowed ; PWD=/tmp ; USER=root ; COMMAND=/usr/bin/apt-get install x\n"


class Audit(unittest.TestCase):
    def clean(self, text):
        r = K.audit_check(text)
        self.assertEqual((r["entries"], r["kfroot_calls"], r["violations"]), (1, 1, []), r)

    def test_a_wrapped_logfile_entry_is_clean(self):
        self.clean(WRAPPED)

    def test_a_flat_logfile_entry_is_clean(self):
        self.clean(FLAT)

    def test_a_syslog_entry_is_clean(self):
        self.clean(SYSLOG)

    def test_a_wrapped_stray_entry_is_still_found(self):
        wrapped_stray = "Nov 15 12:36:53 : ci : a password is required ; PWD=/tmp ;\n    USER=root ; COMMAND=/usr/bin/id -u\n"
        self.assertEqual(len(K.audit_check(wrapped_stray)["violations"]), 1)

    def test_the_failure_reason_is_recorded_on_the_entry(self):
        self.assertEqual(K.audit_entries(STRAY)[0]["failure"], "a password is required")
        self.assertEqual(K.audit_entries(NOTALLOWED)[0]["failure"], "command not allowed")
        self.assertEqual(K.audit_entries(FLAT)[0]["failure"], "")

    def test_the_command_line_fails_closed_on_an_unparsable_entry(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "log"
            log.write_text("garbage COMMAND=/bin/sh -c evil\n")
            p = subprocess.run([sys.executable, str(KFROOT_PY), "audit-check", "--since", "0", "--log", str(log)], capture_output=True, text=True)
            self.assertEqual(p.returncode, 1, p.stdout)

    def test_a_stray_sudo_is_a_violation(self):
        r = K.audit_check(FLAT + STRAY)
        self.assertEqual(len(r["violations"]), 1)
        self.assertEqual(r["violations"][0]["command"], "/usr/bin/id -u")

    def test_a_refused_command_is_a_violation_even_if_it_names_nothing_else(self):
        self.assertEqual(len(K.audit_check(NOTALLOWED)["violations"]), 1)

    def test_a_failure_marker_on_a_wrapper_line_is_a_violation(self):
        line = "Nov 15 12:36:53 : ci : a password is required ; PWD=/tmp ; USER=root ; COMMAND=/usr/local/sbin/kfroot-wrapper true\n"
        self.assertEqual(len(K.audit_check(line)["violations"]), 1)

    def test_a_lookalike_path_is_not_the_wrapper(self):
        for cmd in ("/tmp/usr/local/sbin/kfroot-wrapper true", "/usr/local/sbin/kfroot-wrapper2 true", "kfroot-wrapper true"):
            with self.subTest(cmd):
                line = f"Nov 15 12:36:53 : ci : PWD=/tmp ; USER=root ; COMMAND={cmd}\n"
                self.assertEqual(len(K.audit_check(line)["violations"]), 1)

    def test_other_users_are_not_this_runs_business_but_ci_is(self):
        other = FLAT.replace(": ci :", ": postgres :").replace("kfroot-wrapper", "something-else")
        self.assertEqual(K.audit_check(other)["entries"], 0)
        self.assertEqual(len(K.audit_check(other.replace("postgres", "ci"))["violations"]), 1)

    def test_an_unreadable_line_that_names_a_command_fails_closed(self):
        r = K.audit_check("garbage line COMMAND=/bin/sh -c evil\n")
        self.assertEqual(len(r["violations"]), 1)

    def test_lines_without_a_command_are_ignored(self):
        text = "Nov 15 pam_unix(sudo:session): session opened for user root(uid=0) by (uid=1003)\n"
        self.assertEqual(K.audit_check(text)["entries"], 0)

    def test_the_window_starts_at_the_mark_not_at_the_top(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "log"
            log.write_text(STRAY)
            mark = K.audit_mark(str(log))
            self.assertEqual(mark, len(STRAY.encode()))
            with open(log, "a") as f:
                f.write(FLAT)
            self.assertEqual(K.audit_check(K.audit_text(str(log), mark))["violations"], [])
            self.assertEqual(len(K.audit_check(K.audit_text(str(log), 0))["violations"]), 1)

    def test_a_rotated_log_is_read_whole(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "log"
            log.write_text(STRAY)
            log.write_text(STRAY[:20])  # shorter than the mark
            self.assertIn("Nov 15", K.audit_text(str(log), len(STRAY)))

    def test_a_missing_or_unreadable_log_is_a_refusal_not_an_empty_window(self):
        with self.assertRaises(K.KfrootError):
            K.audit_text("/nonexistent/kfroot-sudo.log", 0)
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(K.KfrootError):
                K.audit_text(d, 0)  # a directory cannot be read as a log

    def test_the_command_line_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "log"
            log.write_text(FLAT)
            base = [sys.executable, str(KFROOT_PY), "audit-check", "--since", "0", "--log", str(log)]
            self.assertEqual(subprocess.run(base, capture_output=True).returncode, 0)
            log.write_text(FLAT + STRAY)
            self.assertEqual(subprocess.run(base, capture_output=True).returncode, 1)
            gone = subprocess.run(base[:-1] + [str(Path(d) / "absent")], capture_output=True)
            self.assertEqual(gone.returncode, 2)


CAPS0 = "0000000000000000"
FULL = "000001ffffffffff"


def status(uid=1002, inh=CAPS0, prm=CAPS0, eff=CAPS0, bnd=CAPS0, amb=CAPS0, nnp="1"):
    lines = ["Name:\tqemu-system-x86", f"Uid:\t{uid}\t{uid}\t{uid}\t{uid}", f"Gid:\t{uid}\t{uid}\t{uid}\t{uid}",
             f"CapInh:\t{inh}", f"CapPrm:\t{prm}", f"CapEff:\t{eff}", f"CapBnd:\t{bnd}", f"CapAmb:\t{amb}"]
    if nnp is not None:
        lines.append(f"NoNewPrivs:\t{nnp}")
    return "\n".join(lines) + "\n"


class FakeProc:
    def __init__(self, tc):
        self.tmp = Path(tempfile.mkdtemp(prefix="kfroot-proc-"))
        tc.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.tmp)]))

    def add(self, pid, ppid=1, comm="proc", **kw):
        d = self.tmp / str(pid)
        d.mkdir()
        (d / "status").write_text(status(**kw))
        (d / "stat").write_text(f"{pid} ({comm} with ) paren) S {ppid} {pid} {pid} 0 -1 0\n")
        (d / "comm").write_text(comm + "\n")
        return pid


class ProcAssertion(unittest.TestCase):
    def check(self, **kw):
        return K.check_status(K.parse_status(status(**kw)))

    def test_a_dropped_process_has_no_violation(self):
        self.assertEqual(self.check(), [])

    def test_root_uid_in_any_of_the_four_ids_is_a_violation(self):
        self.assertTrue(self.check(uid=0))
        fields = K.parse_status(status().replace("Uid:\t1002\t1002\t1002\t1002", "Uid:\t1002\t1002\t0\t1002"))
        self.assertTrue(any(v.startswith("uid_root") for v in K.check_status(fields)))

    def test_each_capability_set_is_checked(self):
        for name in ("prm", "eff", "bnd", "inh", "amb"):
            with self.subTest(name):
                bad = self.check(**{name: FULL})
                self.assertEqual(len(bad), 1)
                self.assertTrue(bad[0].startswith(f"cap{name}_held"), bad)

    def test_one_held_capability_is_enough(self):
        self.assertTrue(self.check(eff="0000000000200000"))  # CAP_SYS_ADMIN alone

    def test_no_new_privs_must_be_one(self):
        self.assertTrue(any("no_new_privs_not_set" in v for v in self.check(nnp="0")))
        self.assertTrue(any("no_new_privs_not_set" in v for v in self.check(nnp=None)))

    def test_an_unreadable_field_is_a_violation_not_a_pass(self):
        fields = K.parse_status("Name:\tx\n")
        bad = K.check_status(fields)
        self.assertIn("uid_unreadable", bad)
        self.assertEqual(len([v for v in bad if v.endswith("_unreadable")]), 6)
        self.assertTrue(K.check_status(K.parse_status(status().replace("CapBnd:\t" + CAPS0, "CapBnd:\tzz"))))

    def test_a_dropped_tree_passes_and_every_process_is_read(self):
        fp = FakeProc(self)
        fp.add(100, comm="gate.sh")
        fp.add(101, ppid=100, comm="qemu-system-x86")
        fp.add(102, ppid=101, comm="helper")
        r = K.assert_tree([100], str(fp.tmp))
        self.assertEqual(r["violations"], [])
        self.assertEqual(sorted(p["pid"] for p in r["processes"]), [100, 101, 102])

    def test_a_root_descendant_is_found(self):
        fp = FakeProc(self)
        fp.add(100)
        fp.add(101, ppid=100, comm="sudo", uid=0, eff=FULL, prm=FULL, bnd=FULL, nnp="0")
        r = K.assert_tree([100], str(fp.tmp))
        self.assertTrue(any("101(sudo) uid_root" in v for v in r["violations"]), r)

    def test_an_unrelated_root_process_is_not_in_the_tree(self):
        fp = FakeProc(self)
        fp.add(100)
        fp.add(200, uid=0, eff=FULL)
        self.assertEqual(K.assert_tree([100], str(fp.tmp))["violations"], [])

    def test_a_comm_with_spaces_and_parentheses_keeps_its_parent(self):
        fp = FakeProc(self)
        fp.add(100)
        fp.add(101, ppid=100, comm="a b) c")
        self.assertEqual(K.tree_pids([100], str(fp.tmp)), [100, 101])

    def test_a_named_pid_that_is_gone_is_a_violation(self):
        fp = FakeProc(self)
        fp.add(100)
        r = K.assert_tree([100, 555], str(fp.tmp))
        self.assertTrue(any("pid_unreadable 555" in v for v in r["violations"]), r)

    def test_nothing_read_is_not_a_pass(self):
        fp = FakeProc(self)
        self.assertEqual(K.assert_tree([], str(fp.tmp))["violations"], ["no_process_read"])

    def run_cli(self, fp, *args):
        return subprocess.run([sys.executable, str(KFROOT_PY), "assert-tree", *args], capture_output=True, text=True,
                              env={**os.environ, "KFROOT_PROC": str(fp.tmp)})

    def test_the_command_line_require_mode_fails_on_root_and_record_mode_does_not(self):
        fp = FakeProc(self)
        fp.add(100, uid=0, eff=FULL, prm=FULL, bnd=FULL, nnp="0")
        out = Path(tempfile.mkdtemp()) / "r.json"
        req = self.run_cli(fp, "--pid", "100", "--mode", "require", "--out", str(out))
        self.assertEqual(req.returncode, 1, req.stdout)
        self.assertIn("ok=0", req.stdout)
        self.assertIn("uid=0", req.stdout)
        rec = self.run_cli(fp, "--pid", "100", "--mode", "record")
        self.assertEqual(rec.returncode, 0)
        self.assertIn("ok=0", rec.stdout)
        doc = json.loads(out.read_text())
        self.assertFalse(doc["ok"])
        self.assertEqual(doc["mode"], "require")

    def test_the_command_line_summary_line_for_a_dropped_tree(self):
        fp = FakeProc(self)
        fp.add(100)
        fp.add(101, ppid=100)
        r = self.run_cli(fp, "--pid", "100", "--mode", "require")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip(), "KFROOT_ASSERT mode=require ok=1 uid=1002 capeff=0 capprm=0 capbnd=0 nnp=1 procs=2")

    def test_a_pid_file_names_a_process_and_a_missing_one_is_a_violation(self):
        fp = FakeProc(self)
        fp.add(100)
        pf = Path(tempfile.mkdtemp()) / "qemu.pid"
        pf.write_text("100\n")
        self.assertEqual(self.run_cli(fp, "--pid-file", str(pf), "--mode", "require").returncode, 0)
        gone = self.run_cli(fp, "--pid-file", str(pf.parent / "absent.pid"), "--mode", "require")
        self.assertEqual(gone.returncode, 1)
        self.assertIn("pid_gone", gone.stdout)

    def test_the_sudo_window_is_part_of_the_assertion(self):
        fp = FakeProc(self)
        fp.add(100)
        log = Path(tempfile.mkdtemp()) / "sudo.log"
        log.write_text(FLAT)
        ok = self.run_cli(fp, "--pid", "100", "--mode", "require", "--sudo-since", "0", "--sudo-log", str(log))
        self.assertEqual(ok.returncode, 0, ok.stdout)
        self.assertIn("sudo_entries=1 sudo_violations=0", ok.stdout)
        log.write_text(FLAT + STRAY)
        bad = self.run_cli(fp, "--pid", "100", "--mode", "require", "--sudo-since", "0", "--sudo-log", str(log))
        self.assertEqual(bad.returncode, 1)
        self.assertIn("sudo_violations=1", bad.stdout)

    def test_a_mutant_that_runs_the_step_as_root_is_caught_by_the_real_proc(self):
        # this very process, whatever uid it has: an undropped process (NoNewPrivs 0, full bounding set) is a violation
        r = subprocess.run([sys.executable, str(KFROOT_PY), "assert-tree", "--pid", str(os.getpid()), "--mode", "require"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("ok=0", r.stdout)


class LibShell(unittest.TestCase):
    def sh(self, script, **env):
        e = {**os.environ, **env}
        return subprocess.run(["bash", "-c", f". {shlex.quote(str(LIB))}; {script}"], capture_output=True, text=True, env=e, timeout=60)

    def fake_kfroot(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        write_exe(tmp / "kfroot", '#!/bin/sh\necho "KFROOT-BINARY $*"\n')
        return tmp

    def test_as_root_the_command_runs_directly(self):
        r = self.sh("kfroot echo direct", KFROOT_EUID="0")
        self.assertEqual((r.returncode, r.stdout), (0, "direct\n"))

    def test_unprivileged_goes_through_the_kfroot_program(self):
        bindir = self.fake_kfroot()
        st = bindir / "status"
        st.write_text("Name:\tbash\nNoNewPrivs:\t0\n")
        r = self.sh("kfroot apt-get install x", KFROOT_EUID="1002", KFROOT_SELF_STATUS=str(st), PATH=f"{bindir}:{os.environ['PATH']}")
        self.assertEqual((r.returncode, r.stdout), (0, "KFROOT-BINARY apt-get install x\n"))

    def test_inside_a_no_new_privs_tree_it_is_refused_with_126_and_runs_nothing(self):
        bindir = self.fake_kfroot()
        st = bindir / "status"
        st.write_text("Name:\tbash\nNoNewPrivs:\t1\n")
        r = self.sh("kfroot apt-get install x", KFROOT_EUID="1002", KFROOT_SELF_STATUS=str(st), PATH=f"{bindir}:{os.environ['PATH']}")
        self.assertEqual(r.returncode, 126)
        self.assertNotIn("KFROOT-BINARY", r.stdout)
        self.assertIn("KFROOT_REFUSED", r.stderr)

    def dmesg_env(self, restrict, euid="1002"):
        bindir = self.fake_kfroot()
        write_exe(bindir / "dmesg", "#!/bin/sh\necho DIRECT-DMESG\n")
        st = bindir / "status"
        st.write_text("NoNewPrivs:\t0\n")
        flag = bindir / "restrict"
        flag.write_text(f"{restrict}\n")
        return dict(KFROOT_EUID=euid, KFROOT_SELF_STATUS=str(st), PATH=f"{bindir}:{os.environ['PATH']}", KFROOT_DMESG_RESTRICT=str(flag))

    def test_a_restricted_kernel_log_is_read_through_kfroot_by_an_unprivileged_caller(self):
        self.assertEqual(self.sh("kf_dmesg", **self.dmesg_env(1)).stdout, "KFROOT-BINARY dmesg\n")

    def test_an_unrestricted_kernel_log_is_read_directly(self):
        self.assertEqual(self.sh("kf_dmesg", **self.dmesg_env(0)).stdout, "DIRECT-DMESG\n")

    def test_root_reads_the_kernel_log_directly_whatever_the_restriction(self):
        self.assertEqual(self.sh("kf_dmesg", **self.dmesg_env(1, euid="0")).stdout, "DIRECT-DMESG\n")

    def test_an_unreadable_restriction_file_counts_as_restricted(self):
        env = self.dmesg_env(1)
        env["KFROOT_DMESG_RESTRICT"] = "/nonexistent/dmesg_restrict"
        self.assertEqual(self.sh("kf_dmesg", **env).stdout, "KFROOT-BINARY dmesg\n")

    def test_the_assert_helper_modes(self):
        with tempfile.TemporaryDirectory() as d:
            off = self.sh(f"kf_rootless_assert x {d}", KF_ROOTLESS="off")
            self.assertEqual((off.returncode, off.stdout.strip()), (0, "KFROOT_ASSERT mode=off ok=skipped label=x"))
            bad = self.sh(f"kf_rootless_assert x {d}", KF_ROOTLESS="sometimes")
            self.assertEqual(bad.returncode, 1)
            # this test process is undropped: require fails, record reports and passes
            req = self.sh(f"kf_rootless_assert x {d} --pid $$", KF_ROOTLESS="require")
            self.assertEqual(req.returncode, 1, req.stdout)
            rec = self.sh(f"kf_rootless_assert x {d} --pid $$", KF_ROOTLESS="record")
            self.assertEqual(rec.returncode, 0, rec.stdout + rec.stderr)
            self.assertIn("ok=0", rec.stdout)
            self.assertTrue((Path(d) / "rootless_x.json").exists())

    def test_a_crash_of_the_checker_is_not_a_pass_in_require_mode(self):
        with tempfile.TemporaryDirectory() as d:
            r = self.sh(f"kf_rootless_assert x {d}", KF_ROOTLESS="require")  # nothing to assert: the tool refuses (rc 2)
            self.assertNotEqual(r.returncode, 0)


def code(path):
    return "\n".join(s for s in Path(path).read_text().splitlines() if not s.lstrip().startswith("#"))


class HarnessIsGreppable(unittest.TestCase):
    """Every root operation of the Windows harness is a `kfroot` line; no blanket root refusal is left."""

    SCRIPTS = (GATE, APPS, WINVM)

    def test_no_blanket_root_refusal_remains(self):
        for path in (GATE, APPS):
            text = code(path)
            self.assertNotIn("refuse not-root", text, path.name)
            self.assertNotRegex(text, r'id -u\)" = 0 \] \|\| refuse', path.name)

    def test_no_bare_sudo_in_the_harness(self):
        for path in (*self.SCRIPTS, LIB):
            self.assertNotRegex(code(path), r"(?m)(^\s*|[;|&(]\s*|\$\(\s*)sudo\s", path.name)

    def test_apt_and_apparmor_run_only_behind_kfroot(self):
        gate = code(GATE)
        self.assertRegex(gate, r"kfroot \"\$\{apt_run\[@\]\}\"")
        for line in gate.splitlines():
            if re.search(r"\bapt-get\b", line):
                self.assertTrue("kfroot" in line or line.strip().startswith("apt_run=") or "apt_run=(" in line or "type -t apt-get" in line, line)
        vm = code(WINVM)
        self.assertRegex(vm, r"kfroot tee -a \"\$local\"")
        self.assertRegex(vm, r"kfroot apparmor_parser -r \"\$prof\"")
        for line in vm.splitlines():
            if re.search(r"\bapparmor_parser\b", line):
                self.assertIn("kfroot", line)

    def test_the_kernel_log_is_read_through_kf_dmesg_only(self):
        for path in (GATE, APPS, WINVM):
            for line in code(path).splitlines():
                if re.search(r"(^|[\s$(|;])dmesg\b", line) and "_host_dmesg" not in line and "kf_dmesg" not in line:
                    self.fail(f"{path.name}: raw dmesg: {line}")
            self.assertIn("kf_dmesg", code(path), path.name)

    def test_each_root_step_says_why_it_needs_root(self):
        for path in (GATE, WINVM):
            lines = Path(path).read_text().splitlines()
            for i, line in enumerate(lines):
                if re.search(r"\bkfroot (tee|apparmor_parser)\b|DEBIAN_FRONTEND=noninteractive kfroot", line):
                    window = "\n".join(lines[max(0, i - 4):i + 1])
                    self.assertRegex(window, r"# +root:", f"{path.name}:{i + 1}")

    def test_the_scripts_source_the_library(self):
        for path in (GATE, APPS, WINVM):
            self.assertIn("scripts/lib/kfroot.sh", code(path), path.name)

    def test_the_gate_asserts_before_it_checks_anything_else(self):
        text = code(GATE)
        pre = text[text.index("stage_begin preflight"):]
        self.assertLess(pre.index("rootless_check preflight"), pre.index("no-/dev/kvm-access"))

    def test_the_gate_checks_capabilities_by_name(self):
        text = code(GATE)
        for why in ("no-/dev/kvm-access", "no-/dev/nvidiactl-access", "not-writable-", "dmesg-unreadable", "no-cargo-for-uid-"):
            self.assertIn(why, text)
        self.assertRegex(text, r"\[ -r /dev/kvm \] && \[ -w /dev/kvm \]")

    def test_the_pass_line_records_the_rootless_state(self):
        self.assertRegex(code(GATE), r'verdict PASS NONE "rootless=\$ROOTLESS_STATE ')
        self.assertIn("rootless=$ROOTLESS_STATE", code(APPS))

    def test_a_violated_require_run_is_never_a_pass(self):
        gate = code(GATE)
        i = gate.index('elif [ "${ROOTLESS_FINAL_RC:-0}" != 0 ]')
        self.assertLess(i, gate.index("verdict PASS NONE"))
        self.assertIn("verdict RETRY OPERATOR", gate[i:i + 400])
        self.assertIn('[ "$KIND" = PASS ] && [ "${ROOTLESS_FINAL_RC:-0}" != 0 ]', code(APPS))

    def test_the_prepare_mode_does_the_root_steps_and_exits(self):
        text = code(GATE)
        self.assertIn('if [ "${1:-}" = --prepare ]', text)
        self.assertRegex(text, r"if \[ \"\$PREPARE\" = 1 \]; then\n\s+VERDICT=PREPARED\n\s+prepare_root")
        self.assertIn('bash "$WV" prepare', text)
        self.assertIn("prepare) install -d", code(WINVM))

    def test_a_missing_package_in_a_no_new_privs_tree_is_an_operator_retry_naming_prepare(self):
        text = code(GATE)
        self.assertRegex(text, r'\[ "\$prc" = 126 \]')
        self.assertIn("--prepare", text[text.index('[ "$prc" = 126 ]'):][:400])


class GateSelftest(unittest.TestCase):
    def test_the_gate_selftest_covers_the_rootless_verdicts(self):
        p = subprocess.run(["bash", str(GATE), "--selftest"], capture_output=True, text=True, timeout=300,
                           env={k: v for k, v in os.environ.items() if k not in ("KF_ROOTLESS", "KF_SUDO_LOG_MARK")})
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertRegex(p.stdout, r"^WINDOWS_GATE_SELFTEST_OK cases=27$")
        text = code(GATE)
        for case in ("pass-not-rootless", "guest-fail-not-rootless", "pass-records-rootless"):
            self.assertIn(case, text)

    def test_a_not_rootless_run_cannot_turn_a_pass_green_in_the_selftest(self):
        # SELFTEST_ROOTLESS_RC=1 reaches conclude(): the PASS branch must become a RETRY (the selftest case is the proof)
        p = subprocess.run(["bash", str(GATE), "--selftest"], capture_output=True, text=True, timeout=300)
        self.assertNotIn("case=pass-not-rootless", p.stdout)


class GateRuns(unittest.TestCase):
    """The real gate script, its rootless step aimed at a stub or at the real tool."""

    def run_gate(self, mode, stub=None, wrap=None, tag="tagr"):
        d = Path(tempfile.mkdtemp(prefix="kfroot-gate-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        d.chmod(0o777)
        env = {**os.environ, "WINVM_ROOT": str(d / "w"), "BENCH": str(d / "b")}
        env.pop("KF_SUDO_LOG_MARK", None)
        if mode is not None:
            env["KF_ROOTLESS"] = mode
        else:
            env.pop("KF_ROOTLESS", None)
        if stub:
            s = d / "stub.py"
            s.write_text(stub)
            env["KF_KFROOT_PY"] = str(s)
        argv = ["bash", str(GATE), tag, str(d)]
        if wrap:
            argv = wrap + argv
        p = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=120)
        return p, d

    STUB_BAD = "import sys\nprint('KFROOT_ASSERT mode=x ok=0 uid=0 capeff=held')\nsys.exit(1 if '--mode' in sys.argv and sys.argv[sys.argv.index('--mode')+1] == 'require' else 0)\n"
    STUB_OK = "print('KFROOT_ASSERT mode=x ok=1 uid=1002 capeff=0 capprm=0 capbnd=0 nnp=1 procs=1')\n"

    def test_require_mode_with_a_root_tree_is_refused_before_anything_else(self):
        p, _ = self.run_gate("require", self.STUB_BAD)
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("WINDOWS_GATE_REFUSED why=rootless-violated", p.stdout)
        self.assertNotIn("WINDOWS_GATE_PASS", p.stdout)
        self.assertRegex(p.stdout.strip().splitlines()[-1], r"^WINDOWS_GATE_EXIT rc=2 ")

    def test_record_mode_records_a_violation_and_goes_on_to_the_next_check(self):
        p, d = self.run_gate("record", self.STUB_BAD)
        self.assertIn("KFROOT_ASSERT mode=x ok=0", p.stdout)
        self.assertNotIn("why=rootless-violated", p.stdout)
        self.assertEqual(p.returncode, 2)  # refused later, by name (this machine has no 20 GB of RAM, no kvm, ...)
        self.assertRegex(p.stdout, r"WINDOWS_GATE_REFUSED why=(no-/dev/kvm-access|not-writable|no-cargo|dmesg-unreadable|disk-|ram-|no-flags)")

    def test_an_ok_assertion_is_not_a_refusal(self):
        p, _ = self.run_gate("require", self.STUB_OK)
        self.assertNotIn("why=rootless-violated", p.stdout, p.stdout)

    def test_off_mode_asserts_nothing(self):
        p, _ = self.run_gate("off", self.STUB_BAD)
        self.assertNotIn("KFROOT_ASSERT mode=x", p.stdout)
        self.assertIn("KFROOT_ASSERT mode=off", p.stdout)

    def test_the_default_mode_is_record(self):
        p, _ = self.run_gate(None, self.STUB_BAD)
        self.assertIn("KFROOT_ASSERT mode=x ok=0", p.stdout)
        self.assertNotIn("why=rootless-violated", p.stdout)

    def test_a_crashing_assertion_in_require_mode_is_a_refusal(self):
        p, _ = self.run_gate("require", "import sys\nsys.exit(2)\n")
        self.assertIn("WINDOWS_GATE_REFUSED why=rootless-violated", p.stdout)

    def test_an_unknown_mode_is_a_refusal(self):
        p, _ = self.run_gate("sometimes", self.STUB_OK)
        self.assertIn("WINDOWS_GATE_REFUSED why=rootless-violated", p.stdout)
        self.assertEqual(p.returncode, 2)

    def test_a_crashing_assertion_in_record_mode_does_not_stop_the_gate(self):
        p, _ = self.run_gate("record", "import sys\nsys.exit(2)\n")
        self.assertNotIn("why=rootless-violated", p.stdout)
        self.assertIn("error=rc2", p.stdout)

    def test_the_real_tool_refuses_an_undropped_root_gate_in_require_mode(self):
        if os.geteuid() != 0:
            self.skipTest("the undropped test process is checked by the proc tests; this one needs root")
        p, _ = self.run_gate("require")
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("why=rootless-violated", p.stdout)
        self.assertIn("uid_root", p.stdout)

    def test_the_real_tool_passes_a_dropped_gate_and_the_next_refusal_is_by_name(self):
        if os.geteuid() != 0 or not K.shutil.which("setpriv"):
            self.skipTest("needs root and setpriv to drop")
        wrap = K.drop_argv(65534, 65534, 65534) + ["--"]
        p, _ = self.run_gate("require", wrap=wrap)
        if "not-writable" in p.stdout and "/workspace" not in p.stdout and str(ROOT) in p.stdout:
            pass
        self.assertNotIn("why=rootless-violated", p.stdout, p.stdout + p.stderr)
        self.assertIn("KFROOT_ASSERT mode=require ok=1 uid=65534 capeff=0 capprm=0 capbnd=0 nnp=1", p.stdout)
        self.assertRegex(p.stdout, r"WINDOWS_GATE_REFUSED why=(no-/dev/kvm-access|not-writable-|no-cargo)")


class BootstrapSemantics(unittest.TestCase):
    """The plan executed against stubs, and its numbers read as numbers (not only as text)."""

    def steps(self, **kw):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        (tmp / "dev/kvm").write_text("")
        return {s.name: s for s in K.plan("ci", "/workspace", str(tmp / "dev"), str(tmp / "proc"), **kw)}

    def shell_step(self, step, stubs):
        """Run the `sh -c` body of a plan step with PATH stubs that log their argv."""
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        log = tmp / "calls"
        for name, rc in stubs.items():
            write_exe(tmp / "bin" / name, f'#!/bin/sh\necho "{name} $*" >> {log}\nexit {rc}\n')
        p = subprocess.run(step.argv, env={**os.environ, "PATH": f"{tmp / 'bin'}:/usr/bin:/bin"}, capture_output=True, text=True)
        return p, (log.read_text() if log.exists() else "")

    def test_a_missing_user_is_created_in_group_kvm(self):
        p, calls = self.shell_step(self.steps()["user"], {"id": 1, "useradd": 0, "usermod": 0})
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("useradd --create-home --shell /bin/bash --groups kvm ci", calls)
        self.assertNotIn("usermod", calls)

    def test_an_existing_user_is_added_to_group_kvm(self):
        p, calls = self.shell_step(self.steps()["user"], {"id": 0, "useradd": 0, "usermod": 0})
        self.assertIn("usermod -aG kvm ci", calls)
        self.assertNotIn("useradd", calls)

    def test_the_gpu_lock_is_world_read_write(self):
        step = self.steps()["gpu_lock"]
        mode = int(re.search(r"chmod (0[0-7]{3}) ", " ".join(step.argv))[1], 8)
        self.assertEqual(mode & 0o006, 0o006)

    def test_the_sudo_log_is_group_readable_and_not_world_readable(self):
        step = self.steps()["sudo_log"]
        mode = int(re.search(r"chmod (0[0-7]{3}) ", " ".join(step.argv))[1], 8)
        self.assertEqual((mode & 0o040, mode & 0o007), (0o040, 0))
        self.assertIn("chown root:ci", " ".join(step.argv))

    def test_the_kernel_log_restriction_is_set_to_zero_not_one(self):
        step = self.steps()["dmesg_readable"]
        self.assertEqual(re.search(r"kernel\.dmesg_restrict=(\d)", " ".join(step.argv))[1], "0")

    def test_the_memlock_limit_is_unlimited_in_the_written_file(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        write_exe(tmp / "visudo", "#!/bin/sh\nexit 0\n")
        real = os.geteuid
        os.geteuid = lambda: 0
        self.addCleanup(lambda: setattr(os, "geteuid", real))
        K.bootstrap("ci", "/workspace", (), False, lambda a, **k: subprocess.CompletedProcess(a, 0, "", ""), str(tmp / "root"),
                    str(tmp / "visudo"), str(tmp / "dev"), str(tmp / "proc"))
        self.assertRegex((tmp / "root/etc/security/limits.d/kfroot.conf").read_text(), r"(?m)^ci - memlock unlimited$")

    def test_a_failing_user_step_stops_everything_after_it(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        (tmp / "dev").mkdir()
        write_exe(tmp / "visudo", "#!/bin/sh\nexit 0\n")
        ran = []

        def runner(argv, **k):
            ran.append(argv)
            return subprocess.CompletedProcess(argv, 1 if "useradd" in " ".join(argv) else 0, "", "")

        real = os.geteuid
        os.geteuid = lambda: 0
        self.addCleanup(lambda: setattr(os, "geteuid", real))
        with self.assertRaises(K.KfrootError):
            K.bootstrap("ci", "/workspace", (), False, runner, str(tmp / "root"), str(tmp / "visudo"), str(tmp / "dev"), str(tmp / "proc"))
        self.assertFalse(any("install -d" in " ".join(a) for a in ran))
        self.assertFalse((tmp / "root/etc/security/limits.d/kfroot.conf").exists())

    def test_a_dry_run_writes_no_file_and_reports_no_status(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(tmp)]))
        out = K.bootstrap("ci", dry_run=True, root=str(tmp / "root"), dev=str(tmp / "dev"), proc=str(tmp / "proc"))
        self.assertFalse((tmp / "root").exists())
        self.assertTrue(all(r["rc"] is None for r in out))
        p = subprocess.run([sys.executable, str(KFROOT_PY), "bootstrap", "--dry-run"], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertTrue(p.stdout.startswith("KFROOT_BOOTSTRAP {"))


class AssertSemantics(unittest.TestCase):
    def cli(self, fp, *args):
        return subprocess.run([sys.executable, str(KFROOT_PY), "assert-tree", *args], capture_output=True, text=True,
                              env={**os.environ, "KFROOT_PROC": str(fp.tmp)})

    def test_a_capless_non_root_process_without_no_new_privs_is_a_violation(self):
        fp = FakeProc(self)
        fp.add(100, nnp="0")
        r = self.cli(fp, "--pid", "100", "--mode", "require")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("nnp=0", r.stdout)

    def test_no_new_privs_is_the_only_missing_thing_in_the_report(self):
        fp = FakeProc(self)
        fp.add(100, nnp="0")
        out = Path(tempfile.mkdtemp()) / "r.json"
        self.cli(fp, "--pid", "100", "--mode", "require", "--out", str(out))
        self.assertEqual(json.loads(out.read_text())["violations"], ["100(proc) no_new_privs_not_set 0"])

    def test_a_status_file_without_the_bounding_set_is_a_violation(self):
        fp = FakeProc(self)
        fp.add(100)
        path = fp.tmp / "100/status"
        path.write_text("".join(ln for ln in path.read_text().splitlines(True) if not ln.startswith("CapBnd")))
        r = self.cli(fp, "--pid", "100", "--mode", "require")
        self.assertEqual(r.returncode, 1)
        self.assertIn("capbnd_unreadable", r.stdout)

    def test_a_garbled_uid_line_is_a_violation(self):
        fields = K.parse_status(status().replace("Uid:\t1002\t1002\t1002\t1002", "Uid:\tx\ty"))
        self.assertIn("uid_unreadable", K.check_status(fields))

    def test_an_expected_pid_that_is_gone_is_named(self):
        fp = FakeProc(self)
        fp.add(100)
        r = K.assert_tree([100], str(fp.tmp), gone=["pidfile run/swtpm.pid"])
        self.assertTrue(any(v.startswith("pid_gone") for v in r["violations"]), r)

    def test_nothing_to_read_is_a_violation_for_an_empty_list_and_without_the_tree(self):
        fp = FakeProc(self)
        self.assertEqual(K.assert_tree([], str(fp.tmp))["violations"], ["no_process_read"])
        self.assertEqual(K.assert_tree([], str(fp.tmp), with_tree=False)["violations"], ["no_process_read"])

    def test_the_json_report_carries_the_sudo_violations_and_fails_ok(self):
        fp = FakeProc(self)
        fp.add(100)
        log = Path(tempfile.mkdtemp()) / "sudo.log"
        log.write_text(STRAY)
        out = log.parent / "r.json"
        r = self.cli(fp, "--pid", "100", "--mode", "record", "--sudo-since", "0", "--sudo-log", str(log), "--out", str(out))
        self.assertEqual(r.returncode, 0)
        doc = json.loads(out.read_text())
        self.assertFalse(doc["ok"])
        self.assertTrue(any(v.startswith("sudo ") for v in doc["violations"]), doc)
        self.assertEqual(len(doc["sudo"]["violations"]), 1)

    def test_a_clean_tree_with_a_stray_sudo_entry_is_not_ok_in_the_summary(self):
        fp = FakeProc(self)
        fp.add(100)
        log = Path(tempfile.mkdtemp()) / "sudo.log"
        log.write_text(STRAY)
        r = self.cli(fp, "--pid", "100", "--mode", "record", "--sudo-since", "0", "--sudo-log", str(log))
        self.assertIn("ok=0", r.stdout)
        self.assertIn("KFROOT_ASSERT_VIOLATION sudo /usr/bin/id", r.stdout)

    def test_an_unreadable_sudo_log_is_a_refusal_not_a_pass(self):
        fp = FakeProc(self)
        fp.add(100)
        r = self.cli(fp, "--pid", "100", "--mode", "require", "--sudo-since", "0", "--sudo-log", "/nonexistent/sudo.log")
        self.assertEqual(r.returncode, 2)
        self.assertIn("audit_log_missing", r.stderr)


class LibStateMachine(unittest.TestCase):
    """rootless_check and rootless_final of scripts/lib/kfroot.sh with a stub checker."""

    def stub(self, body):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        (d / "stub.py").write_text(body)
        return d

    OK = "print('KFROOT_ASSERT mode=m ok=1 uid=1002')\n"
    BAD = "import sys\nprint('KFROOT_ASSERT mode=m ok=0 uid=0')\nsys.exit(1 if 'require' in sys.argv else 0)\n"

    def run_sh(self, script, stub_body, **env):
        d = self.stub(stub_body)
        full = f"say() {{ echo \"$*\"; }}; EV={d}; . {shlex.quote(str(LIB))}; {script}"
        return subprocess.run(["bash", "-c", full], capture_output=True, text=True, timeout=60,
                              env={**os.environ, "KF_KFROOT_PY": str(d / "stub.py"), **env})

    def test_an_ok_assertion_sets_the_state_ok(self):
        r = self.run_sh('rootless_check a --pid 1; echo "rc=$? state=$ROOTLESS_STATE"', self.OK, KF_ROOTLESS="require")
        self.assertIn("rc=0 state=ok", r.stdout)

    def test_a_violation_sets_the_state_violated_and_fails_in_require_mode(self):
        r = self.run_sh('rootless_check a --pid 1; echo "rc=$? state=$ROOTLESS_STATE"', self.BAD, KF_ROOTLESS="require")
        self.assertIn("rc=1 state=violated", r.stdout)

    def test_a_violation_in_record_mode_is_recorded_and_does_not_fail(self):
        r = self.run_sh('rootless_check a --pid 1; echo "rc=$? state=$ROOTLESS_STATE"', self.BAD, KF_ROOTLESS="record")
        self.assertIn("rc=0 state=violated", r.stdout)

    def test_the_state_stays_violated_after_a_later_ok(self):
        r = self.run_sh('rootless_check a --pid 1; ROOTLESS_STATE=violated; KF_BODY=1; rootless_check b --pid 1; echo "state=$ROOTLESS_STATE"',
                        self.OK, KF_ROOTLESS="record")
        self.assertIn("state=violated", r.stdout)

    ALTERNATING = ("import os,sys\nf=os.environ['KF_TEST_COUNTER']\nn=int(open(f).read()) if os.path.exists(f) else 0\nopen(f,'w').write(str(n+1))\n"
                   "print('KFROOT_ASSERT ok=0 x=1' if n == 0 else 'KFROOT_ASSERT ok=1 x=1')\n")

    def test_the_state_stays_violated_when_a_later_real_check_is_ok(self):
        counter = Path(tempfile.mkdtemp()) / "n"
        r = self.run_sh('rootless_check a --pid 1; rootless_check b --pid 1; echo "state=$ROOTLESS_STATE"', self.ALTERNATING,
                        KF_ROOTLESS="record", KF_TEST_COUNTER=str(counter))
        self.assertIn("state=violated", r.stdout)

    def test_a_clean_final_does_not_clear_an_earlier_violation(self):
        counter = Path(tempfile.mkdtemp()) / "n"
        r = self.run_sh('rootless_check a --pid 1; rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.ALTERNATING,
                        KF_ROOTLESS="record", KF_SUDO_LOG_MARK="5", KF_TEST_COUNTER=str(counter))
        self.assertIn("state=violated", r.stdout)

    def test_an_unset_mark_is_the_same_as_an_empty_one_in_require_mode(self):
        r = self.run_sh('unset KF_SUDO_LOG_MARK; rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.OK, KF_ROOTLESS="require")
        self.assertIn("final=1 state=violated", r.stdout)

    def test_a_crashing_checker_at_the_end_of_a_require_run_fails_the_final(self):
        r = self.run_sh('rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', "import sys\nsys.exit(2)\n",
                        KF_ROOTLESS="require", KF_SUDO_LOG_MARK="5")
        self.assertIn("final=1 state=violated", r.stdout)

    def test_off_mode_leaves_the_state_off(self):
        r = self.run_sh('rootless_check a --pid 1; echo "rc=$? state=$ROOTLESS_STATE"', self.BAD, KF_ROOTLESS="off")
        self.assertIn("rc=0 state=off", r.stdout)

    def test_a_crash_in_record_mode_prints_an_error_line_and_returns_zero(self):
        r = self.run_sh('kf_rootless_assert a "$EV" --pid 1; echo "rc=$?"', "import sys\nsys.exit(2)\n", KF_ROOTLESS="record")
        self.assertIn("rc=0", r.stdout)
        self.assertIn("ok=0 label=a error=rc2", r.stdout)

    def test_a_crash_in_require_mode_returns_its_status(self):
        r = self.run_sh('kf_rootless_assert a "$EV" --pid 1; echo "rc=$?"', "import sys\nsys.exit(2)\n", KF_ROOTLESS="require")
        self.assertIn("rc=2", r.stdout)

    def test_an_unknown_mode_is_refused_with_one(self):
        r = self.run_sh('kf_rootless_assert a "$EV"; echo "rc=$?"', self.OK, KF_ROOTLESS="sometimes")
        self.assertIn("rc=1", r.stdout)
        self.assertIn("KF_ROOTLESS must be require, record or off", r.stdout)

    def test_final_without_a_mark_in_require_mode_is_a_violation(self):
        r = self.run_sh('rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.OK, KF_ROOTLESS="require", KF_SUDO_LOG_MARK="")
        self.assertIn("final=1 state=violated", r.stdout)

    def test_final_without_a_mark_in_record_mode_is_noted_not_failed(self):
        r = self.run_sh('rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.OK, KF_ROOTLESS="record", KF_SUDO_LOG_MARK="")
        self.assertIn("final=0 state=unchecked", r.stdout)
        self.assertIn("no-sudo-log-mark", r.stdout)

    def test_final_with_a_mark_and_a_violation_fails(self):
        r = self.run_sh('rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.BAD, KF_ROOTLESS="require", KF_SUDO_LOG_MARK="5")
        self.assertIn("final=1 state=violated", r.stdout)

    def test_final_with_a_mark_and_a_clean_window_passes(self):
        r = self.run_sh('rootless_final; echo "final=$ROOTLESS_FINAL_RC state=$ROOTLESS_STATE"', self.OK, KF_ROOTLESS="require", KF_SUDO_LOG_MARK="5")
        self.assertIn("final=0 state=ok", r.stdout)

    def test_the_mark_is_passed_to_the_checker_as_the_sudo_window_start(self):
        body = "import sys\nprint('ARGV', *sys.argv[1:])\nprint('KFROOT_ASSERT ok=1')\n"
        r = self.run_sh("rootless_final", body, KF_ROOTLESS="record", KF_SUDO_LOG_MARK="4242")
        self.assertIn("--sudo-since 4242", r.stdout)

    def test_a_kernel_log_read_works_inside_a_no_new_privs_tree_when_it_is_unrestricted(self):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        write_exe(d / "dmesg", "#!/bin/sh\necho DIRECT-DMESG\n")
        (d / "status").write_text("NoNewPrivs:\t1\n")
        (d / "restrict").write_text("0\n")
        r = subprocess.run(["bash", "-c", f". {shlex.quote(str(LIB))}; kf_dmesg"], capture_output=True, text=True,
                           env={**os.environ, "KFROOT_EUID": "1002", "KFROOT_SELF_STATUS": str(d / "status"),
                                "KFROOT_DMESG_RESTRICT": str(d / "restrict"), "PATH": f"{d}:{os.environ['PATH']}"})
        self.assertEqual(r.stdout, "DIRECT-DMESG\n")

    def test_a_restricted_kernel_log_inside_a_no_new_privs_tree_is_refused_not_escalated(self):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        write_exe(d / "kfroot", '#!/bin/sh\necho ESCALATED\n')
        write_exe(d / "dmesg", "#!/bin/sh\necho DIRECT-DMESG\n")
        (d / "status").write_text("NoNewPrivs:\t1\n")
        (d / "restrict").write_text("1\n")
        r = subprocess.run(["bash", "-c", f". {shlex.quote(str(LIB))}; kf_dmesg; echo rc=$?"], capture_output=True, text=True,
                           env={**os.environ, "KFROOT_EUID": "1002", "KFROOT_SELF_STATUS": str(d / "status"),
                                "KFROOT_DMESG_RESTRICT": str(d / "restrict"), "PATH": f"{d}:{os.environ['PATH']}"})
        self.assertIn("rc=126", r.stdout)
        self.assertNotIn("ESCALATED", r.stdout)

    def test_root_runs_a_command_directly_even_with_a_kfroot_program_on_the_path(self):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        write_exe(d / "kfroot", '#!/bin/sh\necho ESCALATED\n')
        r = subprocess.run(["bash", "-c", f". {shlex.quote(str(LIB))}; kfroot echo direct"], capture_output=True, text=True,
                           env={**os.environ, "KFROOT_EUID": "0", "PATH": f"{d}:{os.environ['PATH']}"})
        self.assertEqual(r.stdout, "direct\n")

    def test_a_root_caller_ignores_the_no_new_privs_flag_because_it_needs_no_sudo(self):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        (d / "status").write_text("NoNewPrivs:\t1\n")
        r = subprocess.run(["bash", "-c", f". {shlex.quote(str(LIB))}; kfroot echo direct; echo rc=$?"], capture_output=True, text=True,
                           env={**os.environ, "KFROOT_EUID": "0", "KFROOT_SELF_STATUS": str(d / "status")})
        self.assertEqual(r.stdout, "direct\nrc=0\n")


def mini_tree(tc, win_vm_body=None, extra_gate=False):
    """A private tree with copies of the gate and the library and a stub win_vm.sh: the gate computes SOURCE from its own path."""
    d = Path(tempfile.mkdtemp(prefix="kfroot-mini-"))
    tc.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
    (d / "scripts/bench/box").mkdir(parents=True)
    (d / "scripts/lib").mkdir(parents=True)
    (d / "scripts/ci").mkdir(parents=True)
    (d / "scripts/bench/windows").mkdir(parents=True)
    shutil_copy(GATE, d / "scripts/bench/box/windows_gate.sh")
    shutil_copy(LIB, d / "scripts/lib/kfroot.sh")
    shutil_copy(KFROOT_PY, d / "scripts/ci/kfroot.py")
    (d / "scripts/bench/windows/kf3_flags.sample.env").write_text("KF3_X=1\n")
    write_exe(d / "scripts/bench/windows/win_vm.sh", win_vm_body or "#!/bin/bash\necho \"WINVM-STUB $*\"\n")
    return d


def shutil_copy(src, dst):
    dst.write_text(Path(src).read_text())
    dst.chmod(0o755)


class GatePrepareAndPrereqs(unittest.TestCase):
    def stubs(self, apt_rc=0):
        d = Path(tempfile.mkdtemp(prefix="kfroot-stubs-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        self.log = d / "calls"
        write_exe(d / "apt-get", f'#!/bin/sh\necho "apt-get $*" >> {self.log}\nexit {apt_rc}\n')
        write_exe(d / "kfroot", f'#!/bin/sh\necho "kfroot $*" >> {self.log}\nexec "$@"\n')
        return d

    def prepare(self, apt_rc=0, **env):
        tree = mini_tree(self, "#!/bin/bash\necho \"WINVM-STUB $*\" >> \"$KF_TEST_LOG\"\nexit ${KF_TEST_WV_RC:-0}\n")
        stubs = self.stubs(apt_rc)
        prov = tree / "prov"
        prov.mkdir()
        p = subprocess.run(["bash", str(tree / "scripts/bench/box/windows_gate.sh"), "--prepare", str(prov)], capture_output=True, text=True,
                           timeout=120, env={**os.environ, "PATH": f"{stubs}:{os.environ['PATH']}", "KFROOT_EUID": "0", "KF_TEST_LOG": str(self.log),
                                             "KF_ROOTLESS": "off", **env})
        return p, (self.log.read_text() if self.log.exists() else "")

    def test_prepare_installs_the_packages_runs_the_apparmor_step_and_stops(self):
        p, calls = self.prepare()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("WINDOWS_GATE_PREPARED", p.stdout)
        self.assertRegex(calls, r"apt-get .*install -y -qq ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full")
        self.assertIn("WINVM-STUB prepare", calls)
        self.assertNotIn("GATE_STAGE preflight", p.stdout)  # it exits: no stage of the real run

    def test_a_failing_package_install_fails_the_prepare_and_never_runs_the_apparmor_step(self):
        p, calls = self.prepare(apt_rc=100)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("WINDOWS_GATE_PREPARE_FAILED packages", p.stdout)
        self.assertNotIn("WINDOWS_GATE_PREPARED", p.stdout)
        self.assertNotIn("WINVM-STUB", calls)
        self.assertNotIn("GATE_STAGE preflight", p.stdout)

    def test_a_failing_apparmor_step_fails_the_prepare(self):
        p, calls = self.prepare(KF_TEST_WV_RC="3")
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("WINDOWS_GATE_PREPARE_FAILED apparmor", p.stdout)
        self.assertNotIn("WINDOWS_GATE_PREPARED", p.stdout)

    def test_prepare_is_not_a_verdict_the_payload_could_grade_as_a_pass(self):
        p, _ = self.prepare()
        self.assertNotIn("WINDOWS_GATE_PASS", p.stdout)

    def extract(self, name):
        text = GATE.read_text()
        m = re.search(r"(?ms)^%s\(\) \{.*?^\}\n" % re.escape(name), text)
        self.assertIsNotNone(m, name)
        return m[0]

    def run_prereqs(self, euid, nnp, tmp_status):
        stubs = self.stubs()
        tmp_status.write_text(f"NoNewPrivs:\t{nnp}\n")
        script = ". %s; EV=%s\n%s\ninstall_prereqs; echo rc=$?" % (shlex.quote(str(LIB)), shlex.quote(str(stubs)), self.extract("install_prereqs"))
        p = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60,
                           env={**os.environ, "PATH": f"{stubs}:{os.environ['PATH']}", "KFROOT_EUID": str(euid),
                                "KFROOT_SELF_STATUS": str(tmp_status), "KF_APT_CACHE_ROOT": "", "KF_PAYLOAD_ROOT": ""})
        return p, (self.log.read_text() if self.log.exists() else "")

    def test_an_unprivileged_caller_installs_packages_through_the_kfroot_program(self):
        if Path("/usr/share/OVMF/OVMF_CODE_4M.secboot.fd").exists() and all(K.shutil.which(c) for c in ("swtpm", "sbsign", "xorriso")):
            self.skipTest("this machine has every prerequisite: nothing would be installed")
        st = Path(tempfile.mkdtemp()) / "status"
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(st.parent)]))
        p, calls = self.run_prereqs(1002, 0, st)
        self.assertIn("kfroot timeout 900 apt-get", calls)
        self.assertIn("apt-get", calls.replace("kfroot timeout 900 apt-get", ""))  # the stub kfroot exec'ed it

    def test_inside_a_no_new_privs_tree_a_missing_package_is_rc_126_and_nothing_is_installed(self):
        if Path("/usr/share/OVMF/OVMF_CODE_4M.secboot.fd").exists() and all(K.shutil.which(c) for c in ("swtpm", "sbsign", "xorriso")):
            self.skipTest("this machine has every prerequisite")
        stubs = self.stubs()
        # the library's own kfroot function (not the stub program) is what refuses: no kfroot program on PATH
        (stubs / "kfroot").unlink()
        st = stubs / "status"
        p, calls = self.run_prereqs(1002, 1, st)
        self.assertIn("rc=126", p.stdout)
        self.assertNotIn("apt-get", calls)

    def stage_block(self):
        text = GATE.read_text()
        m = re.search(r"(?ms)^stage_begin prereqs\n(.*?)^stage_end\n", text)
        self.assertIsNotNone(m)
        return m[1]

    def run_stage(self, rc):
        script = ('stage_begin() { :; }; stage_end() { :; }; EV=/nonexistent; finish() { echo FINISH; }; '
                  'verdict() { echo "VERDICT $*"; }; install_prereqs() { return %d; }\n%s' % (rc, self.stage_block()))
        return subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60).stdout

    def test_the_prereqs_stage_calls_a_no_new_privs_refusal_an_operator_retry_that_names_prepare(self):
        out = self.run_stage(126)
        self.assertIn("VERDICT RETRY OPERATOR", out)
        self.assertIn("--prepare", out)
        self.assertNotIn("PROVIDER", out)

    def test_the_prereqs_stage_calls_an_apt_failure_a_provider_retry(self):
        out = self.run_stage(100)
        self.assertIn("VERDICT RETRY PROVIDER", out)
        self.assertNotIn("--prepare", out)

    def test_the_prereqs_stage_is_silent_when_nothing_is_missing(self):
        self.assertNotIn("VERDICT", self.run_stage(0))


class WinVmPrepare(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp(prefix="kfroot-vm-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.d)]))
        self.aa = self.d / "apparmor.d"
        (self.aa / "local").mkdir(parents=True)
        (self.aa / "usr.bin.swtpm").write_text("# profile\n#include <local/usr.bin.swtpm>\n")
        (self.aa / "local/usr.bin.swtpm").write_text("")
        self.log = self.d / "calls"
        write_exe(self.d / "bin/kfroot", f'#!/bin/sh\necho "kfroot $*" >> {self.log}\nexec "$@"\n')
        write_exe(self.d / "bin/apparmor_parser", f'#!/bin/sh\necho "apparmor_parser $*" >> {self.log}\n')
        # a copy of the script and the library in a private tree: only scripts/lib and the script are needed for `prepare`
        (self.d / "tree/scripts/bench/windows").mkdir(parents=True)
        (self.d / "tree/scripts/lib").mkdir(parents=True)
        shutil_copy(WINVM, self.d / "tree/scripts/bench/windows/win_vm.sh")
        shutil_copy(LIB, self.d / "tree/scripts/lib/kfroot.sh")
        self.script = self.d / "tree/scripts/bench/windows/win_vm.sh"
        self.root = self.d / "winvm"

    def prepare(self, euid="1002"):
        return subprocess.run(["bash", str(self.script), "prepare"], capture_output=True, text=True, timeout=60,
                              env={**os.environ, "PATH": f"{self.d / 'bin'}:{os.environ['PATH']}", "KFROOT_EUID": euid,
                                   "KFROOT_SELF_STATUS": str(self.d / "nnp0"), "WINVM_ROOT": str(self.root),
                                   "WINVM_APPARMOR_DIR": str(self.aa)})

    def setup_nnp(self, value):
        (self.d / "nnp0").write_text(f"NoNewPrivs:\t{value}\n")

    def test_the_rule_is_written_and_the_profile_loaded_through_kfroot(self):
        self.setup_nnp(0)
        p = self.prepare()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        calls = self.log.read_text()
        self.assertIn(f"kfroot tee -a {self.aa}/local/usr.bin.swtpm", calls)
        self.assertIn(f"kfroot apparmor_parser -r {self.aa}/usr.bin.swtpm", calls)
        self.assertIn(f"{self.root}/** rwk,", (self.aa / "local/usr.bin.swtpm").read_text())

    def test_the_second_prepare_needs_no_root_at_all(self):
        self.setup_nnp(0)
        self.prepare()
        self.log.write_text("")
        p = self.prepare()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(self.log.read_text(), "")

    def test_prepare_creates_the_vm_root_private(self):
        self.setup_nnp(0)
        self.prepare()
        self.assertEqual(stat.S_IMODE(self.root.stat().st_mode), 0o700)

    def test_in_a_no_new_privs_tree_the_missing_rule_is_a_refusal_not_a_silent_skip(self):
        self.setup_nnp(1)
        p = self.prepare()
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("KFROOT_REFUSED", p.stderr)
        self.assertNotIn("apparmor_parser", self.log.read_text() if self.log.exists() else "")

    def test_as_root_the_steps_run_directly(self):
        self.setup_nnp(1)
        p = self.prepare(euid="0")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        calls = self.log.read_text()
        self.assertNotIn("kfroot ", calls)
        self.assertIn("apparmor_parser -r", calls)
        self.assertIn("** rwk,", (self.aa / "local/usr.bin.swtpm").read_text())


class AppsRuns(unittest.TestCase):
    def test_require_mode_with_a_root_tree_is_refused(self):
        d = Path(tempfile.mkdtemp(prefix="kfroot-apps-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        d.chmod(0o777)
        stub = d / "stub.py"
        stub.write_text("import sys\nprint('KFROOT_ASSERT mode=x ok=0 uid=0')\nsys.exit(1)\n")
        p = subprocess.run(["bash", str(APPS), "tagr", str(d)], capture_output=True, text=True, timeout=120,
                           env={**os.environ, "KF_ROOTLESS": "require", "KF_KFROOT_PY": str(stub), "WINVM_ROOT": str(d / "w"), "BENCH": str(d / "b")})
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("WINAPPS_REFUSED why=rootless-violated", p.stdout)
        self.assertNotIn("WINAPPS_PASS", p.stdout)

    def test_record_mode_goes_on(self):
        d = Path(tempfile.mkdtemp(prefix="kfroot-apps-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(d)]))
        stub = d / "stub.py"
        stub.write_text("import sys\nprint('KFROOT_ASSERT mode=x ok=0 uid=0')\nsys.exit(1)\n")
        p = subprocess.run(["bash", str(APPS), "tagr", str(d)], capture_output=True, text=True, timeout=120,
                           env={**os.environ, "KF_ROOTLESS": "record", "KF_KFROOT_PY": str(stub), "WINVM_ROOT": str(d / "w"), "BENCH": str(d / "b")})
        self.assertNotIn("why=rootless-violated", p.stdout)
        self.assertIn("KFROOT_ASSERT mode=x ok=0", p.stdout)


class HarnessStatic(unittest.TestCase):
    def test_the_gate_names_qemu_in_its_boot_1_assertion(self):
        self.assertRegex(code(GATE), r'rootless_check boot1 [^\n]*--pid-file "\$VM/run/qemu\.pid"')

    def test_the_gate_names_swtpm_in_its_boot_1_assertion(self):
        self.assertRegex(code(GATE), r'rootless_check boot1 [^\n]*--pid-file "\$VM/run/swtpm\.pid"')

    def test_the_boot_1_assertion_comes_after_the_guest_answered_and_fails_the_run_in_require_mode(self):
        gate = code(GATE)
        i = gate.index("rootless_check boot1")
        self.assertLess(gate.rindex("wait_ssh \"$(cap 900)\"", 0, i), i)
        self.assertIn("|| fail_op", gate[i:i + 400])

    def test_the_gate_checks_kvm_for_read_and_write(self):
        self.assertIn("[ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ] || refuse no-/dev/kvm-access", code(GATE))

    def test_the_apps_stage_checks_kvm_for_read_and_write(self):
        self.assertIn("[ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ] || refuse no-/dev/kvm-access", code(APPS))

    def test_an_unreadable_kernel_log_is_a_refusal_in_the_gate(self):
        self.assertIn("kf_dmesg > /dev/null 2>&1 || refuse dmesg-unreadable", code(GATE))

    def test_an_unreadable_kernel_log_is_a_refusal_in_the_apps_stage(self):
        self.assertIn("kf_dmesg > /dev/null 2>&1 || refuse dmesg-unreadable", code(APPS))

    def test_the_gate_refuses_an_unwritable_directory_by_name(self):
        self.assertIn('[ -d "$t" ] && [ -w "$t" ] || refuse "not-writable-$d"', code(GATE))

    def test_the_apps_stage_refuses_an_unwritable_directory_by_name(self):
        self.assertIn('[ -d "$t" ] && [ -w "$t" ] || refuse "not-writable-$d"', code(APPS))

    def raw_dmesg(self, path):
        return [ln for ln in code(path).splitlines() if re.search(r"(^|[\s$(|;])dmesg\b", ln) and "_host_dmesg" not in ln and "kf_dmesg" not in ln]

    def test_the_gate_has_no_raw_dmesg(self):
        self.assertEqual(self.raw_dmesg(GATE), [])
        self.assertIn("kf_dmesg", code(GATE))

    def test_the_apps_stage_has_no_raw_dmesg(self):
        self.assertEqual(self.raw_dmesg(APPS), [])
        self.assertIn("kf_dmesg", code(APPS))

    def test_win_vm_has_no_raw_dmesg(self):
        self.assertEqual(self.raw_dmesg(WINVM), [])
        self.assertGreaterEqual(code(WINVM).count("kf_dmesg"), 2)

    def test_the_apps_verdict_records_the_rootless_state(self):
        self.assertIn('why+="; rootless=$ROOTLESS_STATE"', code(APPS))

    def test_the_apps_verdict_turns_a_rootless_violation_into_an_operator_retry(self):
        self.assertRegex(code(APPS), r'\[ "\$KIND" = PASS \] && \[ "\$\{ROOTLESS_FINAL_RC:-0\}" != 0 \]; then KIND=RETRY CLASS=OPERATOR')

    def test_the_apps_stage_takes_the_sudo_window_after_the_guests_are_gone(self):
        apps = code(APPS)
        self.assertLess(apps.index("teardown_all\nGUEST_STARTED=0"), apps.index("\nrootless_final\n"))

    def test_the_apps_stage_audits_sudo_before_it_grades(self):
        apps = code(APPS)
        self.assertEqual(apps.count("\nrootless_final\n"), 1)
        self.assertLess(apps.index("\nrootless_final\n"), apps.index("\nstage hostref_after\n"))
        self.assertLess(apps.index("\nrootless_final\n"), apps.index("\nstage grade\n"))

    def test_the_apps_stage_asserts_before_its_other_checks(self):
        apps = code(APPS)
        self.assertLess(apps.index("rootless_check preflight"), apps.index("no-/dev/kvm-access"))
        self.assertIn("|| refuse rootless-violated", apps)

    def test_the_gate_takes_the_sudo_window_after_the_teardown(self):
        gate = code(GATE)
        self.assertRegex(gate, r"    teardown_guest\n    rootless_final\n")


if __name__ == "__main__":
    unittest.main()
