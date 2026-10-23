# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Round4 cannot allocate before its destroy guard is armed."""
import importlib.util
from pathlib import Path
import unittest

PATH = Path(__file__).resolve().parents[1] / 'diesweep/lease_guard.py'


def policy():
    spec = importlib.util.spec_from_file_location('s4_lease_guard', PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def offer(**changes):
    row = dict(dph_total=.20, vms_enabled=True, num_gpus=1, disk_space=180,
               geolocation='Ontario, CA', inet_down_cost=.001, inet_up_cost=.001)
    row.update(changes)
    return row


class BudgetTests(unittest.TestCase):
    def test_owner_price_boundary_is_inclusive(self):
        policy().validate_offer(offer(dph_total=2.50))

    def test_above_owner_price_refuses(self):
        with self.assertRaises(ValueError):
            policy().validate_offer(offer(dph_total=2.51))

    def test_denied_country_codes_refuse(self):
        for country in ('CU', 'IR', 'KP', 'SY'):
            with self.subTest(country=country), self.assertRaises(ValueError):
                policy().validate_offer(offer(geolocation='region, ' + country))

    def test_denied_country_names_refuse(self):
        for country in ('Cuba', 'Iran', 'North Korea', 'Syria'):
            with self.subTest(country=country), self.assertRaises(ValueError):
                policy().validate_offer(offer(geolocation=country))

    def test_missing_country_and_nonfinite_price_refuse(self):
        for changes in ({'geolocation': ''}, {'dph_total': float('nan')},
                        {'dph_total': float('inf')}, {'dph_total': 0}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                policy().validate_offer(offer(**changes))

    def test_non_kvm_or_small_disk_refuses(self):
        for changes in ({'vms_enabled': False}, {'disk_space': 179}, {'num_gpus': 2}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                policy().validate_offer(offer(**changes))

    def test_reservation_stays_below_stop_and_three_hours(self):
        plan = policy().reservation(offer(), spent=0, credit=28.8, now=1000)
        self.assertLessEqual(plan['reserved_usd'], 2.50)
        self.assertLessEqual(plan['deadline_epoch'] - 1000, 3 * 3600)
        self.assertAlmostEqual(plan['rate_guard'], .30)

    def test_expensive_quote_cannot_spend_stop_reserve(self):
        with self.assertRaises(ValueError):
            policy().reservation(offer(dph_total=1), spent=0, credit=28.8, now=1000)

    def test_prior_round_spend_is_included(self):
        with self.assertRaises(ValueError):
            policy().reservation(offer(), spent=2, credit=28.8, now=1000)

    def test_five_dollar_credit_reserve_is_untouched(self):
        with self.assertRaises(ValueError):
            policy().reservation(offer(), spent=0, credit=5.1, now=1000)

    def test_transfer_quotes_have_explicit_reservation(self):
        plan = policy().reservation(offer(), spent=0, credit=28.8, now=1000)
        self.assertGreaterEqual(plan['transfer_reserved_usd'], 80 * .001 + 2 * .001)

    def test_negative_spend_and_invalid_duration_refuse(self):
        for changes in ({'spent': -1}, {'minutes': 181}, {'minutes': 0}):
            args = dict(spent=0, credit=28.8, now=1000)
            args.update(changes)
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                policy().reservation(offer(), **args)


class PrearmTests(unittest.TestCase):
    def execute(self, ready=True):
        calls = []
        def arm():
            calls.append('armed')
            return ready
        def create():
            calls.append('create')
            return {'success': True, 'new_contract': 123}
        def save(iid):
            calls.append(('registered', iid))
        result = policy().armed_create(arm, create, save)
        return result, calls

    def test_guard_is_ready_before_create(self):
        _, calls = self.execute()
        self.assertEqual(calls[:2], ['armed', 'create'])

    def test_created_exact_id_is_registered(self):
        result, calls = self.execute()
        self.assertEqual(result, 123)
        self.assertEqual(calls[-1], ('registered', 123))

    def test_missing_ready_marker_refuses_create(self):
        with self.assertRaises(ValueError):
            self.execute(False)

    def test_failed_create_never_registers_any_id(self):
        calls = []
        with self.assertRaises(ValueError):
            policy().armed_create(lambda: True, lambda: {'success': False}, calls.append)
        self.assertEqual(calls, [])

    def test_missing_id_is_never_a_destroy_target(self):
        self.assertIsNone(policy().owned_id({'intent': 'fixture'}, 'fixture'))

    def test_foreign_intent_and_invalid_ids_refuse(self):
        for plan in ({'intent': 'foreign', 'id': 123},
                     {'intent': 'fixture', 'id': 0}, {'intent': 'fixture', 'id': '123'}):
            with self.subTest(plan=plan), self.assertRaises(ValueError):
                policy().owned_id(plan, 'fixture')


class WatchTests(unittest.TestCase):
    def run_watch(self, failures=0):
        import json
        import os
        import subprocess
        import sys
        import tempfile
        import time
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            client = root / 'vastai'
            client.write_text('#!' + sys.executable + '\n' + '''
import json, os, pathlib, sys
root = pathlib.Path(os.environ['LEASE_FIXTURE'])
with (root / 'calls').open('a') as out:
    out.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'show':
    count = root / 'count'
    n = int(count.read_text()) if count.exists() else 0
    count.write_text(str(n + 1))
    if n < int(os.environ['FAILURES']):
        raise SystemExit(7)
    print(json.dumps([{'id': 999}]))
''')
            client.chmod(0o755)
            intent = root / 'intent.json'
            intent.write_text(json.dumps({'intent': 'fixture',
                                          'deadline_epoch': time.time() + .5}))
            env = dict(os.environ, PATH=td + os.pathsep + os.environ['PATH'],
                       LEASE_FIXTURE=td, FAILURES=str(failures))
            child = subprocess.Popen([sys.executable, str(PATH), 'watch', str(intent)],
                                     env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                ready = root / 'intent.ready.json'
                for _ in range(100):
                    if ready.exists() or child.poll() is not None:
                        break
                    time.sleep(.01)
                self.assertTrue(ready.exists())
                self.assertFalse((root / 'calls').exists())
                plan = json.loads(intent.read_text())
                plan['id'] = 123
                temp = root / 'new.json'
                temp.write_text(json.dumps(plan))
                temp.replace(intent)
                stdout, stderr = child.communicate(timeout=12)
                calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()]
                receipt = root / 'intent.absent.json'
                result = json.loads(receipt.read_text()) if receipt.exists() else None
                return child.returncode, calls, result, stderr.decode()
            finally:
                if child.poll() is None:
                    child.terminate()
                child.communicate(timeout=5)

    def test_real_guard_signals_ready_before_any_api_action(self):
        rc, calls, receipt, err = self.run_watch()
        self.assertEqual(rc, 0, err)
        self.assertEqual(calls[0], ['destroy', 'instance', '123', '-y'])

    def test_real_guard_never_destroys_other_account_ids(self):
        rc, calls, receipt, err = self.run_watch()
        self.assertEqual(rc, 0, err)
        self.assertEqual(receipt['id'], 123)
        self.assertTrue(all('999' not in row for row in calls))

    def test_transient_api_error_retries_until_absence(self):
        rc, calls, receipt, err = self.run_watch(failures=1)
        self.assertEqual(rc, 0, err)
        self.assertIsNotNone(receipt)

    def test_transient_api_error_keeps_only_owned_destroy_target(self):
        rc, calls, receipt, err = self.run_watch(failures=1)
        self.assertGreaterEqual(sum(row[0] == 'show' for row in calls), 2)
        self.assertEqual([row[2] for row in calls if row[0] == 'destroy'], ['123', '123'])
