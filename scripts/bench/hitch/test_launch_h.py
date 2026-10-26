#!/usr/bin/env python3
"""GPU-free contract for the hitch launcher's extra flags; never runs a launcher."""
import os
from pathlib import Path
import subprocess
import unittest

SOURCE = Path(__file__).with_name('launch_h.sh')


def flags(value=None):
    line = next(line for line in SOURCE.read_text().splitlines()
                if line.startswith('export KF3_REV='))
    env = {'PATH': os.defpath}
    if value is not None:
        env['WIN_FLAGS'] = value
    result = subprocess.run(['bash', '-eu', '-c',
                             'REV=fixture\n' + line + '\nprintf "%s" "$WIN_FLAGS"'],
                            env=env, text=True, capture_output=True, check=True)
    return result.stdout


class ExtraFlags(unittest.TestCase):
    def test_unset_extra_flags_are_empty(self):
        self.assertEqual(flags(), '')

    def test_explicit_empty_extra_flags_stay_empty(self):
        self.assertEqual(flags(''), '')

    def test_explicit_research_arm_is_preserved(self):
        self.assertEqual(flags('KF3_LIFECYCLE_MODE=always_scan'),
                         'KF3_LIFECYCLE_MODE=always_scan')


if __name__ == '__main__':
    unittest.main()
