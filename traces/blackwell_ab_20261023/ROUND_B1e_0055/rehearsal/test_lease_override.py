"""B1e per-lease override (controller 00:55): 8-core CPU floor, pinned offer, 3 h lease ceiling.

Run: KF_BW3_D=/workspace/bw3-round-b1e-run python3 -m unittest test_lease_override
Defaults must stay unchanged; the override is explicit, named and accepts only its granted value.
"""

import datetime as dt
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1e-run"))
W = Path("/workspace/bw-native-edge-wt/scripts/bench/blackwell")
sys.path.insert(0, str(W))
sys.path.insert(0, str(D))
import round_budget as B  # noqa: E402
import lease_override as LO  # noqa: E402

NOW = dt.datetime(2026, 10, 23, 22, 0, tzinfo=dt.timezone.utc)


def offer(cores, oid=57427095):
    return {"id": oid, "cpu_cores_effective": cores}


class CpuFloor(unittest.TestCase):
    def test_default_floor_stays_16(self):
        self.assertEqual(LO.cpu_floor(None), 16)
        self.assertEqual(LO.DEFAULT_CPU_FLOOR, 16)
        self.assertFalse(LO.cores_ok(offer(8.0), LO.cpu_floor(None)))
        self.assertTrue(LO.cores_ok(offer(16.0), LO.cpu_floor(None)))

    def test_override_accepts_exactly_8_and_rejects_7(self):
        floor = LO.cpu_floor(8)
        self.assertEqual(floor, 8)
        self.assertTrue(LO.cores_ok(offer(8.0), floor))
        self.assertFalse(LO.cores_ok(offer(7.0), floor))
        self.assertFalse(LO.cores_ok(offer(7.9), floor))
        self.assertFalse(LO.cores_ok({"id": 1}, floor))

    def test_only_the_granted_override_value_is_accepted(self):
        for value in (7, 9, 12, 4, 0, "8", 8.5):
            with self.assertRaises(ValueError):
                LO.cpu_floor(value)

    def test_lease_constants_name_the_grant(self):
        self.assertEqual(LO.CPU_FLOOR_OVERRIDE, 8)
        self.assertEqual(LO.PINNED_OFFER, 57427095)
        self.assertEqual(LO.LABEL, "8-core deviation")


    def test_default_rejects_an_8_core_offer(self):
        self.assertFalse(LO.cores_ok(offer(8.0), LO.cpu_floor(None)))
        self.assertFalse(LO.cores_ok(offer(15.0), LO.DEFAULT_CPU_FLOOR))

    def test_seven_core_pinned_offer_rejected_under_override(self):
        floor = LO.cpu_floor(LO.CPU_FLOOR_OVERRIDE)
        rows = [x for x in LO.pinned([offer(7.0), offer(6.0)]) if LO.cores_ok(x, floor)]
        self.assertEqual(rows, [])

    def test_seven_is_not_a_valid_override(self):
        with self.assertRaises(ValueError):
            LO.cpu_floor(7)


class PinnedOffer(unittest.TestCase):
    def test_pin_keeps_only_the_granted_offer(self):
        rows = [offer(8.0), offer(8.0, 1), offer(32.0, 2)]
        self.assertEqual(LO.pinned(rows), [rows[0]])

    def test_pin_absent_means_nothing_to_rent(self):
        self.assertEqual(LO.pinned([offer(8.0, 1)]), [])


class LeaseCeiling(unittest.TestCase):
    def test_default_plan_keeps_150_minutes_and_module_constant(self):
        before = B.MAX_SECONDS
        plan = LO.plan(0.295, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=None)
        self.assertEqual(plan["seconds"], 150 * 60)
        self.assertEqual(B.MAX_SECONDS, before)
        self.assertEqual(B.MAX_SECONDS, 150 * 60)

    def test_override_allows_3h_within_stop_and_restores_default(self):
        plan = LO.plan(0.295, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=LO.MAX_SECONDS)
        self.assertEqual(plan["seconds"], 3 * 3600)
        self.assertLess(plan["reservation_usd"], 2.0)
        self.assertEqual(B.MAX_SECONDS, 150 * 60)
        self.assertEqual(B.remaining(plan, NOW), 3 * 3600)

    def test_override_ceiling_value_is_exact(self):
        for value in (3 * 3600 + 1, 4 * 3600, 150 * 60 + 1):
            with self.assertRaises(ValueError):
                LO.plan(0.295, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=value)

    def test_cost_stop_still_binds_under_override(self):
        plan = LO.plan(1.2, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=LO.MAX_SECONDS)
        self.assertLess(plan["seconds"], 3 * 3600)
        self.assertLess(plan["reservation_usd"], 2.0)

    def test_timer_ceiling(self):
        self.assertEqual(LO.timer_ceiling_seconds(None), 150 * 60)
        self.assertEqual(LO.timer_ceiling_seconds(LO.MAX_SECONDS), 3 * 3600)


class Labels(unittest.TestCase):
    def test_deviation_fields(self):
        fields = LO.deviation_fields(8.0)
        self.assertEqual(fields["cpu_floor_override"], 8)
        self.assertEqual(fields["cpu_cores_effective"], 8.0)
        self.assertEqual(fields["profile"], "8-core deviation")

    def test_label_evidence_records_deviation(self):
        import label_evidence as E

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "deviation.json").write_text(json.dumps(LO.deviation_fields(8.0)))
            (root / "a.json").write_text('{"x": 1}')
            (root / "b.log").write_text("raw line\n")
            (root / "c.jsonl").write_text('{"y": 2}\n')
            E.label_exports(root, deviation=LO.deviation_fields(8.0))
            a = json.loads((root / "a.json").read_text())
            self.assertEqual(a["cpu_floor_override"], 8)
            self.assertEqual(a["cpu_cores_effective"], 8.0)
            self.assertEqual(a["profile"], "8-core deviation")
            text = (root / "b.log").read_text()
            self.assertIn("8-core deviation cpu_floor_override=8 cpu_cores_effective=8.0", text)
            row = json.loads((root / "c.jsonl").read_text().splitlines()[0])
            self.assertEqual(row["cpu_floor_override"], 8)


    def test_text_label_json_and_text(self):
        import label_evidence as E

        dev = LO.deviation_fields(8.0)
        value = json.loads(E.text_label("x.json", '{"k": 1}', dev))
        self.assertEqual((value["cpu_floor_override"], value["profile"]), (8, "8-core deviation"))
        self.assertIn("8-core deviation", E.text_label("x.txt", "line\n", dev))


class OperatorWiring(unittest.TestCase):
    def test_operator_uses_override(self):
        ops = (D / "ops.py").read_text()
        self.assertIn("cpu_cores_effective>={floor}", ops)
        self.assertNotIn("cpu_cores_effective>=16", ops)
        self.assertIn("LO.cores_ok(x, floor)", ops)
        self.assertIn("LO.pinned(", ops)
        self.assertIn("max_seconds=LO.MAX_SECONDS", ops)
        self.assertIn("LO.timer_ceiling_seconds(LO.MAX_SECONDS)", ops)
        self.assertIn("**LO.deviation_fields(", ops)

    def test_operator_has_no_default_floor_in_search(self):
        ops = (D / "ops.py").read_text()
        self.assertNotIn("cpu_cores_effective>=16", ops)
        self.assertIn('f"cpu_cores_effective>={floor} cpu_ram>=32', ops)
        self.assertIn("floor = LO.cpu_floor(LO.CPU_FLOOR_OVERRIDE)", ops)

    def test_operator_pins_after_bad_machine_exclusion(self):
        ops = (D / "ops.py").read_text()
        a = ops.index('offers = P.exclude_bad(offers, D / "known_bad_machines.json")')
        b = ops.index("    offers = LO.pinned(offers)\n")
        self.assertLess(a, b)

    def test_operator_timer_ceiling_is_the_override(self):
        ops = (D / "ops.py").read_text()
        self.assertIn("assert 0 < seconds <= LO.timer_ceiling_seconds(LO.MAX_SECONDS)", ops)
        self.assertNotIn("assert 0 < seconds <= 150 * 60", ops)

    def test_budget_watch_accepts_a_3h_timer(self):
        text = (D / "budget_watch.py").read_text()
        self.assertNotIn("seconds <= 170 * 60", text)
        self.assertIn("at most 180 minutes", text)

    def test_side_timers_allow_exactly_3h(self):
        self.assertIn("seconds <= 180 * 60", (D / "budget_watch.py").read_text())
        launch = (D / "launch_ready.py").read_text()
        self.assertIn('row["auto_destroy_minutes"] <= 180', launch)


if __name__ == "__main__":
    unittest.main()
