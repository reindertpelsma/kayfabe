#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Synthetic metadata regressions; no captured driver values or reply bodies."""
import csv
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


TOOL = Path(__file__).resolve().parents[1] / "align_metadata.py"
WIRE = ["dir", "element_sequence", "function", "control_or_class",
        "rpc_result", "control_status"]
PRETTY = ["ordinal", "function", "control_or_class", "effective_result"]


def command(seq, fn=76, selector="0x42", envelope="0x0", body="-"):
    request = dict(zip(WIRE, ["cmd", str(seq), str(fn), selector, "0x0", "-"]))
    response = dict(zip(WIRE, ["status", str(seq + 10000), str(fn),
                               selector, envelope, body]))
    return request, response


def pretty(seq, fn=76, selector="0x42", result="0x0"):
    return dict(zip(PRETTY, [str(seq), str(fn), selector, result]))


class AlignmentTests(unittest.TestCase):
    def run_alignment(self, left, right, left_fields=WIRE, right_fields=PRETTY):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / n for n in ("left.tsv", "right.tsv", "out.tsv")]
            for path, fields, rows in zip(paths, [left_fields, right_fields], [left, right]):
                with path.open("w") as f:
                    w = csv.DictWriter(f, fields, delimiter="\t", lineterminator="\n")
                    w.writeheader()
                    w.writerows(rows)
            result = subprocess.run([sys.executable, str(TOOL), *map(str, paths)],
                                    capture_output=True, text=True, timeout=10)
            rows = list(csv.DictReader(paths[2].open(), delimiter="\t")) if paths[2].exists() else []
            return result, rows

    def test_alignment_continues_after_native_selected_call_500(self):
        pairs = [command(i, selector=hex(i + 0x100)) for i in range(560)]
        left = [p[0] for p in pairs] + [p[1] for p in pairs]
        result, rows = self.run_alignment(left, [pretty(207, selector=hex(0x100 + 559))])
        self.assertEqual(result.returncode, 0, result.stderr)
        matched = [r for r in rows if r.get("virtual_ordinal") == "207"]
        self.assertEqual(len(matched), 1)
        self.assertEqual(matched[0]["native_command_sequence"], "559")

    def test_all_calls_and_both_unmatched_sides_remain_visible(self):
        pairs = [command(0), command(1, selector="0x43"), command(2, fn=10, selector="-"),
                 command(3, fn=70, selector="-"), command(4, selector="0x44")]
        left = [p[0] for p in pairs] + [p[1] for p in pairs]
        right = [pretty(0), pretty(1, selector="0x99"), pretty(2, fn=10, selector="-"),
                 pretty(3, fn=70, selector="-")]
        result, rows = self.run_alignment(left, right)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([r["virtual_ordinal"] for r in rows if r["virtual_ordinal"] != "-"],
                         ["0", "1", "2", "3"])
        self.assertEqual([r["native_command_sequence"] for r in rows
                          if r["native_command_sequence"] != "-"], ["0", "1", "2", "3", "4"])
        self.assertEqual({r["relation"] for r in rows}, {"match", "native_only", "virtual_only"})

    def test_numeric_key_spelling_does_not_create_a_divergence(self):
        pair = command(0, selector="0x00000042")
        result, rows = self.run_alignment(list(pair), [pretty(0, fn="0x4c", selector="66")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["relation"], "match")

    def test_envelope_and_body_status_are_preserved_separately(self):
        pair = command(0, body="0x56")
        result, rows = self.run_alignment(list(pair), [pretty(0, result="0x56")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows[0]["native_envelope"], "0x0")
        self.assertEqual(rows[0]["native_control_status"], "0x56")
        self.assertEqual(rows[0]["effective_status_equal"], "1")
        self.assertEqual(rows[0]["virtual_envelope"], "-")

    def test_repeated_api_keys_are_marked_ambiguous(self):
        pairs = [command(0), command(1)]
        result, rows = self.run_alignment([p[0] for p in pairs] + [p[1] for p in pairs],
                                          [pretty(0), pretty(1)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([r["api_key_repeated"] for r in rows], ["1", "1"])

    def test_a_missing_reply_is_unknown_not_success(self):
        request, _ = command(0)
        result, rows = self.run_alignment([request], [pretty(0)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows[0]["native_effective_result"], "unknown")
        self.assertEqual(rows[0]["effective_status_equal"], "unknown")

    def test_successful_reply_key_conflict_is_rejected(self):
        request, reply = command(0)
        reply["control_or_class"] = "0x43"
        result, rows = self.run_alignment([request, reply], [pretty(0)])
        self.assertEqual(result.returncode, 2)
        self.assertIn("reply key conflict", result.stderr)
        self.assertEqual(rows, [])

    def test_pretty_to_pretty_supports_an_ada_display_reference(self):
        result, rows = self.run_alignment([pretty(8), pretty(9, fn=10, selector="-")],
                                          [pretty(2), pretty(3, fn=10, selector="-")],
                                          left_fields=PRETTY)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([r["relation"] for r in rows], ["match", "match"])

    def test_duplicate_command_ordinal_is_rejected(self):
        result, rows = self.run_alignment([pretty(0), pretty(0)], [pretty(0)],
                                          left_fields=PRETTY)
        self.assertEqual(result.returncode, 2)
        self.assertIn("command order", result.stderr)
        self.assertEqual(rows, [])


if __name__ == "__main__":
    unittest.main()
