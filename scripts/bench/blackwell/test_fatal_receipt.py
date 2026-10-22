"""A second fail-closed watcher must preserve the first stop's evidence."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import phase_xid_guard as G


class FatalReceipt(unittest.TestCase):
    def test_first_fault_publishes_its_complete_reason(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fatal'
            reason = {'STOP': 'new fault without current arm/init proof', 'owned_pgid': 101}
            G.record_fatal(path, reason)
            self.assertEqual(json.loads(path.read_text()), reason)
            self.assertEqual(list(Path(directory).iterdir()), [path])

    def test_second_watcher_cannot_replace_original_fault(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fatal'
            original = b'{"STOP":"first cause","detached_qemu_stopped":true}\n'
            path.write_bytes(original)
            G.record_fatal(path, {'STOP': 'prior fatal marker', 'detached_qemu_stopped': False})
            self.assertEqual(path.read_bytes(), original)

    def test_existing_unparseable_fatal_still_stops_and_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fatal'
            original = b'partial original stop evidence\n'
            path.write_bytes(original)
            G.record_fatal(path, {'STOP': 'prior fatal marker'})
            self.assertTrue(path.exists())
            self.assertEqual(path.read_bytes(), original)

    def test_simultaneous_watchers_publish_one_whole_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fatal'
            barrier = threading.Barrier(2)
            reasons = [{'STOP': 'fault A', 'owned_pgid': 101}, {'STOP': 'fault B', 'owned_pgid': 102}]

            def publish(reason):
                barrier.wait(timeout=5)
                G.record_fatal(path, reason)

            with ThreadPoolExecutor(max_workers=2) as workers:
                list(workers.map(publish, reasons))
            self.assertIn(json.loads(path.read_text()), reasons)
            self.assertEqual(list(Path(directory).iterdir()), [path])
