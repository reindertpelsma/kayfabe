"""Native-only dispatch, one-lease budget and strict authentication wiring."""

import ast
import json
from pathlib import Path
import unittest

K = Path(__file__).resolve().parents[1] / "kit"


class Wiring(unittest.TestCase):
    def test_native_policy_is_selected_by_the_operator(self):
        s = (K / "ops.py").read_text()
        self.assertIn("import native_ready", s)
        self.assertIn("native_ready.ready(D, L, save)", s)

    def test_the_native_helper_is_template_scoped(self):
        p = K / "native_ready.py"
        self.assertTrue(p.exists())
        s = p.read_text()
        self.assertIn('row["native_template_hash"] == TEMPLATE', s)
        self.assertIn('row["no_kayfabe"] is True', s)

    def test_attempt_budget_is_lowered_everywhere(self):
        s = (K / "ops.py").read_text()
        self.assertEqual(s.count("stop_usd=2.0, cap_usd=2.5"), 3)
        self.assertIn('"budget_stop": 2.0', s)
        self.assertIn('"phase_cap": 2.5', s)

    def test_plan_calls_use_the_attempt_budget(self):
        s = (K / "ops.py").read_text()
        calls = [n for n in ast.walk(ast.parse(s)) if isinstance(n, ast.Call)
                 and isinstance(n.func, ast.Attribute) and n.func.attr == "plan"]
        self.assertEqual(len(calls), 3)
        for call in calls:
            values = {x.arg: ast.literal_eval(x.value) for x in call.keywords}
            self.assertEqual(values["stop_usd"], 2.0)
            self.assertEqual(values["cap_usd"], 2.5)

    def test_window_proof_precedes_login_and_latest_key_is_pinned(self):
        p = K / "native_ready.py"
        self.assertTrue(p.exists())
        s = p.read_text()
        self.assertLess(s.index("N.settle_pin("), s.index('"ssh", "-F"'))
        self.assertIn('proof["key"]', s)
        self.assertIn('"StrictHostKeyChecking yes', s)
        self.assertIn('" User vast', s)

    def test_first_reachable_anchor_is_not_restarted(self):
        p = K / "native_ready.py"
        self.assertTrue(p.exists())
        s = p.read_text()
        self.assertEqual(s.count('first_reachable_at=first_reachable'), 1)
        self.assertIn('"native_first_reachable_utc"', s)
        self.assertIn("N.ABSOLUTE_SECONDS", s)

    def test_watchdog_and_capture_teardown_guards_are_retained(self):
        s = (K / "ops.py").read_text()
        self.assertLess(s.index('armed["state"] == "ARMED"'),
                        s.index('"create",\n            "instance"'))
        self.assertIn("verify_capture_before_destroy(", s)
        self.assertIn("API absence not verified; keep timer armed", s)
        self.assertIn('state["attempts_used"] == 0', s)
