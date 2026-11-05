#!/usr/bin/env python3
"""Exercise the launcher's argument builder without starting a VM or a broker."""
import os
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'display/interactive.sh'


class GuestDriver(unittest.TestCase):
    def arguments(self, driver=None):
        source = SCRIPT.read_text()
        builder = 'qemu_args() {' + source.split('qemu_args() {', 1)[1].split('\nbanner()', 1)[0]
        fixture = '''
set -uo pipefail
SOCK_OF() { echo /fixture/display.sock; }
cp() { :; }
die() { echo "$*" >&2; exit 2; }
IMG=/fixture/guest.qcow2
SUID=1234
'''
        env = os.environ.copy()
        for key in list(env):
            if key.startswith('KF_') or key.startswith('KF3_'):
                env.pop(key)
        if driver is not None:
            env['KF_GUEST_DRIVER'] = driver
        return subprocess.run(['bash', '-c', fixture + builder +
            '\nqemu_args /fixture/run || exit $?\nprintf "%s\\0" "${QARGS[@]}"'],
            env=env, capture_output=True, timeout=5)

    def test_default_retains_the_pinned_580_guest_abi(self):
        result = self.arguments()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'guest-driver=580.159.04,', result.stdout)

    def test_595_guest_selects_595_even_with_an_independent_host(self):
        result = self.arguments('595.91.07')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'guest-driver=595.91.07,', result.stdout)
        self.assertNotIn(b'guest-driver=580.', result.stdout)

    def test_a_driver_cannot_inject_another_qom_property(self):
        for invalid in ('', '595.91.07,display=off', '595.91.07 extra', 'latest'):
            with self.subTest(driver=invalid):
                result = self.arguments(invalid)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertEqual(result.stdout, b'')


if __name__ == '__main__':
    unittest.main()
