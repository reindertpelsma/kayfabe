"""B1p operator workspace: offer scope, budget, label, ownership and the fix pins (BW5)."""

import ast
import json
import os
from pathlib import Path
import re
import sys
import unittest

HERE = Path(__file__).resolve().parent
WS = Path(os.environ.get("KF_BW3_D", "/workspace/bw5-round-b1p-a1-run"))
OPS = WS / "ops.py"
AUTH = WS / "owner_authority.json"
FIX = "007ba1eb03db9e21d351e465e95b46cbc3134a95"
B1N_ARMS = ["on_urt", "on_relay_off", "on_tdr60", "lx_reload", "on_diag_pnp"]
ARMS = ["on_fix", "on_fix_pnp", "lx_reload"]
BW4_LEASES = (57472433, 57485318, 57486060)
sys.path.insert(0, str(HERE.parents[3] / "scripts/bench/blackwell"))
import saved_xid32_observation as O  # noqa: E402


def text(name):
    return (WS / name).read_text()


class OfferScope(unittest.TestCase):
    def setUp(self):
        self.source = OPS.read_text()
        self.tree = ast.parse(self.source)

    def literal(self, name):
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == name:
                return ast.literal_eval(node.value)
        raise AssertionError(name + " missing")

    def test_searched_skus_are_the_authority_skus_with_a_pinned_pair(self):
        skus = self.literal("skus")
        want = json.loads(AUTH.read_text())["skus"]
        self.assertEqual(skus, want)
        self.assertTrue(set(skus.values()) <= set(O.DIE_PAIRS))
        if (WS / "budget_override.py").exists():
            self.assertEqual(skus, {"RTX_5090": "GB202"})
        else:
            self.assertEqual(skus, {"RTX_5070": "GB205"})

    def test_bw4_leases_are_owned_ids(self):
        owned = self.literal("owned")
        for iid in BW4_LEASES:
            self.assertIn(iid, owned)

    def test_deverified_hosts_are_not_admitted(self):
        self.assertIn('x.get("verification") in ("verified", "unverified")', self.source)
        self.assertNotIn('"unverified", "deverified")', self.source)
        self.assertIn('not x.get("is_vm_deverified", False)', self.source)

    def test_budget_and_reserve_are_the_task_values(self):
        if (WS / "budget_override.py").exists():
            self.assertIn("BO.price_ok(x)", self.source)
            self.assertNotIn("stop_usd=2.0, cap_usd=2.5", self.source)
        else:
            self.assertEqual(self.source.count("stop_usd=2.0, cap_usd=2.5"), 3)
            self.assertIn('float(x.get("dph_total", 99)) <= 2.50', self.source)
        self.assertIn('"credit_reserve": 8', self.source)

    def test_denied_countries_label_and_fix(self):
        self.assertIn('{"CU", "IR", "KP", "SY", "CN", "HK", "MO"}', self.source)
        self.assertRegex(self.source, r'"claude-bw5-B1[pq]1-OBSERVATION-20261024"')
        self.assertNotIn("claude-bw4-", self.source)
        self.assertIn(f'"fix_candidate": "{FIX}"', self.source)

    def test_owner_authority_is_checked_against_the_guard_table(self):
        self.assertIn(
            'authority["die_pairs"] == {k: list(v) for k, v in SX.DIE_PAIRS.items()}', self.source
        )
        auth = json.loads(AUTH.read_text())
        self.assertEqual(auth["arms"], ARMS)
        self.assertEqual(auth["fix_source"], FIX)
        self.assertIs(auth["deverified_admitted"], False)
        self.assertEqual(auth["leases_authorized"], 1)


class Wiring(unittest.TestCase):
    def test_no_workspace_file_points_at_the_bw4_workspace_or_worktree(self):
        for path in sorted(WS.glob("*.py")):
            source = path.read_text()
            old_paths = (
                "/workspace/bw4-round-b1n-a1-run",
                "/workspace/bw4-wt",
                "ROUND_B1n_bw4/att",
            )
            for old in old_paths:
                self.assertNotIn(old, source, path.name)

    def test_upload_sends_the_kit_but_no_b1_commit_object(self):
        source = text("upload.py")
        self.assertNotIn("b1_commit", source)
        self.assertNotIn("b1_merge", source)
        self.assertIn("for f in K.iterdir():", source)
        self.assertFalse((WS / "b1_commit.txt").exists())

    def test_exports_polls_and_probes_name_the_b1p_arms_only(self):
        for name in ("export_meta.py", "poll.py", "milestone_probe.py"):
            source = text(name)
            for arm in ARMS:
                self.assertIn(arm, source, name)
            for arm in sorted(set(B1N_ARMS) - set(ARMS)):
                self.assertNotIn("'" + arm + "'", source, name)
                self.assertNotIn('"' + arm + '"', source, name)
                self.assertNotIn("arm_" + arm + ".log", source, name)
        self.assertIn("observed_fix_build.log", text("milestone_probe.py"))
        self.assertIn("observed_fix_binary.json", text("export_meta.py"))

    def test_final_audit_pins_the_fix_worktree(self):
        source = text("final_remote_audit.py")
        self.assertIn('("observed_fix", "/workspace/bw-fix")', source)
        self.assertIn(f'assert sources["observed_fix"] == "{FIX}"', source)
        self.assertNotIn("cb80cbd29", source)

    def test_dry_loop_runs_the_b1p_suites_and_hashes_the_b1p_rehearsal(self):
        source = text("dry_loop.py")
        for suite in ("test_b1p_kit", "test_b1p_ops", "test_b1p_snippets", "test_die_pair_table"):
            self.assertIn('"' + suite + '"', source)
        for suite in ("test_b1n_kit", "test_b1n_ops", "test_b1n_snippets"):
            self.assertNotIn('"' + suite + '"', source)
        self.assertIn("ROUND_B1p_bw5/rehearsal", source)

    def test_launch_records_the_fix_source(self):
        self.assertIn(f'fix_source="{FIX}"', text("launch_ready.py"))
        self.assertIn('R = Path("/workspace/bw5-wt")', text("launch_ready.py"))
        self.assertRegex(
            text("launch_ready.py"),
            r'T = R / "traces/blackwell_ab_20261023/ROUND_B1[pq]_bw5/attempt1"',
        )

    def test_grant_pin_is_machine_30498_first(self):
        pin = json.loads(text("grant_pin.json"))
        self.assertIn(pin["machine"], (30498, None))
        state = json.loads(text("stretch.json"))
        self.assertEqual(state["attempts_used"], 0)

    def test_the_lease_ledger_exists_and_is_empty_before_the_create(self):
        kit = Path(os.environ.get("KF_BW3_K", HERE.parent / "attempt1" / "run_kit"))
        ledger = kit.parent / "owned_instances.json"
        if (WS / "own-id").exists():
            self.skipTest("a lease exists; the ledger then holds it")
        self.assertEqual(json.loads(ledger.read_text()), [])
        self.assertIn('assert L.read_text().strip() == "[]"', OPS.read_text())

    def test_workspace_is_fresh(self):
        for name in ("own-id", "lease.json", "alias", "round_plan.json"):
            self.assertFalse((WS / name).exists(), name)
        self.assertFalse(re.search(r"5747243[0-9]", text("budget_watch.py")))


if __name__ == "__main__":
    unittest.main()
