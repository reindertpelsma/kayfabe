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

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1j-a1_mostcores-run"))
BW = Path("/workspace/bw-native-edge-wt/scripts/bench/blackwell")
TB = Path("/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023")
sys.path.insert(0, str(BW))
sys.path.insert(0, str(D))
import round_budget as B  # noqa: E402
import budget_override as BO  # noqa: E402
import grant_b1j as GJ  # noqa: E402

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


class Selection(unittest.TestCase):
    def test_most_cores_default_profile_first(self):
        rows = [
            {"id": 1, "machine_id": 1, "cpu_cores_effective": 16.0},
            {"id": 2, "machine_id": 2, "cpu_cores_effective": 32.0},
            {"id": 3, "machine_id": 3, "cpu_cores_effective": 14.2},
        ]
        self.assertEqual([r["id"] for r in GJ.select(rows)], [2])

    def test_only_sub16_left_returns_nothing_for_the_default_workspace(self):
        self.assertEqual(
            GJ.select([{"id": 3, "machine_id": 58852, "cpu_cores_effective": 14.2}]), []
        )


def oracle(offers, credit=27.6):
    tmp = Path(tempfile.mkdtemp())
    try:
        sd = tmp / "D"
        sd.mkdir()
        media = sd / "media"
        media.mkdir()
        files = []
        for i in range(4):
            p = media / f"m{i}"
            p.write_bytes(bytes([i]) * 10)
            files.append(
                {
                    "file": p.name,
                    "expected_bytes": 10,
                    "expected_sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                }
            )
        (sd / "media_manifest.json").write_text(json.dumps({"all_verified": True, "files": files}))
        for p in D.glob("*.py"):
            shutil.copy2(p, sd / p.name)
        for name in ("owner_authority.json", "known_bad_machines.json", "grant_pin.json"):
            shutil.copy2(D / name, sd / name)
        ledger = tmp / "owned.json"
        ledger.write_text("[]\n")
        src = (D / "ops.py").read_text().replace(f'D = Path("{D}")', f'D = Path("{sd}")')
        a = src.index("L = W / ")
        src = src[:a] + f'L = Path("{ledger}")' + src[src.index("\n", a) :]
        (sd / "ops.py").write_text(src)
        cache = next(n for n in src.split('"') if n.startswith("single_market_B1j_"))
        (sd / cache).write_text(json.dumps({"utc": "2026-10-24T02:00:00+00:00", "offers": offers}))
        b = tmp / "bin"
        b.mkdir()
        (b / "vastai").write_text(
            "#!/bin/sh\ncase \"$1 $2\" in\n 'show instances') echo '[]';;\n"
            f" 'show user') echo '{{\"credit\": {credit}}}';;\n"
            " *) exit 9;;\nesac\n"
        )
        (b / "vastai").chmod(0o755)
        env = {**os.environ, "PATH": f"{b}:{os.environ['PATH']}", "PYTHONDONTWRITEBYTECODE": "1"}
        p = subprocess.run(
            [sys.executable, str(sd / "ops.py"), "preflight"],
            capture_output=True,
            text=True,
            env=env,
            timeout=60,
        )
        if p.returncode == 75:
            return None
        assert p.returncode == 0, p.stdout + p.stderr
        return json.loads(p.stdout.splitlines()[-1])
    finally:
        shutil.rmtree(tmp)


class Operator(unittest.TestCase):
    def snapshot(self):
        return json.loads((TB / "ROUND_B1i_0400/attempt1_m30498/market.json").read_text())["offers"]

    def test_b1i_snapshot_selects_the_32_core_netherlands_5090(self):
        r = oracle(self.snapshot())
        self.assertEqual(r["machine"], 55678)
        self.assertEqual(r["plan"]["stop_usd"], 3.0)
        self.assertGreaterEqual(r["plan"]["seconds"], 90 * 60)

    def test_offer_above_one_dollar_never_selected(self):
        only = [o for o in self.snapshot() if o["machine_id"] == 153274]
        self.assertIsNone(oracle(only))

    def offer(self, oid, machine, cores, price):
        base = dict(self.snapshot()[4])
        base.update(
            id=oid,
            machine_id=machine,
            cpu_cores_effective=cores,
            dph_total=price,
            is_vm_deverified=False,
            geolocation="The Netherlands, NL",
        )
        return base

    def test_more_cores_beats_cheaper(self):
        r = oracle([self.offer(1, 101, 16.0, 0.5), self.offer(2, 102, 32.0, 0.9)])
        self.assertEqual(r["machine"], 102)

    def test_fourteen_core_offer_not_taken_by_the_default_workspace(self):
        self.assertIsNone(oracle([self.offer(3, 58852, 14.2, 0.87)]))

    def test_readiness_uses_the_lease_stop(self):
        ops = (D / "ops.py").read_text()
        self.assertEqual(ops.count("< BO.STOP_USD"), 1)
        self.assertNotIn("                        < 2.0\n", ops)

    def test_row_records_budget_override(self):
        ops = (D / "ops.py").read_text()
        self.assertIn('        "phase_cap": BO.CAP_USD,\n        **BO.fields(),\n', ops)

    def test_wiring(self):
        ops = (D / "ops.py").read_text()
        self.assertNotIn("stop_usd=2.0, cap_usd=2.5", ops)
        self.assertIn("BO.plan(", ops)
        self.assertIn("< BO.STOP_USD", ops)
        self.assertIn("**BO.fields()", ops)
        self.assertIn("BO.price_ok(x)", ops)
        self.assertIn('"budget_stop": BO.STOP_USD', ops)

    def test_dry_loop_covers_this_suite(self):
        sys.path.insert(0, "/workspace/bw3-round-b1g-common")
        import dry_loop as DL

        self.assertIn("test_budget_b1j", {s for _, s in DL.SUITES})


if __name__ == "__main__":
    unittest.main()
