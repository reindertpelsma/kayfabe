#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The fast suite's fault arms boot the kf3 device with `unmapped-reads-zero=off` (STATUS: LIVE, 2026-11-24).

`--defer-liveness` and `--missing-page-fault` assert the FAULT of a read through an unmapped guest VA. The device
property `unmapped-reads-zero` defaults to ON for every guest (owner ruling BH, docs/OWNER_RULINGS.md), and with it ON
that read returns zeros and never faults, so both arms fail (docs/design/V3_SPARSE_RESIDENCY_RCA.md, section 8).
`scripts/fastguest/run_fast_guest.sh` therefore adds `unmapped-reads-zero=off` to the device line of a boot that runs
either arm and of no other boot. These tests run that script with a fake QEMU that records its argument vector: no GPU,
no guest, no network.

The script under test is the checkout's; `scripts/ci/fastsuite_fill_mutants.py` points `KF_FASTSUITE_ROOT` at a mutated
copy of the tree (a test-only switch, read nowhere else).
"""

import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(os.environ.get("KF_FASTSUITE_ROOT") or Path(__file__).resolve().parents[2])
LANE = ROOT / "scripts/fastguest/run_fast_guest.sh"
SUITE = ROOT / "scripts/fastguest/fast_suite.sh"
PROP = "unmapped-reads-zero=off"
FAULT_ARMS = ("--defer-liveness", "--missing-page-fault")


def suite_arms():
    text = SUITE.read_text(encoding="utf-8")
    block = re.search(r"ARMS=\((--.*?)\)\n", text, re.S).group(1)
    return re.findall(r"--[a-z0-9-]+", block)


def run_lane(arms, extra="guest-driver=580.159.04", gpus=None):
    """Run the lane once with a recording fake QEMU. Returns (returncode, qemu argv or None, output)."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        fg = root / "fg"
        fg.mkdir()
        for name in ("vmlinuz", "initrd.cpio.gz"):
            (fg / name).write_text("fixture; no GPU execution")

        def executable(path, text):
            path.write_text(text)
            path.chmod(0o755)

        executable(bin_dir / "pgrep", "#!/bin/sh\nexit 1\n")
        executable(
            bin_dir / "nvidia-smi",
            '#!/bin/sh\ncase "$*" in *memory.total*) echo 8192;; *) echo 0;; esac\n',
        )
        fake = root / "fake-qemu"
        executable(
            fake,
            "#!/usr/bin/env python3\nimport json,os,sys\n"
            "open(os.environ['FIXTURE_ROOT']+'/argv','w').write(json.dumps(sys.argv[1:]))\n",
        )
        env = os.environ.copy()
        env.update(
            PATH=str(bin_dir) + ":" + env["PATH"],
            FIXTURE_ROOT=str(root),
            KF_DEVICE="kf3",
            QEMU_BIN=str(fake),
            BENCH_DIR=str(root),
            KF_FASTGUEST_DIR=str(fg),
            KF_LOCK=str(root / "lock"),
            KF3_DEV_EXTRA=extra,
            KF_REQUIRE_FORWARD="0",
            KF_ARMS=arms,
        )
        env.pop("KF3_GPUS", None)
        if gpus:
            env["KF3_GPUS"] = gpus
        result = subprocess.run(
            ["bash", str(LANE), "fill", "5"], env=env, text=True, capture_output=True, timeout=30
        )
        argv_file = root / "argv"
        argv = json.loads(argv_file.read_text()) if argv_file.exists() else None
        return result.returncode, argv, result.stdout + result.stderr


def devices(argv):
    return [argv[i + 1] for i, a in enumerate(argv) if a == "-device" and argv[i + 1].startswith("kf3-gpu")]


class FaultArmsFillOff(unittest.TestCase):
    def test_each_fault_arm_boots_with_the_fill_off(self):
        for arm in FAULT_ARMS:
            with self.subTest(arm=arm):
                rc, argv, out = run_lane(arm)
                self.assertIsNotNone(argv, out)
                devs = devices(argv)
                self.assertEqual(len(devs), 1, out)
                self.assertEqual(devs[0].split(",").count(PROP), 1, devs[0])

    def test_every_other_suite_arm_keeps_the_default_fill(self):
        arms = suite_arms()
        self.assertEqual(len(arms), 30, arms)
        self.assertTrue(set(FAULT_ARMS) <= set(arms), "the fast suite no longer lists a fault arm")
        for arm in arms:
            if arm in FAULT_ARMS:
                continue
            with self.subTest(arm=arm):
                rc, argv, out = run_lane(arm)
                self.assertIsNotNone(argv, out)
                self.assertNotIn("unmapped-reads-zero", " ".join(devices(argv)), arm)

    def test_a_fault_arm_among_others_turns_the_fill_off_for_that_boot(self):
        rc, argv, out = run_lane("--timer --missing-page-fault --engines")
        self.assertIsNotNone(argv, out)
        self.assertEqual(devices(argv)[0].split(",").count(PROP), 1, out)

    def test_every_device_of_a_multi_gpu_boot_gets_it(self):
        rc, argv, out = run_lane("--defer-liveness", gpus="0,1")
        self.assertIsNotNone(argv, out)
        devs = devices(argv)
        self.assertEqual(len(devs), 2, out)
        for d in devs:
            self.assertEqual(d.split(",").count(PROP), 1, d)

    def test_multi_gpu_boot_of_the_other_fault_arm(self):
        rc, argv, out = run_lane("--missing-page-fault", gpus="0,1")
        self.assertIsNotNone(argv, out)
        devs = devices(argv)
        self.assertEqual(len(devs), 2, out)
        for d in devs:
            self.assertEqual(d.split(",").count(PROP), 1, d)

    def test_a_fault_arm_listed_last_counts(self):
        rc, argv, out = run_lane("--timer --engines --defer-liveness")
        self.assertIsNotNone(argv, out)
        self.assertEqual(devices(argv)[0].split(",").count(PROP), 1, out)

    def test_the_probe_flag_the_lane_appends_does_not_hide_a_fault_arm(self):
        # The lane folds `--probe-launch-dma` into the arm token; the property must still be derived from the arm.
        rc, argv, out = run_lane("--missing-page-fault")
        self.assertIsNotNone(argv, out)
        self.assertIn("--missing-page-fault,--probe-launch-dma", " ".join(argv))
        self.assertEqual(devices(argv)[0].split(",").count(PROP), 1, out)

    def test_the_lane_says_why_in_its_log(self):
        rc, argv, out = run_lane("--missing-page-fault")
        self.assertIn(PROP, out)
        rc, argv, out = run_lane("--timer")
        self.assertNotIn("unmapped-reads-zero", out)


class ConflictingFill(unittest.TestCase):
    def test_a_fault_arm_refuses_an_explicit_fill_on(self):
        rc, argv, out = run_lane("--defer-liveness", extra="guest-driver=580.159.04,unmapped-reads-zero=on")
        self.assertEqual(rc, 2, out)
        self.assertIsNone(argv, "QEMU must not start with a fill the arm cannot pass under")
        self.assertIn("unmapped-reads-zero", out)

    def test_a_fault_arm_refuses_the_other_spellings_of_on(self):
        for value in ("true", "yes", "1"):
            with self.subTest(value=value):
                rc, argv, out = run_lane(
                    "--missing-page-fault", extra="guest-driver=580.159.04,unmapped-reads-zero=" + value
                )
                self.assertEqual(rc, 2, out)
                self.assertIsNone(argv)

    def test_an_explicit_fill_off_is_accepted_without_a_duplicate(self):
        rc, argv, out = run_lane("--defer-liveness", extra="guest-driver=580.159.04,unmapped-reads-zero=off")
        self.assertIsNotNone(argv, out)
        self.assertEqual(devices(argv)[0].split(",").count(PROP), 1, out)

    def test_a_non_fault_arm_may_run_with_the_fill_on_or_off(self):
        for value in ("on", "off"):
            with self.subTest(value=value):
                rc, argv, out = run_lane(
                    "--timer", extra="guest-driver=580.159.04,unmapped-reads-zero=" + value
                )
                self.assertIsNotNone(argv, out)
                self.assertIn("unmapped-reads-zero=" + value, devices(argv)[0])


class DocumentedWhy(unittest.TestCase):
    def test_the_lane_header_names_the_document_and_the_arms(self):
        text = LANE.read_text(encoding="utf-8")
        for needle in ("unmapped-reads-zero=off", "V3_SPARSE_RESIDENCY_RCA.md", "--defer-liveness", "--missing-page-fault"):
            self.assertIn(needle, text)

    def test_the_suite_lists_why_two_arms_boot_with_the_fill_off(self):
        text = SUITE.read_text(encoding="utf-8")
        self.assertIn("unmapped-reads-zero=off", text)


if __name__ == "__main__":
    unittest.main()
