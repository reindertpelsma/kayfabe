"""Named canonical killers must parse both supported Python unittest formats."""
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import vast_box_mutants as mutants


class MutationOutput(unittest.TestCase):
    def parsed(self, text):
        result = SimpleNamespace(returncode=1, stdout=text, stderr="")
        with patch.object(mutants.subprocess, "run", return_value=result):
            return mutants.run_tests(Path("/fixture"), ["EnvValidation.test_one"])[1]

    def test_python310_fail_subtest_counts_the_named_method(self):
        text = "FAIL: test_one (test_vast_box.EnvValidation) (name='KF_PROTOCOL', value='3')\n"
        self.assertEqual(self.parsed(text), {"EnvValidation.test_one"})

    def test_python310_error_counts_the_named_method(self):
        self.assertEqual(
            self.parsed("ERROR: test_two (test_vast_box.Onstart)\n"), {"Onstart.test_two"}
        )

    def test_python314_fail_counts_the_named_method(self):
        self.assertEqual(
            self.parsed("FAIL: test_one (test_vast_box.EnvValidation.test_one)\n"),
            {"EnvValidation.test_one"},
        )

    def test_python314_error_subtest_counts_the_named_method(self):
        text = "ERROR: test_two (test_vast_box.Onstart.test_two) (case='bad')\n"
        self.assertEqual(self.parsed(text), {"Onstart.test_two"})

    def test_fail_with_a_different_inner_method_is_not_a_killer(self):
        self.assertEqual(
            self.parsed("FAIL: test_one (test_vast_box.EnvValidation.test_other)\n"), set()
        )

    def test_error_with_a_different_inner_method_is_not_a_killer(self):
        self.assertEqual(
            self.parsed("ERROR: test_two (test_vast_box.Onstart.test_other)\n"), set()
        )

    def test_fail_from_another_module_is_not_a_killer(self):
        self.assertEqual(self.parsed("FAIL: test_one (unrelated.EnvValidation.test_one)\n"), set())

    def test_error_from_another_module_is_not_a_killer(self):
        self.assertEqual(self.parsed("ERROR: test_two (unrelated.Onstart)\n"), set())

    def test_process_traceback_is_not_a_named_killer(self):
        self.assertEqual(self.parsed("Traceback (most recent call last):\nRuntimeError: stopped\n"), set())
