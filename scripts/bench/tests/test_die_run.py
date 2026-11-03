#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A failed Windows evidence/setup call must fail its run, even if screenshots succeed."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'diesweep'


class DieRunTests(unittest.TestCase):
    def run_phase(self, phase, failure=''):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / 'die_run.sh'
            shutil.copyfile(SOURCE / 'die_run.sh', script)
            (root / 'phase1.ps1.in').write_text('fixture @PW@\n')
            (root / 'win_die.sh').write_text('''#!/bin/bash
if [[ $1 == ps && ${2##*/} == "$FAIL_NAME" ]]; then exit 77; fi
exit 0
''')
            bin_dir = root / 'bin'
            bin_dir.mkdir()
            for name in ('sleep', 'nvidia-smi'):
                p = bin_dir / name
                p.write_text('#!/bin/bash\nexit 0\n')
                p.chmod(0o700)
            env = dict(os.environ, PATH=str(bin_dir) + ':' + os.environ['PATH'],
                       WIN_DIR=str(root / 'win'), WIN_RUN=str(root / 'run'),
                       PHASE2_SECONDS='0', FAIL_NAME=failure)
            result = subprocess.run(['bash', str(script), phase], env=env,
                                    capture_output=True, text=True, timeout=10)
            log = (root / 'run' / (phase + '.log')).read_text()
            return result.returncode, log

    def test_evidence_failure_fails_phase_one(self):
        rc, log = self.run_phase('phase1', 'evidence.ps1')
        self.assertEqual(rc, 77, log)
        self.assertIn('DIE_RUN_EXIT phase1 rc=77', log)

    def test_driver_binding_failure_fails_phase_one(self):
        rc, log = self.run_phase('phase1', 'drvbind.ps1')
        self.assertEqual(rc, 77, log)

    def test_final_evidence_failure_fails_phase_two(self):
        rc, log = self.run_phase('phase2', 'evidence.ps1')
        self.assertEqual(rc, 77, log)

    def test_successful_phases_return_zero(self):
        for phase in ('phase1', 'phase2'):
            with self.subTest(phase=phase):
                rc, log = self.run_phase(phase)
                self.assertEqual(rc, 0, log)
                self.assertIn('DIE_RUN_EXIT ' + phase + ' rc=0', log)


if __name__ == '__main__':
    unittest.main()
