#!/usr/bin/env python3
"""Cargo's expected failed-test exit must not be mistaken for a build fault."""
import importlib.util
from pathlib import Path
import unittest

class MutationResult(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[1] / 'mutation_result.py'
        spec = importlib.util.spec_from_file_location('mutation_result', path)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def test_expected_cargo_test_diagnostic_is_a_kill(self):
        log = 'test one ... FAILED\ntest two ... FAILED\ntest result: FAILED. 0 passed; 2 failed; 0 ignored;\nerror: test failed, to rerun pass --test focused\n'
        self.assertEqual(self.module.classify(101, log)['status'], 'killed')

    def test_build_errors_are_faults_even_with_stale_failure_lines(self):
        log = 'test one ... FAILED\ntest two ... FAILED\nerror[E0432]: unresolved import\nerror: could not compile package\n'
        self.assertEqual(self.module.classify(101, log)['status'], 'fault')

    def test_harness_errors_and_missing_results_are_faults(self):
        for log in ['error: test failed, to rerun pass --test focused\n',
                    'test one ... FAILED\ntest two ... FAILED\nerror: test failed: process exited with signal 6\n']:
            self.assertEqual(self.module.classify(101, log)['status'], 'fault')

    def test_green_or_insufficient_kills_do_not_pass(self):
        self.assertEqual(self.module.classify(0, 'test result: ok. 2 passed; 0 failed; 0 ignored;')['status'], 'survived')
        self.assertEqual(self.module.classify(101, 'test one ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;')['status'], 'survived')

if __name__ == '__main__':
    unittest.main()
