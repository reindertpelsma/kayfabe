"""Attribution, counter lifetime and censoring checks for the offline high-water reader."""
import importlib.util
from pathlib import Path
import unittest
SPEC = importlib.util.spec_from_file_location(
    "water", Path(__file__).with_name("ens5-highwater.py")
)
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)


class WaterTests(unittest.TestCase):
    def test_no_per_space_inference_from_session_heartbeat(self):
        self.assertEqual(m.scan(["sparse[made=999 freed=1]"], []), [])

    def test_unattributed_space_is_not_assigned_to_a_workload(self):
        self.assertEqual(m.scan(["SPARSE space=0x1 made=99 freed=2"], []), [])

    def test_old_delta_does_not_invent_initial_roots(self):
        out = m.scan(["SPARSE space=0x1 made=10 freed=3"],
                     [{"workload": "enscape-01", "first_line": 1, "last_line": 1}])
        self.assertEqual(out[0]["peak_made_minus_freed"], 7)
        self.assertIsNone(out[0]["exact_peak_blocks"])
        self.assertEqual(out[0]["coverage"], "censored")

    def test_explicit_gauge_and_native_handle_reuse(self):
        lines = ["SPARSE space=0x1 made=100 freed=9 peak_blocks=94",
                 "SPARSE space=0x1 made=5 freed=1 peak_blocks=7"]
        out = m.scan(lines, [{"workload": "Heaven", "first_line": 1, "last_line": 2}])
        self.assertEqual([r["epoch"] for r in out], [0, 1])
        self.assertEqual([r["exact_peak_blocks"] for r in out], [94, 7])


if __name__ == "__main__":
    unittest.main()
