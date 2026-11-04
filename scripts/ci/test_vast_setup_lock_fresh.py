"""The committed setup-cache lock must describe THIS tree (lane 2, 2026-11-04).

A box restores the dependency cache only when scripts/ci/setup-cache.lock.json names the tree's own
package policy (provision_box.sh --packages ci/bench) and Cargo.lock digests (vast_setup.validate_lock).
Cargo.lock changed on next in 92e899c07 (2026-10-31) without a new lock, so every next-based tree
failed KF_SETUP_CACHE_REFUSED on a Worker box (runs 41746861215, 2026-11-04) and nothing GPU-free
noticed. Regenerate the lock (scripts/ci/build_setup_cache.sh, then publish the assets) whenever
Cargo.lock, firmware/kf-gop/Cargo.lock or a provision_box.sh package list changes.
FALSIFIER: a changed Cargo.lock or package list leaves these tests green.
"""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import vast_setup  # noqa: E402


def lock():
    return json.loads((ROOT / "scripts/ci/setup-cache.lock.json").read_text())


class LockFreshness(unittest.TestCase):
    def test_package_policy_matches_the_tree(self):
        self.assertEqual(
            lock()["package_policy_sha256"],
            vast_setup.policy_digest(ROOT),
            "provision_box.sh package lists changed: regenerate scripts/ci/setup-cache.lock.json",
        )

    def test_locked_files_match_the_tree(self):
        committed = lock()["locks"]
        self.assertEqual(set(committed), set(vast_setup.LOCK_PATHS))
        for name in vast_setup.LOCK_PATHS:
            with self.subTest(name):
                self.assertEqual(
                    committed[name],
                    vast_setup.digest(ROOT / name),
                    name + " changed: regenerate scripts/ci/setup-cache.lock.json",
                )

    def test_the_box_check_refuses_a_changed_tree(self):
        # The mutant: the same check the box runs (validate_lock) on a copy whose Cargo.lock or
        # package list differs must refuse; on the tree itself it must accept.
        facts = dict(vast_setup.PLATFORM)
        committed = lock()
        vast_setup.validate_lock(committed, ROOT, facts, vast_setup.policy_digest(ROOT))
        with tempfile.TemporaryDirectory() as d:
            copy = Path(d)
            for name in vast_setup.LOCK_PATHS:
                (copy / name).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, copy / name)
            with open(copy / "Cargo.lock", "a") as f:
                f.write("\n# mutant\n")
            with self.assertRaises(Exception):
                vast_setup.validate_lock(committed, copy, facts, vast_setup.policy_digest(ROOT))
        with self.assertRaises(Exception):
            vast_setup.validate_lock(committed, ROOT, facts, "0" * 64)

    def test_the_package_policy_is_read_from_provision_box(self):
        out = subprocess.check_output(
            ["bash", str(ROOT / "scripts/bench/provision_box.sh"), "--packages", "ci"], text=True
        )
        self.assertTrue(out.split())


if __name__ == "__main__":
    unittest.main()
