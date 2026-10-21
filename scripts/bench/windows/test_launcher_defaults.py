#!/usr/bin/env python3
"""GPU-free checks of the exported Windows flags; never runs QEMU or a launcher."""
import os
from pathlib import Path
import subprocess
import unittest

SOURCE = Path(__file__).with_name('windows_broker.sh')
DIAGNOSTICS = {'KF3_RPC_TRACE', 'KF3_BAR0_TRACE', 'KF3_MAPLOG',
               'KF3_DISPLAY_METHOD_TRACE'}


def flags(extra=''):
    lines = SOURCE.read_text().splitlines()
    assignments = [next(line for line in lines if line.startswith(prefix))
                   for prefix in ('FLAGS=', 'ENV=(')]
    script = '\n'.join(assignments) + '\nprintf "%s\\0" "${ENV[@]}"'
    result = subprocess.run(['bash', '-eu', '-c', script],
                            env={'PATH': os.defpath, 'WIN_FLAGS': extra},
                            capture_output=True, check=True)
    return dict(entry.decode().split('=', 1)
                for entry in result.stdout.split(b'\0') if entry)


class LauncherDefaults(unittest.TestCase):
    def test_default_launch_arms_no_measurement_and_no_dead_timer_switch(self):
        selected = flags()
        self.assertFalse(DIAGNOSTICS.intersection(selected))
        self.assertNotIn('KF3_TIMER_MAP', selected)

    def test_requesting_one_diagnostic_does_not_arm_any_others(self):
        selected = flags('KF3_PROF=1')
        self.assertEqual(selected['KF3_PROF'], '1')
        self.assertFalse(DIAGNOSTICS.intersection(selected))

    def test_each_diagnostic_stays_explicitly_available(self):
        selected = flags(' '.join(name + '=1' for name in sorted(DIAGNOSTICS)))
        self.assertTrue(all(selected[name] == '1' for name in DIAGNOSTICS))

    def test_the_existing_research_behavior_group_stays_selected(self):
        selected = flags()
        for name in ('KF3_KERNEL_GR_CE', 'KF3_KERNEL_GR_WORK',
                     'KF3_DEFERRED_API', 'KF3_TRANSLATED_CE_RELAY'):
            self.assertEqual(selected[name], '1')

    def test_the_removed_timer_flag_has_no_launcher_assignment(self):
        self.assertNotIn('KF3_TIMER_MAP', flags())


if __name__ == '__main__':
    unittest.main()
