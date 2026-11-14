"""Actual adapter boundaries require exact profile, fixed plan and full manifest proof."""

import datetime as dt
import hashlib
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_gb202_profile import offer, budget, load_file


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


class ReachedFakeBoundary(Exception):
    pass


@unittest.skipUnless(os.environ.get('SW5_PROFILE_OPERATOR'), 'actual private adapter needs env')
class ProfileAdapter(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, os.environ['SW5_PROFILE_OPERATOR'])
        cls.O = importlib.import_module('operator_backend')
        cls.G = importlib.import_module('create_receipt_guard')

    def issue_legacy_fixture(self, root, mode, now):
        legacy = Path(os.environ['SW5_LEGACY_OPERATOR'])
        O, G = load_file(legacy / 'operator_backend.py', 'legacy_adapter_fixture'), self.G
        B = load_file(legacy / 'round_budget.py', 'legacy_plan_fixture')
        evidence = root / 'legacy_lease_fixture'
        evidence.mkdir()
        plan = B.plan(0.17, 42, now, transfer_usd=0.05)
        candidate = dict(id=900, machine_id=32570, host_id=166946)
        ref = G.prepare(
            evidence,
            candidate,
            plan,
            71,
            dict(owner_id=71, complete=True, rows=[]),
            plan.get('source', '7dcedefea093de078188efd5f712561bde746362'),
        )
        save(evidence / 'watch_ready.json', dict(state='ARMED', intent_sha256=ref.sha256))
        b = O.Backend.__new__(O.Backend)
        b.here, b.offer, b.plan = legacy, candidate, plan
        b.evidence, b.intent_ref, b.label = evidence, ref, G.load(ref)['label']
        b.watch = type('FakeWatch', (), dict(poll=lambda _: None))()

        def transport(*args):
            if mode == 'prior_gb206_timeout':
                raise O.subprocess.TimeoutExpired('fixture-create', 45)
            return O.subprocess.CompletedProcess([], 0, b'', b'')

        with patch.object(O.approval_gate, 'require_go', return_value={}), patch.object(
            O.P, 'snapshot', return_value=dict(owner_id=71, complete=True, rows=[])
        ), patch.object(O.P, 'create_once', side_effect=transport) as create:
            with self.assertRaises(G.Unresolved):
                b.create()
            self.assertEqual(create.call_count, 1)
        self.assertTrue((root / 'stage_create_started.json').is_file())

    def fixture(self, action, corrupt=None, expect_boundary=False):
        O, G = self.O, self.G
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            here = root / 'operator5_gb202'
            here.mkdir()
            (here / 'pinned.py').write_text('# fake fixture source\n')
            blacklist = []
            if corrupt == 'blacklist_machine':
                blacklist = [dict(machine_id=offer()['machine_id'])]
            if corrupt == 'blacklist_host':
                blacklist = [dict(host_id=offer()['host_id'])]
            save(here / 'known_bad_machines.json', blacklist)
            runtime = root / 'fake_cli.py'
            runtime.write_text('# fake fixture dependency\n')
            now = dt.datetime.now(dt.timezone.utc)
            plan = dict(
                credit_before=42.0,
                transfer_usd=0.05078125,
                nonruntime_margin_usd=0.5,
                armed_utc=now.isoformat(),
                deadline_utc=(now + dt.timedelta(seconds=5400)).isoformat(),
                seconds=5400,
                quoted_rate=offer()['dph_total'],
                rate_bound=1.7173333333333334,
                reservation_usd=3.12678125,
                credit_after_reservation=38.87321875,
                stop_usd=3.20,
                cap_usd=3.25,
                reserve_usd=8.0,
                leases_authorized=1,
                capture_lead_seconds=600,
                work_lead_seconds=660,
                framebuffer_mib=8192,
                profile_id='GB202_5090_90MIN',
                target_die='GB202',
                source='7dcedefea093de078188efd5f712561bde746362',
            )
            # Fixtures pin actual source/plan bytes. They do not authorize cloud access.
            save(root / 'gb202_admitted_plan.json', plan)
            save(root / 'gb202_admitted_budget.json', budget())
            manifest = dict(
                guard_head='fixturehead',
                profile_id='GB202_5090_90MIN',
                selected_offer=offer(),
                operator_sources={
                    'pinned.py': digest(here / 'pinned.py'),
                    'known_bad_machines.json': digest(here / 'known_bad_machines.json'),
                },
                runtime_source_digests={str(runtime): digest(runtime)},
                product_source=plan['source'],
            )
            if corrupt == 'empty_head':
                manifest['guard_head'] = None
            save(root / 'review_manifest_gb202.json', manifest)
            # A legacy-shaped manifest demonstrates old adapter ignored the added proof.
            save(root / 'review_manifest.json', manifest)
            review = dict(
                decision='GO',
                guard_head='fixturehead',
                profile_id='GB202_5090_90MIN',
                manifest_sha256=digest(root / 'review_manifest_gb202.json'),
                plan_sha256=digest(root / 'gb202_admitted_plan.json'),
                budget_sha256=digest(root / 'gb202_admitted_budget.json'),
                selected_offer=offer(),
            )
            if corrupt == 'empty_head':
                review['guard_head'] = None
            if corrupt in ('profile', 'manifest', 'plan', 'budget', 'head'):
                key = {
                    'profile': 'profile_id',
                    'manifest': 'manifest_sha256',
                    'plan': 'plan_sha256',
                    'budget': 'budget_sha256',
                    'head': 'guard_head',
                }[corrupt]
                review[key] = 'stale-or-foreign'
            if corrupt != 'missing':
                save(root / 'root_guard_go.json', review)
            if corrupt == 'source':
                (here / 'pinned.py').write_text('# changed fake source\n')
            if corrupt == 'dependency':
                runtime.write_text('# changed fake dependency\n')
            backend_plan = dict(plan)
            if corrupt == 'backend_plan':
                backend_plan['deadline_utc'] = (now + dt.timedelta(seconds=5500)).isoformat()
            evidence = root / 'lease_fixture'
            evidence.mkdir()
            ref = G.prepare(
                evidence,
                offer(),
                plan,
                71,
                dict(owner_id=71, complete=True, rows=[]),
                plan['source'],
            )
            save(evidence / 'watch_ready.json', dict(state='ARMED', intent_sha256=ref.sha256))
            b = O.Backend.__new__(O.Backend)
            b.here, b.offer, b.plan = here, offer(), backend_plan
            b.evidence, b.intent_ref, b.label = evidence, ref, G.load(ref)['label']
            b.watch = type('FakeWatch', (), dict(poll=lambda _: None))()
            if corrupt in ('prior_gb206_empty', 'prior_gb206_timeout'):
                self.issue_legacy_fixture(root, corrupt, now)
            with patch.object(
                O.P, 'snapshot', return_value=dict(owner_id=71, complete=True, rows=[])
            ), patch.object(
                O.subprocess, 'Popen', side_effect=ReachedFakeBoundary
            ) as popen, patch.object(
                O.P, 'create_once', return_value=O.subprocess.CompletedProcess([], 0, b'', b'')
            ) as create:
                try:
                    b.arm(backend_plan) if action == 'arm' else b.create()
                except Exception:
                    pass
                called = popen.call_count if action == 'arm' else create.call_count
                self.assertEqual(
                    called, 1 if expect_boundary else 0, f'{action} boundary, corrupt={corrupt}'
                )
                if action == 'arm':
                    create.assert_not_called()
                else:
                    popen.assert_not_called()

    def test_valid_full_proof_reaches_only_fake_arm(self):
        self.fixture('arm', expect_boundary=True)

    def test_valid_full_proof_reaches_only_fake_create(self):
        self.fixture('create', expect_boundary=True)

    def test_gb206_empty_stage_marker_blocks_new_profile_create(self):
        self.fixture('create', 'prior_gb206_empty')

    def test_gb206_timeout_stage_marker_blocks_new_profile_create(self):
        self.fixture('create', 'prior_gb206_timeout')


def refused(action, corrupt):
    def test(self):
        self.fixture(action, corrupt)

    return test


for action in ('arm', 'create'):
    for corrupt in (
        'missing',
        'profile',
        'manifest',
        'plan',
        'budget',
        'head',
        'source',
        'dependency',
        'backend_plan',
        'blacklist_machine',
        'blacklist_host',
        'empty_head',
    ):
        setattr(ProfileAdapter, f'test_{action}_refuses_{corrupt}', refused(action, corrupt))


if __name__ == '__main__':
    unittest.main()
