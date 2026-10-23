"""Actual Code0 from the one-enable command must survive a failed later SSH diagnostic."""

from pathlib import Path
import subprocess
import tempfile
import unittest

ARM = (
    Path(__file__).resolve().parents[3]
    / "traces/blackwell_ab_20261023/ROUND_B1_1830/run_kit/arm.sh"
)


class SavedObservationHold(unittest.TestCase):
    def should_hold(self, enable, diagnostic=""):
        source = ARM.read_text()
        start = source.index("if grep -q '\"code\":0'")
        condition = source[start:source.index(" start_seconds=$SECONDS", start)]
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            (base / "diag.txt").write_text(diagnostic)
            (base / "early.txt").write_text("")
            (base / "enable.txt").write_text(enable)
            for suffix, target in [
                ("arm_${run}_diag.txt", "diag.txt"),
                ("arms/$run/early_code0.txt", "early.txt"),
                ("arm_${run}_enable.txt", "enable.txt"),
            ]:
                condition = condition.replace("/root/prov/" + suffix, str(base / target))
            result = subprocess.run(
                ["bash", "-c", condition + " echo HOLD; else echo SKIP; fi"],
                capture_output=True, text=True, check=True,
            )
        return result.stdout.strip() == "HOLD"

    def test_actual_enable_code0_starts_hold_after_diagnostic_ssh_disconnect(self):
        enable = '{"DriverVersion":"32.0.15.8088","ConfigManagerErrorCode":0}\n'
        self.assertTrue(self.should_hold(enable, "Connection closed by remote host"))

    def test_actual_enable_code0_starts_hold_after_diagnostic_timeout(self):
        enable = 'ENABLE580_PNPUTIL_RC=0\n'
        enable += '{"DriverVersion":"32.0.15.8088","ConfigManagerErrorCode":0}\n'
        self.assertTrue(self.should_hold(enable))

    def test_command_rc_alone_never_starts_hold(self):
        self.assertFalse(self.should_hold("ENABLE580_PNPUTIL_RC=0\n"))

    def test_nonzero_device_status_never_starts_hold(self):
        enable = '{"DriverVersion":"32.0.15.8088","ConfigManagerErrorCode":43}\n'
        self.assertFalse(self.should_hold(enable))


if __name__ == "__main__":
    unittest.main()
