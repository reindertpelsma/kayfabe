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
            out[match[1]] = match[2].strip('"')
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
        # FAKE_CURL_FAIL: every call fails; FAKE_CURL_FAIL_FIRST=k: the first k calls fail (a transient origin error);
        # FAKE_CURL_CORRUPT_FIRST=k: the first k calls deliver other bytes (a truncated or tampered download)
        self.tool("curl", 'echo "curl $*" >> "$FAKE_LOG"\n'
                  'n=$(grep -c "^curl" "$FAKE_LOG")\n'
                  'while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift;; esac; shift; done\n'
                  '[ -z "${FAKE_CURL_FAIL:-}" ] || exit 22\n'
                  '[ "$n" -gt "${FAKE_CURL_FAIL_FIRST:-0}" ] || exit 22\n'
                  'if [ "$n" -le "${FAKE_CURL_CORRUPT_FIRST:-0}" ]; then echo corrupt > "$out"; else cp "$FAKE_BODY" "$out"; fi\n')
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
            "PIN_RETRY_BACKOFF": "0 0",
        }
        environment.update(env or {})
        return subprocess.run(
            ["bash", "-c", ". %s; . %s; PIN_RETRY_BACKOFF='0 0'; %s" % (PINS, LIB, script)],
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
        for step in ("src_cuda_samples", "src_gpu_burn", "src_llama_cpp", "src_clpeak", "cuda_toolkit", "open_dev_packages",
                     "cuda_libs", "cuda_headers"):
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

    def test_the_bundle_tree_is_world_readable_under_the_payload_umask(self):
        # the Vast payload runs under umask 077 (vast_onstart.sh); the guest reads the bundle as an unprivileged user
        if Path("/usr/local/cuda-12.6/bin/nvcc").exists():
            self.skipTest("a real CUDA toolkit is installed on this machine")
        self.box.run(
            'cd "%s" && umask 077 && bash "%s"' % (self.box.dir, APPS / "build_bundle.sh"),
            {"KF_APPS_ROOT": str(self.root), "PIN_OS_RELEASE_FILE": str(self.release)},
            timeout=120,
        )
        for name in ("bundle", "bundle/bin", "bundle/lib", "bundle/samples", "bundle/share"):
            self.assertEqual((self.root / name).stat().st_mode & 0o777, 0o755, name)

    def test_missing_cuda_libraries_are_a_build_failure(self):
        got = self.build()
        self.assertIn("BUILD_cuda_libs=FAIL", got.stdout)
        self.assertIn("missing CUDA library libcudart.so.12", got.stdout)

    def test_missing_cuda_headers_are_a_build_failure(self):
        self.assertIn("BUILD_cuda_headers=FAIL", self.build().stdout)

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


class Retries(unittest.TestCase):
    """Every provisioning fetch is tried up to three times; a persistent failure still fails; the pin check stays."""

    def setUp(self):
        self.box = Sandbox(self)
        self.dest = self.box.dir / "out.bin"
        self.good = sha(b"payload")

    def curls(self):
        return len([c for c in self.box.calls() if c.startswith("curl")])

    def fetch(self, env=None):
        return self.box.run('pin_fetch_file https://example.org/x "%s" "%s" 7' % (self.dest, self.good), env)

    def test_one_transient_failure_is_retried(self):
        got = self.fetch({"FAKE_CURL_FAIL_FIRST": "1"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.curls(), 2)
        self.assertEqual(self.dest.read_bytes(), b"payload")

    def test_two_transient_failures_are_retried(self):
        got = self.fetch({"FAKE_CURL_FAIL_FIRST": "2"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.curls(), 3)

    def test_a_persistent_failure_stops_after_three_attempts_and_installs_nothing(self):
        got = self.fetch({"FAKE_CURL_FAIL": "1"})
        self.assertNotEqual(got.returncode, 0)
        self.assertEqual(self.curls(), 3)
        self.assertIn("PIN_REFUSED", got.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_corrupt_download_is_retried_but_never_installed(self):
        got = self.fetch({"FAKE_CURL_CORRUPT_FIRST": "1"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.curls(), 2)
        self.assertEqual(self.dest.read_bytes(), b"payload")

    def test_a_persistently_wrong_download_is_refused_after_three_attempts(self):
        got = self.fetch({"FAKE_CURL_CORRUPT_FIRST": "9"})
        self.assertNotEqual(got.returncode, 0)
        self.assertEqual(self.curls(), 3)
        self.assertFalse(self.dest.exists())

    def test_pin_retry_runs_a_command_at_most_three_times(self):
        got = self.box.run('cnt=0; f() { cnt=$((cnt+1)); echo "try $cnt"; return 7; }; pin_retry f; echo "rc=$?"')
        self.assertEqual(got.stdout.split(), ["try", "1", "try", "2", "try", "3", "rc=7"])

    def tries(self, attempts=None, backoff=None, body="return 7"):
        import time

        prefix = "" if attempts is None else "PIN_RETRY_ATTEMPTS='%s'; " % attempts
        prefix += "" if backoff is None else "PIN_RETRY_BACKOFF='%s'; " % backoff
        started = time.monotonic()
        got = self.box.run(
            prefix + 'cnt=0; f() { cnt=$((cnt+1)); echo try; %s; }; pin_retry f; echo "rc=$?"' % body, timeout=40
        )
        self.elapsed = time.monotonic() - started
        return got.stdout.split().count("try"), got

    def test_non_numeric_attempt_counts_fail_closed_to_three(self):
        for value in ("abc", "3; echo pwned", " ", "2.5"):
            with self.subTest(value):
                count, got = self.tries(value)
                self.assertEqual(count, 3, got.stdout + got.stderr)
                self.assertNotIn("pwned", got.stdout)

    def test_out_of_range_attempt_counts_fail_closed_to_three(self):
        for value in ("0", "-1", "6", "99999"):
            with self.subTest(value):
                count, got = self.tries(value)
                self.assertEqual(count, 3, got.stdout + got.stderr)

    def test_attempt_counts_from_one_to_five_are_honoured(self):
        for value in ("1", "2", "4", "5"):
            with self.subTest(value):
                count, _ = self.tries(value)
                self.assertEqual(count, int(value))

    def test_a_non_numeric_backoff_means_three_seconds(self):
        count, got = self.tries("2", "abc")
        self.assertEqual(count, 2)
        self.assertIn("rc=7", got.stdout)
        self.assertGreaterEqual(self.elapsed, 2.5)

    def test_a_huge_backoff_means_three_seconds(self):
        count, got = self.tries("2", "99999")
        self.assertEqual(count, 2)
        self.assertGreaterEqual(self.elapsed, 2.5)
        self.assertLess(self.elapsed, 30)

    def test_a_callee_cannot_clobber_the_retry_state(self):
        for body in ("n=100; rc=0; attempts=1; waits=x; w=y; return 7", "unset n rc attempts waits w; return 7"):
            with self.subTest(body):
                count, got = self.tries(body=body)
                self.assertEqual(count, 3, got.stdout)
                self.assertIn("rc=7", got.stdout)

    def test_pin_retry_stops_at_the_first_success(self):
        got = self.box.run('cnt=0; f() { cnt=$((cnt+1)); echo "try $cnt"; [ $cnt -ge 2 ]; }; pin_retry f; echo "rc=$?"')
        self.assertEqual(got.stdout.split(), ["try", "1", "try", "2", "rc=0"])

    def test_the_attempts_are_three_in_the_pins(self):
        self.assertRegex(parse_pins().get("PIN_RETRY_ATTEMPTS", ""), r"^3$")

    def test_apt_retries_are_asked_of_apt_itself(self):
        self.assertEqual(LIB.read_text().count("-o Acquire::Retries=3"), 2)
        self.assertEqual((APPS / "setup_side.sh").read_text().count("-o Acquire::Retries=3"), 2)

    def test_the_toolkit_update_and_install_both_retry(self):
        release = self.box.dir / "os-release"
        release.write_text('ID=ubuntu\nVERSION_ID="22.04"\n')
        got = self.box.run(
            "PIN_CUDA_KEYRING_SHA256_ubuntu2204=%s; pin_install_cuda_toolkit" % self.good,
            {"PIN_OS_RELEASE_FILE": str(release), "FAKE_PKG_VERSION": "12.6.3-1"},
        )
        self.assertEqual(got.returncode, 0, got.stderr)
        real = [c for c in self.box.calls() if c.startswith("apt-get ")]
        self.assertEqual(len(real), 2)
        self.assertIn("-o Acquire::Retries=3", real[0])
        self.assertIn("-o Acquire::Retries=3", real[1])


@unittest.skipUnless(GIT, "git is needed")
class GitRetries(unittest.TestCase):
    """A git wrapper fails the first k fetches (a transient origin error); the commit check still decides."""

    def setUp(self):
        self.box = Sandbox(self)
        src = self.box.dir / "src"
        src.mkdir()
        env = {"PATH": os.environ["PATH"], "HOME": str(self.box.dir), "GIT_CONFIG_GLOBAL": "/dev/null",
               "GIT_CONFIG_SYSTEM": "/dev/null"}

        def git(*args):
            return subprocess.run(["git", "-C", str(src), *args], env=env, capture_output=True, text=True,
                                  check=True).stdout.strip()

        git("init", "-q", "-b", "main")
        (src / "f").write_text("x\n")
        git("add", "f")
        git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "c")
        git("config", "uploadpack.allowAnySHA1InWant", "true")
        self.commit = git("rev-parse", "HEAD")
        self.url = "file://" + str(src)
        self.dest = self.box.dir / "checkout"
        self.box.tool("git", 'if [ "$1" = -C ] && [ "$3" = fetch ]; then\n'
                      '  echo "gitfetch" >> "$FAKE_LOG"\n'
                      '  n=$(grep -c "^gitfetch" "$FAKE_LOG")\n'
                      '  [ "$n" -gt "${FAKE_GIT_FAIL_FIRST:-0}" ] || exit 128\nfi\n'
                      'exec %s "$@"\n' % GIT)

    def fetches(self):
        return len([c for c in self.box.calls() if c == "gitfetch"])

    def checkout(self, env):
        return self.box.run('pin_git_checkout "%s" "%s" "" "%s"' % (self.dest, self.url, self.commit), env)

    def test_one_transient_fetch_failure_is_retried(self):
        got = self.checkout({"FAKE_GIT_FAIL_FIRST": "1"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.fetches(), 2)

    def test_two_transient_fetch_failures_are_retried(self):
        got = self.checkout({"FAKE_GIT_FAIL_FIRST": "2"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(self.fetches(), 3)

    def test_a_persistent_fetch_failure_fails_after_three_attempts(self):
        got = self.checkout({"FAKE_GIT_FAIL_FIRST": "99"})
        self.assertNotEqual(got.returncode, 0)
        self.assertEqual(self.fetches(), 3)
        self.assertFalse(self.dest.exists())


class HfSnapshot(unittest.TestCase):
    """The Qwen2-0.5B files at the pinned revision, in the offline cache layout, every file digest-checked."""

    def setUp(self):
        self.box = Sandbox(self)
        self.good = sha(b"payload")
        self.home = self.box.dir / "hf"
        self.pins = parse_pins()
        self.repo = self.home / "hub/models--Qwen--Qwen2-0.5B-Instruct"
        self.snap = self.repo / "snapshots" / self.pins["PIN_QWEN2_REV"]

    def snapshot(self, env=None, digest=None):
        # every pinned file replaced by one the fake curl can serve: same names, the digest of the fake body
        override = "".join(
            'PIN_QWEN2_FILE_%d="${PIN_QWEN2_FILE_%d%%%% *} %s 7"; ' % (n, n, digest or self.good) for n in range(1, 9)
        )
        return self.box.run(override + 'pin_hf_snapshot "%s"' % self.home, env)

    def entries(self):
        return [self.pins["PIN_QWEN2_FILE_%d" % n].split() for n in range(1, 9)]

    def test_every_file_lands_in_the_pinned_snapshot_and_refs_main_points_at_it(self):
        got = self.snapshot()
        self.assertEqual(got.returncode, 0, got.stderr)
        for name, _, _ in self.entries():
            self.assertTrue((self.snap / name).is_file(), name)
        # the exact bytes: huggingface_hub reads the ref without stripping it, so a trailing newline breaks every
        # offline lookup (a real box, 2026-11-11: SETUP_hf_model=FAIL with all eight files present)
        self.assertEqual((self.repo / "refs/main").read_bytes(), self.pins["PIN_QWEN2_REV"].encode())

    def test_the_open_recipe_writes_the_same_refs_main_bytes(self):
        recipe = json.loads((APPS / "image/recipe.json").read_text())
        refs = [f for f in recipe["write_files"] if f["path"].endswith("models--Qwen--Qwen2-0.5B-Instruct/refs/main")]
        self.assertEqual([f["content"] for f in refs], [self.pins["PIN_QWEN2_REV"]])

    def test_a_second_run_keeps_verified_files_without_the_network(self):
        self.assertEqual(self.snapshot().returncode, 0)
        before = len([c for c in self.box.calls() if c.startswith("curl")])
        got = self.snapshot({"FAKE_CURL_FAIL": "1"})
        self.assertEqual(got.returncode, 0, got.stderr)
        self.assertEqual(len([c for c in self.box.calls() if c.startswith("curl")]), before)

    def test_a_tampered_file_is_fetched_again(self):
        self.assertEqual(self.snapshot().returncode, 0)
        (self.snap / "config.json").write_bytes(b"tampered")
        self.assertEqual(self.snapshot().returncode, 0)
        self.assertEqual((self.snap / "config.json").read_bytes(), b"payload")

    def test_a_download_that_is_not_the_pinned_bytes_is_refused_and_leaves_no_refs(self):
        got = self.snapshot(digest=sha(b"other"))
        self.assertNotEqual(got.returncode, 0)
        self.assertIn("PIN_REFUSED", got.stderr)
        self.assertFalse((self.repo / "refs/main").exists())

    def test_an_unreachable_origin_fails_and_leaves_no_refs(self):
        got = self.snapshot({"FAKE_CURL_FAIL": "1"})
        self.assertNotEqual(got.returncode, 0)
        self.assertFalse((self.repo / "refs/main").exists())

    def test_the_urls_name_the_pinned_revision(self):
        self.snapshot()
        urls = [c for c in self.box.calls() if c.startswith("curl")]
        self.assertEqual(len(urls), 8)
        for url in urls:
            self.assertIn("/resolve/" + self.pins["PIN_QWEN2_REV"] + "/", url)


class AgreesWithTheRecipeForQwen2(unittest.TestCase):
    pins = parse_pins()
    recipe = {a["url"].rsplit("/", 1)[1]: a for a in json.loads((APPS / "image/recipe.json").read_text())["artifacts"]
              if "Qwen2-0.5B-Instruct" in a.get("url", "")}

    def test_revision_and_files(self):
        self.assertRegex(self.pins["PIN_QWEN2_REV"], r"^[0-9a-f]{40}$")
        self.assertEqual(len(self.recipe), 8)
        seen = set()
        for n in range(1, 9):
            name, digest, size = self.pins["PIN_QWEN2_FILE_%d" % n].split()
            entry = self.recipe[name]
            seen.add(name)
            self.assertEqual((entry["sha256"], str(entry["size"])), (digest, size), name)
            self.assertEqual(entry["url"], self.pins["PIN_QWEN2_URL_BASE"] + "/" + name)
            self.assertRegex(digest, r"^[0-9a-f]{64}$")
        self.assertEqual(seen, set(self.recipe))
        self.assertIn(self.pins["PIN_QWEN2_REV"], self.pins["PIN_QWEN2_URL_BASE"])


class SetupSideRows(unittest.TestCase):
    """setup_side.sh with a fake venv, curl and apt-get: each row that can fail prints SETUP_<x>=FAIL and the exit is non-zero."""

    PYTHON = (
        '#!/bin/bash\necho "python $*" >> "$FAKE_LOG"\n'
        'case "$*" in\n'
        '  *"import torch"*) [ -z "$FAKE_IMPORT_FAIL" ] && echo "IMPORT_OK 2.6.0 0.21.0 4.46.3 13.3.0" || '
        'echo "ModuleNotFoundError: No module named torch";;\n'
        '  *"from transformers"*) case "$FAKE_HF" in ok) echo HF_MODEL_CACHED;; silent) ;; *) '
        'echo "OSError: cannot load" >&2; exit 1;; esac;;\n'
        'esac\n'
    )
    PIP = '#!/bin/bash\necho "pip $*" >> "$FAKE_LOG"\nn=$(grep -c "^pip " "$FAKE_LOG")\n[ "$n" -gt "${FAKE_PIP_FAIL_FIRST:-0}" ]\n'

    def setUp(self):
        self.box = Sandbox(self)
        d = self.box.dir
        self.apps = d / "optapps"
        (self.apps / "venv/bin").mkdir(parents=True)
        for name, body in (("python", self.PYTHON), ("pip", self.PIP)):
            path = self.apps / "venv/bin" / name
            path.write_text(body)
            path.chmod(0o755)
        (self.apps / "data/blender").mkdir(parents=True)
        blender = self.apps / "data/blender/blender"
        blender.write_text("#!/bin/bash\necho Blender 4.5.0\n")
        blender.chmod(0o755)
        (self.apps / "data/qwen2.5-1.5b-instruct-q4_k_m.gguf").write_bytes(b"payload")
        self.scripts = d / "scripts"
        self.scripts.mkdir()
        for name in ("setup_side.sh", "pin_lib.sh"):
            shutil.copyfile(APPS / name, self.scripts / name)
        pins = (APPS / "pins.sh").read_text()
        pins = re.sub(r"^PIN_GGUF_SHA256=.*$", "PIN_GGUF_SHA256=" + sha(b"payload"), pins, flags=re.M)
        pins = re.sub(r"^PIN_GGUF_SIZE=.*$", "PIN_GGUF_SIZE=7", pins, flags=re.M)
        for n in range(1, 9):
            pins = re.sub(
                r'^PIN_QWEN2_FILE_%d="(\S+) \S+ \S+"$' % n,
                lambda m: 'PIN_QWEN2_FILE_%d="%s %s 7"' % (n, m[1], sha(b"payload")),
                pins, flags=re.M,
            )
        (self.scripts / "pins.sh").write_text(pins)
        self.box.tool("apt-get", 'echo "apt-get $*" >> "$FAKE_LOG"\n')
        self.box.tool("df", "echo df\n")

    def setup(self, env=None):
        base = {"APPS_ROOT": str(self.apps), "FAKE_HF": "ok", "FAKE_IMPORT_FAIL": "", "PIN_RETRY_BACKOFF": "0 0",
                "OPENCL_VENDORS_DIR": str(self.box.dir / "icd")}
        base.update(env or {})
        return subprocess.run(
            ["bash", str(self.scripts / "setup_side.sh")],
            env={"PATH": str(self.box.bin) + ":" + os.environ["PATH"], "HOME": str(self.box.dir),
                 "FAKE_LOG": str(self.box.log), "FAKE_BODY": str(self.box.body), **base},
            capture_output=True, text=True, timeout=60,
        )

    def test_everything_ok_exits_zero_with_every_row_ok(self):
        got = self.setup()
        self.assertEqual(got.returncode, 0, got.stdout + got.stderr)
        for row in ("SETUP_imports=ok", "SETUP_hf_model=ok", "SETUP_gguf=ok", "SETUP_blender=ok", "SETUP_SIDE_DONE"):
            self.assertIn(row, got.stdout)
        self.assertNotIn("=FAIL", got.stdout)

    def test_the_opencl_icd_is_written_and_has_a_row(self):
        got = self.setup()
        self.assertIn("SETUP_icd=ok", got.stdout)
        self.assertEqual((self.box.dir / "icd/nvidia.icd").read_text().strip(), "libnvidia-opencl.so.1")

    def test_an_icd_directory_that_cannot_be_created_is_a_setup_failure(self):
        blocker = self.box.dir / "blocker"
        blocker.write_text("a file, not a directory")
        got = self.setup({"OPENCL_VENDORS_DIR": str(blocker / "vendors")})
        self.assertIn("SETUP_icd=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)

    def test_an_icd_file_that_cannot_be_written_is_a_setup_failure(self):
        directory = self.box.dir / "icd"
        directory.mkdir()
        (directory / "nvidia.icd").mkdir()  # a directory where the file should be
        got = self.setup()
        self.assertIn("SETUP_icd=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)

    def test_every_expected_setup_row_is_printed_ok_on_a_good_run(self):
        got = self.setup()
        rows = [l.strip() for l in (APPS / "expected_rows.txt").read_text().splitlines() if l.startswith("SETUP_")]
        self.assertEqual(len(rows), 8)
        for row in rows:
            self.assertIn(row + "=ok", got.stdout)

    def test_a_failed_import_is_a_setup_failure(self):
        got = self.setup({"FAKE_IMPORT_FAIL": "1"})
        self.assertIn("SETUP_imports=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)

    def test_the_import_row_needs_the_import_ok_line(self):
        got = self.setup({"FAKE_IMPORT_FAIL": "1"})
        self.assertNotIn("SETUP_imports=ok", got.stdout)
        self.assertIn("SETUP_SIDE_DONE", got.stdout)

    def test_a_model_that_cannot_be_loaded_offline_is_a_setup_failure(self):
        got = self.setup({"FAKE_HF": "fail"})
        self.assertIn("SETUP_hf_model=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)

    def test_a_silent_model_load_is_not_a_success(self):
        got = self.setup({"FAKE_HF": "silent"})
        self.assertIn("SETUP_hf_model=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)

    def test_an_unreachable_model_origin_is_a_setup_failure_and_nothing_is_loaded(self):
        got = self.setup({"FAKE_CURL_FAIL": "1"})
        self.assertIn("SETUP_hf_model=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)
        self.assertFalse([c for c in self.box.calls() if "from transformers" in c])

    def test_the_model_is_loaded_only_offline(self):
        self.setup()
        text = (self.scripts / "setup_side.sh").read_text()
        block = text[text.index("pin_hf_snapshot"): text.index("ok hf_model")]
        self.assertIn("HF_HUB_OFFLINE=1", block)
        self.assertIn("TRANSFORMERS_OFFLINE=1", block)

    # the first three pip calls are the (retried) pip upgrade, so 4 and 5 make the torch install itself fail first
    def test_a_transient_pip_failure_is_retried(self):
        got = self.setup({"FAKE_PIP_FAIL_FIRST": "4"})
        self.assertEqual(got.returncode, 0, got.stdout + got.stderr)
        self.assertIn("SETUP_torch=ok", got.stdout)

    def test_two_transient_pip_failures_are_retried(self):
        got = self.setup({"FAKE_PIP_FAIL_FIRST": "5"})
        self.assertEqual(got.returncode, 0, got.stdout + got.stderr)
        self.assertIn("SETUP_torch=ok", got.stdout)

    def test_a_persistent_pip_failure_is_a_setup_failure_after_three_attempts(self):
        got = self.setup({"FAKE_PIP_FAIL_FIRST": "99"})
        self.assertIn("SETUP_torch=FAIL", got.stdout)
        self.assertNotEqual(got.returncode, 0)
        torch = [c for c in self.box.calls() if c.startswith("pip ") and "torch==2.6.0" in c]
        self.assertEqual(len(torch), 3)

    def test_a_wrong_model_file_is_refused_not_cached(self):
        pins = (self.scripts / "pins.sh").read_text().replace(sha(b"payload"), sha(b"other"))
        (self.scripts / "pins.sh").write_text(pins)
        (self.apps / "data/qwen2.5-1.5b-instruct-q4_k_m.gguf").unlink()
        got = self.setup()
        self.assertIn("SETUP_gguf=FAIL", got.stdout)
        self.assertIn("SETUP_hf_model=FAIL", got.stdout)
        self.assertFalse((self.apps / "data/hf/hub/models--Qwen--Qwen2-0.5B-Instruct/refs/main").exists())


class NewRowsAreDeclared(unittest.TestCase):
    def test_every_new_row_exists_in_the_scripts(self):
        guest = (APPS / "setup_side.sh").read_text()
        bundle = (APPS / "build_bundle.sh").read_text()
        for row in ("imports", "hf_model"):
            self.assertRegex(guest, r"\bok %s \$" % row)
        for row in ("cuda_libs", "cuda_headers"):
            self.assertRegex(bundle, r"\bok %s \$" % row)
        self.assertIn('[ "$SETUP_FAILS" -eq 0 ]', guest)


if __name__ == "__main__":
    unittest.main()
