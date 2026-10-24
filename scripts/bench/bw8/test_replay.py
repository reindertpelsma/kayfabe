"""Retained outcome metadata in synthetic cadence, not a new production/hardware run.

BW6 lease57556167: public revision2cf144fa259c5574275e57d16a8401352e5cb5a7,
ROUND_B1r_bw6/attempt1/arms/on_fix/bw6_result.json. BW5 lease57503040:
ROUND_B1q_bw5 on the same publication lineage, reported in BW3 RoundB1q.
Only dump codes/channel distinctions are replayed. Cadence and complete rows are synthetic.
"""

import unittest

from observer import evaluate
from test_observer import control, event


class ReplayTests(unittest.TestCase):
    def test_bw6_engine_timeout_metadata(self):
        rows = control()
        rows[3]["dumps"] = [{"id": "bw6-engine-timeout", "code": "0x141"}]
        result = evaluate(rows)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["reason"], "new engine timeout/fatal dump")

    def test_bw6_fatal_dump_metadata(self):
        rows = control()
        rows[3]["dumps"] = [{"id": "bw6-fatal", "code": "0x113"}]
        result = evaluate(rows)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["reason"], "new engine timeout/fatal dump")

    def test_bw6_system_bugcheck_metadata(self):
        rows = control()
        rows[3]["events"] = [event()]
        result = evaluate(rows)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["reason"], "System bugcheck/reboot/TDR since enable")

    def test_bw5_nonfatal_metadata_control(self):
        rows = control()
        rows[3]["events"] = [event("Application", "Windows Error Reporting", rid=201)]
        rows[3]["dumps"] = [
            {"id": "bw5-" + code, "code": code} for code in ("0x193", "0x1a8", "0x1b8")
        ]
        result = evaluate(rows)
        self.assertEqual(result["verdict"], "REGRESSION_PASS")
        self.assertEqual(result["acceptance"], "UNRUN")


if __name__ == "__main__":
    unittest.main()
