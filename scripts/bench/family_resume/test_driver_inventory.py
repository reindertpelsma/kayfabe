"""Initial image inventory is evidence; native qualification stays strict."""

from pathlib import Path
import os
import subprocess
import tempfile
import unittest

HELPER = Path(__file__).with_name("driver_inventory.sh")


class InventoryTests(unittest.TestCase):
    def run_case(self, code, *, native=False):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake = root / "nvidia-smi"
            fake.write_text("#!/bin/sh\necho mock-inventory\nexit " + str(code) + "\n")
            fake.chmod(0o700)
            script = (
                'set -euo pipefail\nsource "$1"\nrecord_initial_inventory "$2"\n'
                "echo INVENTORY_RECORDED\n"
            )
            if native:
                script += "nvidia-smi --query-gpu=driver_version\necho GUEST_ALLOWED\n"
            result = subprocess.run(
                ["bash", "-c", script, "inventory-test", str(HELPER), tmp],
                env=dict(os.environ, PATH=tmp + ":" + os.environ["PATH"]),
                capture_output=True,
                text=True,
            )
            rcfile = root / "smi_initial_query.rc"
            return result, rcfile.read_text().strip() if rcfile.exists() else None

    def test_closed_image_inventory_is_recorded_without_blocking_planned_provision(self):
        result, recorded = self.run_case(6)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(recorded, "6")
        self.assertIn("INVENTORY_RECORDED", result.stdout)

    def test_other_initial_inventory_failure_is_recorded(self):
        result, recorded = self.run_case(9)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(recorded, "9")

    def test_native_adapter_failure_still_stops_before_guest(self):
        result, recorded = self.run_case(6, native=True)
        self.assertEqual(recorded, "6")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("GUEST_ALLOWED", result.stdout)

    def test_another_native_adapter_refusal_cannot_be_swallowed(self):
        result, recorded = self.run_case(9, native=True)
        self.assertEqual(recorded, "9")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("GUEST_ALLOWED", result.stdout)


if __name__ == "__main__":
    unittest.main()
