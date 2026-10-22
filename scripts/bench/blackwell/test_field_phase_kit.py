"""Frozen phase wiring: no new product or fault-admission policy."""
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[3]
KIT=ROOT/'traces/blackwell_fields_phase_20261022/run_kit'

class FieldPhaseKit(unittest.TestCase):
    def setUp(self):self.controller=(KIT/'controller.sh').read_text()
    def test_runtime_reply_is_measured_before_native_gates(self):
        self.assertLess(self.controller.index('PRECOMP_RUNTIME_BEGIN'),self.controller.index('v3_gates.sh'))
    def test_refusal_is_recorded_without_privileged_fallback(self):
        self.assertIn('PRECOMP_RUNTIME_EXIT rc=$precomp_rc',self.controller)
        self.assertIn('[ "$precomp_rc" = 0 ] || [ "$precomp_rc" = 2 ]',self.controller)
    def test_existing_constants_are_explicitly_subtracted(self):
        self.assertIn('AUDIT_SUBTRACT GB202 ZBC/8162/GSS sizes already measured',self.controller)
        self.assertNotIn('bash scripts/bench/constants/run_container_probe.sh',self.controller)
    def test_b1_is_conditional_on_actual_observer_code0(self):
        self.assertIn('if actual_code0 on_precision; then',self.controller)
        self.assertIn('OPTIONAL_B1_UNRUN_NO_ACTUAL_CODE0',self.controller)
        self.assertLess(self.controller.index('if actual_code0 on_precision; then'),self.controller.index('run_cell on_b1'))
    def test_code0_gate_uses_observer_proof_not_rpc_progress(self):
        self.assertIn("assert clean(result(diag.read_text()),0)",self.controller)
        self.assertIn("WINDOWS_INIT_OBSERVER_END code0=1",self.controller)
    def test_new_phase_has_no_off_or_bisect_arm(self):
        self.assertNotIn("'off off_c'",self.controller)
        self.assertNotIn('arm_env_off_c.sh',self.controller)
        self.assertEqual(self.controller.count('run_cell on_precision'),1)
    def test_failed_guest_arm_still_collects_without_rebooting_product(self):
        self.assertIn('arm_rc=$?',self.controller)
        self.assertLess(self.controller.index('arm_rc=$?'),self.controller.index('collect_bw.sh'))
        self.assertIn('[ ! -e /root/prov/XID_STOP ]',self.controller)
    def test_early_observer_is_the_frozen_tested_source(self):
        self.assertEqual((KIT/'arm.sh').read_bytes(),(ROOT.parent/'bw-init-observer-wt/scripts/bench/blackwell/arm_fields.sh').read_bytes())

if __name__=='__main__':unittest.main()
