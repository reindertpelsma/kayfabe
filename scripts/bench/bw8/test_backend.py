"""Synthetic one-attempt and immediate owned-ledger receipts; no provider access."""

from pathlib import Path
from types import SimpleNamespace
import json
import tempfile
import unittest
from backend import backend_type

try:
    from backend import source_lock
except ImportError:

    def source_lock(files):
        raise ValueError("source gate absent")


class Base:
    def __init__(self, offer, plan, credit, *, evidence, report, here):
        self.offer, self.plan, self.credit = offer, plan, credit
        self.evidence, self.report_path, self.here = evidence, report, here
        evidence.mkdir()
        (evidence / "owned.json").write_text("[]")
        self.watch = SimpleNamespace(poll=lambda: None)


class Receipts(unittest.TestCase):
    def fixture(self, root, reply):
        self.calls = []

        def api(args):
            self.calls.append(args)
            return reply

        def save(path, body):
            path.write_text(json.dumps(body))

        provider = SimpleNamespace(
            Backend=Base, api=api, save=save, utc=lambda: "2026-10-25T16:30:00+00:00"
        )
        config = dict(
            workspace=str(root),
            evidence=str(root / "evidence"),
            report=str(root / "report.md"),
            operator=str(root),
            provider_image="fixture",
        )
        plan = dict(seconds=5000, deadline_utc="2026-10-25T18:00:00+00:00")
        return backend_type(provider)(
            dict(id=2, machine_id=3, host_id=4, dph_total=0.8), plan, 23, config
        )

    def test_success_is_one_attempt(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = self.fixture(Path(tmp), dict(success=True, new_contract=123))
            self.assertEqual(b.create(), 123)
            with self.assertRaises(FileExistsError):
                b.create()
            self.assertEqual(len(self.calls), 1)

    def test_refusal_is_not_retried(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = self.fixture(Path(tmp), dict(success=False))
            with self.assertRaises(ValueError):
                b.create()
            with self.assertRaises(FileExistsError):
                b.create()
            self.assertEqual(len(self.calls), 1)

    def test_dead_watchdog_prevents_create(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = self.fixture(Path(tmp), {})
            b.watch.poll = lambda: 1
            with self.assertRaises(ValueError):
                b.create()
            self.assertFalse(self.calls)

    def test_killed_watchdog_prevents_create(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = self.fixture(Path(tmp), {})
            b.watch.poll = lambda: -15
            with self.assertRaises(ValueError):
                b.create()
            self.assertFalse(self.calls)

    def test_report_immediately_binds_exact_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            b = self.fixture(root, dict(success=True, new_contract=123))
            b.create()
            b.report(
                dict(
                    id=123, deadline_utc=b.plan["deadline_utc"], watch=dict(pid=456, starttime=789)
                )
            )
            row = json.loads((root / "evidence/owned.json").read_text())[0]
            self.assertEqual(row["id"], 123)
            self.assertEqual(row["auto_destroy_pid"], 456)
            self.assertEqual((root / "own-id").read_text(), "123\n")

    def test_report_contains_deadline_and_rate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            b = self.fixture(root, dict(success=True, new_contract=123))
            b.create()
            b.report(
                dict(
                    id=123, deadline_utc=b.plan["deadline_utc"], watch=dict(pid=456, starttime=789)
                )
            )
            text = (root / "report.md").read_text()
            self.assertIn(b.plan["deadline_utc"], text)
            self.assertIn("requested_disk_actual_rate", text)

    def test_source_lock_good(self):
        import hashlib

        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "source.py"
            p.write_bytes(b"fixture")
            self.assertTrue(source_lock({str(p): hashlib.sha256(b"fixture").hexdigest()}))

    def test_source_lock_second_good_file(self):
        import hashlib

        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "other.sh"
            p.write_bytes(b"second")
            self.assertTrue(source_lock({str(p): hashlib.sha256(b"second").hexdigest()}))

    def test_source_changed(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "source.py"
            p.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                source_lock({str(p): "0" * 64})

    def test_second_source_changed(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "source.sh"
            p.write_bytes(b"other changed source")
            with self.assertRaises(ValueError):
                source_lock({str(p): "1" * 64})

    def test_report_binds_deadline_and_timer_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            b = self.fixture(root, dict(success=True, new_contract=123))
            b.create()
            b.report(
                dict(
                    id=123, deadline_utc=b.plan["deadline_utc"], watch=dict(pid=456, starttime=789)
                )
            )
            row = json.loads((root / "evidence/owned.json").read_text())[0]
            self.assertEqual(row["round_deadline_utc"], b.plan["deadline_utc"])
            self.assertEqual(row["auto_destroy_pid_starttime"], 789)
            self.assertTrue(row["auto_destroy_verified"])

    def test_source_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises((ValueError, FileNotFoundError)):
                source_lock({str(Path(tmp) / "missing.py"): "0" * 64})


if __name__ == "__main__":
    unittest.main()
