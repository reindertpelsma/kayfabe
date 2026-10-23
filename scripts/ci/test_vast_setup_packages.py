"""Package-list contracts; source only the side-effect-free policy preamble."""

from pathlib import Path
import subprocess
import tempfile
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

    def test_ci_provisions_guest_ssh_and_key_generation(self):
        self.assertIn("openssh-client", self.plan("ci"))

    def test_manual_fresh_box_has_the_same_ssh_dependency(self):
        self.assertIn("openssh-client", self.plan("full"))

    def missing_ssh_tool_triggers_install(self, missing):
        source = PROVISION.read_text()
        policy = source[: source.index('echo "PROVISION_START')]
        probe = source[source.index('NEED=""') : source.index('if ! command -v cargo')]
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / "apt"
            script = policy + '''
command() {
    if [ "$1" = -v ]; then
        [ "$2" != "$missing_tool" ] || return 1
        printf '/fixture/%s\\n' "$2"
    else builtin command "$@"; fi
}
ldd() { return 1; }
wait_for_dpkg() { :; }
apt-get() { printf '%s\\n' "$*" >> "$apt_trace"; }
missing_tool=$1 apt_trace=$2 KF_PROVISION_PROFILE=ci
'''
            process = subprocess.run(
                ["bash", "-c", script + probe, "fixture", missing, str(trace)],
                capture_output=True, text=True, timeout=5,
            )
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertTrue(trace.is_file(), process.stdout)
            self.assertIn("openssh-client", trace.read_text())

    def test_missing_guest_key_tool_does_not_skip_apt(self):
        self.missing_ssh_tool_triggers_install("ssh-keygen")

    def test_missing_guest_ssh_client_does_not_skip_apt(self):
        self.missing_ssh_tool_triggers_install("ssh")

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

    def test_full_profile_adds_only_the_optional_stack(self):
        self.assertEqual(
            set(self.plan("full")) - set(self.plan("ci")), {"clang", "lld", "libssl-dev"}
        )

    def test_bench_has_no_unused_optional_development_stack(self):
        self.assertFalse({"clang", "lld", "libssl-dev"} & set(self.plan("bench")))

    def test_each_plan_is_unique_and_plain_package_names(self):
        for profile in ["ci", "full", "bench"]:
            names = self.plan(profile)
            self.assertGreater(len(names), 5)
            self.assertEqual(len(names), len(set(names)))
            for name in names:
                self.assertRegex(name, r"^[a-z0-9][a-z0-9+.-]*$")


if __name__ == "__main__":
    unittest.main()
