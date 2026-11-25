#!/usr/bin/env python3
"""GPU-free, VM-free tests for WINVM_NET in scripts/bench/windows/win_vm.sh (dry-run mode only).

Proves: the default and `offline` keep slirp restrict=on; `online` gives restrict=off; any other value is
refused; `online` is refused for the proprietary image (each way the script recognises it) and the refusal
starts no QEMU. Not proven: that a real guest has internet when online.
"""
import os
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/bench/windows/win_vm.sh"


@unittest.skipUnless(SCRIPT.exists() and shutil.which("bash"), "needs bash and the repository's scripts")
class WinVmNet(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winvm-net-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.vmdir = self.tmp / "root" / "kfwin"
        (self.vmdir / "secrets").mkdir(parents=True)
        (self.vmdir / "logs").mkdir()
        (self.vmdir / "installed").write_text("t\n")

    def dry(self, **extra):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(self.tmp), "LANG": "C",
               "WINVM_ROOT": str(self.tmp / "root"), "KF_GPU_LOCK": str(self.tmp / "gpu.lock"),
               "BENCH": str(self.tmp / "bench"), "WINVM_QEMU": "/fake/qemu", **extra}
        r = subprocess.run(["bash", str(SCRIPT), "run", "--dry-run", "--tag", "t"], env=env, stdin=subprocess.DEVNULL,
                           capture_output=True, text=True, timeout=60, check=False)
        q = [l for l in r.stdout.splitlines() if l.startswith("DRYRUN_QEMU ")]
        return r, (shlex.split(q[0][12:]) if q else [])

    def restrict(self, argv):
        nd = [a for a in argv if a.startswith("user,id=n0")]
        self.assertEqual(len(nd), 1, argv)
        return [p for p in nd[0].split(",") if p.startswith("restrict=")][0]

    def refused(self, **extra):
        r, argv = self.dry(**extra)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual(argv, [], "a refusal must not print or start a QEMU command")
        self.assertIn("REFUSED", r.stdout + r.stderr)
        return r.stdout + r.stderr

    def test_default_is_offline(self):
        r, argv = self.dry()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.restrict(argv), "restrict=on")

    def test_offline_explicit(self):
        r, argv = self.dry(WINVM_NET="offline")
        self.assertEqual(self.restrict(argv), "restrict=on")

    def test_online_opens_the_net(self):
        r, argv = self.dry(WINVM_NET="online")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.restrict(argv), "restrict=off")

    def test_other_values_refused(self):
        for v in ("ONLINE", "on", "1", "off"):
            self.assertIn("WINVM_NET", self.refused(WINVM_NET=v))

    def test_online_refused_for_official_apps_media(self):
        iso = self.tmp / "official.iso"
        iso.write_text("x")
        self.assertIn("proprietary", self.refused(WINVM_NET="online", WINVM_OFFICIAL_APPS_ISO=str(iso)))

    def test_online_refused_for_marker_file(self):
        (self.vmdir / "proprietary").write_text("")
        self.assertIn("proprietary", self.refused(WINVM_NET="online"))

    def test_online_refused_for_propimage_state(self):
        (self.vmdir / "state.json").write_text('{"state_version": 1}')
        self.assertIn("proprietary", self.refused(WINVM_NET="online"))

    def test_online_refused_for_explicit_flag(self):
        self.assertIn("proprietary", self.refused(WINVM_NET="online", WINVM_PROPRIETARY="1"))

    def test_proprietary_offline_still_runs(self):
        (self.vmdir / "proprietary").write_text("")
        r, argv = self.dry()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.restrict(argv), "restrict=on")


if __name__ == "__main__":
    unittest.main()
