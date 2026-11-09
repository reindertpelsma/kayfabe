#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/ignore_ladder.py (PROPOSAL, 2026-11-05).

Run: python3 -m unittest scripts/ci/test_ignore_ladder.py
Falsifier of the whole file: a pattern line inside the restricted syntax on which git, docker
semantics and the matcher disagree, or a hash that moves when only an ignored file changes.
"""

import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import ignore_ladder as L  # noqa: E402

HAVE_GIT = shutil.which("git") is not None


def real_root():
    """The checkout under test. The mutation harness copies the files elsewhere and says where."""
    return (
        os.environ.get("KF_LADDER_REPO")
        or sh("git", "-C", HERE, "rev-parse", "--show-toplevel").strip()
    )


def sh(*cmd, cwd=None, inp=None):
    return subprocess.run(
        cmd, cwd=cwd, input=inp, check=True, capture_output=True, text=True
    ).stdout


def make_repo(tmp, files, ignore_files=None):
    """A git repo with `files` {path: text|bytes} committed; returns (root, commit sha)."""
    root = os.path.join(tmp, "repo")
    sh("git", "init", "-q", root)
    sh("git", "-C", root, "config", "user.email", "t@example.invalid")
    sh("git", "-C", root, "config", "user.name", "t")
    for path, body in {**files, **(ignore_files or {})}.items():
        full = os.path.join(root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as f:
            f.write(body if isinstance(body, bytes) else body.encode())
    sh("git", "-C", root, "add", "-A", "-f")
    sh("git", "-C", root, "commit", "-q", "-m", "c")
    return root, sh("git", "-C", root, "rev-parse", "HEAD").strip()


class Syntax(unittest.TestCase):
    GOOD = [
        "/target",
        "/target-*",
        "/firmware/**/*.efi",
        "**/*.md",
        "**/__pycache__",
        "/docs/design/*.pdf",
        "/scripts/ci/test_*.py",
        "/a.b/c+d@e=f,g~h",
    ]
    BAD = {
        "!/keep": "negation",
        "foo": "bare name",
        "*.md": "bare glob",
        "docs/x": "no leading slash",
        "/dir/": "trailing slash",
        "/a/**": "trailing **",
        "/a**b": "** inside a segment",
        "/a?b": "question mark",
        "/a[bc]": "bracket",
        "/a\\ b": "backslash",
        "/a b": "space",
        "/a  ": "trailing whitespace",
        "//a": "empty segment",
        "/a//b": "empty segment",
        "/a/./b": "dot segment",
        "/../a": "dotdot",
        "/é": "non-ascii",
        "**/": "trailing slash",
    }

    def test_good_lines_pass(self):
        for line in self.GOOD:
            self.assertEqual(L.lint_line(line), line, line)

    def test_blank_and_comment_are_not_patterns(self):
        self.assertIsNone(L.lint_line(""))
        self.assertIsNone(L.lint_line("# anything !goes [here]"))

    def test_bad_lines_fail(self):
        for line, why in self.BAD.items():
            with self.assertRaises(L.SyntaxProblem, msg="%r (%s)" % (line, why)):
                L.lint_line(line)

    def test_error_message_names_the_rule(self):
        want = {
            "!/keep": "negation",
            "foo": "must start with",
            "/dir/": "trailing /",
            "/a/**": "last segment",
            "/a**b": "whole segment",
            "/a?b": "outside",
            "/a ": "trailing whitespace",
        }
        for line, word in want.items():
            with self.assertRaisesRegex(L.SyntaxProblem, word, msg=line):
                L.lint_line(line)

    def test_read_patterns_reports_the_rule_for_each_bad_line(self):
        for line, word in (
            ("/a/**", "last segment"),
            ("/dir/", "trailing /"),
            ("bare", "must start with"),
            ("!/n", "negation"),
        ):
            with self.assertRaisesRegex(L.SyntaxProblem, word):
                L.read_patterns("/ok\n" + line + "\n", "f")

    def test_comment_must_start_the_line(self):
        with self.assertRaises(L.SyntaxProblem):
            L.lint_line("/a # trailing comment")

    def test_read_patterns_names_the_line(self):
        with self.assertRaises(L.SyntaxProblem) as cm:
            L.read_patterns("/ok\n# c\n\n!/no\n", "f")
        self.assertIn("f:4:", str(cm.exception))

    def test_read_patterns_returns_only_patterns(self):
        self.assertEqual(L.read_patterns("# c\n/a\n\n**/b\n"), ["/a", "**/b"])


class Matcher(unittest.TestCase):
    def m(self, pat, path):
        return L.matches([pat], path)

    def test_anchored_does_not_float(self):
        self.assertTrue(self.m("/target", "target"))
        self.assertTrue(self.m("/target", "target/debug/x"))
        self.assertFalse(self.m("/target", "crates/target"))
        self.assertFalse(self.m("/target", "targets"))

    def test_floating_matches_every_depth_and_root(self):
        self.assertTrue(self.m("**/*.md", "README.md"))
        self.assertTrue(self.m("**/*.md", "a/b/c.md"))
        self.assertFalse(self.m("**/*.md", "a/b/c.mdx"))

    def test_star_does_not_cross_slash(self):
        self.assertTrue(self.m("/docs/*.pdf", "docs/a.pdf"))
        self.assertFalse(self.m("/docs/*.pdf", "docs/x/a.pdf"))

    def test_middle_doublestar_matches_zero_or_more_dirs(self):
        self.assertTrue(self.m("/f/**/*.efi", "f/a.efi"))
        self.assertTrue(self.m("/f/**/*.efi", "f/a/b/c.efi"))
        self.assertFalse(self.m("/f/**/*.efi", "g/a.efi"))

    def test_parent_directory_match_covers_children(self):
        self.assertTrue(self.m("/archive", "archive/a/b/c.rs"))
        self.assertTrue(self.m("**/__pycache__", "x/__pycache__/m.pyc"))

    def test_docker_semantics_table(self):
        d = L.docker_matches
        self.assertTrue(d(["/docs/*.pdf"], "docs/a.pdf"))
        self.assertFalse(d(["/docs/*.pdf"], "docs/x/a.pdf"))  # `*` stops at /
        self.assertTrue(d(["**/*.md"], "top.md"))  # `**/` also matches zero directories
        self.assertTrue(d(["/f/**/*.efi"], "f/a.efi"))
        self.assertTrue(d(["/archive"], "archive/x/y"))
        self.assertFalse(d(["/target"], "crates/target"))

    def test_dotfiles_are_matched_by_star(self):
        self.assertTrue(self.m("/a/*", "a/.hidden"))


@unittest.skipUnless(HAVE_GIT, "needs git")
class AgainstGit(unittest.TestCase):
    """Random patterns from the restricted grammar against the paths of a random tree: the matcher
    and the docker semantics must give git's answer (git check-ignore --no-index)."""

    NAMES = ["a", "b", "c.md", "d.txt", "x.md", "tests", "__pycache__", ".git", "t1", "t2.rs"]

    def rand_path(self, rng):
        return "/".join(rng.choice(self.NAMES) for _ in range(rng.randint(1, 4)))

    def rand_pattern(self, rng):
        segs = []
        for _ in range(rng.randint(1, 3)):
            segs.append(
                rng.choice(
                    ["a", "b", "tests", "*.md", "*", "t*", "*.rs", "d.txt", "__pycache__", "**"]
                )
            )
        while segs and segs[-1] == "**":
            segs.pop()
        if not segs:
            segs = ["a"]
        head = rng.choice(["/", "**/"])
        if segs[0] == "**" and head == "**/":
            segs = segs[1:] or ["a"]
        return head + "/".join(segs)

    def test_random_agreement(self):
        rng = random.Random(20261105)
        with tempfile.TemporaryDirectory() as tmp:
            for case in range(25):
                pats = []
                while len(pats) < rng.randint(1, 4):
                    p = self.rand_pattern(rng)
                    try:
                        L.lint_line(p)
                    except L.SyntaxProblem:
                        continue
                    pats.append(p)
                paths = sorted({self.rand_path(rng) for _ in range(60)})
                f = os.path.join(tmp, "ign%d" % case)
                with open(f, "w") as fh:
                    fh.write("\n".join(pats) + "\n")
                gi = L.git_ignored(paths, f, os.path.join(tmp, "s%d" % case))
                ours = {p for p in paths if L.matches(pats, p)}
                dock = {p for p in paths if L.docker_matches(pats, p)}
                self.assertEqual(ours, gi, "matcher vs git, patterns %r" % pats)
                self.assertEqual(dock, gi, "docker semantics vs git, patterns %r" % pats)

    def test_bare_name_is_where_git_and_docker_part(self):
        # The reason bare names are outside the syntax: git floats `foo`, docker anchors it.
        with tempfile.TemporaryDirectory() as tmp:
            f = os.path.join(tmp, "ign")
            with open(f, "w") as fh:
                fh.write("foo\n")
            gi = L.git_ignored(["a/foo", "foo"], f, os.path.join(tmp, "s"))
            self.assertEqual(gi, {"a/foo", "foo"})
            self.assertFalse(L.docker_matches(["foo"], "a/foo"))  # docker: root only


@unittest.skipUnless(HAVE_GIT, "needs git")
class Hash(unittest.TestCase):
    FILES = {"src/a.rs": "fn a(){}", "docs/x.md": "x", "tests/t.rs": "t", "run.sh": "echo"}
    PATS = ["/docs", "**/*.md"]

    def digest(self, root, rev, pats=None):
        return L.filtered_hash(L.tracked(rev, cwd=root), pats or self.PATS)

    def test_change_of_ignored_file_keeps_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            with open(os.path.join(root, "docs/x.md"), "w") as f:
                f.write("changed")
            sh("git", "-C", root, "commit", "-qam", "docs only")
            r2 = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            self.assertNotEqual(r1, r2)
            self.assertEqual(self.digest(root, r1), self.digest(root, r2))

    def test_change_of_unignored_file_moves_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            with open(os.path.join(root, "src/a.rs"), "w") as f:
                f.write("fn a(){1}")
            sh("git", "-C", root, "commit", "-qam", "code")
            r2 = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            self.assertNotEqual(self.digest(root, r1)[0], self.digest(root, r2)[0])

    def test_mode_change_moves_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            sh("git", "-C", root, "update-index", "--chmod=+x", "run.sh")
            sh("git", "-C", root, "commit", "-qm", "mode")
            r2 = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            self.assertNotEqual(self.digest(root, r1)[0], self.digest(root, r2)[0])

    def test_rename_moves_hash_and_new_file_moves_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            sh("git", "-C", root, "mv", "src/a.rs", "src/b.rs")
            sh("git", "-C", root, "commit", "-qm", "mv")
            r2 = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            self.assertNotEqual(self.digest(root, r1)[0], self.digest(root, r2)[0])

    def test_count_and_known_value(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            h, n = self.digest(root, r1)
            self.assertEqual(n, 3)  # src/a.rs, tests/t.rs, run.sh
            ents = {p: (m, s) for m, s, p in L.tracked(r1, cwd=root)}
            import hashlib

            want = hashlib.sha256()
            for p in sorted(["run.sh", "src/a.rs", "tests/t.rs"]):
                want.update(
                    p.encode() + b"\0" + ents[p][0].encode() + b"\0" + ents[p][1].encode() + b"\n"
                )
            self.assertEqual(h, want.hexdigest())

    def test_hash_does_not_depend_on_entry_order(self):
        ents = [
            ("100644", "a" * 40, "z.rs"),
            ("100644", "b" * 40, "a/b"),
            ("100755", "c" * 40, "a-b"),
        ]
        h1 = L.filtered_hash(ents, [])
        h2 = L.filtered_hash(list(reversed(ents)), [])
        self.assertEqual(h1, h2)

    def test_hash_orders_paths_by_bytes(self):
        import hashlib

        ents = [
            ("100644", "a" * 40, "a/b"),
            ("100644", "b" * 40, "a-b"),
            ("100644", "c" * 40, "a.b"),
        ]
        want = hashlib.sha256()
        for m, sha, p in (ents[1], ents[2], ents[0]):  # '-' 0x2d < '.' 0x2e < '/' 0x2f
            want.update(p.encode() + b"\0" + m.encode() + b"\0" + sha.encode() + b"\n")
        self.assertEqual(L.filtered_hash(ents, [])[0], want.hexdigest())

    def test_ignore_file_choice_changes_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            self.assertNotEqual(self.digest(root, r1)[0], self.digest(root, r1, ["/tests"])[0])

    def test_index_and_rev_agree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES)
            self.assertEqual(
                L.filtered_hash(L.tracked(None, cwd=root), self.PATS),
                L.filtered_hash(L.tracked(r1, cwd=root), self.PATS),
            )

    def test_chain_reports_fall_through(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.FILES, {".workerignore": "\n".join(self.PATS) + "\n"})
            for i, (path, body) in enumerate(
                [("docs/x.md", "1"), ("src/a.rs", "fn b(){}"), ("docs/y.md", "2")]
            ):
                with open(os.path.join(root, path), "w") as f:
                    f.write(body)
                sh("git", "-C", root, "add", "-A")
                sh("git", "-C", root, "commit", "-qm", "c%d" % i)
            out = subprocess.run(
                [
                    sys.executable,
                    os.path.join(HERE, "ignore_ladder.py"),
                    "--root",
                    root,
                    "chain",
                    "HEAD",
                    "3",
                ],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.splitlines()
            self.assertEqual(
                [x.split()[-1] for x in out[:3]], ["FALLS_THROUGH", "new", "FALLS_THROUGH"]
            )
            self.assertEqual(out[3], "CHAIN commits=3 fell_through=2")


@unittest.skipUnless(HAVE_GIT, "needs git")
class Ladder(unittest.TestCase):
    FILES = {"src/a.rs": "x", "docs/x.md": "x"}
    GI = "# c\n/target\n"
    WI = "/target\n/.git\n**/.gitignore\n/.workerignore\n**/*.md\n"
    DI = "/target\n/.git\n**/.gitignore\n/.workerignore\n**/*.md\n/.dockerignore\n/src\n"

    def problems(self, gi=None, wi=None, di=None, files=None):
        with tempfile.TemporaryDirectory() as tmp:
            root, _ = make_repo(
                tmp,
                files or self.FILES,
                {
                    ".gitignore": self.GI if gi is None else gi,
                    ".workerignore": self.WI if wi is None else wi,
                    ".dockerignore": self.DI if di is None else di,
                },
            )
            with open(os.devnull, "w") as dn:
                old, sys.stdout = sys.stdout, dn
                try:
                    return L.check(root)
                finally:
                    sys.stdout = old

    def test_good_ladder_is_clean(self):
        self.assertEqual(self.problems(), [])

    def test_line_missing_from_worker_is_named(self):
        p = self.problems(gi="/target\n/extra\n")
        self.assertTrue(any("ladder" in x and "/extra" in x for x in p), p)

    def test_line_missing_from_docker_is_named(self):
        p = self.problems(di="/target\n/.git\n**/.gitignore\n/.workerignore\n/.dockerignore\n")
        self.assertTrue(any("ladder" in x and "**/*.md" in x for x in p), p)

    def test_the_lint_reports_what_the_strip_would_refuse_a_floor_violation(self):
        extra = "/crates/kf-a/tests\n"
        p = self.problems(
            wi=self.WI + extra,
            di=self.DI + extra,
            files={"src/a.rs": "x", "crates/kf-a/tests/t.rs": "x", "crates/kf-a/src/lib.rs": "x"},
        )
        self.assertTrue(any(x.startswith("strip:") and "t.rs" in x for x in p), p)

    def test_the_lint_reports_what_the_strip_would_refuse_nothing_left(self):
        p = self.problems(
            wi=self.WI + "/.dockerignore\n", files={"docs/x.md": "x", "docs/y.md": "y"}
        )
        self.assertTrue(any(x.startswith("strip:") and "nothing would be left" in x for x in p), p)

    def test_the_lint_stays_clean_for_a_tree_the_strip_accepts(self):
        self.assertEqual(
            self.problems(files={"src/a.rs": "x", "crates/kf-a/tests/t.rs": "x", "docs/x.md": "x"}),
            [],
        )

    def test_worker_must_ignore_itself_and_git(self):
        p = self.problems(wi="/target\n**/*.md\n")
        self.assertTrue(any("self-ignore" in x for x in p), p)

    def test_docker_must_ignore_itself(self):
        p = self.problems(di=self.WI)
        self.assertTrue(any("self-ignore" in x and ".dockerignore" in x for x in p), p)

    def test_negation_is_a_syntax_problem(self):
        p = self.problems(wi=self.WI + "!/src/keep\n")
        self.assertTrue(any(x.startswith("syntax") for x in p), p)

    def test_tracked_file_matching_gitignore_is_reported(self):
        p = self.problems(files={**self.FILES, "target/x": "t"}, gi="/target\n")
        self.assertTrue(any("match .gitignore" in x for x in p), p)

    def test_missing_file_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, _ = make_repo(tmp, self.FILES, {".gitignore": self.GI})
            self.assertTrue(any("missing" in x for x in L.check(root)))

    def test_check_notices_a_matcher_that_disagrees_with_git(self):
        real = L.matches
        L.matches = lambda pats, path: real(pats, path) and not path.startswith("docs/")
        try:
            p = self.problems()
        finally:
            L.matches = real
        self.assertTrue(any("matcher differs from git" in x for x in p), p)

    def test_check_notices_docker_semantics_that_disagree_with_git(self):
        real = L.docker_matches
        L.docker_matches = lambda pats, path: False
        try:
            p = self.problems()
        finally:
            L.docker_matches = real
        self.assertTrue(any("docker-semantics differs from git" in x for x in p), p)

    def test_sparse_list_inverts_the_patterns(self):
        self.assertEqual(L.sparse_list(["/a", "**/*.md"]), ["/*", "!/a", "!**/*.md"])


@unittest.skipUnless(HAVE_GIT, "needs git")
class OwnIgnore(unittest.TestCase):
    """--own-ignore: the key uses the commit's own ignore file (V3_CI_TIERS_AND_IGNORE_LADDER.md 3)."""

    BASE = {"src/a.rs": "a", "docs/x.md": "x", "gen/g.txt": "g"}

    def cli(self, root, *args):
        return subprocess.run(
            [
                sys.executable,
                os.path.join(HERE, "ignore_ladder.py"),
                "--root",
                root,
                "--own-ignore",
                *args,
            ],
            capture_output=True,
            text=True,
        )

    def commit(self, root, files):
        for path, body in files.items():
            full = os.path.join(root, path)
            os.makedirs(os.path.dirname(full) or root, exist_ok=True)
            with open(full, "w") as f:
                f.write(body)
        sh("git", "-C", root, "add", "-A", "-f")
        sh("git", "-C", root, "commit", "-q", "-m", "c")
        return sh("git", "-C", root, "rev-parse", "HEAD").strip()

    def digest(self, root, rev):
        p = self.cli(root, "hash", rev)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        return p.stdout.split()[0]

    def test_newly_ignoring_existing_code_moves_the_key(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.BASE, {".workerignore": "**/*.md\n/.workerignore\n"})
            r2 = self.commit(root, {".workerignore": "**/*.md\n/.workerignore\n/src\n"})
            self.assertNotEqual(self.digest(root, r1), self.digest(root, r2))

    def test_new_file_with_its_own_exclusion_falls_through(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.BASE, {".workerignore": "**/*.md\n/.workerignore\n"})
            r2 = self.commit(
                root, {"scratch/n.txt": "n", ".workerignore": "**/*.md\n/.workerignore\n/scratch\n"}
            )
            self.assertEqual(self.digest(root, r1), self.digest(root, r2))

    def test_editing_only_the_ignore_file_without_effect_falls_through(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.BASE, {".workerignore": "**/*.md\n/.workerignore\n"})
            r2 = self.commit(
                root, {".workerignore": "# comment\n**/*.md\n/.workerignore\n/nothing-here\n"}
            )
            self.assertEqual(self.digest(root, r1), self.digest(root, r2))

    def test_missing_ignore_file_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.BASE)
            p = self.cli(root, "hash", r1)
            self.assertEqual(p.returncode, 3)
            self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)

    def test_unparsable_ignore_file_fails_closed(self):
        for bad in ("!/src/keep\n", "docs/x\n", "/a[bc]\n"):
            with tempfile.TemporaryDirectory() as tmp:
                root, r1 = make_repo(tmp, self.BASE, {".workerignore": bad})
                p = self.cli(root, "hash", r1)
                self.assertEqual(p.returncode, 3, bad)
                self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)

    def test_without_the_flag_the_working_file_is_used(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, r1 = make_repo(tmp, self.BASE)  # no ignore file in the tree
            p = subprocess.run(
                [
                    sys.executable,
                    os.path.join(HERE, "ignore_ladder.py"),
                    "--root",
                    root,
                    "hash",
                    r1,
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(
                p.returncode, 0
            )  # no .workerignore on disk either: an error, not a hash


HAVE_NODE = shutil.which("node") is not None


def run_cli(root, *args):
    return subprocess.run(
        [sys.executable, os.path.join(HERE, "ignore_ladder.py"), "--root", root, *args],
        capture_output=True,
        text=True,
    )


def tree_files(top):
    out = set()
    for d, dirs, files in os.walk(top):
        dirs[:] = [x for x in dirs if x != ".git"]
        for f in files:
            out.add(os.path.relpath(os.path.join(d, f), top))
    return out


STRIP_FILES = {
    "Cargo.toml": "[workspace]\n",
    "crates/kf-a/src/lib.rs": "pub fn a() {}\n",
    "crates/kf-a/tests/t.rs": "#[test] fn t() {}\n",
    "crates/kf-a/README.md": "prose\n",
    "docs/guide.txt": "doc\n",
    "docs/deep/er/x.txt": "doc\n",
    "traces/run/out.log": "log\n",
    "scripts/ci/test_x.py": "t\n",
    "scripts/ci/vast_payload.py": "p\n",
    "LICENSE": "l\n",
}
STRIP_IGNORE = "/.git\n**/.gitignore\n/.workerignore\n**/*.md\n/docs\n/traces\n/scripts/ci/test_*.py\n/LICENSE\n"
STRIP_KEPT = {
    "Cargo.toml",
    "crates/kf-a/src/lib.rs",
    "crates/kf-a/tests/t.rs",
    "scripts/ci/vast_payload.py",
}


@unittest.skipUnless(HAVE_GIT, "needs git")
class StripStep(unittest.TestCase):
    """The box strip step and the stripped-tree CI job delete what the commit's own .workerignore matches."""

    def repo(self, tmp, files=None, ignore=STRIP_IGNORE):
        return make_repo(tmp, files or STRIP_FILES, {".workerignore": ignore})[0]

    def test_export_keeps_exactly_the_unignored_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            dest = os.path.join(tmp, "out")
            p = run_cli(root, "strip", "--dest", dest)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(tree_files(dest), STRIP_KEPT)
            self.assertIn("IGNORE_STRIP kept=4 removed=7", p.stdout)

    def test_in_place_removes_exactly_the_ignored_files_and_never_touches_git(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(tree_files(root), STRIP_KEPT)
            self.assertTrue(os.path.isfile(os.path.join(root, ".git", "HEAD")))
            self.assertEqual(sh("git", "-C", root, "rev-parse", "HEAD").strip()[:2].isalnum(), True)

    def test_in_place_and_export_agree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            dest = os.path.join(tmp, "out")
            a = run_cli(root, "strip", "--dest", dest)
            b = run_cli(root, "strip", "--in-place")
            self.assertEqual(tree_files(dest), tree_files(root))
            self.assertEqual(
                [x for x in a.stdout.splitlines() if x.startswith("IGNORE_STRIP_KEY")],
                [x for x in b.stdout.splitlines() if x.startswith("IGNORE_STRIP_KEY")],
            )

    def test_in_place_prunes_emptied_directories_and_keeps_untracked_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            os.makedirs(os.path.join(root, "target"))
            with open(os.path.join(root, "target", "build.o"), "w") as f:
                f.write("x")
            run_cli(root, "strip", "--in-place")
            self.assertFalse(os.path.exists(os.path.join(root, "docs")))
            self.assertFalse(os.path.exists(os.path.join(root, "traces")))
            self.assertTrue(os.path.exists(os.path.join(root, "target", "build.o")))

    def test_in_place_is_idempotent_and_reads_the_commit_not_the_worktree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            self.assertEqual(run_cli(root, "strip", "--in-place").returncode, 0)
            # .workerignore itself is gone from the worktree now; a second run still reads HEAD's.
            self.assertFalse(os.path.exists(os.path.join(root, ".workerignore")))
            self.assertEqual(run_cli(root, "strip", "--in-place").returncode, 0)
            self.assertEqual(tree_files(root), STRIP_KEPT)

    def test_an_uncommitted_edit_of_the_ignore_file_does_not_change_what_is_stripped(self):
        for mode in ("--in-place", "--dest"):
            with tempfile.TemporaryDirectory() as tmp:
                root = self.repo(tmp)
                with open(os.path.join(root, ".workerignore"), "w") as f:
                    f.write("/crates\n")  # the worktree file lies; the commit's own text rules
                dest = os.path.join(tmp, "out")
                args = ("strip", mode) + ((dest,) if mode == "--dest" else ())
                self.assertEqual(run_cli(root, *args).returncode, 0)
                self.assertEqual(tree_files(dest if mode == "--dest" else root), STRIP_KEPT, mode)

    def test_a_missing_ignore_file_fails_closed_and_deletes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = make_repo(tmp, STRIP_FILES)[0]
            before = tree_files(root)
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout)
            self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)
            self.assertEqual(tree_files(root), before)
            q = run_cli(root, "strip", "--dest", os.path.join(tmp, "o"))
            self.assertEqual(q.returncode, 3)
            self.assertFalse(
                os.path.exists(os.path.join(tmp, "o")) and os.listdir(os.path.join(tmp, "o"))
            )

    def test_an_unknown_syntax_line_fails_closed_and_deletes_nothing(self):
        for bad in ("!/crates\n", "docs\n", "/a[bc]\n", "/docs/\n", "**/*.md \n"):
            with tempfile.TemporaryDirectory() as tmp:
                root = self.repo(tmp, ignore=STRIP_IGNORE + bad)
                before = tree_files(root)
                p = run_cli(root, "strip", "--in-place")
                self.assertEqual(p.returncode, 3, bad + p.stdout)
                self.assertEqual(tree_files(root), before, bad)

    def test_nothing_left_after_stripping_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, files={"docs/a.txt": "a\n", "traces/b.txt": "b\n"})
            before = tree_files(root)
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout)
            self.assertIn("nothing would be left", p.stdout)
            self.assertEqual(tree_files(root), before)

    def test_nothing_left_also_blocks_the_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, files={"docs/a.txt": "a\n"}, ignore="/docs\n/.workerignore\n")
            p = run_cli(root, "strip", "--dest", os.path.join(tmp, "o"))
            self.assertEqual(p.returncode, 3, p.stdout)

    def test_exactly_one_of_in_place_and_dest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            self.assertEqual(run_cli(root, "strip").returncode, 3)
            both = run_cli(root, "strip", "--in-place", "--dest", os.path.join(tmp, "o"))
            self.assertEqual(both.returncode, 3)
            self.assertIn("exactly one", both.stdout)
            self.assertTrue(STRIP_KEPT < tree_files(root) and "docs/guide.txt" in tree_files(root))

    def test_strip_without_a_target_is_refused_with_the_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            p = run_cli(root, "strip")
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertIn("exactly one of --in-place and --dest", p.stdout)
            self.assertNotIn("Traceback", p.stderr)

    def test_strip_with_both_targets_deletes_nothing_and_exports_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            before = tree_files(root)
            dest = os.path.join(tmp, "o")
            p = run_cli(root, "strip", "--in-place", "--dest", dest)
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertEqual(tree_files(root), before)
            self.assertFalse(os.path.exists(dest))

    def test_a_non_empty_destination_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            dest = os.path.join(tmp, "o")
            os.makedirs(dest)
            with open(os.path.join(dest, "stale"), "w") as f:
                f.write("x")
            p = run_cli(root, "strip", "--dest", dest)
            self.assertEqual(p.returncode, 3)
            self.assertEqual(os.listdir(dest), ["stale"])

    def test_a_submodule_directory_matching_the_ignore_file_is_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, ignore=STRIP_IGNORE + "/third_party/linux\n")
            sha = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            sh(
                "git",
                "-C",
                root,
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,%s,third_party/linux" % sha,
            )
            sh(
                "git",
                "-C",
                root,
                "-c",
                "user.email=t@x.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "gitlink",
            )
            os.makedirs(os.path.join(root, "third_party", "linux", "deep"))
            with open(os.path.join(root, "third_party", "linux", "deep", "f.c"), "w") as f:
                f.write("x")
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertFalse(os.path.exists(os.path.join(root, "third_party", "linux")))
            self.assertFalse(os.path.exists(os.path.join(root, "third_party")))

    def test_a_kept_symlink_that_escapes_the_tree_fails_the_export_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            os.symlink("/etc/passwd", os.path.join(root, "evil-link"))
            sh("git", "-C", root, "add", "-f", "evil-link")
            sh(
                "git",
                "-C",
                root,
                "-c",
                "user.email=t@x.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "link",
            )
            p = run_cli(root, "strip", "--dest", os.path.join(tmp, "out"))
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)
            self.assertNotIn("Traceback", p.stderr)

    def test_a_kept_relative_symlink_out_of_the_tree_fails_the_export_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            os.symlink("../../../outside", os.path.join(root, "crates", "kf-a", "up-link"))
            sh("git", "-C", root, "add", "-f", "crates/kf-a/up-link")
            sh(
                "git",
                "-C",
                root,
                "-c",
                "user.email=t@x.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "link",
            )
            p = run_cli(root, "strip", "--dest", os.path.join(tmp, "out"))
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertNotIn("Traceback", p.stderr)

    def test_a_symlink_is_removed_not_followed(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside = os.path.join(tmp, "outside")
            os.makedirs(outside)
            with open(os.path.join(outside, "precious"), "w") as f:
                f.write("keep")
            root = self.repo(tmp)
            os.symlink(outside, os.path.join(root, "docs", "link"))
            sh("git", "-C", root, "add", "-f", "docs/link")
            sh(
                "git",
                "-C",
                root,
                "-c",
                "user.email=t@x.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "link",
            )
            self.assertEqual(run_cli(root, "strip", "--in-place").returncode, 0)
            self.assertTrue(os.path.exists(os.path.join(outside, "precious")))
            self.assertFalse(os.path.lexists(os.path.join(root, "docs", "link")))

    def test_a_parent_swapped_for_a_symlink_out_of_the_tree_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside = os.path.join(tmp, "outside")
            os.makedirs(os.path.join(outside, "deep"))
            with open(os.path.join(outside, "deep", "x.txt"), "w") as f:
                f.write("keep")
            root = self.repo(tmp)
            shutil.rmtree(os.path.join(root, "docs"))
            os.symlink(outside, os.path.join(root, "docs"))
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout)
            self.assertTrue(os.path.exists(os.path.join(outside, "deep", "x.txt")))

    def refuses(self, evil):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            before = tree_files(root)
            removed = [("100644", "1" * 40, "docs/guide.txt"), ("100644", "2" * 40, evil)]
            with self.assertRaises(L.FailClosed, msg=evil):
                L.delete_entries(root, removed)
            self.assertEqual(tree_files(root), before, evil)

    def test_a_dot_git_component_is_refused_before_anything_is_deleted(self):
        for evil in (".git/config", "x/.git/y", ".git"):
            self.refuses(evil)

    def test_an_empty_segment_is_refused_before_anything_is_deleted(self):
        for evil in ("a//b", "a/", "docs//guide.txt"):
            self.refuses(evil)

    def test_dot_dot_and_absolute_paths_are_refused_before_anything_is_deleted(self):
        for evil in ("../outside", "a/../../b", "/abs/p"):
            self.refuses(evil)

    def test_a_nested_parent_swapped_for_a_symlink_out_of_the_tree_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside = os.path.join(tmp, "outside")
            os.makedirs(outside)
            with open(os.path.join(outside, "x.txt"), "w") as f:
                f.write("keep")
            root = self.repo(tmp)
            shutil.rmtree(os.path.join(root, "docs", "deep", "er"))
            os.symlink(outside, os.path.join(root, "docs", "deep", "er"))
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout)
            self.assertTrue(os.path.exists(os.path.join(outside, "x.txt")))

    def test_a_parent_symlinked_to_another_directory_inside_the_tree_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            victim = os.path.join(root, "crates", "kf-a", "src", "guide.txt")
            with open(victim, "w") as f:
                f.write("a source file that must survive")
            shutil.rmtree(os.path.join(root, "docs"))
            os.symlink(os.path.join(root, "crates", "kf-a", "src"), os.path.join(root, "docs"))
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertTrue(os.path.exists(victim))
            self.assertTrue(os.path.exists(os.path.join(root, "crates", "kf-a", "src", "lib.rs")))

    def test_a_nested_parent_symlinked_inside_the_tree_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, files={**STRIP_FILES, "docs/deep/er/x.txt": "d\n"})
            victim = os.path.join(root, "crates", "kf-a", "src", "x.txt")
            with open(victim, "w") as f:
                f.write("must survive")
            shutil.rmtree(os.path.join(root, "docs", "deep", "er"))
            os.symlink(
                os.path.join(root, "crates", "kf-a", "src"),
                os.path.join(root, "docs", "deep", "er"),
            )
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertTrue(os.path.exists(victim))

    def test_a_gitlink_path_that_is_a_symlink_is_unlinked_not_followed(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside = os.path.join(tmp, "outside")
            os.makedirs(outside)
            with open(os.path.join(outside, "precious"), "w") as f:
                f.write("keep")
            root = self.repo(tmp, ignore=STRIP_IGNORE + "/third_party/linux\n")
            sha = sh("git", "-C", root, "rev-parse", "HEAD").strip()
            sh(
                "git",
                "-C",
                root,
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,%s,third_party/linux" % sha,
            )
            sh(
                "git",
                "-C",
                root,
                "-c",
                "user.email=t@x.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "g",
            )
            os.makedirs(os.path.join(root, "third_party"))
            os.symlink(outside, os.path.join(root, "third_party", "linux"))
            p = run_cli(root, "strip", "--in-place")
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertTrue(os.path.exists(os.path.join(outside, "precious")))
            self.assertFalse(os.path.lexists(os.path.join(root, "third_party", "linux")))

    def test_a_destination_that_is_the_repository_itself_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp)
            before = tree_files(root)
            p = run_cli(root, "strip", "--dest", root)
            self.assertEqual(p.returncode, 3, p.stdout)
            self.assertEqual(tree_files(root), before)

    def test_matchjson_answers_for_the_other_implementations_and_refuses_bad_lines(self):
        import json as _json

        ok = subprocess.run(
            [sys.executable, os.path.join(HERE, "ignore_ladder.py"), "matchjson"],
            input=_json.dumps(
                {
                    "patterns": ["/docs", "**/*.md"],
                    "paths": ["docs/a", "x/y.md", "z"],
                    "entries": [["100644", "1" * 40, "z"], ["100644", "2" * 40, "docs/a"]],
                }
            ),
            capture_output=True,
            text=True,
        )
        self.assertEqual(ok.returncode, 0, ok.stderr)
        got = _json.loads(ok.stdout)
        self.assertEqual(got["ignored"], [True, True, False])
        self.assertEqual(len(got["key"]), 64)
        bad = subprocess.run(
            [sys.executable, os.path.join(HERE, "ignore_ladder.py"), "matchjson"],
            input=_json.dumps({"patterns": ["docs"], "paths": ["docs"]}),
            capture_output=True,
            text=True,
        )
        self.assertEqual(bad.returncode, 3)


@unittest.skipUnless(HAVE_GIT, "needs git")
class Floor(unittest.TestCase):
    """Deleting a test or a build input with an ignore line would make it a test that never ran."""

    def attempt(self, extra_files, extra_ignore):
        with tempfile.TemporaryDirectory() as tmp:
            root = make_repo(
                tmp, {**STRIP_FILES, **extra_files}, {".workerignore": STRIP_IGNORE + extra_ignore}
            )[0]
            before = tree_files(root)
            p = run_cli(root, "strip", "--in-place")
            return p, before == tree_files(root)

    def test_ignoring_a_test_file_under_crates_fails_closed(self):
        p, untouched = self.attempt({}, "/crates/kf-a/tests/t.rs\n")
        self.assertEqual(p.returncode, 3, p.stdout)
        self.assertIn("t.rs", p.stdout)
        self.assertTrue(untouched)

    def test_ignoring_a_whole_crate_fails_closed(self):
        p, untouched = self.attempt({}, "/crates/kf-a\n")
        self.assertEqual(p.returncode, 3, p.stdout)
        self.assertTrue(untouched)

    def test_ignoring_cargo_toml_or_the_toolchain_fails_closed(self):
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "clippy.toml",
            ".gitmodules",
        ):
            p, untouched = self.attempt({name: "x\n"}, "/%s\n" % name)
            self.assertEqual(p.returncode, 3, name + p.stdout)
            self.assertTrue(untouched, name)

    def test_ignoring_docker_files_fails_closed(self):
        for name in (
            "Dockerfile",
            "Dockerfile.box",
            "docker-compose.yml",
            "docker-compose.override.yaml",
            ".dockerignore",
        ):
            p, untouched = self.attempt({name: "x\n"}, "/%s\n" % name)
            self.assertEqual(p.returncode, 3, name + p.stdout)
            self.assertTrue(untouched, name)

    def test_ignoring_the_oracle_submodule_paths_fails_closed(self):
        for sub in ("ogkm", "ogkm-580"):
            files = {"third_party/%s/src/x.h" % sub: "h\n"}
            p, untouched = self.attempt(files, "/third_party/%s\n" % sub)
            self.assertEqual(p.returncode, 3, sub + p.stdout)
            self.assertTrue(untouched, sub)

    def test_ignoring_tools_and_firmware_and_qemu_and_cuda_fails_closed(self):
        for root in ("tools", "firmware", "qemu", "cuda", ".cargo"):
            p, untouched = self.attempt({root + "/x/y.c": "c\n"}, "/%s/x\n" % root)
            self.assertEqual(p.returncode, 3, root + p.stdout)
            self.assertTrue(untouched, root)

    def test_a_deep_glob_that_reaches_a_test_fails_closed(self):
        p, untouched = self.attempt({}, "/crates/*/tests\n")
        self.assertEqual(p.returncode, 3, p.stdout)
        p, untouched = self.attempt({}, "**/t.rs\n")
        self.assertEqual(p.returncode, 3, p.stdout)

    def test_prose_and_gitignore_files_under_code_roots_may_be_ignored(self):
        p, untouched = self.attempt({"tools/x/.gitignore": "target\n", "tools/x/run.sh": "x\n"}, "")
        self.assertEqual(p.returncode, 0, p.stdout)

    def test_the_floor_names_each_offender_and_counts_the_rest(self):
        files = {"crates/kf-a/tests/t%d.rs" % i: "x\n" for i in range(8)}
        p, _ = self.attempt(files, "/crates/kf-a/tests\n")
        self.assertEqual(p.returncode, 3)
        self.assertIn("(+", p.stdout)

    def test_is_kept_code_table(self):
        yes = [
            "crates/kf-a/src/lib.rs",
            "crates/kf-a/tests/t.rs",
            "crates/kf-a/Cargo.toml",
            "qemu/hw/misc/kf3/kf3.c",
            "cuda/walk/walk.cu",
            "firmware/kf-gop/src/main.rs",
            "tools/legal/NOTICE.txt",
            ".cargo/config.toml",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "clippy.toml",
            ".gitmodules",
            ".dockerignore",
            "Dockerfile",
            "Dockerfile.box",
            "docker-compose.yml",
            "docker-compose.override.yaml",
            "third_party/ogkm",
            "third_party/ogkm-580",
            "third_party/ogkm-580/src/x.h",
            "third_party/ogkm/src/x.h",
        ]
        no = [
            "crates/kf-a/README.md",
            "crates/kf-a/.gitignore",
            "docs/a.txt",
            "traces/x",
            "scripts/ci/test_x.py",
            "scripts/ci/vast_payload.py",
            "LICENSE",
            "KAYFABE_REV.txt",
            "third_party/linux",
            "third_party/gvisor/x",
            "third_party/ogkm-other/x",
            "crates-extra/x.rs",
            "Dockerfiles/x",
            "docker-compose",
            "xDockerfile",
        ]
        for p in yes:
            self.assertTrue(L.is_kept_code(p), p)
        for p in no:
            self.assertFalse(L.is_kept_code(p), p)

    def test_the_floor_is_a_pure_function_of_paths_and_patterns(self):
        pats = L.read_patterns("/crates/kf-a/tests\n/docs\n")
        self.assertEqual(
            L.floor_violations(
                ["crates/kf-a/tests/t.rs", "crates/kf-a/src/lib.rs", "docs/a"], pats
            ),
            ["crates/kf-a/tests/t.rs"],
        )


@unittest.skipUnless(HAVE_GIT, "needs git")
class TreeKey(unittest.TestCase):
    """The key is the bytes ci/vast-broker/src/treekey.js computes: kf-tree-v2 over the unignored tree."""

    ENT = [
        ("100644", "a" * 40, "src/a.rs"),
        ("100755", "b" * 40, "src/run.sh"),
        ("120000", "c" * 40, "src/link"),
        ("160000", "d" * 40, "third_party/sub"),
        ("100644", "e" * 40, "docs/x.txt"),
    ]
    LINES = ["/docs"]

    def key(self, entries=None, lines=None):
        lines = self.LINES if lines is None else lines
        return L.tree_key(entries or self.ENT, lines, L.read_patterns("\n".join(lines) + "\n"))

    def test_the_key_does_not_depend_on_listing_order(self):
        self.assertEqual(self.key(), self.key(list(reversed(self.ENT))))

    def test_a_change_to_an_unignored_blob_moves_the_key(self):
        e = list(self.ENT)
        e[0] = ("100644", "f" * 40, "src/a.rs")
        self.assertNotEqual(self.key(), self.key(e))

    def test_a_mode_change_moves_the_key(self):
        e = list(self.ENT)
        e[1] = ("100644", "b" * 40, "src/run.sh")
        self.assertNotEqual(self.key(), self.key(e))

    def test_a_rename_moves_the_key(self):
        e = list(self.ENT)
        e[0] = ("100644", "a" * 40, "src/b.rs")
        self.assertNotEqual(self.key(), self.key(e))

    def test_a_change_confined_to_an_ignored_file_keeps_the_key(self):
        e = list(self.ENT)
        e[4] = ("100644", "9" * 40, "docs/x.txt")
        self.assertEqual(self.key(), self.key(e))

    def test_a_different_ignore_set_gives_a_different_key_even_if_the_tree_is_the_same(self):
        self.assertNotEqual(self.key(lines=["/docs"]), self.key(lines=["/docs", "/nothing-here"]))

    def test_comments_and_blank_lines_are_not_part_of_the_key(self):
        text = "# c\n\n/docs\n"
        pats = L.read_patterns(text)
        self.assertEqual(pats, ["/docs"])

    def test_an_empty_tree_has_no_key(self):
        with self.assertRaises(L.FailClosed):
            L.tree_key([("100644", "a" * 40, "docs/x")], ["/docs"], L.read_patterns("/docs\n"))

    def test_the_key_text_is_the_documented_one(self):
        import hashlib

        lines = ["/docs"]
        digest = hashlib.sha256(b"/docs").hexdigest()
        want = hashlib.sha256(
            ("kf-tree-v2\nignore %s\n" % digest).encode()
            + b"".join(
                p.encode() + b"\0" + m.encode() + b"\0" + s.encode() + b"\n"
                for m, s, p in sorted(self.ENT, key=lambda e: e[2].encode())
                if not p.startswith("docs/")
            )
        ).hexdigest()
        self.assertEqual(self.key(lines=lines), want)

    @unittest.skipUnless(HAVE_NODE, "needs node")
    def test_treekey_js_computes_the_same_key_for_this_checkout(self):
        try:
            root = real_root()
        except subprocess.CalledProcessError:
            self.skipTest("not inside a git checkout")
        if not os.path.exists(os.path.join(root, "ci/vast-broker/scripts/tree-key.mjs")):
            self.skipTest("Worker tree absent")
        p = run_cli(root, "keycheck")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("IGNORE_KEY OK", p.stdout)

    def test_treekey_and_keycheck_of_a_commit_without_the_file_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = make_repo(tmp, STRIP_FILES)[0]
            for cmd in ("treekey", "keycheck"):
                p = run_cli(root, cmd)
                self.assertEqual(p.returncode, 3, cmd + p.stdout + p.stderr)
                self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)

    def test_keycheck_without_the_worker_tree_names_the_missing_helper(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = make_repo(tmp, STRIP_FILES, {".workerignore": STRIP_IGNORE})[0]
            p = run_cli(root, "keycheck")
            self.assertIn("IGNORE_LADDER_FAIL_CLOSED", p.stdout)
            self.assertIn("tree-key.mjs", p.stdout)
            self.assertNotIn("IGNORE_KEY python=", p.stdout)

    def test_keycheck_needs_the_worker_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = make_repo(tmp, STRIP_FILES, {".workerignore": STRIP_IGNORE})[0]
            p = run_cli(root, "keycheck")
            self.assertEqual(p.returncode, 3, p.stdout)

    def keycheck_with(self, fake):
        try:
            root = real_root()
        except subprocess.CalledProcessError:
            self.skipTest("not inside a git checkout")
        if not os.path.exists(os.path.join(root, "ci/vast-broker/scripts/tree-key.mjs")):
            self.skipTest("Worker tree absent")
        real = L.tree_key
        L.tree_key = fake(real)
        old, sys.stdout = sys.stdout, open(os.devnull, "w")
        try:
            return L.keycheck("HEAD", root)
        finally:
            sys.stdout.close()
            sys.stdout = old
            L.tree_key = real

    @unittest.skipUnless(HAVE_NODE, "needs node")
    def test_keycheck_fails_when_the_python_key_is_all_zero(self):
        self.assertEqual(self.keycheck_with(lambda real: lambda *a, **k: "0" * 64), 1)

    @unittest.skipUnless(HAVE_NODE, "needs node")
    def test_keycheck_fails_when_the_python_key_differs_in_one_digit(self):
        def fake(real):
            def f(*a, **k):
                key = real(*a, **k)
                return key[:-1] + ("0" if key[-1] != "0" else "1")

            return f

        self.assertEqual(self.keycheck_with(fake), 1)

    @unittest.skipUnless(HAVE_NODE, "needs node")
    def test_keycheck_reports_a_disagreement_as_a_failure(self):
        try:
            root = real_root()
        except subprocess.CalledProcessError:
            self.skipTest("not inside a git checkout")
        if not os.path.exists(os.path.join(root, "ci/vast-broker/scripts/tree-key.mjs")):
            self.skipTest("Worker tree absent")
        real = L.tree_key
        L.tree_key = lambda *a, **k: "0" * 64
        old, sys.stdout = sys.stdout, open(os.devnull, "w")
        try:
            self.assertEqual(L.keycheck("HEAD", root), 1)
        finally:
            sys.stdout.close()
            sys.stdout = old
            L.tree_key = real


@unittest.skipUnless(HAVE_GIT, "needs git")
class RealTree(unittest.TestCase):
    def test_repo_ignore_files_are_a_valid_ladder_that_git_agrees_with(self):
        try:
            root = real_root()
        except subprocess.CalledProcessError:
            self.skipTest("not inside a git checkout")
        if not all(
            os.path.exists(os.path.join(root, n)) for n in (".workerignore", ".dockerignore")
        ):
            self.skipTest("ignore files not present")
        old, sys.stdout = sys.stdout, open(os.devnull, "w")
        try:
            problems = L.check(root)
        finally:
            sys.stdout.close()
            sys.stdout = old
        self.assertEqual(problems, [])


def docker_works():
    if shutil.which("docker") is None:
        return False
    try:
        subprocess.run(["docker", "info"], check=True, capture_output=True, timeout=20)
        return True
    except Exception:
        return False


@unittest.skipUnless(HAVE_GIT and docker_works(), "needs git and a docker daemon")
class RealDocker(unittest.TestCase):
    def test_docker_copies_exactly_the_unignored_files(self):
        files = {
            "src/a.rs": "x",
            "src/tests/t.rs": "t",
            "docs/x.md": "x",
            "top.md": "t",
            "keep/n.md.txt": "k",
            "t/__pycache__/a.pyc": "p",
            "firmware/k/b/c.efi": "e",
            "firmware/k/d.txt": "d",
        }
        ign = "**/*.md\n**/__pycache__\n/firmware/**/*.efi\n/src/tests\n"
        with tempfile.TemporaryDirectory() as tmp:
            root, rev = make_repo(tmp, files)
            pats = L.read_patterns(ign)
            got = L.docker_context_files(rev, ign, os.path.join(tmp, "w"), cwd=root)
            got.discard(".dockerignore")
            want = {p for p in files if not L.matches(pats, p)}
            self.assertEqual(got, want)


if __name__ == "__main__":
    unittest.main()
