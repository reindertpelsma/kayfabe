"""2026-10-23: exercise the actual owned-id lease timer without network or sleeping."""
from pathlib import Path
import json
import runpy
import subprocess
import sys
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('budget_watch.py')
OWN = 12345678
FOREIGN = 87654321

class LeaseTimer(unittest.TestCase):
    def invoke(self, answers=None, seconds=10200, ledger=None):
        ledger = [{'id': OWN, 'destroyed': False}] if ledger is None else ledger
        answers = ['[]'] if answers is None else answers
        writes = []
        def write(path, text, *args, **kwargs):
            writes.append((path.name, json.loads(text)))
            return len(text)
        with patch.object(sys, 'argv', [str(SCRIPT), str(OWN), str(seconds)]), \
             patch.object(Path, 'read_text', return_value=json.dumps(ledger)), \
             patch.object(Path, 'write_text', autospec=True, side_effect=write), \
             patch('time.sleep', side_effect=lambda value: writes.append(
                 ('_sleep_', {'seconds': value}))) as sleep, \
             patch('subprocess.run') as destroy, \
             patch('subprocess.check_output', side_effect=answers) as show:
            runpy.run_path(str(SCRIPT), run_name='__main__')
        return writes, sleep, destroy, show

    def test_170_minute_timer_announces_ready_before_sleep(self):
        writes, sleep, destroy, show = self.invoke()
        self.assertEqual(sleep.call_args_list[0].args, (10200.0,))
        ready = [v for n, v in writes if n.startswith('timer_ready_')]
        self.assertEqual(len(ready), 1)
        self.assertEqual(ready[0]['id'], OWN)
        self.assertEqual(ready[0]['seconds'], 10200.0)
        names = [n for n, v in writes]
        self.assertLess(next(i for i, n in enumerate(names) if n.startswith('timer_ready_')),
                        names.index('_sleep_'))

    def test_shorter_budget_timer_also_announces_ready(self):
        writes, sleep, destroy, show = self.invoke(seconds=5400)
        self.assertEqual(sleep.call_args_list[0].args, (5400.0,))
        self.assertEqual(len([n for n, v in writes if n.startswith('timer_ready_')]), 1)

    def test_unknown_id_refuses_before_any_sleep_or_api(self):
        with self.assertRaises((AssertionError, ValueError)):
            self.invoke(ledger=[{'id': FOREIGN, 'destroyed': False}])

    def test_empty_owned_ledger_refuses(self):
        with self.assertRaises((AssertionError, ValueError)):
            self.invoke(ledger=[])

    def test_closed_id_cannot_arm_a_fresh_timer(self):
        with self.assertRaises((AssertionError, ValueError)):
            self.invoke(ledger=[{'id': OWN, 'destroyed': True}])

    def test_zero_delay_refuses(self):
        with self.assertRaises((AssertionError, ValueError)):
            self.invoke(seconds=0)

    def test_over_170_minutes_refuses(self):
        with self.assertRaises((AssertionError, ValueError)):
            self.invoke(seconds=10201)

    def test_another_lane_instance_does_not_block_own_absence(self):
        writes, sleep, destroy, show = self.invoke(answers=[json.dumps([{'id': FOREIGN}])])
        self.assertEqual(destroy.call_count, 1)
        self.assertTrue(any(v.get('absence_verified_utc') for n, v in writes))
        self.assertEqual(destroy.call_args.args[0], ['vastai','destroy','instance',str(OWN),'-y'])

    def test_many_foreign_instances_are_never_destroyed(self):
        writes, sleep, destroy, show = self.invoke(answers=[json.dumps([{'id': FOREIGN}, {'id': 99}])])
        self.assertEqual(destroy.call_count, 1)
        self.assertTrue(all(str(FOREIGN) not in c.args[0] for c in destroy.call_args_list))
        self.assertTrue(any(v.get('absence_verified_utc') for n, v in writes))

    def test_transient_api_failure_retries(self):
        error = subprocess.CalledProcessError(1, ['vastai','show','instances'])
        writes, sleep, destroy, show = self.invoke(answers=[error,'[]'])
        self.assertEqual(destroy.call_count, 2)
        self.assertTrue(any(v.get('absence_verified_utc') for n, v in writes))

    def test_transient_invalid_json_retries(self):
        writes, sleep, destroy, show = self.invoke(answers=['invalid-json','[]'])
        self.assertEqual(destroy.call_count, 2)
        self.assertTrue(any(v.get('absence_verified_utc') for n, v in writes))

    def test_live_own_id_retries_until_exact_absence(self):
        live = json.dumps([{'id': OWN},{'id': FOREIGN}])
        writes, sleep, destroy, show = self.invoke(answers=[live,live,'[]'])
        self.assertEqual(destroy.call_count, 3)
        self.assertTrue(any(v.get('absence_verified_utc') for n, v in writes))

    def test_persistent_live_id_never_gets_an_absence_receipt(self):
        with self.assertRaises(RuntimeError):
            self.invoke(answers=[json.dumps([{'id': OWN}])]*5)

if __name__ == '__main__':
    unittest.main()
