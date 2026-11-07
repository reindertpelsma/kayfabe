#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tests for evidence_archive.py and fetch_evidence_archive.py. STATUS: LIVE, 2026-11-07.

Everything runs on a synthetic tree in a temporary directory; the mutants (a changed byte, an
extra or missing member, a wrong archive hash) must each be refused.
"""

import contextlib
import importlib.util
import io
import shutil
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ea = load("evidence_archive")
fetch = load("fetch_evidence_archive")

FILES = {
    "traces/alpha_1/shots/a.png": b"\x89PNG-a" * 5000,
    "traces/alpha_1/shots/b.png": b"\x89PNG-b" * 3000,
    "traces/alpha_1/logs/run.log.gz": b"gz" * 100,
    "traces/beta_2/big.log": b"line\n" * 20000,
}
KEPT = {"traces/alpha_1/README.md": b"# alpha\n", "traces/beta_2/small.txt": b"x\n"}


@contextlib.contextmanager
def quiet():
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        yield


def write_tree(root, files):
    for name, data in files.items():
        p = root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)


@unittest.skipUnless(shutil.which("zstd"), "needs the zstd command")
class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.root = self.tmp / "repo"
        write_tree(self.root, FILES)
        write_tree(self.root, KEPT)
        self.list = self.tmp / "list.txt"
        self.list.write_text("\n".join(sorted(FILES)) + "\n")
        self.out = self.tmp / "out"

    def tearDown(self):
        self._tmp.cleanup()

    def build(self, out=None, root=None):
        with quiet():
            ea.main(["--root", str(root or self.root), "build", "--list", str(self.list), "--out", str(out or self.out)])

    def stubs(self):
        index = ea.load_index(self.root)
        for d, row in index.items():
            (self.root / "traces" / d / "MOVED.md").write_text(f"archive sha256 {row['sha256']}\n")

    def test_build_is_deterministic_and_verifies(self):
        self.build()
        again = self.tmp / "again"
        # a copy of the tree with different mtimes and an unrelated owner-visible change elsewhere
        other = self.tmp / "repo2"
        shutil.copytree(self.root, other)
        for p in other.rglob("*"):
            if p.is_file():
                p.touch()
        (other / "traces/alpha_1/MANIFEST.tsv").unlink()
        self.build(out=again, root=other)
        for d in ("alpha_1", "beta_2"):
            self.assertEqual((self.out / f"{d}.tar.zst").read_bytes(), (again / f"{d}.tar.zst").read_bytes())
        self.assertEqual((self.out / "SHA256SUMS").read_text().count("\n"), 2)
        rows = ea.read_manifest(self.root / "traces/alpha_1/MANIFEST.tsv")
        self.assertEqual(list(rows), sorted(n for n in FILES if "alpha_1" in n))
        with quiet():
            ea.verify(self.out / "alpha_1.tar.zst", self.root / "traces/alpha_1/MANIFEST.tsv")

    def test_archive_members_are_normalised(self):
        self.build()
        stream = ea.zstd_stream(self.out / "alpha_1.tar.zst")
        with tarfile.open(fileobj=stream, mode="r|") as tar:
            for m in tar:
                self.assertEqual((m.mtime, m.uid, m.gid, m.uname, m.gname, m.mode), (0, 0, 0, "", "", 0o644))
        stream.close()

    def test_verify_refuses_each_mutant(self):
        self.build()
        manifest = self.root / "traces/alpha_1/MANIFEST.tsv"
        good = manifest.read_text()
        lines = good.split("\n")
        # a changed hash
        bad = lines[:]
        bad[1] = bad[1][:-1] + ("0" if bad[1][-1] != "0" else "1")
        manifest.write_text("\n".join(bad))
        with self.assertRaises(ea.EvidenceError):
            ea.verify(self.out / "alpha_1.tar.zst", manifest)
        # a changed size
        parts = lines[1].split("\t")
        bad = lines[:]
        bad[1] = f"{parts[0]}\t{int(parts[1]) + 1}\t{parts[2]}"
        manifest.write_text("\n".join(bad))
        with self.assertRaises(ea.EvidenceError):
            ea.verify(self.out / "alpha_1.tar.zst", manifest)
        # a manifest row with no member
        manifest.write_text(good.rstrip("\n") + "\ntraces/alpha_1/zzz.png\t1\t" + "0" * 64 + "\n")
        with self.assertRaises(ea.EvidenceError):
            ea.verify(self.out / "alpha_1.tar.zst", manifest)
        # a member the manifest does not list
        manifest.write_text("\n".join([lines[0], lines[1], ""]))
        with self.assertRaises(ea.EvidenceError):
            ea.verify(self.out / "alpha_1.tar.zst", manifest)
        # an unsorted manifest and a path outside traces/
        manifest.write_text("\n".join([lines[0], lines[2], lines[1], lines[3], ""]))
        with self.assertRaises(ea.EvidenceError):
            ea.read_manifest(manifest)
        manifest.write_text(f"{lines[0]}\n../etc/x\t1\t{'0' * 64}\n")
        with self.assertRaises(ea.EvidenceError):
            ea.read_manifest(manifest)

    def relocate(self):
        """Delete the relocated files and the directories they leave empty (git has no empty dirs)."""
        for n in FILES:
            (self.root / n).unlink()
        for p in sorted(self.root.rglob("*"), key=lambda q: -len(q.parts)):
            if p.is_dir() and not any(p.iterdir()):
                p.rmdir()

    def test_check_tree_and_where(self):
        self.build()
        self.stubs()
        with quiet():
            ea.main(["--root", str(self.root), "check-tree"])
        # relocated: delete the files; the tree still checks
        self.relocate()
        with quiet():
            ea.main(["--root", str(self.root), "check-tree"])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            ea.main(["--root", str(self.root), "where", "traces/alpha_1/shots/a.png"])
            ea.main(["--root", str(self.root), "where", "traces/alpha_1/README.md"])
            ea.main(["--root", str(self.root), "where", "traces/alpha_1/shots"])
        out = buf.getvalue()
        self.assertIn("relocated: traces/alpha_1/shots/a.png", out)
        self.assertIn("in tree: traces/alpha_1/README.md", out)
        self.assertIn("directory with 2 relocated files", out)
        with self.assertRaises(SystemExit), quiet():
            ea.main(["--root", str(self.root), "where", "traces/alpha_1/nope.png"])
        # a file that is back but differs from its row is refused
        (self.root / "traces/alpha_1/shots").mkdir()
        (self.root / "traces/alpha_1/shots/a.png").write_bytes(b"other")
        with self.assertRaises(SystemExit), quiet():
            ea.main(["--root", str(self.root), "check-tree"])

    def test_check_tree_wants_moved_md_with_the_hash(self):
        self.build()
        with self.assertRaises(SystemExit), quiet():
            ea.main(["--root", str(self.root), "check-tree"])
        self.stubs()
        (self.root / "traces/beta_2/MOVED.md").write_text("no hash here\n")
        with self.assertRaises(SystemExit), quiet():
            ea.main(["--root", str(self.root), "check-tree"])

    def test_fetch_restores_and_refuses_wrong_archive(self):
        self.build()
        self.stubs()
        self.relocate()
        dest = self.tmp / "restored"
        base = self.out.as_uri()
        cache = self.tmp / "cache"
        argv = ["--root", str(self.root), "--dest", str(dest), "--base-url", base, "--cache", str(cache)]
        with quiet():
            self.assertEqual(fetch.main(["--all", *argv]), 0)
        for n, data in FILES.items():
            self.assertEqual((dest / n).read_bytes(), data)
        with quiet():
            self.assertEqual(fetch.main(["alpha_1", *argv]), 0)  # idempotent
        # a corrupted archive on the mirror is refused and not extracted
        bad_base = self.tmp / "mirror"
        shutil.copytree(self.out, bad_base)
        blob = bytearray((bad_base / "beta_2.tar.zst").read_bytes())
        blob[len(blob) // 2] ^= 0xFF
        (bad_base / "beta_2.tar.zst").write_bytes(bytes(blob))
        dest2 = self.tmp / "restored2"
        with quiet():
            rc = fetch.main(
                ["beta_2", "--root", str(self.root), "--dest", str(dest2), "--base-url", bad_base.as_uri(), "--cache", str(self.tmp / "cache2")]
            )
        self.assertEqual(rc, 2)
        self.assertFalse((dest2 / "traces/beta_2/big.log").exists())
        # an unknown directory
        with quiet():
            self.assertEqual(fetch.main(["nope", *argv]), 2)
        # an existing, differing file is not overwritten without --force
        (dest / "traces/beta_2/big.log").write_bytes(b"mine")
        with quiet():
            self.assertEqual(fetch.main(["beta_2", *argv]), 2)
            self.assertEqual(fetch.main(["beta_2", "--force", *argv]), 0)
        self.assertEqual((dest / "traces/beta_2/big.log").read_bytes(), FILES["traces/beta_2/big.log"])

    def test_build_refuses_bad_lists(self):
        self.list.write_text("traces/alpha_1/shots/a.png\ntraces/alpha_1/shots/a.png\n")
        with self.assertRaises(ea.EvidenceError):
            ea.cmd_build(type("A", (), {"root": str(self.root), "list": str(self.list), "out": str(self.out)}))
        self.list.write_text("docs/x.md\n")
        with self.assertRaises(ea.EvidenceError):
            ea.cmd_build(type("A", (), {"root": str(self.root), "list": str(self.list), "out": str(self.out)}))


if __name__ == "__main__":
    unittest.main()
