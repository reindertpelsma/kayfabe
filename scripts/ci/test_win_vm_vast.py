#!/usr/bin/env python3
"""GPU-free, VM-free falsifiers for the Vast-lane options of scripts/bench/windows/win_vm.sh.

Nothing here starts QEMU, takes the GPU lock or touches a guest. The script is driven through its
WINVM_DRY_RUN mode (it prints the QEMU command line it would start) and through fake `ssh`/`scp`
programs on PATH that record their arguments and stdin. What this does NOT prove: that any of it
works on a real box (flock waits, a real ssh to a real Windows guest, QEMU accepting the arguments).
"""

import importlib.util
import os
import re
import shlex
import shutil
import stat
import subprocess
import tempfile
import unittest
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / "scripts/bench/windows"
SCRIPT = WIN / "win_vm.sh"
OLD_MAC = "52:54:00:4b:46:57"
SAMPLE = WIN / "kf3_flags.sample.env"

FAKE_SSH = """#!/bin/bash
# records argv (one per line) and stdin; answers the desktop-prep script with canned read-back lines
f="$FAKE_LOG/$(basename "$0").$$"
printf '%s\\n' "$@" > "$f.argv"
cat > "$f.stdin"
case "$*" in
  *desktop_prep.ps1*)
    echo "DESKTOP_PREP OverlayTestMode=5 want=5 OK"
    echo "DESKTOP_PREP_DONE fail=${FAKE_FAIL:-0}" ;;
esac
exit 0
"""


@unittest.skipUnless(SCRIPT.exists() and shutil.which("bash"), "needs bash and the repository's scripts")
class WinVmVast(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winvm-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.bin = self.tmp / "bin"
        self.log = self.tmp / "fakelog"
        self.bin.mkdir()
        self.log.mkdir()
        for name in ("ssh", "scp"):
            p = self.bin / name
            p.write_text(FAKE_SSH)
            p.chmod(p.stat().st_mode | stat.S_IXUSR)

    # -- helpers -------------------------------------------------------------------------------
    def env(self, **extra):
        e = {
            "PATH": f"{self.bin}:{os.environ.get('PATH', '/usr/bin:/bin')}",
            "HOME": str(self.tmp),
            "LANG": "C",
            "WINVM_ROOT": str(self.tmp / "root"),
            "KF_GPU_LOCK": str(self.tmp / "gpu.lock"),
            "BENCH": str(self.tmp / "bench"),
            "WINVM_QEMU": "/fake/qemu-system-x86_64",
            "FAKE_LOG": str(self.log),
        }
        e.update(extra)
        return e

    def vm(self, name="kfwin"):
        d = self.tmp / "root" / name
        (d / "secrets").mkdir(parents=True, exist_ok=True)
        (d / "logs").mkdir(exist_ok=True)
        (d / "installed").write_text("test\n")
        return d

    def sh(self, *args, **extra):
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            env=self.env(**extra),
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )

    def dry(self, sub="run", *args, name="kfwin", **extra):
        if sub == "run":
            self.vm(name)
            if "--tag" not in args:
                args = (*args, "--tag", "t")  # the default tag is a timestamp
        extra.setdefault("WINVM_NAME", name)
        r = self.sh(sub, "--dry-run", *args, **extra)
        out = r.stdout
        qline = [l for l in out.splitlines() if l.startswith("DRYRUN_QEMU ")]
        argv = shlex.split(qline[0][len("DRYRUN_QEMU "):]) if qline else []
        env = dict(l[len("DRYRUN_ENV "):].split("=", 1) for l in out.splitlines() if l.startswith("DRYRUN_ENV "))
        return r, argv, env

    def mac_of(self, argv):
        nic = [a for a in argv if a.startswith("virtio-net-pci,")]
        self.assertEqual(len(nic), 1)
        return re.search(r"mac=([0-9a-f:]+)", nic[0]).group(1)

    def line(self, r, prefix):
        got = [l for l in r.stdout.splitlines() if l.startswith(prefix)]
        self.assertEqual(len(got), 1, r.stdout + r.stderr)
        return got[0]

    # -- static --------------------------------------------------------------------------------
    def test_scripts_parse(self):
        r = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True, check=False)
        self.assertEqual(r.returncode, 0, r.stderr)

    @unittest.skipUnless(shutil.which("shellcheck"), "shellcheck not installed")
    def test_shellcheck_warnings(self):
        r = subprocess.run(["shellcheck", "-x", "-S", "warning", str(SCRIPT)], capture_output=True, text=True, check=False)
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_help_lists_the_new_options(self):
        r = self.sh("help")
        self.assertEqual(r.returncode, 0, r.stderr)
        for word in ("desktop-prep", "--env-file", "WINVM_LOCK_WAIT_S", "WINVM_INSTALL_WAIT_S",
                     "WINVM_SSH_CONNECT_TIMEOUT", "WINVM_VM_IDENTITY", "WINVM_DRY_RUN"):
            self.assertIn(word, r.stdout)

    def test_password_never_reaches_output_in_desktop_prep_script(self):
        text = (WIN / "desktop_prep.ps1").read_text()
        for n, l in enumerate(text.splitlines(), 1):
            if l.lstrip().startswith("#"):
                continue
            if "$pw" in l:
                self.assertNotRegex(l, r"(Say|Write-Output|Write-Host|Out-File)\b[^;}]*\$pw\b", f"line {n} could print the password")

    # -- old defaults are unchanged ---------------------------------------------------------------
    def test_old_defaults_run(self):
        r, argv, env = self.dry()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.mac_of(argv), OLD_MAC)
        self.assertNotIn("-uuid", argv)
        self.assertNotIn("-smbios", argv)
        self.assertEqual(env, {"KF3_RPC_TRACE": "1"})  # the one default flag the old `run` already set
        self.assertIn("wait_s=7200", self.line(r, "DRYRUN_GPU_LOCK"))
        self.assertEqual(self.line(r, "DRYRUN_SSH_CONNECT_TIMEOUT"), "DRYRUN_SSH_CONNECT_TIMEOUT 8")
        self.assertFalse((self.tmp / "root/kfwin/identity.env").exists())
        self.assertFalse((self.tmp / "gpu.lock").exists(), "dry-run must not take or create the GPU lock")
        self.assertFalse((self.tmp / "root/kfwin/disk").exists(), "dry-run must not create an overlay")

    def test_old_defaults_install(self):
        r, argv, _ = self.dry("install")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.line(r, "DRYRUN_INSTALL_WAIT_S"), "DRYRUN_INSTALL_WAIT_S 14400")
        self.assertEqual(self.mac_of(argv), OLD_MAC)
        self.assertNotIn("-uuid", argv)

    def test_kf3_device_line_is_the_documented_one(self):
        r, argv, _ = self.dry("run", "--guest-driver", "580.65.06")
        dev = [a for a in argv if a.startswith("kf3-gpu,")][0]
        self.assertTrue(dev.startswith("kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,gop-efi="))
        self.assertTrue(dev.endswith(",id=kf0,guest-driver=580.65.06"))

    # -- (1) (2) waits ------------------------------------------------------------------------------
    def test_lock_and_install_wait_are_configurable(self):
        r, _, _ = self.dry("run", WINVM_LOCK_WAIT_S="120")
        self.assertIn("wait_s=120", self.line(r, "DRYRUN_GPU_LOCK"))
        r, _, _ = self.dry("install", WINVM_INSTALL_WAIT_S="4200")
        self.assertEqual(self.line(r, "DRYRUN_INSTALL_WAIT_S"), "DRYRUN_INSTALL_WAIT_S 4200")

    def test_bad_numbers_are_refused(self):
        for var in ("WINVM_LOCK_WAIT_S", "WINVM_INSTALL_WAIT_S", "WINVM_SSH_CONNECT_TIMEOUT"):
            for bad in ("0", "abc", "-5", "12;id", "1 2"):
                with self.subTest(var=var, bad=bad):
                    r = self.sh("status", **{var: bad})
                    self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                    self.assertIn("REFUSED", r.stderr)
                    self.assertIn(var, r.stderr)

    # -- (4) env file ---------------------------------------------------------------------------------
    def test_sample_env_file_is_the_19_flag_list_and_marked_unsure(self):
        text = SAMPLE.read_text()
        flags = [l for l in text.splitlines() if l and not l.startswith("#")]
        self.assertEqual(len(flags), 19)
        for l in flags:
            self.assertRegex(l, r"^KF3_[A-Z0-9_]+=[0-9]+$")
        self.assertEqual(len(set(flags)), 19)
        self.assertIn("UNSURE", text)
        self.assertIn("confirm", text)

    def test_env_file_exports_exactly_its_flags(self):
        r, _, env = self.dry("run", "--env-file", str(SAMPLE))
        self.assertEqual(r.returncode, 0, r.stderr)
        want = dict(l.split("=", 1) for l in SAMPLE.read_text().splitlines() if l and not l.startswith("#"))
        want["KF3_RPC_TRACE"] = "1"
        self.assertEqual(env, want)

    def test_env_file_can_turn_rpc_trace_off_and_allows_comments(self):
        f = self.tmp / "f.env"
        f.write_text("# a comment\n\n   # indented comment\nKF3_RPC_TRACE=0\nKF3_ASYNC_PREEMPT=1\r\n")
        r, _, env = self.dry("run", "--env-file", str(f))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(env, {"KF3_RPC_TRACE": "0", "KF3_ASYNC_PREEMPT": "1"})

    def test_env_file_refuses_everything_else_without_echoing_it(self):
        bad_lines = [
            "KF3_X=1;echo SENTINEL_LEAK",
            "KF3_X=$(echo SENTINEL_LEAK)",
            "FOO=1",
            "kf3_x=1",
            "export KF3_X=1",
            "KF3_X=a",
            "KF3_X=",
            "KF3_X = 1",
            "KF3_X=1 # SENTINEL_LEAK",
            "LD_PRELOAD=SENTINEL_LEAK",
            "KF3_X=-1",
        ]
        for bad in bad_lines:
            with self.subTest(bad=bad):
                f = self.tmp / "bad.env"
                f.write_text("KF3_OK=1\n" + bad + "\n")
                r, argv, _ = self.dry("run", "--env-file", str(f))
                self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                self.assertIn("line 2 refused", r.stderr)
                self.assertNotIn("SENTINEL_LEAK", r.stdout + r.stderr)
                self.assertEqual(argv, [], "nothing may be started or printed for a refused file")

    def test_env_file_empty_or_missing_is_refused(self):
        f = self.tmp / "empty.env"
        f.write_text("# only a comment\n")
        self.assertEqual(self.dry("run", "--env-file", str(f))[0].returncode, 2)
        self.assertEqual(self.dry("run", "--env-file", str(self.tmp / "nope.env"))[0].returncode, 2)

    # -- (6) identity ---------------------------------------------------------------------------------
    def test_identity_is_added_persisted_and_per_vm(self):
        r, a1, _ = self.dry("run", WINVM_VM_IDENTITY="1")
        _, old, _ = self.dry("run", name="plain")
        self.assertEqual(r.returncode, 0, r.stderr)
        i = a1.index("-uuid")
        u = uuid.UUID(a1[i + 1])
        self.assertEqual(u.version, 4)
        self.assertRegex(a1[a1.index("-smbios") + 1], r"^type=1,serial=KF[0-9A-F]{10}$")
        mac = self.mac_of(a1)
        self.assertRegex(mac, r"^52:54:00(:[0-9a-f]{2}){3}$")
        self.assertNotEqual(mac, OLD_MAC)
        idf = self.tmp / "root/kfwin/identity.env"
        self.assertEqual(stat.S_IMODE(idf.stat().st_mode), 0o600)
        # the same VM keeps it across boots, even when the env var is no longer set
        _, a2, _ = self.dry("run")
        self.assertEqual(a1, a2)
        # the command line differs from the old one only by the identity
        stripped = list(a1)
        j = stripped.index("-uuid")
        del stripped[j:j + 2]
        j = stripped.index("-smbios")
        del stripped[j:j + 2]
        stripped = [s.replace(mac, OLD_MAC).replace("kfwin", "plain") for s in stripped]
        self.assertEqual(stripped, old)
        # another VM gets another identity
        _, b, _ = self.dry("run", name="other", WINVM_VM_IDENTITY="1")
        self.assertNotEqual(b[b.index("-uuid") + 1], a1[a1.index("-uuid") + 1])
        self.assertNotEqual(self.mac_of(b), mac)
        self.assertNotEqual(b[b.index("-smbios") + 1], a1[a1.index("-smbios") + 1])

    def test_identity_applies_to_install_too_and_matches_run(self):
        _, inst, _ = self.dry("install", WINVM_VM_IDENTITY="1")
        _, run, _ = self.dry("run")
        self.assertEqual(inst[inst.index("-uuid") + 1], run[run.index("-uuid") + 1])
        self.assertEqual(self.mac_of(inst), self.mac_of(run))

    def test_damaged_identity_file_is_refused(self):
        self.dry("run", WINVM_VM_IDENTITY="1")
        idf = self.tmp / "root/kfwin/identity.env"
        for body in ("VM_UUID=not-a-uuid\nVM_SERIAL=KF0123456789\nVM_MAC=52:54:00:01:02:03\n",
                     "VM_UUID=3b1c7a0e-9d7c-4a53-8e55-0a6a7e4f2d11\n",
                     "VM_UUID=3b1c7a0e-9d7c-4a53-8e55-0a6a7e4f2d11\nVM_SERIAL=KF0123456789\nVM_MAC=52:54:00:01:02:03\nEVIL=1\n"):
            with self.subTest(body=body):
                idf.write_text(body)
                r, argv, _ = self.dry("run")
                self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                self.assertEqual(argv, [])

    # -- (5) ssh timeout ----------------------------------------------------------------------------
    def ssh_options(self):
        files = sorted(self.log.glob("ssh.*.argv"))
        self.assertTrue(files, "the fake ssh was not called")
        return files[-1].read_text().splitlines()

    def test_ssh_connect_timeout_default_and_override(self):
        self.vm()
        r = self.sh("ssh", "hostname")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ConnectTimeout=8", self.ssh_options())
        for f in self.log.iterdir():
            f.unlink()
        r = self.sh("ssh", "hostname", WINVM_SSH_CONNECT_TIMEOUT="100")
        self.assertEqual(r.returncode, 0, r.stderr)
        opts = self.ssh_options()
        self.assertIn("ConnectTimeout=100", opts)
        self.assertNotIn("ConnectTimeout=8", opts)

    # -- (3) desktop prep ----------------------------------------------------------------------------
    PW = "pa+ss/w0rd=SENTINEL_PW"

    def prep(self, **extra):
        d = self.vm()
        (d / "secrets/win_password").write_text(self.PW)
        return self.sh("desktop-prep", **extra)

    def prep_call(self):
        calls = [f for f in self.log.glob("ssh.*.argv") if "desktop_prep.ps1" in f.read_text()]
        self.assertEqual(len(calls), 1)
        return calls[0].read_text().splitlines(), calls[0].with_suffix(".stdin").read_text()

    def test_desktop_prep_sends_the_password_on_stdin_only(self):
        r = self.prep()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        argv, stdin = self.prep_call()
        self.assertEqual(stdin, self.PW)
        for f in self.log.glob("*.argv"):
            self.assertNotIn("SENTINEL_PW", f.read_text())
        self.assertNotIn("SENTINEL_PW", r.stdout + r.stderr)
        cmd = argv[-1]
        self.assertIn("-User kf", cmd)
        self.assertIn("-PasswordFromStdin", cmd)
        self.assertIn("DESKTOP_PREP_OK user=kf", r.stdout)

    def test_desktop_prep_user_is_a_parameter(self):
        r = self.prep(KF_GUEST_USER="bob")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("-User bob", self.prep_call()[0][-1])

    def test_desktop_prep_refuses_a_bad_user_and_a_failed_readback(self):
        r = self.prep(KF_GUEST_USER="a b; x")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertFalse(list(self.log.glob("ssh.*.argv")), "no ssh call for a refused user")
        r = self.prep(FAKE_FAIL="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("DESKTOP_PREP_FAILED", r.stdout + r.stderr)


class SigninUser(unittest.TestCase):
    def load(self):
        spec = importlib.util.spec_from_file_location("signin_under_test", WIN / "signin.py")
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    def test_default_is_the_old_account_and_override_is_validated(self):
        m = self.load()
        old = os.environ.pop("KF_GUEST_USER", None)
        try:
            self.assertEqual(m.guest_user(), "vast")
            os.environ["KF_GUEST_USER"] = "kf"
            self.assertEqual(m.guest_user(), "kf")
            for bad in ("", "a b", "x\\y", "k;f", "a" * 21):
                os.environ["KF_GUEST_USER"] = bad
                with self.assertRaises(ValueError):
                    m.guest_user()
        finally:
            os.environ.pop("KF_GUEST_USER", None)
            if old is not None:
                os.environ["KF_GUEST_USER"] = old


if __name__ == "__main__":
    unittest.main()
