"""B1q (BW5 attempt 2) operator workspace: the settled readiness pin and the fresh attempt paths."""

import ast
import json
import os
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
WS = Path(os.environ.get("KF_BW3_D", "/workspace/bw6-round-b1r-a1-run"))
T = HERE.parent / "attempt1"


def text(name):
    return (WS / name).read_text()


class SettledReadiness(unittest.TestCase):
    def setUp(self):
        self.source = text("ops.py")

    def test_readiness_uses_the_settled_pin_with_a_rescan(self):
        self.assertIn("proof = C.settle_pin(\n", self.source)
        self.assertIn(
            "                        rescan,\n                        console_text,\n", self.source
        )
        self.assertNotIn("C.wait_for_pin(", self.source)
        self.assertIn("            def rescan():\n", self.source)
        ast.parse(self.source)

    def test_rescan_runs_the_same_bounded_ed25519_keyscan(self):
        body = self.source[self.source.index("            def rescan():") :]
        body = body[: body.index("            for fields in candidates:")]
        self.assertIn('["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip]', body)
        self.assertIn("timeout=10", body)
        self.assertIn('f[1] == "ssh-ed25519"', body)
        self.assertIn("return keys[0] if len(keys) == 1 else None", body)

    def test_known_hosts_pins_the_settled_key_not_the_first_scan(self):
        self.assertIn('alias + " ssh-ed25519 " + proof["key"] + "\\n"', self.source)
        self.assertNotIn('alias + " " + fields[1] + " " + fields[2]', self.source)

    def test_label_cache_and_paths_are_b1q(self):
        self.assertIn('"codex-bw6-B1r1-20261024"', self.source)
        self.assertIn("single_market_B1r_1.json", self.source)
        for path in sorted(WS.glob("*.py")):
            source = path.read_text()
            for old in ("/workspace/bw5-round-b1p-a1-run", "ROUND_B1p_bw5/attempt1"):
                self.assertNotIn(old, source, path.name)
        self.assertIn(
            'T = R / "traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1"', text("launch_ready.py")
        )

    def test_the_dry_loop_runs_the_settle_suite_and_this_suite(self):
        source = text("dry_loop.py")
        self.assertIn('(BW, "test_console_pin_settle")', source)
        self.assertIn('"test_b1r_pin"', source)
        self.assertIn("ROUND_B1r_bw6/rehearsal", source)

    def test_the_fresh_ledger_is_empty_before_the_create(self):
        if (WS / "own-id").exists():
            self.skipTest("a lease exists; the ledger then holds it")
        self.assertEqual(json.loads((T / "owned_instances.json").read_text()), [])

    def test_pin_receipts_are_exported_into_the_evidence(self):
        line = 'L.parent / "lease_metadata" / ("console_pin_" + str(iid) + ".jsonl")'
        self.assertIn(line, self.source)


if __name__ == "__main__":
    unittest.main()
