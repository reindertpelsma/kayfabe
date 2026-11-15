#!/usr/bin/env python3
"""GPU-free, VM-free falsifiers for the DEFAULTS of `win_vm.sh run` (STATUS: LIVE, 2026-11-15).

Owner, 2026-11-15: "To start windows you do not need an env file. Then the defaults are wrong." Measured the same day on
the pc: `run` with kf3 and no env file, no --guest-driver and no gpu-uuid ended in Windows Code 43; the same guest launched
the way the CI gate launches it (scripts/bench/box/windows_gate.sh) reached ConfigManagerErrorCode 0.

Everything here drives `win_vm.sh run --dry-run` (it prints DRYRUN_QEMU and DRYRUN_ENV and starts nothing). What this does
NOT prove: that a default `run` boots a guest to nvidia-smi and code 0. That needs a box (see scripts/bench/windows/README.md).
"""

import os
import re
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / "scripts/bench/windows"
SCRIPT = WIN / "win_vm.sh"
DEFAULTS = WIN / "kf3_windows_defaults.env"
SAMPLE = WIN / "kf3_flags.sample.env"
GATE = ROOT / "scripts/bench/box/windows_gate.sh"
UUID = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"
OTHER_UUID = "11111111-2222-4333-8444-555555555555"


def flags_of(path):
    return [l for l in path.read_text().splitlines() if l and not l.startswith("#")]


@unittest.skipUnless(SCRIPT.exists() and shutil.which("bash"), "needs bash and the repository's scripts")
class WinVmDefaults(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winvm-defaults-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.vmdir = self.tmp / "root" / "kfwin"
        (self.vmdir / "secrets").mkdir(parents=True)
        (self.vmdir / "logs").mkdir()
        (self.vmdir / "installed").write_text("t\n")

    def identity(self, uuid=UUID, text=None):
        self.vmdir.joinpath("identity.env").write_text(
            text if text is not None else f"# test\nVM_UUID={uuid}\nVM_SERIAL=KF0123456789\nVM_MAC=52:54:00:aa:bb:cc\n")

    def run_sh(self, *args, dry=True, env=None):
        e = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(self.tmp), "LANG": "C",
             "WINVM_ROOT": str(self.tmp / "root"), "KF_GPU_LOCK": str(self.tmp / "gpu.lock"),
             "BENCH": str(self.tmp / "bench"), "WINVM_QEMU": "/fake/qemu-system-x86_64"}
        e.update(env or {})
        a = ["run", *(["--dry-run"] if dry else []), "--tag", "t", *args]
        return subprocess.run(["bash", str(SCRIPT), *a], env=e, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=60, check=False)

    def dry(self, *args, env=None):
        r = self.run_sh(*args, env=env)
        out = r.stdout.splitlines()
        q = [l for l in out if l.startswith("DRYRUN_QEMU ")]
        argv = shlex.split(q[0][len("DRYRUN_QEMU "):]) if q else []
        kenv = dict(l[len("DRYRUN_ENV "):].split("=", 1) for l in out if l.startswith("DRYRUN_ENV "))
        return r, argv, kenv

    @staticmethod
    def kf3_dev(argv):
        d = [a for a in argv if a.startswith("kf3-gpu,")]
        assert len(d) == 1, argv
        return d[0]

    # -- the defaults file -----------------------------------------------------------------------------
    def test_defaults_file_is_valid_and_has_no_diagnostics(self):
        flags = flags_of(DEFAULTS)
        self.assertEqual(len(flags), len(set(flags)))
        for l in flags:
            self.assertRegex(l, r"^KF3_[A-Z0-9_]+=[0-9]+$")
        for l in flags:
            self.assertNotRegex(l, r"TRACE|CENSUS|MAPLOG|SNAPSHOT", f"{l}: a diagnostic must stay opt-in")
        self.assertIn("STATUS: LIVE, 2026-11-15", DEFAULTS.read_text())

    def test_sample_file_keeps_the_same_flag_list(self):
        # the gates and the propimage bundle still pass the old name; the two lists must not drift
        self.assertEqual(flags_of(SAMPLE), flags_of(DEFAULTS))

    # -- (1) a default run carries the Windows set ---------------------------------------------------------
    def test_default_run_carries_flags_guest_driver_and_per_vm_gpu_uuid(self):
        self.identity()
        r, argv, env = self.dry()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        want = dict(l.split("=", 1) for l in flags_of(DEFAULTS))
        want["KF3_RPC_TRACE"] = "1"
        self.assertEqual(env, want)
        dev = self.kf3_dev(argv)
        self.assertTrue(dev.endswith(f",id=kf0,guest-driver=580.65.06,gpu-uuid=GPU-{UUID}"), dev)

    def test_guest_driver_env_override_and_no_default_when_empty(self):
        self.identity()
        _, argv, _ = self.dry(env={"WINVM_GUEST_DRIVER": "580.95.05"})
        self.assertIn(",guest-driver=580.95.05,", self.kf3_dev(argv) + ",")
        _, argv, _ = self.dry(env={"WINVM_GUEST_DRIVER": ""})
        self.assertNotIn("guest-driver", self.kf3_dev(argv))

    # -- explicit arguments win ----------------------------------------------------------------------------
    def test_explicit_env_file_replaces_the_default_flags(self):
        self.identity()
        f = self.tmp / "mine.env"
        f.write_text("KF3_ASYNC_PREEMPT=1\n")
        r, argv, env = self.dry("--env-file", str(f))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(env, {"KF3_ASYNC_PREEMPT": "1", "KF3_RPC_TRACE": "1"})
        # guest-driver and gpu-uuid defaults still apply: only the file was named
        self.assertIn("guest-driver=580.65.06", self.kf3_dev(argv))

    def test_explicit_guest_driver_wins(self):
        self.identity()
        _, argv, _ = self.dry("--guest-driver", "575.57.08")
        dev = self.kf3_dev(argv)
        self.assertIn(",guest-driver=575.57.08", dev)
        self.assertNotIn("580.65.06", dev)

    def test_explicit_gpu_uuid_wins_and_is_not_duplicated(self):
        self.identity()
        for extra in (f"gpu-uuid=GPU-{OTHER_UUID}", f"x-foo=1,gpu-uuid=GPU-{OTHER_UUID}"):
            with self.subTest(extra=extra):
                _, argv, _ = self.dry("--kf3-extra", extra)
                dev = self.kf3_dev(argv)
                self.assertEqual(dev.count("gpu-uuid="), 1, dev)
                self.assertIn(f"gpu-uuid=GPU-{OTHER_UUID}", dev)
                self.assertNotIn(UUID, dev)

    def test_other_kf3_extra_keeps_the_default_gpu_uuid(self):
        self.identity()
        _, argv, _ = self.dry("--kf3-extra", "x-foo=1")
        dev = self.kf3_dev(argv)
        self.assertTrue(dev.endswith(f",x-foo=1,gpu-uuid=GPU-{UUID}"), dev)

    def test_a_flag_already_in_the_environment_wins_over_the_defaults_file(self):
        self.identity()
        _, _, env = self.dry(env={"KF3_ASYNC_PREEMPT": "0"})
        self.assertEqual(env["KF3_ASYNC_PREEMPT"], "0")
        self.assertEqual(env["KF3_DEFERRED_API"], "1")

    # -- opt-outs --------------------------------------------------------------------------------------------
    def test_no_kf3_gets_none_of_the_defaults_and_needs_no_identity(self):
        r, argv, env = self.dry("--no-kf3")  # no identity.env on purpose
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(env, {})
        self.assertFalse([a for a in argv if a.startswith("kf3-gpu")])
        self.assertNotIn("guest-driver", " ".join(argv))
        self.assertNotIn("gpu-uuid", " ".join(argv))

    def test_no_default_flags_takes_none_of_them(self):
        r, argv, env = self.dry("--no-default-flags")  # no identity.env on purpose
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(env, {"KF3_RPC_TRACE": "1"})
        dev = self.kf3_dev(argv)
        self.assertTrue(dev.endswith(",id=kf0"), dev)

    # -- the identity is required, not guessed -----------------------------------------------------------------
    def test_default_run_without_identity_is_refused_with_a_message(self):
        r, argv, _ = self.dry()
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual(argv, [])
        self.assertIn("VM_UUID", r.stderr)
        self.assertIn("--no-default-flags", r.stderr)
        self.assertFalse((self.vmdir / "identity.env").exists(), "a run must not invent an identity")

    def test_default_run_with_a_bad_identity_is_refused(self):
        for text in ("VM_UUID=not-a-uuid\nVM_SERIAL=KF0123456789\nVM_MAC=52:54:00:aa:bb:cc\n",
                     "VM_SERIAL=KF0123456789\nVM_MAC=52:54:00:aa:bb:cc\n"):
            with self.subTest(text=text):
                self.identity(text=text)
                r, argv, _ = self.dry()
                self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                self.assertEqual(argv, [])

    def test_a_refused_detached_run_refuses_in_the_parent_before_forking(self):
        r = self.run_sh("--detach", dry=False)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("VM_UUID", r.stderr)
        self.assertEqual(list((self.vmdir / "logs").iterdir()), [], "nothing may have been forked")

    # -- (3) the gate's own arguments give the same run -----------------------------------------------------------
    def test_gate_arguments_and_defaults_give_the_identical_run(self):
        self.identity()
        common = ["--overlay", "gate1", "--arm", "B", "--tag", "gate1"]
        gate = ["--guest-driver", "580.65.06", "--kf3-extra", f"gpu-uuid=GPU-{UUID}", "--env-file", str(SAMPLE)]
        r1 = self.run_sh("--detach", *common, *gate)
        r2 = self.run_sh("--detach", *common)
        self.assertEqual(r1.returncode, 0, r1.stdout + r1.stderr)
        self.assertEqual(r2.returncode, 0, r2.stdout + r2.stderr)

        def pick(r):
            return [l for l in r.stdout.splitlines() if l.startswith(("DRYRUN_QEMU ", "DRYRUN_ENV "))]

        self.assertTrue(any(l.startswith("DRYRUN_ENV KF3_DEFERRED_API=1") for l in pick(r1)))
        self.assertEqual(pick(r1), pick(r2))

    def test_the_gate_still_passes_its_explicit_arguments(self):
        # the explicit form is what the box gate runs today; this fails if someone edits it away without a new test
        text = GATE.read_text()
        self.assertRegex(text, r'run --detach --overlay "\$TAG" --arm "\$ARM" --guest-driver "\$GDRV" --kf3-extra "gpu-uuid=GPU-\$vmuuid"')
        self.assertIn('--env-file "$FLAGS"', text)

    # -- (2) WINVM_USB_TABLET -------------------------------------------------------------------------------------
    def test_usb_tablet_is_off_by_default_and_opt_in_adds_only_the_xhci_pair(self):
        self.identity()
        _, off, _ = self.dry()
        _, on, _ = self.dry(env={"WINVM_USB_TABLET": "1"})
        self.assertNotIn("qemu-xhci,id=xhci", off)
        self.assertNotIn("usb-tablet,bus=xhci.0", off)
        i = on.index("qemu-xhci,id=xhci")
        self.assertEqual(on[i - 1:i + 3], ["-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0"])
        self.assertEqual(on[:i - 1] + on[i + 3:], off)
        # never virtio-input: those swallow all input in a guest without the vioinput driver (measured 2026-11-15)
        self.assertFalse([a for a in on if re.search(r"virtio-(input|keyboard|mouse|tablet)", a)])
        _, zero, _ = self.dry(env={"WINVM_USB_TABLET": "0"})
        self.assertEqual(zero, off)

    # -- docs ---------------------------------------------------------------------------------------------------
    def test_help_and_readme_name_the_new_options(self):
        r = subprocess.run(["bash", str(SCRIPT), "help"], capture_output=True, text=True, check=False)
        for w in ("--no-default-flags", "WINVM_USB_TABLET", "WINVM_GUEST_DRIVER", "kf3_windows_defaults.env"):
            self.assertIn(w, r.stdout)
        readme = (WIN / "README.md").read_text()
        for w in ("--no-default-flags", "WINVM_USB_TABLET", "kf3_windows_defaults.env"):
            self.assertIn(w, readme)


if __name__ == "__main__":
    unittest.main()
