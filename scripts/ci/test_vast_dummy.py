"""GPU-free contract checks for the disposable acceptance payload."""

import importlib.util, unittest
from pathlib import Path
from unittest.mock import patch


def load():
    spec = importlib.util.spec_from_file_location(
        "dummy", Path(__file__).with_name("vast_dummy.py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DummyTests(unittest.TestCase):
    def test_hardware_exact(self):
        with patch(
            "subprocess.check_output", return_value="NVIDIA GeForce RTX 3090, 8.6, 580.95.05\n"
        ):
            self.assertEqual(
                load().hardware(),
                {
                    "gpu_name": "NVIDIA GeForce RTX 3090",
                    "compute_cap": 860,
                    "driver_version": "580.95.05",
                },
            )

    def test_hardware_rejects_unknown(self):
        for value in ["unknown, 8.6, 580.95.05", "RTX 3090, nope, 580.95.05"]:
            with patch("subprocess.check_output", return_value=value):
                with self.assertRaises(ValueError):
                    load().hardware()

    def test_checksum_a(self):
        with self.assertRaises(ValueError):
            load().verify_blob(b"wrong", "blob-a.txt")

    def test_checksum_b(self):
        with self.assertRaises(ValueError):
            load().verify_blob(b"wrong", "blob-b.txt")


if __name__ == "__main__":
    unittest.main()
