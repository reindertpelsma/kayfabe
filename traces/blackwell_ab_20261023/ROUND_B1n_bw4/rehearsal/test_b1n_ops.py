"""B1n operator offer scope: only dies with a pinned observation pair; never 'deverified'."""

import ast
import json
import os
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
WS = Path(os.environ.get("KF_BW3_D", "/workspace/bw4-round-b1n-a1-run"))
OPS = Path(os.environ.get("KF_BW4_OPS", WS / "ops.py"))
AUTH = WS / "owner_authority.json"
sys.path.insert(0, str(HERE.parents[3] / "scripts/bench/blackwell"))
import saved_xid32_observation as O  # noqa: E402


class OfferScope(unittest.TestCase):
    def setUp(self):
        self.source = OPS.read_text()
        self.tree = ast.parse(self.source)

    def skus(self):
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == "skus":
                return ast.literal_eval(node.value)
        raise AssertionError("skus table missing")

    def test_every_searched_die_has_a_pinned_pair(self):
        skus = self.skus()
        want = {"RTX_5070": "GB205", "RTX_5090": "GB202"}
        if AUTH.exists():
            want = json.loads(AUTH.read_text()).get("skus", want)
        self.assertEqual(skus, want)
        self.assertTrue(set(skus.values()) <= set(O.DIE_PAIRS))

    def test_deverified_hosts_are_not_admitted(self):
        self.assertIn('x.get("verification") in ("verified", "unverified")', self.source)
        self.assertNotIn('"unverified", "deverified")', self.source)
        self.assertIn('not x.get("is_vm_deverified", False)', self.source)

    def test_budget_and_reserve_are_the_owner_values(self):
        self.assertEqual(self.source.count("stop_usd=2.0, cap_usd=2.5"), 3)
        self.assertIn('"credit_reserve": 8', self.source)
        self.assertIn('float(x.get("dph_total", 99)) <= 2.50', self.source)

    def test_denied_countries_and_label(self):
        self.assertIn('{"CU", "IR", "KP", "SY", "CN", "HK", "MO"}', self.source)
        self.assertRegex(self.source, r'"claude-bw4-B1[no]1-OBSERVATION-20261024"')

    def test_owner_authority_is_checked_against_the_guard_table(self):
        self.assertIn(
            'authority["die_pairs"] == {k: list(v) for k, v in SX.DIE_PAIRS.items()}', self.source
        )


if __name__ == "__main__":
    unittest.main()
