"""B1j (controller 04:20): named per-lease budget override and most-cores selection.

Run: KF_BW3_D=<workspace> python3 -m unittest test_budget_b1j
"""

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw6-round-b1r-a1-run"))
BW = Path("/workspace/bw6-wt/scripts/bench/blackwell")
TB = Path("/workspace/bw6-wt/traces/blackwell_ab_20261023")
sys.path.insert(0, str(BW))
sys.path.insert(0, str(D))
import round_budget as B  # noqa: E402
import budget_override as BO  # noqa: E402

NOW = dt.datetime(2026, 10, 24, 2, 30, tzinfo=dt.timezone.utc)


class Defaults(unittest.TestCase):
    def test_module_defaults_unchanged(self):
        self.assertEqual((B.STOP_USD, B.CAP_USD, B.MAX_RATE), (2.50, 3.00, 2.50))
        self.assertEqual((BO.DEFAULT_STOP_USD, BO.DEFAULT_CAP_USD), (2.0, 2.5))

    def test_default_plan_refuses_raised_stop(self):
        with self.assertRaises(ValueError):
            B.plan(0.869, 27.6, NOW, stop_usd=3.0, cap_usd=3.5)

    def test_default_floor_rejects_the_rtx5090_offer(self):
        plan = B.plan(0.869, 27.6, NOW, stop_usd=2.0, cap_usd=2.5)
        self.assertLess(plan["seconds"], 90 * 60)


class Override(unittest.TestCase):
    def test_override_values_are_exact(self):
        self.assertEqual((BO.STOP_USD, BO.CAP_USD, BO.MAX_RATE), (3.0, 3.5, 1.0))
        self.assertEqual(BO.RESERVE_USD, 8.0)

    def test_floor_uses_the_lease_stop(self):
        plan = BO.plan(0.869, 27.6, NOW)
        self.assertGreaterEqual(plan["seconds"], 90 * 60)
        self.assertLess(plan["reservation_usd"], 3.0)
        self.assertEqual((plan["stop_usd"], plan["cap_usd"]), (3.0, 3.5))

    def test_0_94_offer_also_qualifies(self):
        self.assertGreaterEqual(BO.plan(0.936, 27.6, NOW)["seconds"], 90 * 60)

    def test_price_cap_one_dollar(self):
        self.assertTrue(BO.price_ok({"dph_total": 0.936}))
        self.assertFalse(BO.price_ok({"dph_total": 1.454}))
        self.assertFalse(BO.price_ok({"dph_total": 1.0001}))

    def test_module_constants_restored(self):
        BO.plan(0.869, 27.6, NOW)
        self.assertEqual((B.STOP_USD, B.CAP_USD), (2.50, 3.00))

    def test_reserve_still_binds(self):
        with self.assertRaises(ValueError):
            BO.plan(0.869, 9.0, NOW)

    def test_default_ceiling_back_in_force_after_override(self):
        BO.plan(0.869, 27.6, NOW)
        with self.assertRaises(ValueError):
            B.plan(0.869, 27.6, NOW, stop_usd=3.0, cap_usd=3.5)

    def test_plan_refuses_above_one_dollar(self):
        with self.assertRaises(ValueError):
            BO.plan(1.2, 27.6, NOW)

    def test_plan_refuses_just_above_one_dollar(self):
        with self.assertRaises(ValueError):
            BO.plan(1.0001, 27.6, NOW)

    def test_price_ok_rejects_1_20(self):
        self.assertFalse(BO.price_ok({"dph_total": 1.2}))

    def test_fields_recorded(self):
        f = BO.fields()
        self.assertEqual(
            f["budget_override"],
            {"stop_usd": 3.0, "cap_usd": 3.5, "max_rate": 1.0, "authority": "controller 04:20 B1j"},
        )
