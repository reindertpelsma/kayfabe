"""Private archive transport integrity checks, synthetic data only."""

import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

try:
    from capture import pack, verify_archive
except ImportError:

    def missing(*args, **kwargs):
        raise ValueError("capture implementation absent")

    pack = verify_archive = missing


try:
    from capture import verify_pulled
except ImportError:

    def verify_pulled(*args):
        raise ValueError("receipt verification absent")


class PulledReceipts(unittest.TestCase):
    def fixture(self, root):
        folder = root / "box"
        folder.mkdir()
        (folder / "private.log").write_bytes(b"private")
        return [
            dict(
                run="box",
                verified=1,
                failed=[],
                not_pulled=[],
                files={"private.log": dict(sha256=hashlib.sha256(b"private").hexdigest())},
            )
        ]

    def test_verified_original_pull(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            self.assertTrue(verify_pulled(self.fixture(p), p))

    def test_verified_second_pull(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            rows = self.fixture(p)
            rows[0]["run"] = "on_fix"
            (p / "box").rename(p / "on_fix")
            self.assertTrue(verify_pulled(rows, p))

    def test_changed_file_before_destroy(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            rows = self.fixture(p)
            (p / "box/private.log").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                verify_pulled(rows, p)

    def test_second_changed_file_before_destroy(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            rows = self.fixture(p)
            rows[0]["files"]["private.log"]["sha256"] = "0" * 64
            with self.assertRaises(ValueError):
                verify_pulled(rows, p)

    def test_failed_pull_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            rows = self.fixture(p)
            rows[0]["failed"] = ["private.log"]
            with self.assertRaises(ValueError):
                verify_pulled(rows, p)

    def test_partial_pull_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            rows = self.fixture(p)
            rows[0]["not_pulled"] = ["private.log"]
            with self.assertRaises(ValueError):
                verify_pulled(rows, p)


class Capture(unittest.TestCase):
    def test_pack_and_verify(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "sample.jsonl").write_text("private synthetic DATA\n")
            row = pack([(source, "evidence")], root / "capture.tgz")
            self.assertEqual(row["files"], 1)
            self.assertTrue(verify_archive(root / "capture.tgz", row["sha256"]))

    def test_nested_probe_frontier_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "arm").mkdir()
            (source / "arm/probe.log").write_text("private synthetic frontier\n")
            row = pack([(source, "evidence")], root / "capture.tgz")
            with tarfile.open(root / "capture.tgz") as archive:
                self.assertIn("evidence/arm/probe.log", archive.getnames())
            self.assertTrue(verify_archive(root / "capture.tgz", row["sha256"]))

    def test_wrong_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "x"
            p.write_bytes(b"x")
            with self.assertRaises(ValueError):
                verify_archive(p, "0" * 64)

    def test_truncated_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "x"
            p.write_bytes(b"x")
            with self.assertRaises(ValueError):
                verify_archive(p, "bad")

    def test_byte_limit(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            (p / "x.log").write_bytes(b"123")
            with self.assertRaises(ValueError):
                pack([(p, "evidence")], p / "out.tgz", max_bytes=2)

    def test_file_limit(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            (p / "x.log").write_bytes(b"123")
            with self.assertRaises(ValueError):
                pack([(p, "evidence")], p / "out.tgz", max_files=0)

    def unsafe(self, name):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "x.tgz"
            with tarfile.open(p, "w:gz") as archive:
                member = tarfile.TarInfo(name)
                member.size = 1
                archive.addfile(member, io.BytesIO(b"x"))
            with self.assertRaises(ValueError):
                verify_archive(p, hashlib.sha256(p.read_bytes()).hexdigest())

    def test_absolute_member(self):
        self.unsafe("/outside")

    def test_parent_member(self):
        self.unsafe("evidence/../../outside")

    def test_second_parent_member(self):
        self.unsafe("evidence/../outside")

    def test_unknown_root(self):
        self.unsafe("other/file")

    def test_second_unknown_root(self):
        self.unsafe("unexpected/file")


if __name__ == "__main__":
    unittest.main()
