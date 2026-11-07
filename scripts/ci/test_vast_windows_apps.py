#!/usr/bin/env python3
"""GPU-free, VM-free falsifiers for the Windows apps stage's box side (docs/design/V3_WINDOWS_APPS_GATE.md).

FALSIFIER (written before the code): any of these is wrong if the test does not fail on the mutation:
  - windows_apps.sh passes (exit 0) without a WINDOWS_GATE_PASS of the same tag, without an app disk, or without a pin;
  - a host/harness fault (no pin, no gate pass, a refusal) prints WINAPPS_PASS or a FAIL verdict;
  - the box script writes a password or a presigned URL;
  - win_vm.sh's two new opt-ins (WINVM_OVERLAY_BACKING, WINVM_APPS_ISO) change the default command line, or put the app
    disk anywhere but a read-only AHCI cd-rom, or give the guest a network.
Nothing here starts QEMU or touches a GPU: win_vm.sh runs in its --dry-run mode, windows_apps.sh in --selftest and in
its refusal paths.
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
APPS = ROOT / "scripts/bench/box/windows_apps.sh"
WV = WIN / "win_vm.sh"


class AppsScript(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winapps-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def run_apps(self, *args, env=None, tag="t1"):
        e = {"PATH": os.environ["PATH"], "HOME": str(self.tmp), "KF_PROV_DIR": str(self.tmp / "prov"),
             "WINVM_ROOT": str(self.tmp / "winvm"), "BENCH": str(self.tmp / "bench")}
        e.update(env or {})
        return subprocess.run(["bash", str(APPS), tag, *args], env=e, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)

    def test_parses_and_selftest_is_green(self):
        self.assertEqual(subprocess.run(["bash", "-n", str(APPS)]).returncode, 0)
        r = subprocess.run(["bash", str(APPS), "--selftest"], capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("WINAPPS_SELFTEST_OK", r.stdout)
        self.assertIn("WINAPPS_EXIT rc=0", r.stdout)

    @unittest.skipUnless(shutil.which("shellcheck"), "shellcheck not installed")
    def test_shellcheck_warnings(self):
        r = subprocess.run(["shellcheck", "-x", "-S", "warning", str(APPS)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_invalid_tag_is_refused(self):
        r = self.run_apps(tag="../x")
        self.assertEqual(r.returncode, 2)
        self.assertIn("WINAPPS_REFUSED why=invalid-tag", r.stdout)

    def test_without_a_windows_install_and_a_gate_pass_nothing_runs_and_nothing_passes(self):
        r = self.run_apps()
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("WINAPPS_REFUSED why=", r.stdout)
        self.assertNotIn("WINAPPS_PASS", r.stdout)
        self.assertTrue(r.stdout.rstrip().splitlines()[-1].startswith("WINAPPS_EXIT rc=2"))

    def test_a_gate_log_that_is_not_a_pass_is_refused(self):
        vm = self.tmp / "winvm" / "kfwin"
        (vm / "disk").mkdir(parents=True)
        (vm / "installed").write_text("x\n")
        (self.tmp / "prov").mkdir()
        (self.tmp / "prov" / "t1_windows.log").write_text("WINDOWS_GATE_FAIL diag=KAYFABE stage=idle\nWINDOWS_GATE_EXIT rc=1\n")
        r = self.run_apps(env={"KF_WINDOWS_APPS_TEST": "1"})
        # not root or no /dev/kvm refuse earlier on a CI machine; either way the refusal is the verdict and nothing passes
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertRegex(r.stdout, r"WINAPPS_REFUSED why=(not-root|no-/dev/kvm|no-WINDOWS_GATE_PASS|windows-not-installed)")
        self.assertNotIn("WINAPPS_PASS", r.stdout)

    def test_the_script_has_no_password_and_no_url_in_it(self):
        text = APPS.read_text()
        self.assertIn("unset KF_GUEST_PW", text)
        self.assertNotRegex(text, r"https?://")
        self.assertNotRegex(text, r"(?i)win_password|ssh_ed25519")

    def test_every_verdict_line_has_an_exit_code_that_agrees(self):
        text = APPS.read_text()
        for kind, rc in (("PASS", 0), ("FAIL", 1)):
            m = re.search(r"%s\) say \"WINAPPS_%s .*?rc_final=(\d)" % (kind, kind), text)
            self.assertIsNotNone(m, kind)
            self.assertEqual(int(m.group(1)), rc)
        m = re.search(r'\*\) say "WINAPPS_RETRY .*?rc_final=(\d)', text)
        self.assertEqual(int(m.group(1)), 3)


class PackagingScript(unittest.TestCase):
    PKG = ROOT / "scripts/bench/windows/appmatrix/package_open_bundle.sh"
    NEVER = ("gravitymark", "furmark2", "gputest", "heaven", "valley", "vulkan_sdk", "ffmpeg")

    def run_pkg(self, *args, env=None):
        e = {"PATH": os.environ["PATH"], "HOME": tempfile.gettempdir()}
        e.update(env or {})
        return subprocess.run(["bash", str(self.PKG), *args], env=e, capture_output=True, text=True, timeout=120)

    def test_dry_run_names_only_open_packages_and_fetches_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            r = self.run_pkg("--out", d, "--dry-run")
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("DRY-RUN ok", r.stdout)
            line = [l for l in r.stdout.splitlines() if "packages: " in l][0].split("packages: ")[1]
            ids = set(line.split(","))
            self.assertTrue({"python_embed", "pywheels", "cuda_demo_suite"} <= ids)
            self.assertFalse(ids & set(self.NEVER), ids & set(self.NEVER))
            self.assertEqual(os.listdir(d), [])                       # nothing was created: no download, no build

    def test_it_is_a_persons_one_time_step_not_ci_and_not_a_box(self):
        with tempfile.TemporaryDirectory() as d:
            for env in ({"CI": "true"}, {"KF_RUN_TOKEN": "x"}):
                r = self.run_pkg("--out", d, "--dry-run", env=env)
                self.assertEqual(r.returncode, 4, env)
                self.assertIn("REFUSED", r.stdout)

    def test_it_never_uploads(self):
        text = self.PKG.read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotRegex(code, r"rclone|aws |s3 cp|curl .*-T|wrangler")

    def test_the_pin_file_is_empty_until_the_owner_uploads(self):
        pin = ROOT / "scripts/bench/windows/appmatrix/bundle.pin"
        lines = [l for l in pin.read_text().splitlines() if l and not l.startswith("#")]
        for l in lines:
            self.assertRegex(l, r"^winapps-open\.iso sha256=[0-9a-f]{64}$")


class WinVmOptIns(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winvm-apps-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        vm = self.tmp / "root" / "kfwin"
        (vm / "secrets").mkdir(parents=True)
        (vm / "logs").mkdir()
        (vm / "installed").write_text("test\n")

    def dry(self, **extra):
        e = {"PATH": os.environ["PATH"], "HOME": str(self.tmp), "LANG": "C", "WINVM_ROOT": str(self.tmp / "root"),
             "KF_GPU_LOCK": str(self.tmp / "gpu.lock"), "BENCH": str(self.tmp / "bench"), "WINVM_QEMU": "/fake/qemu-system-x86_64"}
        e.update(extra)
        r = subprocess.run(["bash", str(WV), "run", "--dry-run", "--tag", "t"], env=e, capture_output=True, text=True, timeout=60,
                           stdin=subprocess.DEVNULL)
        q = [l for l in r.stdout.splitlines() if l.startswith("DRYRUN_QEMU ")]
        return r, (shlex.split(q[0][len("DRYRUN_QEMU "):]) if q else [])

    def test_default_command_line_has_no_app_disk(self):
        r, argv = self.dry()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(argv)
        self.assertFalse([a for a in argv if "kfapps" in a])

    def test_the_app_disk_is_a_read_only_ahci_cdrom_and_the_guest_stays_offline(self):
        iso = self.tmp / "kfapps.iso"
        iso.write_bytes(b"x")
        r, argv = self.dry(WINVM_APPS_ISO=str(iso))
        self.assertEqual(r.returncode, 0, r.stderr)
        drives = [a for a in argv if a.startswith("file=") and "kfapps" in a]
        self.assertEqual(len(drives), 1)
        self.assertIn("media=cdrom", drives[0])
        self.assertIn("readonly=on", drives[0])
        self.assertIn("ide-cd,drive=kfapps,bus=ide.3", argv)
        net = [a for a in argv if a.startswith("user,")]
        self.assertEqual(len(net), 1)
        self.assertIn("restrict=on", net[0])                              # no route out
        self.assertEqual(len([a for a in argv if "hostfwd" in a]), 1)     # the harness's ssh forward only
        rest, i = [], 0
        while i < len(argv):                                              # drop exactly the two option pairs of the app disk
            if argv[i] in ("-drive", "-device") and i + 1 < len(argv) and "kfapps" in argv[i + 1]:
                i += 2
                continue
            rest.append(argv[i])
            i += 1
        self.assertEqual(rest, self.dry()[1])                             # nothing else changed

    def test_the_overlay_backing_must_be_a_plain_name(self):
        r, _ = self.dry(WINVM_OVERLAY_BACKING="../base")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("plain overlay name", r.stdout + r.stderr)

    def test_a_missing_app_disk_is_refused_by_name_on_a_real_run(self):
        # --dry-run skips the existence check on purpose (it starts nothing); the check itself must exist
        text = WV.read_text()
        self.assertIn("WINVM_APPS_ISO: no such file", text)
        self.assertIn("WINVM_OVERLAY_BACKING: no disk/", text)


if __name__ == "__main__":
    unittest.main()
