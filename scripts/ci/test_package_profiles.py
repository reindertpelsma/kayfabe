"""The package profiles that the open rows add to provision_box.sh, against the setup cache policy and the code that uses them (2026-11-20).

Cross-file contracts live here, not in test_vast_setup_packages.py: setup_policy_mutants.py copies that file alone into a tree with only provision_box.sh. This file is
copied together with vast_setup.py, the lock and the files it reads (setup_policy_mutants.py, two named killers per mutant).

A payload's `apt-get` is the cache-only shim, so a package a stage installs must be in the setup cache's policy (vast_setup.package_names, the digest in
setup-cache.lock.json). The cache itself (debs.tar.gz) is rebuilt by scripts/ci/build_setup_cache.sh and published by the owner; see docs/design/VAST_SETUP_CACHE_20261023.md.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import vast_setup  # noqa: E402

PROVISION = ROOT / "scripts/bench/provision_box.sh"
# the headers each module of private_python.REQUIRED_MODULES needs to be built at all (venv and ensurepip are pure Python)
MODULE_PACKAGE = {"ssl": "libssl-dev", "zlib": "zlib1g-dev", "ctypes": "libffi-dev", "bz2": "libbz2-dev", "lzma": "liblzma-dev", "sqlite3": "libsqlite3-dev"}


def plan(profile):
    return subprocess.run(["bash", str(PROVISION), "--packages", profile], capture_output=True, text=True, check=True).stdout.split()


def lock_digest():
    return json.loads((HERE / "setup-cache.lock.json").read_text())["package_policy_sha256"]


class PrivatePythonPackages(unittest.TestCase):
    def test_pybuild_is_in_the_setup_cache_policy_because_the_payload_apt_is_cache_only(self):
        self.assertTrue(set(plan("pybuild")) <= set(vast_setup.package_names(ROOT)))

    def test_the_lock_names_the_policy_that_holds_pybuild_and_not_the_one_without_it(self):
        self.assertEqual(lock_digest(), vast_setup.policy_digest(ROOT))
        without = sorted(set(vast_setup.package_names(ROOT)) - (set(plan("pybuild")) - set(sum((plan(p) for p in ("ci", "bench", "windows", "apps")), []))))
        self.assertNotEqual(lock_digest(), hashlib.sha256(json.dumps(without, separators=(",", ":")).encode()).hexdigest())

    def test_the_packages_pybuild_adds_are_in_no_other_profile(self):
        extra = set(plan("pybuild")) - {"build-essential", "libssl-dev"}          # build-essential is in `ci`, libssl-dev in `full` (never `ci`)
        for profile in ("ci", "full", "bench", "windows", "apps"):
            self.assertFalse(extra & set(plan(profile)), profile)
        self.assertNotIn("libssl-dev", plan("ci"))

    def test_every_module_the_private_python_must_import_has_its_header_package_in_pybuild(self):
        text = (ROOT / "scripts/apps/mlrows/private_python.py").read_text()
        modules = re.search(r"REQUIRED_MODULES = \(([^)]*)\)", text)[1]
        names = set(re.findall(r'"(\w+)"', modules))
        self.assertEqual(names - {"venv", "ensurepip"}, set(MODULE_PACKAGE), "a module was added to or taken from REQUIRED_MODULES: name its header package here")
        self.assertTrue(set(MODULE_PACKAGE.values()) <= set(plan("pybuild")))

    def test_the_private_python_reads_its_dependencies_from_this_profile_and_nothing_else_names_them(self):
        text = (ROOT / "scripts/apps/mlrows/private_python.py").read_text()
        self.assertIn('PACKAGE_PROFILE = "pybuild"', text)
        for name in MODULE_PACKAGE.values():
            self.assertNotIn(name, text.replace("# ", ""), name)


class NvcodecControlPackages(unittest.TestCase):
    """The host control of nvcodec_matrix (scripts/ci/apps_discovery_stage.py) installs the distribution's ffmpeg through provision.sh of the open rows."""

    def test_nvcodec_control_is_in_the_setup_cache_policy_because_the_payload_apt_is_cache_only(self):
        self.assertEqual(plan("nvcodec_control"), ["ffmpeg"])
        self.assertTrue(set(plan("nvcodec_control")) <= set(vast_setup.package_names(ROOT)))

    def test_the_lock_names_the_policy_that_holds_ffmpeg_and_not_the_one_without_it(self):
        self.assertEqual(lock_digest(), vast_setup.policy_digest(ROOT))
        without = sorted(set(vast_setup.package_names(ROOT)) - {"ffmpeg"})
        self.assertNotEqual(lock_digest(), hashlib.sha256(json.dumps(without, separators=(",", ":")).encode()).hexdigest())

    def test_ffmpeg_is_in_no_other_profile(self):
        for profile in ("ci", "full", "bench", "windows", "apps", "pybuild"):
            self.assertNotIn("ffmpeg", plan(profile), profile)

    def test_the_profile_is_what_the_provisioning_the_stage_runs_installs(self):
        provision = (ROOT / "scripts/apps/open_rows/provision.sh").read_text()
        self.assertIn("apt_install ffmpeg", provision)                                 # the package the profile names
        stage = (HERE / "apps_discovery_stage.py").read_text()
        self.assertIn('"nvcodec_probe ffmpeg"', stage)                                 # the steps of provision.sh the stage asks for: the probe and ffmpeg, nothing else


if __name__ == "__main__":
    unittest.main()
