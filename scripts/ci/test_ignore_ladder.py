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


@unittest.skipUnless(HAVE_GIT, "needs git")
class RealTree(unittest.TestCase):
    def test_repo_ignore_files_are_a_valid_ladder_that_git_agrees_with(self):
        try:
            root = sh("git", "-C", HERE, "rev-parse", "--show-toplevel").strip()
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
