# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Exercise the workflow's actual unsafe-count pipeline under bash pipefail."""

import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class UnsafeCount(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        start = workflow.index('actual=$(find "$c/src"')
        end = workflow.index('\n            echo "$c: $actual', start)
        cls.script = 'c=$1\n' + workflow[start:end] + '\nprintf "%s\\n" "$actual"\n'

    def count(self, root):
        return subprocess.run(
            ["bash", "-euo", "pipefail", "-c", self.script, "unsafe-count", str(root)],
            text=True,
            capture_output=True,
            check=False,
        )

    def test_perimeter_with_no_unsafe_rust_is_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "src").mkdir()
            (root / "src" / "address_unsafe.rs").write_text("pub fn bounded() {}\n")
            result = self.count(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "0\n")

    def test_empty_surface_is_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "src").mkdir()
            result = self.count(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "0\n")

    def test_nested_surface_counts_each_relaxation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            nested = root / "src" / "nested"
            nested.mkdir(parents=True)
            (nested / "ffi_unsafe.rs").write_text(
                "unsafe fn run() { unsafe { operation() } }\n"
            )
            result = self.count(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "2\n")

    def test_unreadable_input_is_not_a_zero_count(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "src").mkdir()
            # A dangling file works under root too; chmod(0) would not.
            (root / "src" / "missing_unsafe.rs").symlink_to("absent")
            result = self.count(root)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
