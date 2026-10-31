"""win_vm.sh builds the kf3 device string in one function; WINVM_KF3_DISPLAY picks its shape.

The function sits between `# KF3_DEVICE_PROPS_BEGIN` and `# KF3_DEVICE_PROPS_END` so a test can
run exactly that text and nothing else of the harness (which needs KVM, swtpm and a GPU).
"""

import pathlib
import re
import subprocess
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "windows" / "win_vm.sh"
BASE = "kf3-gpu,fb-mb=6144,bar1-size=134217728,bar2-size=33554432"


def block():
    text = SCRIPT.read_text()
    m = re.search(r"# KF3_DEVICE_PROPS_BEGIN\n(.*?)# KF3_DEVICE_PROPS_END", text, re.S)
    if m is None:
        raise AssertionError("win_vm.sh has no KF3_DEVICE_PROPS block")
    return m.group(1)


def props(display, gop="/x/kf-gop.signed.efi"):
    script = (
        "die() { echo \"REFUSED: $*\" >&2; exit 2; }\n"
        + block()
        + f"\nKF3_FB_MB=6144\nkf3_device_props {gop!r} {display!r}\n"
    )
    return subprocess.run(["bash", "-c", script], capture_output=True, text=True)


class DeviceProps(unittest.TestCase):
    def test_display_on_is_the_default_shape(self):
        for display in ("", "on"):
            r = props(display)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(
                r.stdout.strip(),
                BASE + ",display=on,gop=on,gop-efi=/x/kf-gop.signed.efi,id=kf0",
            )

    def test_display_off_is_headless_with_no_gop(self):
        r = props("off")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), BASE + ",id=kf0")
        for word in ("display", "gop"):
            self.assertNotIn(word, r.stdout)

    def test_any_other_value_is_refused_by_name(self):
        for display in ("OFF", "1", "no", "true"):
            r = props(display)
            self.assertEqual(r.returncode, 2, display)
            self.assertIn("WINVM_KF3_DISPLAY", r.stderr)

    def test_the_run_command_signs_the_gop_only_when_displayed(self):
        # next's --dry-run gate sits beside the display=off gate: off never signs, dry-run names the
        # path without signing, and only a real displayed run builds and signs the GOP
        text = SCRIPT.read_text()
        self.assertIn('if [ "${WINVM_KF3_DISPLAY:-on}" = off ]; then gop=""', text)
        self.assertIn("elif dry_run; then gop=", text)
        self.assertIn("else gop=$(gop_signed); fi", text)
        self.assertIn('dev=$(kf3_device_props "$gop" "${WINVM_KF3_DISPLAY:-on}")', text)


if __name__ == "__main__":
    unittest.main()
