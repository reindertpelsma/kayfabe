"""Run the actual observation controller tail and hold block with hardware commands stubbed."""

from pathlib import Path
import subprocess
import tempfile
import unittest

KIT = Path(__file__).resolve().parents[3] / "traces/blackwell_ab_20261023/observation_run/run_kit"


class Protocol(unittest.TestCase):
    def schedule(self, ready=1, budget=0, first_rc=0):
        source = (KIT / "controller.sh").read_text()
        tail = source.split('[ "${#measured[@]}" = 2 ]', 1)[1]
        stub = f"""
set -euo pipefail
BASE=derived; BIN=derived_binary; measured=(B_binary B_source)
b1=(B1_binary B1_source); b1_ready={ready}
run_cell() {{ echo "CELL $1"; if [ "$1" = on_observe_b ]; then return {first_rc}; fi; }}
python3() {{ return {budget}; }}
"""
        return subprocess.run(["bash", "-c", stub + tail], capture_output=True, text=True)

    def test_baseline_then_b1_uses_the_same_two_observation_cells(self):
        result = self.schedule()
        self.assertEqual(result.returncode, 0, result.stderr)
        cells = [line for line in result.stdout.splitlines() if line.startswith("CELL ")]
        self.assertEqual(cells, ["CELL on_observe_b", "CELL on_observe_b1"])

    def test_failed_b1_build_is_skipped(self):
        result = self.schedule(ready=0)
        self.assertIn("CELL on_observe_b\n", result.stdout)
        self.assertNotIn("CELL on_observe_b1", result.stdout)

    def test_late_b1_start_is_skipped(self):
        result = self.schedule(budget=30)
        self.assertIn("CELL on_observe_b\n", result.stdout)
        self.assertNotIn("CELL on_observe_b1", result.stdout)

    def test_baseline_safety_refusal_prevents_b1(self):
        result = self.schedule(first_rc=7)
        self.assertEqual(result.returncode, 7)
        self.assertNotIn("CELL on_observe_b1", result.stdout)

    def hold_with_early_code0(self, rc):
        source = (KIT / "arm.sh").read_text()
        marker = 'timeout 120 bash "$W" ssh '
        start = source.index(marker, source.index("ENABLE580") if "ENABLE580" in source else 0)
        end = source.index("\nif ! grep -q GUEST_PANICKED", start)
        block = source[start:end]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "arms/cell").mkdir(parents=True)
            (root / "logs").mkdir()
            (root / "logs/cell_run.log").write_text("")
            (root / "arms/cell/early_code0.txt").write_text('BW580_RESULT {"code":0}\n')
            block = block.replace("/root/prov", directory)
            stub = f"""
set -euo pipefail
W=unused; VM={directory}; run=cell; owned_pid=$$; held_seconds=0
timeout() {{ return {rc}; }}
sleep() {{ SECONDS=$((SECONDS+601)); }}
"""
            return subprocess.run(["bash", "-c", stub + block], capture_output=True, text=True)

    def test_early_code0_still_holds_after_ssh_disconnect(self):
        result = self.hold_with_early_code0(255)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ARM_HOLD_OBSERVED", result.stdout)

    def test_early_code0_still_holds_after_diagnostic_timeout(self):
        result = self.hold_with_early_code0(124)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ARM_HOLD_OBSERVED", result.stdout)


if __name__ == "__main__":
    unittest.main()
