#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The box strips .workerignore's files before it tests (STATUS: LIVE, 2026-11-09).

OWNER_RULINGS section AQ: a test the boxes depend on runs on the tree left after the strip. This is
the static half of that promise: every path a box script names must survive the strip, unless the
mention is prose (a docstring or a message) and says so here. The dynamic half is the stripped-tree
CI job (build, test, clippy on the real stripped tree).

Same notion of "box script" and of "names a path" as ci/vast-broker/test/onstart-sparse.test.js, the
test that guards the box's sparse-checkout list.
Falsifier: a box script reads a file the strip deletes, and this test stays green.
"""

import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import ignore_ladder as L  # noqa: E402


def root():
    r = os.environ.get("KF_LADDER_REPO")
    if r:
        return r
    return subprocess.run(
        ["git", "-C", HERE, "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


ROOTS = "(?:crates|cuda|qemu|scripts|tools|firmware|broker|docs|traces|archive|third_party|ci)"
TOKEN = re.compile(r"(?<![A-Za-z0-9_.-])" + ROOTS + r"/[A-Za-z0-9_./-]*[A-Za-z0-9_]")
BOX_SCRIPT = [
    re.compile(r"^scripts/ci/(vast_[^/]*\.(sh|py)|setup_cache_build\.py|setup-cache\.lock\.json)$"),
    re.compile(
        r"^scripts/(bench/(box/[^/]*\.sh|cuda_ladder\.sh|build_kf3\.sh|v3_gates\.sh|"
        r"provision_[^/]*\.sh|host_xid_watch\.sh)|fastguest/[^/]*\.sh)$"
    ),
]
# (script, named path) -> why the mention is not a read. A stale entry fails the test.
PROSE = {
    (
        "scripts/ci/vast_evidence.py",
        "ci/vast-broker/src/evidence.js",
    ): "docstring: names the Worker module",
    (
        "scripts/ci/vast_payload.py",
        "ci/vast-broker/src/tier.js",
    ): "docstring: names the Worker module",
    (
        "scripts/ci/vast_run_report.py",
        "ci/vast-broker/src/runreport.js",
    ): "docstring: names the Worker module",
}


class BoxPathsSurviveTheStrip(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = root()
        with open(os.path.join(cls.root, ".workerignore"), encoding="utf-8") as f:
            cls.pats = L.read_patterns(f.read(), ".workerignore")
        out = subprocess.run(
            ["git", "-C", cls.root, "ls-files", "-z"], capture_output=True, check=True
        ).stdout
        cls.tracked = [os.fsdecode(x) for x in out.split(b"\0") if x]
        cls.scripts = [
            p
            for p in cls.tracked
            if any(r.match(p) for r in BOX_SCRIPT) and not L.matches(cls.pats, p)
        ]

    def named(self):
        """{(script, token): [resolved tracked paths]} over the code lines of every box script."""
        found = {}
        tracked = set(self.tracked)
        for script in self.scripts:
            with open(os.path.join(self.root, script), encoding="utf-8", errors="replace") as f:
                lines = f.read().split("\n")
            for line in lines:
                if re.match(r"^\s*(#|//|\*|/\*)", line) or re.match(r"^\s*(echo|printf)\b", line):
                    continue
                for token in TOKEN.findall(line):
                    # A script that names a directory does not read its README: prose is exempt.
                    got = [
                        p
                        for p in tracked
                        if (p == token or p.startswith(token + "/")) and not p.endswith(".md")
                    ]
                    if got:
                        found[(script, token)] = got
        return found

    def test_there_are_box_scripts_to_check(self):
        self.assertGreater(len(self.scripts), 30)

    def test_no_box_script_is_itself_stripped(self):
        everything = [p for p in self.tracked if any(r.match(p) for r in BOX_SCRIPT)]
        # The only matches of the glob that the strip removes are harnesses of the box scripts.
        removed = sorted(set(everything) - set(self.scripts))
        self.assertEqual(removed, ["scripts/ci/vast_box_mutants.py"])

    def test_every_named_path_survives_the_strip_or_is_prose(self):
        bad = []
        for (script, token), got in sorted(self.named().items()):
            gone = [p for p in got if L.matches(self.pats, p)]
            if gone and (script, token) not in PROSE:
                bad.append(
                    "%s names %s: %d of %d files are stripped"
                    % (script, token, len(gone), len(got))
                )
        self.assertEqual(bad, [])

    def test_a_prose_exemption_is_still_needed(self):
        stale = []
        named = self.named()
        for key in PROSE:
            got = named.get(key)
            if not got or not any(L.matches(self.pats, p) for p in got):
                stale.append(key)
        self.assertEqual(stale, [], "remove the exemption: the mention is gone or the path is kept")

    def test_the_archived_cuda_source_the_ladder_compiles_lives_beside_cup3(self):
        text = open(
            os.path.join(self.root, "scripts/bench/cuda_ladder.sh"), encoding="utf-8"
        ).read()
        self.assertIn("CUP2_C=$REPO/scripts/bench/cup2.c", text)
        self.assertNotIn("CUP2_C=$REPO/archive", text)
        copy = os.path.join(self.root, "scripts/bench/cup2.c")
        original = os.path.join(self.root, "archive/nvkvm/tests/mode2/cup2.c")
        self.assertTrue(os.path.exists(copy))
        if os.path.exists(original):
            self.assertEqual(open(copy, "rb").read(), open(original, "rb").read())

    def test_the_oracle_and_build_inputs_the_workspace_reads_are_not_stripped(self):
        for path in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "clippy.toml",
            ".gitmodules",
            ".dockerignore",
            "tools/derive_hwref.sh",
            "crates/kf-chip/data/hwref-580.159.04.tsv",
            "third_party/ogkm-580",
            "third_party/ogkm",
            "scripts/ci/fetch_oracles.sh",
            "scripts/ci/ignore_ladder.py",
            "scripts/ci/full_tree_tests.py",
            "scripts/ci/full_tree_tests.tsv",
            "scripts/ci/runner_namespaces.py",
        ):
            with self.subTest(path):
                self.assertTrue(
                    any(p == path or p.startswith(path + "/") for p in self.tracked), path
                )
                self.assertFalse(L.matches(self.pats, path), path)

    def test_the_stripped_ci_job_only_calls_files_that_survive_its_own_strip(self):
        import yaml

        with open(os.path.join(self.root, ".github/workflows/ci.yml"), encoding="utf-8") as f:
            steps = yaml.safe_load(f)["jobs"]["stripped"]["steps"]
        names = [s.get("name", "") for s in steps]
        strip_at = next(i for i, n in enumerate(names) if "strip the checkout" in n)
        token = re.compile(
            r"(?<![A-Za-z0-9_./-])(?:scripts|tools|crates|ci|docs|traces)/[A-Za-z0-9_./-]*[A-Za-z0-9_]"
        )
        tracked = set(self.tracked)
        missing = []
        for i, step in enumerate(steps):
            for line in str(step.get("run", "")).split("\n"):
                # Comments and the text of an echo or an error annotation are prose, not a call.
                if line.lstrip().startswith(("#", '"')) or "echo" in line or "::error" in line:
                    continue
                for t in token.findall(line):
                    if t not in tracked and not any(p.startswith(t + "/") for p in tracked):
                        continue
                    # Before the strip step everything is still there; from it on, only survivors.
                    if i > strip_at and L.matches(self.pats, t):
                        missing.append("step %r names %s, which the strip deletes" % (names[i], t))
        self.assertGreater(len(steps), 8)
        self.assertEqual(missing, [])

    def test_every_include_str_and_include_bytes_target_of_a_workspace_crate_survives(self):
        bad = []
        tracked = set(self.tracked)
        for f in self.tracked:
            if not f.endswith(".rs") or L.matches(self.pats, f) or f.startswith("archive/"):
                continue
            with open(os.path.join(self.root, f), encoding="utf-8", errors="replace") as fh:
                src = fh.read()
            for m in re.finditer(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)', src):
                target = os.path.normpath(os.path.join(os.path.dirname(f), m.group(1)))
                if target in tracked and L.matches(self.pats, target):
                    bad.append("%s includes %s" % (f, target))
        self.assertEqual(bad, [])


if __name__ == "__main__":
    sys.exit(unittest.main())
