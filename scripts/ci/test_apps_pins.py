#!/usr/bin/env python3
"""Pinned inputs of the Linux apps hand path (2026-11-10), GPU-free: fake curl, local git remotes, fake dpkg/apt-get.

FALSIFIER (written before the code): a fetch that is ours to pin is accepted with an empty, malformed or mismatching
pin (git commit, file sha256, CUDA keyring); a moved tag passes for the pinned commit; the CUDA toolkit is installed
through the cache-only apt shim, or the shim is bypassed anywhere else; build_bundle.sh or setup_side.sh fetches an
input without going through the pin helpers; pins.sh disagrees with the open image recipe; build_bundle.sh exits 0 or
leaves a bundle.tgz after a failed step; an open package the script installs is missing from the setup-cache policy.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
APPS = ROOT / "scripts/apps"
PINS = APPS / "pins.sh"
LIB = APPS / "pin_lib.sh"
GIT = shutil.which("git")


def parse_pins():
    out = {}
    for line in PINS.read_text().splitlines():
        match = re.fullmatch(r"([A-Za-z0-9_]+)=(.*)", line)
        if match:
            out[match[1]] = match[2]
    return out


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Sandbox:
    """A temp dir with fake curl/dpkg/apt-get/dpkg-query in front of PATH, logging every call."""

    def __init__(self, test):
        self.dir = Path(tempfile.mkdtemp(prefix="apps-pins-"))
        test.addCleanup(shutil.rmtree, self.dir, True)
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        self.log = self.dir / "calls.log"
        self.log.write_text("")
        self.body = self.dir / "body"
        self.body.write_bytes(b"payload")
        self.tool("curl", 'echo "curl $*" >> "$FAKE_LOG"\n'
                  'while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift;; esac; shift; done\n'
                  '[ -z "${FAKE_CURL_FAIL:-}" ] || exit 22\ncp "$FAKE_BODY" "$out"\n')
        self.tool("dpkg", 'echo "dpkg $*" >> "$FAKE_LOG"\n')
        self.tool("apt-get", 'echo "apt-get $*" >> "$FAKE_LOG"\n')
        self.tool("dpkg-query", 'echo -n "${FAKE_PKG_VERSION:-}"\n')

    def tool(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/bash\n" + body)
        path.chmod(0o755)

    def calls(self):
        return self.log.read_text().splitlines()

    def run(self, script, env=None, timeout=60):
        environment = {
            "PATH": str(self.bin) + ":" + os.environ["PATH"],
            "HOME": str(self.dir),
            "FAKE_LOG": str(self.log),
            "FAKE_BODY": str(self.body),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_SYSTEM": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0",
        }
        environment.update(env or {})
        return subprocess.run(
            ["bash", "-c", ". %s; . %s; %s" % (PINS, LIB, script)],
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout,
        )


class PinsFile(unittest.TestCase):
    pins = parse_pins()

    def test_every_commit_is_forty_hex(self):
        commits = {k: v for k, v in self.pins.items() if k.endswith("_COMMIT")}
        self.assertEqual(len(commits), 4)
        for key, value in commits.items():
            self.assertRegex(value, r"^[0-9a-f]{40}$", key)

    def test_every_digest_is_sha256_hex(self):
        digests = {k: v for k, v in self.pins.items() if "SHA256" in k}
        self.assertGreaterEqual(len(digests), 5)
        for key, value in digests.items():
            self.assertRegex(value, r"^[0-9a-f]{64}$", key)

    def test_every_size_is_an_integer_and_every_url_is_https(self):
        for key, value in self.pins.items():
            if key.endswith("_SIZE"):
                self.assertRegex(value, r"^[0-9]+$", key)
            if key.endswith("_URL"):
                self.assertTrue(value.startswith("https://"), key)

    def test_the_cuda_toolkit_is_pinned_to_an_exact_version(self):
        self.assertRegex(self.pins["PIN_CUDA_TOOLKIT_VERSION"], r"^12\.6\.[0-9]+-[0-9]+$")
        self.assertEqual(self.pins["PIN_CUDA_TOOLKIT_PACKAGE"], "cuda-toolkit-12-6")

    def test_the_keyring_is_pinned_for_the_supported_releases(self):
        for release in ("ubuntu2204", "ubuntu2404"):
            self.assertRegex(self.pins["PIN_CUDA_KEYRING_SHA256_" + release], r"^[0-9a-f]{64}$", release)
        self.assertNotEqual(
            self.pins["PIN_CUDA_KEYRING_SHA256_ubuntu2204"], self.pins["PIN_CUDA_KEYRING_SHA256_ubuntu2404"]
        )

    def test_the_model_url_names_the_pinned_revision_never_main(self):
        url = self.pins["PIN_GGUF_URL"]
        self.assertIn("/resolve/" + self.pins["PIN_GGUF_REV"] + "/", url)
        self.assertNotIn("resolve/main", url)
        self.assertRegex(self.pins["PIN_GGUF_REV"], r"^[0-9a-f]{40}$")


class AgreesWithTheOpenImageRecipe(unittest.TestCase):
    """scripts/apps/image/recipe.json pins the same inputs for the open image; the two must not drift."""

    pins = parse_pins()
    recipe = {a["id"]: a for a in json.loads((APPS / "image/recipe.json").read_text())["artifacts"]}

    def test_git_commits(self):
        for artifact, key in (
            ("cuda-samples", "PIN_CUDA_SAMPLES_COMMIT"),
            ("gpu-burn", "PIN_GPU_BURN_COMMIT"),
            ("llama", "PIN_LLAMA_COMMIT"),
        ):
            with self.subTest(artifact):
                self.assertTrue(self.recipe[artifact]["url"].endswith("/" + self.pins[key]), artifact)

    def test_clpeak_tag(self):
        self.assertTrue(self.recipe["clpeak"]["url"].endswith("/" + self.pins["PIN_CLPEAK_TAG"]))

    def test_files(self):
        for artifact, prefix in (("vkpeak", "VKPEAK"), ("blender", "BLENDER"), ("qwen25-1.5b-q4km", "GGUF")):
            with self.subTest(artifact):
                entry = self.recipe[artifact]
                self.assertEqual(entry["url"], self.pins["PIN_%s_URL" % prefix])
                self.assertEqual(entry["sha256"], self.pins["PIN_%s_SHA256" % prefix])
                self.assertEqual(str(entry["size"]), self.pins["PIN_%s_SIZE" % prefix])


@unittest.skipUnless(GIT, "git is needed")
class GitPins(unittest.TestCase):
    def setUp(self):
        self.box = Sandbox(self)
        self.src = self.box.dir / "src"
        self.src.mkdir()
        self.commits = []
        self.git("init", "-q", "-b", "main")
        for n in (1, 2, 3):
            (self.src / "f").write_text("v%d\n" % n)
            self.git("add", "f")
            self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "c%d" % n)
            self.commits.append(self.git("rev-parse", "HEAD").strip())
            if n > 1:
                self.git("tag", "v%d" % (n - 1))  # v1 -> the second commit, v2 -> the third: neither is the first
        self.git("config", "uploadpack.allowAnySHA1InWant", "true")
        self.url = "file://" + str(self.src)
        self.dest = self.box.dir / "checkout"

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.src), *args],
            env={"PATH": os.environ["PATH"], "HOME": str(self.box.dir), "GIT_CONFIG_GLOBAL": "/dev/null",
                 "GIT_CONFIG_SYSTEM": "/dev/null"},
            capture_output=True, text=True, check=True,
        ).stdout

    def checkout(self, commit, ref="", env=None):
        return self.box.run('pin_git_checkout "%s" "%s" "%s" "%s"' % (self.dest, self.url, ref, commit), env)

    def head(self):
        return subprocess.run(["git", "-C", str(self.dest), "rev-parse", "HEAD"], capture_output=True, text=True,
                              env={"PATH": os.environ["PATH"]}).stdout.strip()

    def refused(self, got):
        self.assertNotEqual(got.returncode, 0, got.stdout + got.stderr)
        self.assertIn("PIN_REFUSED", got.stderr)
        self.assertFalse(self.dest.exists())

    def test_the_pinned_commit_is_checked_out(self):
        got = self.checkout(self.commits[0])
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.head(), self.commits[0])

    def test_the_other_pinned_commit_is_checked_out(self):
        got = self.checkout(self.commits[1])
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.head(), self.commits[1])

    def malformed(self, commit):
        # refused as a malformed pin before any fetch, not by luck further down
        got = self.checkout(commit)
        self.refused(got)
        self.assertIn("is not a 40-hex commit", got.stderr)

    def test_an_empty_pin_is_refused(self):
        self.malformed("")

    def test_a_branch_name_is_not_a_pin(self):
        self.malformed("main")

    def test_a_short_commit_is_not_a_pin(self):
        self.malformed(self.commits[0][:12])

    def test_an_uppercase_commit_is_not_a_pin(self):
        self.malformed(self.commits[0].upper())

    def test_a_commit_the_remote_does_not_have_fails_and_leaves_nothing(self):
        got = self.checkout("0" * 40)
        self.assertNotEqual(got.returncode, 0)
        self.assertFalse(self.dest.exists())

    def moved_tag(self, tag):
        # the remote refuses a fetch by commit; the tag is the fallback and points elsewhere: it must not pass
        if not self.sha_fetch_refused():
            self.skipTest("this git serves any commit by sha even without allowAnySHA1InWant")
        got = self.checkout(
            self.commits[0],
            ref=tag,
            env={"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "protocol.version", "GIT_CONFIG_VALUE_0": "0"},
        )
        self.refused(got)
        self.assertIn("not the pinned commit", got.stderr)

    def test_a_moved_tag_is_not_the_pinned_commit(self):
        self.moved_tag("v1")

    def test_another_moved_tag_is_not_the_pinned_commit(self):
        self.moved_tag("v2")

    def sha_fetch_refused(self):
        self.git("config", "uploadpack.allowAnySHA1InWant", "false")
        probe = self.box.dir / "probe"
        done = subprocess.run(
            ["git", "-c", "protocol.version=0", "init", "-q", str(probe)], capture_output=True,
            env={"PATH": os.environ["PATH"], "HOME": str(self.box.dir)})
        fetched = subprocess.run(
            ["git", "-c", "protocol.version=0", "-C", str(probe), "fetch", "-q", "--depth", "1", self.url,
             self.commits[0]],
            capture_output=True, env={"PATH": os.environ["PATH"], "HOME": str(self.box.dir)})
        return done.returncode == 0 and fetched.returncode != 0

    def test_a_stale_checkout_of_another_revision_is_replaced(self):
        self.assertEqual(self.checkout(self.commits[0]).returncode, 0)
        self.assertEqual(self.checkout(self.commits[1]).returncode, 0)
        self.assertEqual(self.head(), self.commits[1])

    def test_a_checkout_already_at_the_pin_is_kept_without_the_network(self):
        self.assertEqual(self.checkout(self.commits[1]).returncode, 0)
        self.url = "file:///nonexistent"
        self.assertEqual(self.checkout(self.commits[1]).returncode, 0)
        self.assertEqual(self.head(), self.commits[1])


class FilePins(unittest.TestCase):
    def setUp(self):
        self.box = Sandbox(self)
        self.dest = self.box.dir / "out.bin"
        self.good = sha(b"payload")

    def fetch(self, digest, size="", env=None):
        return self.box.run('pin_fetch_file https://example.org/x "%s" "%s" "%s"' % (self.dest, digest, size), env)

    def refused(self, got, fetched):
        self.assertNotEqual(got.returncode, 0, got.stdout + got.stderr)
        self.assertIn("PIN_REFUSED", got.stderr)
        self.assertFalse(self.dest.exists())
        self.assertFalse(Path(str(self.dest) + ".part").exists())
        self.assertEqual(any(c.startswith("curl") for c in self.box.calls()), fetched)

    def test_the_pinned_bytes_are_installed(self):
        got = self.fetch(self.good, "7")
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.dest.read_bytes(), b"payload")

    def test_an_empty_digest_is_refused_before_any_download(self):
        self.refused(self.fetch(""), fetched=False)

    def test_a_malformed_digest_is_refused_before_any_download(self):
        self.refused(self.fetch("abc123"), fetched=False)

    def test_an_uppercase_digest_is_refused(self):
        self.refused(self.fetch(self.good.upper()), fetched=False)

    def test_a_different_digest_is_refused_and_leaves_nothing(self):
        self.refused(self.fetch(sha(b"other")), fetched=True)

    def test_the_wrong_size_is_refused_even_with_the_right_digest(self):
        self.refused(self.fetch(self.good, "8"), fetched=True)

    def test_a_failed_download_installs_nothing(self):
        got = self.fetch(self.good, env={"FAKE_CURL_FAIL": "1"})
        self.assertNotEqual(got.returncode, 0)
        self.assertFalse(self.dest.exists())

    def test_verify_refuses_an_existing_file_that_is_not_the_pin(self):
        self.dest.write_bytes(b"tampered")
        got = self.box.run('pin_verify_file "%s" "%s"' % (self.dest, self.good))
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("PIN_REFUSED", got.stderr)

    def test_verify_refuses_an_empty_pin(self):
        self.dest.write_bytes(b"payload")
        got = self.box.run('pin_verify_file "%s" ""' % self.dest)
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("PIN_REFUSED", got.stderr)

    def test_verify_accepts_the_pinned_file(self):
        self.dest.write_bytes(b"payload")
        self.assertEqual(self.box.run('pin_verify_file "%s" "%s" 7' % (self.dest, self.good)).returncode, 0)


class CudaToolkitFromNvidia(unittest.TestCase):
    def setUp(self):
        self.box = Sandbox(self)
        self.release = self.box.dir / "os-release"
        self.release.write_text('ID=ubuntu\nVERSION_ID="22.04"\n')
        self.good = sha(b"payload")
        self.env = {"PIN_OS_RELEASE_FILE": str(self.release), "FAKE_PKG_VERSION": "12.6.3-1"}

    def install(self, pin=None, env=None):
        override = "PIN_CUDA_KEYRING_SHA256_ubuntu2204=%s; " % (self.good if pin is None else pin)
        script = (
            override
            + 'apt-get() { echo "SHIM $*" >> "$FAKE_LOG"; return 99; }\n'  # the payload's cache-only function
            + "pin_install_cuda_toolkit"
        )
        return self.box.run(script, {**self.env, **(env or {})})

    def test_the_toolkit_is_installed_from_the_real_apt_get_never_the_shim(self):
        got = self.install()
        self.assertEqual(got.returncode, 0, got.stderr)
        calls = self.box.calls()
        self.assertFalse([c for c in calls if c.startswith("SHIM")], calls)
        real = [c for c in calls if c.startswith("apt-get ")]
        self.assertEqual(len(real), 2, calls)
        self.assertIn(" update", real[0])
        self.assertIn("install", real[1])
        self.assertIn("cuda-toolkit-12-6=12.6.3-1", real[1])
        self.assertIn("--no-install-recommends", real[1])

    def test_the_keyring_is_installed_only_after_the_digest_matched(self):
        got = self.install(pin=sha(b"some other keyring"))
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("PIN_REFUSED", got.stderr)
        self.assertEqual([c for c in self.box.calls() if c.startswith(("dpkg", "apt-get", "SHIM"))], [])

    def test_a_release_without_a_pinned_keyring_is_refused_before_any_download(self):
        self.release.write_text('ID=ubuntu\nVERSION_ID="99.99"\n')
        got = self.install()
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("no pinned CUDA keyring", got.stderr)
        self.assertEqual(self.box.calls(), [])

    def test_an_unpinned_empty_keyring_digest_is_refused(self):
        got = self.install(pin="")
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("no pinned CUDA keyring", got.stderr)
        self.assertEqual(self.box.calls(), [])

    def test_a_different_installed_toolkit_version_is_refused(self):
        got = self.install(env={"FAKE_PKG_VERSION": "12.6.2-1"})
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("pinned 12.6.3-1", got.stderr)

    def test_the_url_names_the_pinned_release_directory(self):
        self.install()
        urls = [c for c in self.box.calls() if c.startswith("curl")]
        self.assertEqual(len(urls), 1)
        self.assertIn("/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb", urls[0])


class StaticContracts(unittest.TestCase):
    bundle = (APPS / "build_bundle.sh").read_text()
    guest = (APPS / "setup_side.sh").read_text()
    prov = (APPS / "provision_guest_apps.sh").read_text()
    lib = LIB.read_text()

    def test_every_script_parses(self):
        for name in ("build_bundle.sh", "pin_lib.sh", "pins.sh", "setup_side.sh", "provision_guest_apps.sh"):
            with self.subTest(name):
                got = subprocess.run(["bash", "-n", str(APPS / name)], capture_output=True, text=True)
                self.assertEqual(got.returncode, 0, got.stderr)

    def test_the_bundle_script_fetches_nothing_outside_the_pin_helpers(self):
        code = "\n".join(l for l in self.bundle.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("git clone", code)
        self.assertNotIn("curl", code)
        self.assertNotIn("LLAMA_PIN", code)
        self.assertNotIn("resolve/main", code)
        self.assertNotIn("command apt-get", code)

    def test_every_source_goes_through_its_own_pin(self):
        for call in (
            'pin_git_checkout "$S/cuda-samples" "$PIN_CUDA_SAMPLES_URL" "$PIN_CUDA_SAMPLES_TAG" "$PIN_CUDA_SAMPLES_COMMIT"',
            'pin_git_checkout "$S/gpu-burn" "$PIN_GPU_BURN_URL" "" "$PIN_GPU_BURN_COMMIT"',
            'pin_git_checkout "$S/llama.cpp" "$PIN_LLAMA_URL" "" "$PIN_LLAMA_COMMIT"',
            'pin_git_checkout "$S/clpeak" "$PIN_CLPEAK_URL" "$PIN_CLPEAK_TAG" "$PIN_CLPEAK_COMMIT"',
            'pin_fetch_file "$PIN_VKPEAK_URL" "$VKDIR/vkpeak.zip" "$PIN_VKPEAK_SHA256" "$PIN_VKPEAK_SIZE"',
            "pin_install_cuda_toolkit",
        ):
            with self.subTest(call):
                self.assertIn(call, self.bundle)

    def test_each_failed_source_is_counted(self):
        for step in ("src_cuda_samples", "src_gpu_burn", "src_llama_cpp", "src_clpeak", "cuda_toolkit", "open_dev_packages"):
            with self.subTest(step):
                self.assertRegex(self.bundle, r"\bok %s \$" % step)

    def test_the_guest_side_pins_its_downloads(self):
        code = "\n".join(l for l in self.guest.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("resolve/main", code)
        self.assertNotIn("| tar xJ", code)
        self.assertIn('pin_fetch_file "$PIN_GGUF_URL" "$G" "$PIN_GGUF_SHA256" "$PIN_GGUF_SIZE"', code)
        self.assertIn('pin_fetch_file "$PIN_BLENDER_URL"', code)
        self.assertIn('"$PIN_BLENDER_SHA256"', code)

    def test_the_guest_side_downloads_only_through_the_pin_helpers(self):
        # the one raw curl left is the manual, owner-approved Geekbench fetch (KF_APPS_GEEKBENCH=1, docs/DOWNLOADS.md)
        raw = [
            l for l in self.guest.splitlines()
            if "curl" in l and not l.lstrip().startswith("#") and "cdn.geekbench.com" not in l
            and "python3-venv python3-pip curl ca-certificates" not in l
        ]
        self.assertEqual(raw, [])

    def test_the_guest_side_refuses_to_run_without_the_pins(self):
        with tempfile.TemporaryDirectory() as directory:
            shutil.copyfile(APPS / "setup_side.sh", Path(directory) / "setup_side.sh")
            got = subprocess.run(["bash", str(Path(directory) / "setup_side.sh")], capture_output=True, text=True,
                                 timeout=30, env={"PATH": os.environ["PATH"]})
        self.assertEqual(got.returncode, 1)
        self.assertIn("SETUP_pins=FAIL", got.stdout)

    def test_the_pin_files_travel_to_the_guest(self):
        self.assertIn('"$HERE/pins.sh" "$HERE/pin_lib.sh"', self.prov)

    def test_a_failed_guest_setup_fails_the_provisioning_script(self):
        self.assertIn("SETUP_FAILS", self.prov)
        self.assertIn("exit 5", self.prov)

    def test_the_shim_is_bypassed_in_exactly_one_place(self):
        found = {}
        for path in sorted(ROOT.glob("scripts/apps/**/*.sh")) + sorted(ROOT.glob("scripts/apps/**/*.py")):
            count = len(re.findall(r"command\s+apt-get", path.read_text()))
            if count:
                found[path.name] = count
        self.assertEqual(found, {"pin_lib.sh": 2})
        self.assertLess(self.lib.index("DELIBERATE BYPASS"), self.lib.index("command apt-get"))

    def test_no_nvidia_package_is_in_the_setup_cache_policy(self):
        names = set()
        for profile in ("ci", "bench", "windows", "apps"):
            names.update(
                subprocess.run(
                    ["bash", str(ROOT / "scripts/bench/provision_box.sh"), "--packages", profile],
                    capture_output=True, text=True, check=True,
                ).stdout.split()
            )
        self.assertFalse([n for n in names if re.search(r"cuda|nvidia|nvcc|cublas|cudnn", n)])

    def test_the_apps_profile_is_exactly_the_open_dev_packages(self):
        got = subprocess.run(
            ["bash", str(ROOT / "scripts/bench/provision_box.sh"), "--packages", "apps"],
            capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertEqual(
            sorted(got), ["cmake", "libegl-dev", "libgles-dev", "ocl-icd-opencl-dev", "opencl-headers", "unzip"]
        )

    def test_the_open_packages_the_bundle_script_installs_are_in_the_policy(self):
        names = set()
        for profile in ("ci", "apps"):
            names.update(
                subprocess.run(
                    ["bash", str(ROOT / "scripts/bench/provision_box.sh"), "--packages", profile],
                    capture_output=True, text=True, check=True,
                ).stdout.split()
            )
        block = re.search(r"^apt-get install -y -qq ((?:.|\n)*?)>/tmp/open_dev_apt.log", self.bundle, re.M)
        wanted = set(block[1].replace("\\", " ").split())
        wanted = {w for w in wanted if not w.startswith("-")}
        self.assertTrue({"cmake", "unzip", "libegl-dev", "libgles-dev", "ocl-icd-opencl-dev", "opencl-headers"} <= wanted)
        self.assertEqual(sorted(wanted - names), [])


class BundleScriptFailsClosed(unittest.TestCase):
    """build_bundle.sh with every external tool failing: it must exit 1, say why and leave no bundle."""

    def setUp(self):
        self.box = Sandbox(self)
        for tool in ("apt-get", "dpkg", "curl", "git", "nvcc", "gcc", "cmake", "make", "unzip", "pkg-config"):
            self.box.tool(tool, 'echo "%s $*" >> "$FAKE_LOG"\nexit 1\n' % tool)
        self.root = self.box.dir / "apps"
        self.root.mkdir()
        self.release = self.box.dir / "os-release"
        self.release.write_text('ID=ubuntu\nVERSION_ID="22.04"\n')

    def build(self):
        if Path("/usr/local/cuda-12.6/bin/nvcc").exists():
            self.skipTest("a real CUDA toolkit is installed on this machine")
        return self.box.run(
            'cd "%s" && bash "%s"' % (self.box.dir, APPS / "build_bundle.sh"),
            {"KF_APPS_ROOT": str(self.root), "PIN_OS_RELEASE_FILE": str(self.release)},
            timeout=120,
        )

    def test_it_exits_nonzero_and_names_the_failures(self):
        got = self.build()
        self.assertEqual(got.returncode, 1, got.stdout[-2000:] + got.stderr[-2000:])
        self.assertIn("BUILD_cuda_toolkit=FAIL", got.stdout)
        self.assertIn("BUILD_src_gpu_burn=FAIL", got.stdout)
        self.assertRegex(got.stdout, r"BUILD_BUNDLE_FAIL count=[1-9][0-9]*")
        self.assertNotIn("BUILD_BUNDLE_DONE", got.stdout)

    def test_the_exit_status_is_one(self):
        self.assertEqual(self.build().returncode, 1)

    def test_it_writes_no_bundle(self):
        self.build()
        self.assertFalse((self.root / "bundle.tgz").exists())
        self.assertFalse((self.root / "bundle.tgz.tmp").exists())

    def test_it_removes_a_stale_bundle(self):
        (self.root / "bundle.tgz").write_text("stale")
        self.build()
        self.assertFalse((self.root / "bundle.tgz").exists())

    def test_it_never_installs_a_toolkit_without_the_keyring_check(self):
        self.build()
        calls = self.box.calls()
        self.assertTrue(any(c.startswith("curl") and "cuda-keyring" in c for c in calls))
        self.assertFalse([c for c in calls if c.startswith("dpkg -i")])


if __name__ == "__main__":
    unittest.main()
