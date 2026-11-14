"""GB202 profile preparation: isolated limits, exact native scope and fixed admission."""

import datetime as dt
import importlib.util
import json
import os
from pathlib import Path
import unittest

try:
    import gb202_profile as P
except ImportError:
    P = None

NOW = dt.datetime(2026, 10, 28, 22, 0, tzinfo=dt.timezone.utc)
SOURCE = '7dcedefea093de078188efd5f712561bde746362'


def offer():
    return dict(
        id=58156107,
        machine_id=155138,
        host_id=79466,
        gpu_name='RTX 5090',
        gpu_ram=32607,
        verification='verified',
        vms_enabled=True,
        is_vm_deverified=False,
        num_gpus=1,
        geolocation='Quebec, CA',
        cpu_cores_effective=48,
        cpu_ram=128785,
        disk_space=4993.2,
        reliability=0.9645086,
        inet_down=3492.6,
        dph_total=1.4933333333333334,
        inet_down_cost=0.00390625,
        inet_up_cost=0.00390625,
    )


def budget():
    return dict(
        prior_sw5_stage_hold_usd='2.794777622',
        today_billing_all_charges_usd='2.909',
        old_closed_ci_hold_usd='1.00',
        retry_closed_ci_hold_usd='2.00',
        other_liabilities_usd='0.00',
        credit_usd='42.0',
        complete_account_inventory=True,
        account_instances_observed=0,
    )


def load_file(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BudgetProfile(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(P, 'explicit GB202 profile not implemented')

    def test_selected_quote_has_full_5400_seconds(self):
        plan = P.make_plan(offer(), 42.0, NOW)
        self.assertEqual(plan['seconds'], 5400)
        self.assertAlmostEqual(plan['reservation_usd'], 3.12678125)

    def test_ceiling_quote_still_has_full_5400_seconds(self):
        row = dict(offer(), dph_total=1.50)
        plan = P.make_plan(row, 42, NOW)
        self.assertEqual(plan['seconds'], 5400)
        self.assertAlmostEqual(plan['reservation_usd'], 3.13828125)

    def test_profile_stop_and_cap_are_explicit(self):
        plan = P.make_plan(offer(), 42, NOW)
        self.assertEqual((plan['stop_usd'], plan['cap_usd']), (3.20, 3.25))

    def test_profile_identity_and_cleanup_are_explicit(self):
        plan = P.make_plan(offer(), 42, NOW)
        self.assertEqual(plan['profile_id'], 'GB202_5090_90MIN')
        self.assertEqual((plan['work_lead_seconds'], plan['capture_lead_seconds']), (660, 600))
        self.assertEqual(plan['source'], SOURCE)
        self.assertEqual(plan['framebuffer_mib'], 8192)

    def test_shared_holds_include_full_billing_and_both_ci(self):
        plan = P.make_plan(offer(), 42.0, NOW)
        result = P.check_budget(budget(), plan)
        self.assertAlmostEqual(float(result), 11.830558872)

    def test_shared_cap_rejects_higher_full_billing(self):
        plan = P.make_plan(offer(), 42.0, NOW)
        with self.assertRaises(ValueError):
            P.check_budget(dict(budget(), today_billing_all_charges_usd='3.20'), plan)

    def test_shared_cap_rejects_other_liability(self):
        plan = P.make_plan(offer(), 42.0, NOW)
        with self.assertRaises(ValueError):
            P.check_budget(dict(budget(), other_liabilities_usd='.20'), plan)

    def test_make_plan_preserves_reserve_at_credit_ten(self):
        with self.assertRaises(ValueError):
            P.make_plan(offer(), 10, NOW)

    def test_make_plan_preserves_reserve_at_credit_eleven(self):
        with self.assertRaises(ValueError):
            P.make_plan(offer(), 11, NOW)

    def test_full_liability_preserves_reserve_credit_thirteen(self):
        plan = P.make_plan(offer(), 13, NOW)
        with self.assertRaises(ValueError):
            P.check_budget(dict(budget(), credit_usd='13'), plan)

    def test_full_liability_preserves_reserve_credit_sixteen(self):
        plan = P.make_plan(offer(), 16, NOW)
        with self.assertRaises(ValueError):
            P.check_budget(dict(budget(), credit_usd='16'), plan)

    def test_fixed_work_cutoff_exactly_expired(self):
        plan = P.make_plan(offer(), 42, NOW)
        with self.assertRaises(ValueError):
            P.check_plan(plan, offer(), NOW + dt.timedelta(seconds=4740))

    def test_fixed_hard_deadline_expired(self):
        plan = P.make_plan(offer(), 42, NOW)
        with self.assertRaises(ValueError):
            P.check_plan(plan, offer(), NOW + dt.timedelta(seconds=6000))


def budget_refusal(field, value):
    def test(self):
        plan = P.make_plan(offer(), 42.0, NOW)
        with self.assertRaises(ValueError):
            P.check_budget(dict(budget(), **{field: value}), plan)

    return test


for field, value in [
    ('prior_sw5_stage_hold_usd', '0'),
    ('old_closed_ci_hold_usd', '0'),
    ('retry_closed_ci_hold_usd', '0'),
    ('other_liabilities_usd', '-1'),
    ('today_billing_all_charges_usd', 'NaN'),
    ('credit_usd', '10'),
    ('complete_account_inventory', False),
    ('account_instances_observed', 1),
]:
    setattr(BudgetProfile, 'test_budget_refuses_' + field, budget_refusal(field, value))


def offer_refusal(field, value):
    def test(self):
        with self.assertRaises(ValueError):
            P.make_plan(dict(offer(), **{field: value}), 42, NOW)

    return test


for label, field, value in [
    ('rate_over', 'dph_total', 1.50001),
    ('rate_over_second', 'dph_total', 1.51),
    ('rate_cloud_only', 'dph_total', 2.0),
    ('machine', 'machine_id', 30498),
    ('host', 'host_id', 70221),
    ('wrong_offer', 'id', 900),
    ('cpu', 'cpu_cores_effective', 6),
    ('ram', 'cpu_ram', 8000),
    ('disk', 'disk_space', 100),
    ('reliability', 'reliability', 0.949),
    ('verification', 'verification', 'unverified'),
    ('vm', 'vms_enabled', False),
    ('deverified', 'is_vm_deverified', True),
    ('multigpu', 'num_gpus', 2),
    ('country', 'geolocation', 'Somewhere, CN'),
    ('small_store', 'gpu_ram', 8151),
    ('download', 'inet_down', 100),
]:
    setattr(BudgetProfile, 'test_offer_refuses_' + label, offer_refusal(field, value))


def plan_refusal(field, value):
    def test(self):
        plan = P.make_plan(offer(), 42, NOW)
        plan[field] = value
        with self.assertRaises(ValueError):
            P.check_plan(plan, offer(), NOW)

    return test


for field, value in [
    ('seconds', 7200),
    ('stop_usd', 4),
    ('cap_usd', 5),
    ('reserve_usd', 0),
    ('framebuffer_mib', 4096),
    ('profile_id', 'GB206'),
    ('rate_bound', 2.5),
    ('capture_lead_seconds', 420),
    ('work_lead_seconds', 480),
    ('deadline_utc', (NOW + dt.timedelta(seconds=5401)).isoformat()),
]:
    setattr(BudgetProfile, 'test_plan_refuses_' + field, plan_refusal(field, value))


@unittest.skipUnless(os.environ.get('SW5_PROFILE_OPERATOR'), 'private kit replay requires env')
class NativeScope(unittest.TestCase):
    def setUp(self):
        self.directory = Path(os.environ['SW5_PROFILE_OPERATOR'])
        self.binding = load_file(self.directory / 'die_binding.py', 'gb202_binding_fixture')
        self.pending = load_file(self.directory / 'pending_die.py', 'gb202_pending_fixture')
        repo = Path(__file__).resolve().parents[3]
        self.dies = (repo / 'crates/kf-chip/src/die.rs').read_text()
        self.arches = (repo / 'crates/kf-chip/src/lib.rs').read_text()

    def bind(self, implementation=2, revision=1, name='Fixture5090', first_implementation=2):
        pin = dict(
            ssh_verified=True,
            identity=dict(
                state='match',
                expected_fingerprint='fixture',
                fingerprint='fixture',
                fingerprints=['fixture'],
            ),
            driver='595.91.07',
            pci='0x123410de',
            arch_impl=[432, first_implementation],
        )
        probe = (
            f'host_driver=595.91.07 arch=(432, {implementation}, {revision})\n'
            'PASS native read-only timer map/read/drop x3\n'
        )
        return self.binding.bind(probe, name + ',595.91.07,0x123410de', self.dies, self.arches, pin)

    def test_gb202_native_rm_first_revision(self):
        self.assertEqual(self.bind()['die'], 'GB202')

    def test_gb202_native_rm_other_revision_name_is_advisory(self):
        self.assertEqual(self.bind(revision=161, name='Fixture')['die'], 'GB202')

    def test_gb206_second_probe_does_not_bind_gb202_intent(self):
        with self.assertRaises(ValueError):
            self.bind(implementation=6)

    def test_gb203_second_probe_does_not_bind_gb202_intent(self):
        with self.assertRaises(ValueError):
            self.bind(implementation=3)

    def test_consistent_gb206_actual_rm_is_outside_profile(self):
        with self.assertRaises(ValueError):
            self.bind(implementation=6, first_implementation=6)

    def test_consistent_gb203_actual_rm_is_outside_profile(self):
        with self.assertRaises(ValueError):
            self.bind(implementation=3, first_implementation=3)

    def test_gb202_pending_target(self):
        self.assertEqual(self.pending.require_pending(dict(die='GB202'), [])['die'], 'GB202')

    def test_gb203_pending_target_rejected(self):
        with self.assertRaises(ValueError):
            self.pending.require_pending(dict(die='GB203'), [])

    def test_gb206_pending_target_rejected(self):
        with self.assertRaises(ValueError):
            self.pending.require_pending(dict(die='GB206'), [])


@unittest.skipUnless(os.environ.get('SW5_LEGACY_OPERATOR'), 'legacy kit requires env')
class LegacyIsolation(unittest.TestCase):
    def setUp(self):
        directory = Path(os.environ['SW5_LEGACY_OPERATOR'])
        self.legacy = load_file(directory / 'round_budget.py', 'legacy_budget_fixture')

    def test_legacy_rate_ceiling_still_point_six(self):
        with self.assertRaises(ValueError):
            self.legacy.plan(0.61, 42, NOW)

    def test_legacy_selected_5090_still_refused(self):
        with self.assertRaises(ValueError):
            self.legacy.plan(offer()['dph_total'], 42, NOW)

    def test_legacy_stop_cannot_be_raised(self):
        with self.assertRaises(ValueError):
            self.legacy.plan(0.24, 42, NOW, stop_usd=3.20, cap_usd=3.25)

    def test_legacy_cap_cannot_be_raised(self):
        with self.assertRaises(ValueError):
            self.legacy.plan(0.24, 42, NOW, stop_usd=1.5, cap_usd=3.25)


if __name__ == '__main__':
    unittest.main()
