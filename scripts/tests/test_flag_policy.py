#!/usr/bin/env python3
"""Audit regression cases: a new source read must not silently escape the flag ledger."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class FlagPolicy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("flag_policy_audit", ROOT / "scripts/flag_policy_audit.py")
        cls.audit = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.audit)

    def test_new_rust_read_requires_a_policy_row(self):
        names = self.audit.names_in('std::env::var("KF3_NEW_MEASUREMENT")')
        self.assertEqual(self.audit.missing(names, {"KF3_RPC_TRACE"}), {"KF3_NEW_MEASUREMENT"})

    def test_new_c_read_and_launcher_set_require_rows(self):
        names = self.audit.names_in('getenv("KF3_NEW_CONTROL"); export KF3_NEW_BOOT=1')
        self.assertEqual(self.audit.missing(names, set()), {"KF3_NEW_CONTROL", "KF3_NEW_BOOT"})

    def test_recognised_diagnostics_are_not_reported_missing(self):
        names = self.audit.names_in('let on = env::var("KF3_RPC_TRACE");')
        self.assertFalse(self.audit.missing(names, {"KF3_RPC_TRACE"}))

    def test_actual_source_and_launcher_names_have_policy_rows(self):
        table = self.audit.read_table(ROOT / "docs/design/V3_FLAG_POLICY_BETA_20261020.tsv")
        discovered = self.audit.inventory(ROOT)
        self.assertFalse(self.audit.missing(set(discovered), set(table)))

    def test_every_kept_diagnostic_control_is_optional_and_inactive_when_unset(self):
        table = self.audit.read_table(ROOT / "docs/design/V3_FLAG_POLICY_BETA_20261020.tsv")
        kept = [row for row in table.values() if row["verdict"] == "KEEP" and row["class"] in
                {"measurement", "debug", "negative control"}]
        self.assertGreater(len(kept), 20)
        for row in kept:
            self.assertEqual(row["required_to_boot"], "false", row["name"])
            self.assertEqual(row["active_when_unset"], "false", row["name"])


if __name__ == "__main__":
    unittest.main()
