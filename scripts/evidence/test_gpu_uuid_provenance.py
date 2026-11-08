"""Independent cross-reference checks for the corrected GA106 evidence prose."""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2] / "traces/real_ga106"
FULL = re.compile(r"GPU-([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})", re.I)


class ProvenanceTests(unittest.TestCase):
    def test_both_provenance_references_name_the_table_board(self):
        lines = (ROOT / "README.md").read_text().splitlines()
        table = next(line for line in lines if line.startswith("| part |"))
        board = FULL.search(table).group(1)
        references = [line for line in lines if "provenance table above" in line]
        self.assertEqual(len(references), 2)
        for line in references:
            match = FULL.search(line)
            self.assertIsNotNone(
                match, "provenance reference must name a full synthetic identity"
            )
            self.assertEqual(match.group(1), board)

    def test_r18_summary_names_the_same_board_as_its_smi_capture(self):
        lines = (ROOT / "rmladder_r18_cecaps_real_ga106.txt").read_text().splitlines()
        summary = FULL.search(lines[0])
        self.assertIsNotNone(summary, "R18 summary must name a full synthetic identity")
        smi = FULL.search(next(line for line in lines if "nvidia-smi -L:" in line))
        self.assertEqual(summary.group(1), smi.group(1))

    def test_second_board_is_distinct_and_both_sections_agree(self):
        lines = (ROOT / "README.md").read_text().splitlines()
        first = FULL.search(
            next(line for line in lines if line.startswith("| part |"))
        ).group(1)
        second = FULL.search(
            next(line for line in lines if "SECOND, DIFFERENT" in line)
        ).group(1)
        addition = FULL.search(
            next(line for line in lines if "All read from" in line)
        ).group(1)
        self.assertNotEqual(first, second)
        self.assertEqual(second, addition)
