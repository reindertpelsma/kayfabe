"""B1l GB202 RTX 5090-only watcher rules (controller 05:00, watcher-only rule change)."""

import json
from pathlib import Path
import sys
import unittest

W = Path("/workspace/bw3-gb202-watch-run")
sys.path.insert(0, str(W))
sys.dont_write_bytecode = True
import market_watch as M  # noqa: E402

RULES = M.load_rules(W / "ops.py")
TB = Path("/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023")


def offer(oid, machine, gpu, die, cores, price, geo="The Netherlands, NL", vm_dev=False):
    return {"id": oid, "machine_id": machine, "gpu_name": gpu, "candidate_die": die,
            "verification": "verified", "dph_total": price, "reliability": 0.97,
            "inet_down": 1000, "vms_enabled": True, "cpu_cores_effective": cores,
            "cpu_ram": 128000, "disk_space": 500, "is_vm_deverified": vm_dev, "geolocation": geo}


class GB202Watch(unittest.TestCase):
    def test_only_rtx5090_queries(self):
        self.assertEqual(RULES.skus, {"RTX_5090": "GB202"})

    def test_rtx_pro_5000_is_never_queried(self):
        for name in ("RTX_PRO_5000", "RTX_PRO_6000_WS", "RTX_PRO_6000_S", "RTX_PRO_6000_Max-Q"):
            self.assertNotIn(name, RULES.skus)

    def test_raised_budget_admits_the_0_87_rtx5090(self):
        r = M.decide(RULES, [offer(1, 55678, "RTX 5090", "GB202", 32.0, 0.869)], 27.4)
        self.assertEqual(r["selected"]["machine_id"], 55678)

    def test_price_above_one_dollar_refused(self):
        r = M.decide(RULES, [offer(2, 1, "RTX 5090", "GB202", 32.0, 1.2)], 27.4)
        self.assertIsNone(r["selected"])

    def test_vm_deverified_country_and_known_bad_still_refused(self):
        rows = [offer(3, 2, "RTX 5090", "GB202", 32.0, 0.6, vm_dev=True),
                offer(4, 3, "RTX 5090", "GB202", 32.0, 0.6, geo=", CN"),
                offer(5, 69146, "RTX 5090", "GB202", 32.0, 0.6)]
        self.assertIsNone(M.decide(RULES, rows, 27.4)["selected"])

    def test_b1j_snapshot_has_no_match(self):
        offers = json.loads((TB / "ROUND_B1j_0420/attempt1_mostcores/market.json").read_text())
        gb202 = [o for o in offers["offers"] if o["candidate_die"] == "GB202"]
        self.assertIsNone(M.decide(RULES, gb202, 27.4)["selected"])


if __name__ == "__main__":
    unittest.main()
