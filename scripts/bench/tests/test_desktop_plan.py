# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The attached Windows campaign must preserve failures and finish its stability hold."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PATH = Path(__file__).resolve().parents[1] / 'diesweep/desktop/matrix.py'


def load():
    spec = importlib.util.spec_from_file_location('desktop_matrix', PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DesktopPlanTests(unittest.TestCase):
    def run_plan(self, verdict='PASS', restart='', fail_health_at=None, seconds=600):
        m = load()
        calls = []
        count = 0
        def health():
            nonlocal count
            count += 1
            calls.append('health')
            if count == fail_health_at:
                raise RuntimeError('display fault')
        def app(a):
            calls.append(a['id'])
            return dict(app=a['id'], verdict=verdict, restart=restart)
        def hold():
            calls.append('hold')
            return seconds
        records = []
        session = SimpleNamespace(run_app=app)
        return m.run_plan(session, [{'id':'heaven_d3d11'}, {'id':'valley_opengl'}],
                          records.append, health, hold), calls, records

    def test_unigine_rows_run_before_hold(self):
        rc, calls, records = self.run_plan()
        self.assertEqual(rc, 0)
        self.assertLess(calls.index('heaven_d3d11'), calls.index('hold'))
        self.assertLess(calls.index('valley_opengl'), calls.index('hold'))
        self.assertEqual(len(records), 2)

    def test_failure_verdict_is_not_a_pass(self):
        self.assertEqual(self.run_plan(verdict='FAIL')[0], 1)

    def test_notrun_is_not_a_pass(self):
        self.assertEqual(self.run_plan(verdict='NOTRUN')[0], 1)

    def test_timeout_does_not_start_more_gpu_work(self):
        with self.assertRaisesRegex(RuntimeError, 'app-timeout'):
            self.run_plan(verdict='TIMEOUT', restart='app-timeout')

    def test_restart_reason_refuses_a_passed_row(self):
        with self.assertRaisesRegex(RuntimeError, 'guest-rebooted'):
            self.run_plan(restart='guest-rebooted')

    def test_health_failure_before_apps_is_fatal(self):
        with self.assertRaisesRegex(RuntimeError, 'display fault'):
            self.run_plan(fail_health_at=1)

    def test_health_failure_after_app_is_fatal(self):
        with self.assertRaisesRegex(RuntimeError, 'display fault'):
            self.run_plan(fail_health_at=2)

    def test_short_hold_is_not_accepted(self):
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_plan(seconds=599)

    def test_missing_hold_is_not_accepted(self):
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_plan(seconds=0)
