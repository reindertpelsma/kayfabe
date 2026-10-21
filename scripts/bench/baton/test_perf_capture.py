import importlib.util
from pathlib import Path
import unittest


class PerfFilter(unittest.TestCase):
    def check_filter(self, tids):
        spec = importlib.util.spec_from_file_location("capture", Path(__file__).with_name("host_freeze_capture.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        command = mod.perf_command(tids, Path("evidence"))
        self.assertIn("--filter", command, "system-wide recording must be filtered")
        filt = command[command.index("--filter") + 1]
        for tid in tids:
            self.assertIn(f"prev_pid=={tid}", filt)
            self.assertIn(f"next_pid=={tid}", filt)
        self.assertIsNone(mod.perf_command([], Path("evidence")), "empty selection must never record the system")

    def test_one_owned_thread(self):
        self.check_filter([456])

    def test_several_owned_threads(self):
        self.check_filter([123, 456, 789])


if __name__ == "__main__":
    unittest.main()
