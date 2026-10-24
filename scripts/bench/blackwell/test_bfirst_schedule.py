"""Exercise the actual controller's final scheduling block with stubbed hardware cells."""

import subprocess
import unittest
from pathlib import Path

KIT = Path(__file__).resolve().parents[3] / "traces/blackwell_ab_20261023/bfirst_run/run_kit"


class BFirstSchedule(unittest.TestCase):
    def invoke(self, budget=0, measured_rc=0):
        source = (KIT / "controller.sh").read_text()
        tail = source.split('[ "${#measured[@]}" = 2 ]', 1)[1]
        stub = f"""
set -euo pipefail
BASE=derived; BIN=derived_binary; measured=(measured_binary measured_source)
run_cell() {{
    echo "CELL $1"
    if [ "$1" = on_measured ]; then return {measured_rc}; fi
}}
python3() {{ return {budget}; }}
"""
        return subprocess.run(["bash", "-c", stub + tail], capture_output=True, text=True)

    def test_measured_arm_runs_first_when_both_fit(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        cells = [line for line in result.stdout.splitlines() if line.startswith("CELL ")]
        self.assertEqual(cells, ["CELL on_measured", "CELL on_derived"])

    def test_derived_arm_is_skipped_when_start_budget_is_refused(self):
        result = self.invoke(budget=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("CELL on_derived", result.stdout)
        self.assertIn("CELL_UNRUN_BUDGET on_derived", result.stdout)

    def test_measured_arm_still_runs_when_no_time_remains_for_derived(self):
        result = self.invoke(budget=30)
        cells = [line for line in result.stdout.splitlines() if line.startswith("CELL ")]
        self.assertEqual(cells, ["CELL on_measured"])

    def test_measured_refusal_stops_before_derived(self):
        result = self.invoke(measured_rc=17)
        self.assertEqual(result.returncode, 17)
        self.assertNotIn("CELL on_derived", result.stdout)


if __name__ == "__main__":
    unittest.main()
