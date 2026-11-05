"""The Windows install may run its disk with a faster cache mode; runs never do.

WINVM_INSTALL_CACHE=unsafe (a rented box, a throwaway base image that is sealed after a clean
guest shutdown) is read by the two install-time boots only. The measurement boots (`run`) keep
QEMU's default cache, and an unset variable leaves the install command line as it was.
"""

import pathlib
import re
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "windows" / "win_vm.sh"
KNOB = "${WINVM_INSTALL_CACHE:+,cache=$WINVM_INSTALL_CACHE}"


def function_body(name):
    text = SCRIPT.read_text()
    m = re.search(rf"^{name}\(\) \{{\n(.*?)^\}}\n", text, re.S | re.M)
    if m is None:
        raise AssertionError(f"no function {name}")
    return m.group(1)


class InstallCache(unittest.TestCase):
    def test_install_and_check_boots_take_the_knob(self):
        for name in ("install_boot_qa", "check_boot"):
            body = function_body(name)
            self.assertIn("file=$VM/disk/base.qcow2", body, name)
            self.assertIn(KNOB, body, name)

    def test_measurement_boots_do_not(self):
        text = SCRIPT.read_text()
        self.assertEqual(text.count(KNOB), 2, "only the two install-time boots")
        self.assertNotIn("WINVM_INSTALL_CACHE", function_body("cmd_run"))

    def test_unset_leaves_the_drive_string_unchanged(self):
        self.assertTrue(KNOB.startswith("${WINVM_INSTALL_CACHE:+"))

    def test_the_value_is_validated_before_it_reaches_a_drive_string(self):
        text = SCRIPT.read_text()
        self.assertIn("WINVM_INSTALL_CACHE must be unset or a QEMU cache mode", text)

    def test_header_documents_it(self):
        self.assertIn("WINVM_INSTALL_CACHE", SCRIPT.read_text().split("set -euo pipefail")[0])


if __name__ == "__main__":
    unittest.main()
