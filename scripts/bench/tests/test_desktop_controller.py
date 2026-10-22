# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Execute the controller's real shell flow with recording phase implementations."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

CONTROLLER = Path(__file__).resolve().parents[1] / 'diesweep/desktop/controller.sh'


class ControllerTests(unittest.TestCase):
    def run_controller(self, failure=''):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'config.sh').write_text('S4_SOURCE=fixture\n')
            env = dict(os.environ, S4_HOME=str(root), S4_PROV=str(root / 'prov'), FAILURE=failure,
                       CONTROLLER=str(CONTROLLER))
            script = '''set -euo pipefail
source "$CONTROLLER"
s4_step() { echo "STEP $1"; [[ $1 != "$FAILURE" ]] || return 42; }
s4_watch() { echo 'WATCH_STARTED'; }
s4_controller
'''
            return subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True, timeout=5)

    def test_success_runs_native_then_watched_windows(self):
        p = self.run_controller()
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertLess(p.stdout.index('STEP native'), p.stdout.index('WATCH_STARTED'))
        self.assertLess(p.stdout.index('WATCH_STARTED'), p.stdout.index('STEP install'))

    def test_watch_is_required_before_bind_and_desktop(self):
        p = self.run_controller()
        self.assertEqual(p.stdout.count('WATCH_STARTED'), 1)
        self.assertLess(p.stdout.index('WATCH_STARTED'), p.stdout.index('STEP bind'))
        self.assertLess(p.stdout.index('STEP bind'), p.stdout.index('STEP desktop'))

    def test_native_failure_never_starts_windows(self):
        p = self.run_controller('native')
        self.assertEqual(p.returncode, 42)
        self.assertNotIn('STEP install', p.stdout)
        self.assertNotIn('WATCH_STARTED', p.stdout)

    def test_bind_failure_propagates(self):
        self.assertEqual(self.run_controller('bind').returncode, 42)

    def test_bind_failure_prevents_gpu_app_work(self):
        self.assertNotIn('STEP desktop', self.run_controller('bind').stdout)

    def test_desktop_failure_does_not_claim_completion(self):
        p = self.run_controller('desktop')
        self.assertEqual(p.returncode, 42)
        self.assertNotIn('S4_COMPLETE', p.stdout)
