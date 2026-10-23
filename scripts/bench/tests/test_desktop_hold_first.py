# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""An optional hold-first arm must observe600s before dispatching apps."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PATH = Path(__file__).resolve().parents[1] / 'diesweep/desktop/matrix.py'


class HoldFirstTests(unittest.TestCase):
    def observe(self, seconds=600):
        spec = importlib.util.spec_from_file_location('hold_first_matrix', PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.calls = []
        def app(row):
            self.calls.append('app')
            return {'verdict': 'PASS', 'restart': ''}
        def hold():
            self.calls.append('hold')
            return seconds
        return module.run_plan(SimpleNamespace(run_app=app), [{'id': 'fixture'}],
                               lambda result: None, lambda: self.calls.append('health'),
                               hold, hold_first=True)

    def test_explicit_hold_precedes_first_application(self):
        self.assertEqual(self.observe(), 0)
        self.assertLess(self.calls.index('hold'), self.calls.index('app'))

    def test_completed_first_hold_is_not_repeated(self):
        self.observe()
        self.assertEqual(self.calls.count('hold'), 1)
        self.assertEqual(self.calls[-1], 'health')

    def test_short_first_hold_refuses_before_app_dispatch(self):
        with self.assertRaisesRegex(RuntimeError, 'incomplete stability hold'):
            self.observe(599)
        self.assertNotIn('app', self.calls)
