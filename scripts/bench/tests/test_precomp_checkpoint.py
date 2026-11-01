"""Exercise the actual controller checkpoint with native tuples and JSON arrays."""

import ast
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import native_xid_guard as N

CONTROLLER = (
    Path(__file__).resolve().parents[3]
    / "traces/blackwell_provider_validation_20261023/run_kit/controller.sh"
)


class PrecompCheckpoint(unittest.TestCase):
    def setUp(self):
        self.lines = [
            f"2026-10-23T01:00:0{i}+0000 fixture kernel: NVRM: Xid "
            f"(PCI:0000:00:07): 31, pid=100, channel 0x{i:08x}"
            for i in range(5)
        ]
        script = CONTROLLER.read_text().split("python3 - <<'POST_PRECOMP'\n", 1)[1]
        script = script.split("\nPOST_PRECOMP", 1)[0]
        checks = [n for n in ast.walk(ast.parse(script)) if isinstance(n, ast.Assert)]
        self.assertEqual(len(checks), 1)
        self.check = compile(ast.Expression(checks[0].test), str(CONTROLLER), "eval")

    def accepted(self, actual, approved=None):
        journal = "kernel snapshot\n" + "\n".join(actual)
        api = SimpleNamespace(xid_lines=N.xid_lines, kernel_snapshot=lambda: journal)
        approved = json.loads(json.dumps(self.lines)) if approved is None else approved
        return eval(self.check, {"G": SimpleNamespace(N=api), "allowed": approved})

    def test_actual_native_tuple_matches_json_approved_array(self):
        self.assertTrue(self.accepted(self.lines))

    def test_same_full_receipt_survives_another_pci_and_pid(self):
        lines = [s.replace("00:07", "00:08").replace("pid=100", "pid=200") for s in self.lines]
        self.assertTrue(self.accepted(lines, list(lines)))

    def test_tuple_receipt_also_preserves_exact_matching(self):
        self.assertTrue(self.accepted(self.lines, tuple(self.lines)))

    def test_an_unmatched_xid32_still_refuses(self):
        self.assertFalse(self.accepted(self.lines + [self.lines[0].replace(": 31,", ": 32,")]))

    def test_an_unrelated_31_still_refuses(self):
        self.assertFalse(self.accepted(self.lines + [self.lines[0].replace("pid=100", "pid=300")]))

    def test_a_replaced_fault_address_or_channel_refuses(self):
        changed = list(self.lines)
        changed[-1] += " unexpected-address"
        self.assertFalse(self.accepted(changed))

    def test_an_extra_identical_fault_is_not_deduplicated(self):
        self.assertFalse(self.accepted(self.lines + [self.lines[0]]))

    def test_reordered_faults_do_not_silently_match(self):
        self.assertFalse(self.accepted(list(reversed(self.lines))))

    def test_a_missing_fault_refuses(self):
        self.assertFalse(self.accepted(self.lines[:-1]))

    def test_four_matching_receipt_rows_cannot_replace_the_five_controls(self):
        self.assertFalse(self.accepted(self.lines[:-1], self.lines[:-1]))

    def test_an_empty_matching_snapshot_is_not_the_native_baseline(self):
        self.assertFalse(self.accepted([], []))


if __name__ == "__main__":
    unittest.main()
