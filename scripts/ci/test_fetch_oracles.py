#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/fetch_oracles.sh (STATUS: LIVE, 2026-11-09).

The script pins one commit and one URL in its own text. These tests run a COPY with those two lines
rewritten to a local repository, so no network is used and the real pin is never changed.
Run: python3 -m unittest scripts/ci/test_fetch_oracles.py
Falsifier of the file: a run that exits 0 without the pinned commit checked out, or one that
fetches again when the checkout is already at the pin.
"""

import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.environ.get("KF_FETCH_ORACLES_SCRIPT") or os.path.join(HERE, "fetch_oracles.sh")
HAVE = shutil.which("git") is not None and shutil.which("bash") is not None
GIT = [
    "git",
    "-c",
    "user.email=t@x.invalid",
    "-c",
    "user.name=t",
    "-c",
    "protocol.file.allow=always",
]
HEADER = "src/common/inc/swref/published/nv_ref.h"
RPC = "src/nvidia/src/kernel/vgpu/rpc.c"


def sh(*cmd, cwd=None, env=None):
    return subprocess.run(
        cmd, cwd=cwd, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()


def remote(tmp, files=None):
    """A local 'NVIDIA' repository with one commit; returns (path, commit)."""
    r = os.path.join(tmp, "remote")
    sh("git", "init", "-q", r)
    sh("git", "-C", r, "config", "uploadpack.allowAnySHA1InWant", "true")
    for path, body in (
        files if files is not None else {HEADER: "#define X 1\n", "a/b.c": "c\n"}
    ).items():
        full = os.path.join(r, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(body)
    sh(*GIT, "-C", r, "add", "-A")
    sh(*GIT, "-C", r, "commit", "-q", "-m", "oracle")
    # History behind the pin, so that a shallow fetch is observable.
    for n in range(2):
        with open(os.path.join(r, "history%d.txt" % n), "w") as f:
            f.write("h%d\n" % n)
        sh(*GIT, "-C", r, "add", "-A")
        sh(*GIT, "-C", r, "commit", "-q", "-m", "later %d" % n)
    return r, sh("git", "-C", r, "rev-parse", "HEAD")


def superproject(tmp, pin, url, gitlink=None):
    """The 'kayfabe' checkout whose commit pins third_party/ogkm-580, and a script copy for it."""
    root = os.path.join(tmp, "super")
    sh("git", "init", "-q", root)
    with open(os.path.join(root, "README"), "w") as f:
        f.write("x")
    sh(*GIT, "-C", root, "add", "-A")
    if gitlink != "none":
        sh(
            *GIT,
            "-C",
            root,
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,%s,third_party/ogkm-580" % (gitlink or pin),
        )
    sh(*GIT, "-C", root, "commit", "-q", "-m", "s")
    text = open(SCRIPT).read()
    text, n1 = re.subn(r"(?m)^PIN=[0-9a-f]{40}$", "PIN=" + pin, text)
    text, n2 = re.subn(r"(?m)^URL=.*$", "URL=" + url, text)
    text, n3 = re.subn(r"(?m)^PIN_595=[0-9a-f]{40}$", "PIN_595=" + pin, text)
    assert n3 == 1, "the script no longer has one PIN_595= line"
    assert n1 == 1 and n2 == 1, "the script no longer has one PIN= and one URL= line"
    copy = os.path.join(tmp, "fetch_oracles_copy.sh")
    with open(copy, "w") as f:
        f.write(text)
    os.chmod(copy, os.stat(copy).st_mode | stat.S_IEXEC)
    os.makedirs(os.path.join(root, "third_party", "ogkm-580"), exist_ok=True)
    return root, copy


def run(copy, root, env=None, path_prefix=None, trees=None):
    e = {**os.environ, "KF_ORACLE_RETRY_SLEEP": "0", **(env or {})}
    if path_prefix:
        e["PATH"] = path_prefix + os.pathsep + e["PATH"]
    return subprocess.run(
        ["bash", copy, root, *(trees or ["ogkm-580"])], capture_output=True, text=True, env=e
    )


def git_wrapper(tmp, log, fail_first_fetches=0):
    """A `git` in front of PATH that logs every fetch and fails the first N of them."""
    real = shutil.which("git")
    d = os.path.join(tmp, "wrap")
    os.makedirs(d)
    counter = os.path.join(tmp, "count")
    with open(counter, "w") as f:
        f.write("0")
    body = f"""#!/bin/bash
for a in "$@"; do
  if [ "$a" = fetch ]; then
    echo fetch >> {log}
    n=$(cat {counter}); echo $((n + 1)) > {counter}
    if [ "$n" -lt {fail_first_fetches} ]; then exit 1; fi
  fi
done
exec {real} "$@"
"""
    with open(os.path.join(d, "git"), "w") as f:
        f.write(body)
    os.chmod(os.path.join(d, "git"), 0o755)
    return d


@unittest.skipUnless(HAVE, "needs git and bash")
class FetchOracles(unittest.TestCase):
    def setup(self, tmp, files=None, gitlink=None):
        rem, pin = remote(tmp, files)
        root, copy = superproject(tmp, pin, rem, gitlink)
        return rem, pin, root, copy

    def test_fetches_the_pinned_commit_shallow_and_reports_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, pin, root, copy = self.setup(tmp)
            p = run(copy, root)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn("ORACLE_FETCH_OK ogkm-580 sha=" + pin, p.stdout)
            dest = os.path.join(root, "third_party", "ogkm-580")
            self.assertEqual(sh("git", "-C", dest, "rev-parse", "HEAD"), pin)
            self.assertTrue(os.path.isfile(os.path.join(dest, HEADER)))
            self.assertEqual(sh("git", "-C", dest, "rev-list", "--count", "HEAD"), "1")

    def test_the_checkout_is_marked_shallow(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            self.assertEqual(run(copy, root).returncode, 0)
            dest = os.path.join(root, "third_party", "ogkm-580")
            self.assertTrue(os.path.isfile(os.path.join(dest, ".git", "shallow")))

    def test_the_595_tree_lands_in_third_party_ogkm_595_without_a_gitlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, pin, root, copy = self.setup(tmp, files={RPC: "int rpc;\n"}, gitlink="none")
            p = run(copy, root, trees=["ogkm-595"])
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn("ORACLE_FETCH_OK ogkm-595 sha=" + pin, p.stdout)
            self.assertTrue(os.path.isfile(os.path.join(root, "third_party", "ogkm-595", RPC)))

    def test_the_595_tree_without_its_marker_file_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp, files={HEADER: "x\n"}, gitlink="none")
            p = run(copy, root, trees=["ogkm-595"])
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertIn("rpc.c", p.stderr)

    def test_an_unknown_tree_name_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            p = run(copy, root, trees=["ogkm-999"])
            self.assertEqual(p.returncode, 2, p.stdout)
            self.assertIn("unknown tree", p.stderr)

    def test_the_report_numbers_are_positive_integers(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            line = [
                x for x in run(copy, root).stdout.splitlines() if x.startswith("ORACLE_FETCH_OK")
            ][0]
            fields = dict(kv.split("=") for kv in line.split()[2:])
            for key in ("files", "worktree_bytes", "git_bytes"):
                self.assertGreater(int(fields[key]), 0, key)
            self.assertGreaterEqual(int(fields["seconds"]), 0)

    def test_the_report_carries_size_and_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            out = run(copy, root).stdout
            for key in ("files=", "worktree_bytes=", "git_bytes=", "seconds="):
                self.assertIn(key, out)

    def test_a_second_run_does_not_fetch_again(self):
        with tempfile.TemporaryDirectory() as tmp:
            rem, _pin, root, copy = self.setup(tmp)
            self.assertEqual(run(copy, root).returncode, 0)
            shutil.rmtree(rem)
            log = os.path.join(tmp, "fetches")
            wrap = git_wrapper(tmp, log)
            p = run(copy, root, path_prefix=wrap)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertFalse(os.path.exists(log), "the second run fetched")

    def test_a_second_run_works_with_the_remote_gone(self):
        with tempfile.TemporaryDirectory() as tmp:
            rem, pin, root, copy = self.setup(tmp)
            run(copy, root)
            shutil.rmtree(rem)
            p = run(copy, root)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn(pin, p.stdout)

    def test_a_gitlink_that_differs_from_the_pin_fails_without_fetching(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp, gitlink="1" * 40)
            log = os.path.join(tmp, "fetches")
            p = run(copy, root, path_prefix=git_wrapper(tmp, log))
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertIn("move both together", p.stderr)
            self.assertFalse(os.path.exists(log))
            self.assertEqual(os.listdir(os.path.join(root, "third_party", "ogkm-580")), [])

    def test_a_commit_without_the_gitlink_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp, gitlink="none")
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertIn("nothing", p.stderr)

    def test_an_unreachable_remote_fails_after_three_attempts(self):
        with tempfile.TemporaryDirectory() as tmp:
            rem, pin, root, copy = self.setup(tmp)
            shutil.rmtree(rem)
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertEqual(p.stderr.count("ORACLE_FETCH_RETRY"), 3)
            self.assertIn("ORACLE_FETCH_FAIL", p.stderr)
            self.assertNotIn("ORACLE_FETCH_OK", p.stdout)

    def test_a_remote_that_lacks_the_pinned_commit_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            rem, _pin = remote(tmp)
            root2, copy2 = superproject(tmp, "2" * 40, rem)
            p = run(copy2, root2)
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertNotIn("ORACLE_FETCH_OK", p.stdout)

    def test_a_flaky_remote_is_retried(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, pin, root, copy = self.setup(tmp)
            log = os.path.join(tmp, "fetches")
            p = run(copy, root, path_prefix=git_wrapper(tmp, log, fail_first_fetches=2))
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(p.stderr.count("ORACLE_FETCH_RETRY"), 2)
            self.assertIn(pin, p.stdout)

    def test_a_checkout_without_the_reference_header_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp, files={"other.c": "x\n"})
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertIn("nv_ref.h", p.stderr)

    def test_an_empty_reference_header_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp, files={HEADER: ""})
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)

    def test_a_directory_holding_unrelated_files_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            with open(os.path.join(root, "third_party", "ogkm-580", "stray.txt"), "w") as f:
                f.write("x")
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)
            self.assertIn("not empty", p.stderr)
            self.assertEqual(
                os.listdir(os.path.join(root, "third_party", "ogkm-580")), ["stray.txt"]
            )

    def test_a_directory_holding_a_subdirectory_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            _rem, _pin, root, copy = self.setup(tmp)
            os.makedirs(os.path.join(root, "third_party", "ogkm-580", "sub"))
            p = run(copy, root)
            self.assertEqual(p.returncode, 1, p.stdout)

    def test_the_real_script_pins_the_commit_of_the_gitlink(self):
        text = open(SCRIPT).read()
        pin = re.search(r"(?m)^PIN=([0-9a-f]{40})$", text).group(1)
        url = re.search(r"(?m)^URL=(.*)$", text).group(1)
        self.assertEqual(url, "https://github.com/NVIDIA/open-gpu-kernel-modules")
        try:
            root = os.environ.get("KF_LADDER_REPO") or sh(
                "git", "-C", HERE, "rev-parse", "--show-toplevel"
            )
            linked = sh("git", "-C", root, "rev-parse", "HEAD:third_party/ogkm-580")
        except subprocess.CalledProcessError:
            self.skipTest("not inside a kayfabe checkout")
        self.assertEqual(linked, pin)

    def test_the_real_script_has_no_environment_override_of_the_pin_or_the_url(self):
        text = open(SCRIPT).read()
        self.assertRegex(text, r"(?m)^PIN=[0-9a-f]{40}$")
        self.assertRegex(text, r"(?m)^PIN_595=[0-9a-f]{40}$")
        self.assertRegex(text, r"(?m)^URL=https://")
        self.assertNotRegex(text, r"\$\{KF_[A-Z_]*(PIN|URL)")


if __name__ == "__main__":
    sys.exit(unittest.main())
