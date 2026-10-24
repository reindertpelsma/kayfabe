"""BW7 contract, tested against the copied native GB205 reference before adaptation."""

import ast
import hashlib
import importlib.util
import json
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

K = Path(__file__).resolve().parents[1] / "kit"


def load():
    spec = importlib.util.spec_from_file_location("ref_windows", K / "ref_windows.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Operator(unittest.TestCase):
    def setUp(self):
        self.s = (K / "ops.py").read_text()

    def test_only_rtx5090_gb202(self):
        for node in ast.walk(ast.parse(self.s)):
            if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == "skus":
                self.assertEqual(ast.literal_eval(node.value), {"RTX_5090": "GB202"})
                return
        self.fail("missing SKU scope")

    def test_same_native_template(self):
        self.assertIn('"183b10af7fd788d08aca02e31811d70d"', self.s)
        self.assertNotIn('"--image"', self.s)

    def test_price_ceiling(self):
        self.assertIn('float(x.get("dph_total", 99)) <= 1.00', self.s)

    def test_stop_and_cap(self):
        self.assertEqual(self.s.count("stop_usd=2.5, cap_usd=3.0"), 3)

    def test_preferred_machine(self):
        self.assertIn('int(x["machine_id"]) == 152098', self.s)

    def test_settled_pin_before_login(self):
        self.assertIn("C.settle_pin(", self.s)
        self.assertLess(self.s.index("C.settle_pin("), self.s.index('"echo CAPC_SSH_READY"'))

    def test_capture_gate_precedes_destroy(self):
        start = self.s.index('elif mode == "destroy":')
        self.assertIn("verify_capture_before_destroy(", self.s[start:])
        self.assertLess(self.s.index("verify_capture_before_destroy(", start),
                        self.s.index('"destroy", "instance"', start))

    def test_watchdog_before_create_and_exact_absence(self):
        self.assertLess(self.s.index('armed["state"] == "ARMED"'),
                        self.s.index('"create",\n            "instance"'))
        self.assertIn("API absence not verified; keep timer armed", self.s)

    def test_no_blacklist(self):
        self.assertNotIn("P.exclude_bad(offers", self.s)

    def test_hourly_rejection_executes_the_actual_filter(self):
        tree = ast.parse(self.s)
        expression = next(n.value for n in ast.walk(tree) if isinstance(n, ast.Assign)
                          and getattr(n.targets[0], "id", None) == "offers"
                          and isinstance(n.value, ast.ListComp)
                          and "dph_total" in ast.unparse(n.value))
        from types import SimpleNamespace
        offer = dict(verification="verified", reliability=1, inet_down=1000,
                     vms_enabled=True, cpu_ram=65536, cpu_cores_effective=32,
                     is_vm_deverified=False, dph_total=1.01)
        result = eval(compile(ast.Expression(expression), "filter", "eval"),
                      {"offers": [offer], "LP": SimpleNamespace(offer_ok=lambda x, p: True)})
        self.assertEqual(result, [])

    def test_ledger_budget_matches_the_plan(self):
        self.assertIn('"budget_stop": 2.5', self.s)
        self.assertIn('"phase_cap": 3.0', self.s)

    def test_settled_key_is_written_to_known_hosts(self):
        self.assertIn('alias + " ssh-ed25519 " + proof["key"]', self.s)
        self.assertNotIn('alias + " " + fields[1] + " " + fields[2]', self.s)


class Hold(unittest.TestCase):
    def test_all_required_events_and_driver(self):
        s = (K / "ref_hold.ps1").read_text()
        for value in ("1001", "6008", "4101", "nvlddmkm", "Display", "32.0.15.8088"):
            with self.subTest(value=value):
                self.assertIn(value, s)

    def test_ten_minutes_has_start_and_end_boot_identity(self):
        s = (K / "ref_hold.ps1").read_text()
        for value in ("1..10", "Start-Sleep -Seconds 60", "REF_HOLD_START", "boot"):
            with self.subTest(value=value):
                self.assertIn(value, s)


class Capture(unittest.TestCase):
    def run_destroy(self, capture):
        r = load()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "own-id").write_text("2")
            ledger = root / "owned_instances.json"
            ledger.write_text(json.dumps([dict(id=2, ssh_ready_utc="fixture")]))
            if capture:
                (root / "capture_verified.json").write_text(json.dumps(capture))
            body = (K / "ops.py").read_text().split('elif mode == "destroy":', 1)[1]
            def forbidden_api(args):
                raise AssertionError("provider accessed before capture verification")
            ns = dict(D=root, L=ledger, json=json, api=forbidden_api)
            with patch.dict(sys.modules, {"ref_windows": r}):
                exec("if True:\n" + body, ns)

    def test_destroy_without_capture_never_accesses_provider(self):
        with self.assertRaises(FileNotFoundError):
            self.run_destroy(None)

    def test_destroy_incomplete_capture_never_accesses_provider(self):
        with self.assertRaises(ValueError):
            self.run_destroy(dict(files=0, complete=False, bad=[]))

    def test_empty_complete_manifest_refuses(self):
        r = load()
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "guest"
            dest.mkdir()
            (dest / "GUEST_MANIFEST.txt").write_text("CAPTURE_GUEST_DONE\n")
            with patch.object(r, "lease", return_value=(2, "host", 22)), \
                 patch.object(r, "win_conf", return_value="fixture"), \
                 patch.object(r.subprocess, "run"), patch.object(r, "log_event"):
                self.assertNotEqual(r.pull(dest), 0)

    def test_verifier_rehashes_before_destroy(self):
        r = load()
        self.assertTrue(callable(getattr(r, "verify_capture_before_destroy", None)))

    def test_changed_capture_refuses(self):
        r = load()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "data.txt").write_text("after!")
            receipt = dict(files=1, complete=True, bad=[], private=str(root),
                           sha256={"data.txt": hashlib.sha256(b"before").hexdigest()},
                           bytes={"data.txt": 6})
            path = root / "receipt.json"
            path.write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                r.verify_capture_before_destroy(path)

    def test_missing_capture_refuses(self):
        r = load()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "receipt.json"
            path.write_text(json.dumps(dict(files=1, complete=True, bad=[], private=tmp,
                                           sha256={"lost.txt": "0" * 64},
                                           bytes={"lost.txt": 6})))
            with self.assertRaises(ValueError):
                r.verify_capture_before_destroy(path)


if __name__ == "__main__":
    unittest.main()
