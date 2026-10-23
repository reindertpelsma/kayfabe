"""B1b repeats are findings, while unapproved host faults still stop immediately."""

import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "fuzz_findings", Path(__file__).with_name("fuzz_findings.py")
)
F = None
if Path(SPEC.origin).exists():
    F = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(F)


def sample(arm="crowd", violation=False):
    pin = "crowd" if arm == "crowd" else "unpinned"
    grade = "FAIL" if violation else "PASS"
    text = f"FUZZ_ARM={arm} verdict={grade} threads=9 pin={pin} cores=3 viol={int(violation)}\n"
    if violation:
        text += f"FUZZ_VIOLATION=ALIAS_REVOKED arm={arm} n=1\n"
    return text


class FuzzFindings(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(F, "B1b findings protocol missing")

    def run_batch(self, fail_indices=(), safety_at=None):
        calls = []

        def execute(arm, index):
            calls.append((arm, index))
            return int(index in fail_indices), sample(arm, index in fail_indices)

        def safety(index):
            if index == safety_at:
                raise RuntimeError("unapproved host fault")

        return F.batch(execute, safety), calls

    def test_exactly_three_pinned_samples_and_one_same_width_control(self):
        result, calls = self.run_batch()
        self.assertEqual(calls, [("crowd", 0), ("crowd", 1), ("crowd", 2), ("crowd-free", 3)])
        self.assertEqual(result["pinned_samples"], 3)

    def test_two_failures_are_a_native_finding_and_do_not_gate_windows(self):
        result, calls = self.run_batch((0, 1))
        self.assertTrue(result["native_finding"])
        self.assertEqual(result["alias_revoked_recurrences"], 2)
        self.assertTrue(result["continue_windows"])
        self.assertEqual(len(calls), 4)

    def test_three_failures_still_continue_after_the_control(self):
        result, calls = self.run_batch((0, 1, 2))
        self.assertTrue(result["continue_windows"])
        self.assertEqual(result["alias_revoked_recurrences"], 3)
        self.assertEqual(calls[-1], ("crowd-free", 3))

    def test_one_failure_does_not_meet_the_two_of_three_falsifier(self):
        result, calls = self.run_batch((1,))
        self.assertFalse(result["native_finding"])
        self.assertTrue(result["continue_windows"])

    def test_failed_control_is_recorded_without_blocking_windows(self):
        result, calls = self.run_batch((3,))
        self.assertEqual(result["records"][-1]["returncode"], 1)
        self.assertTrue(result["continue_windows"])

    def test_safety_fault_stops_before_another_sample(self):
        with self.assertRaisesRegex(RuntimeError, "unapproved"):
            self.run_batch(safety_at=1)

    def test_safety_fault_after_final_sample_still_stops_windows(self):
        with self.assertRaisesRegex(RuntimeError, "unapproved"):
            self.run_batch(safety_at=4)

    def test_wrong_width_does_not_count_as_a_valid_pinned_sample(self):
        row = F.parse(sample().replace("threads=9", "threads=3"), "crowd", 0)
        self.assertFalse(row["valid_sample"])

    def test_wrong_pin_does_not_count_as_a_valid_pinned_sample(self):
        row = F.parse(sample().replace("pin=crowd", "pin=unpinned"), "crowd", 0)
        self.assertFalse(row["valid_sample"])

    def test_unparsed_log_is_retained_and_does_not_create_a_clean_finding(self):
        result = F.batch(lambda arm, index: (1, "unparsed raw output"), lambda index: None)
        self.assertEqual(result["valid_pinned_samples"], 0)
        self.assertIsNone(result["native_finding"])
        self.assertTrue(result["continue_windows"])


if __name__ == "__main__":
    unittest.main()
