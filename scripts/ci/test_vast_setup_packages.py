"""Package-list contracts; source only the side-effect-free policy preamble."""

from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
PROVISION = ROOT / "scripts/bench/provision_box.sh"


class PackagePolicy(unittest.TestCase):
    def plan(self, profile):
        source = PROVISION.read_text()
        prefix = source[: source.index('echo "PROVISION_START')]
        p = subprocess.run(
            ["bash", "-c", prefix + '\nkf_packages "$1"\n', "policy", profile],
            capture_output=True,
            text=True,
            timeout=5,
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        return p.stdout.split()

    def test_ci_omits_unused_compiler_tooling(self):
        self.assertFalse({"clang", "lld"} & set(self.plan("ci")))

    def test_ci_does_not_install_unused_openssl_headers(self):
        self.assertNotIn("libssl-dev", self.plan("ci"))

    def test_ci_keeps_real_build_driver_and_initrd_dependencies(self):
        self.assertTrue(
            {
                "build-essential",
                "pkg-config",
                "git",
                "curl",
                "python3",
                "kmod",
                "pciutils",
                "cpio",
                "zstd",
                "psmisc",
                "busybox-static",
            }
            <= set(self.plan("ci"))
        )

    def test_manual_full_profile_retains_optional_tools(self):
        self.assertTrue({"clang", "lld", "libssl-dev"} <= set(self.plan("full")))
        self.assertTrue(set(self.plan("ci")) <= set(self.plan("full")))

    def test_bench_keeps_nested_qemu_and_guest_image_tools(self):
        self.assertTrue(
            {
                "ninja-build",
                "meson",
                "python3-venv",
                "libglib2.0-dev",
                "libpixman-1-dev",
                "qemu-utils",
                "cloud-image-utils",
                "genisoimage",
                "qemu-system-x86",
                "ovmf",
                "musl-tools",
                "libslirp-dev",
            }
            <= set(self.plan("bench"))
        )

    def test_each_plan_is_unique_and_plain_package_names(self):
        for profile in ["ci", "full", "bench"]:
            names = self.plan(profile)
            self.assertEqual(len(names), len(set(names)))
            for name in names:
                self.assertRegex(name, r"^[a-z0-9][a-z0-9+.-]*$")


if __name__ == "__main__":
    unittest.main()
