# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import json
import os
from pathlib import Path
import pickle
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'sn_ddi.py'


class ProfilerContextContract(unittest.TestCase):
    def decode(self, records):
        ev = {'105': [], '106': [], '_p1_profiler_order': []}
        for t, enter, tid, cpu, fn in records:
            k = '105' if enter else '106'
            ev[k].append((t, 4, tid, cpu, fn))
            ev['_p1_profiler_order'].append((bool(enter), len(ev[k]) - 1))
        with tempfile.TemporaryDirectory() as directory:
            r = Path(directory)
            (r / 'x_ev.pkl').write_bytes(pickle.dumps((ev, len(records))))
            (r / 'x_metrics.json').write_text(json.dumps({'window_filetime_100ns': [0, 10_000_000], 'frames': 10}))
            subprocess.run([sys.executable, str(SCRIPT), str(r), 'x'], check=True, capture_output=True, env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
            return json.loads((r / 'x_ddi.json').read_text())

    def test_simultaneous_tid_zero_isr_durations_belong_to_their_own_cpu(self):
        r = self.decode([(1000, 1, 0, 0, 'ISR'), (1100, 1, 0, 1, 'ISR'),
                         (1200, 0, 0, 0, 'ISR'), (1400, 0, 0, 1, 'ISR')])
        self.assertEqual(r['functions']['ISR']['med_us'], 30)
        self.assertEqual(r['functions']['ISR']['max_us'], 30)

    def test_distinct_tid_zero_functions_on_other_cpus_do_not_break_each_others_stack(self):
        r = self.decode([(1000, 1, 0, 0, 'A'), (1100, 1, 0, 1, 'B'),
                         (1200, 0, 0, 0, 'A'), (1400, 0, 0, 1, 'B')])
        self.assertEqual(r['mismatched_exits'], 0)
        self.assertEqual(r['unmatched_exits'], 0)
        self.assertEqual(sum(f['calls'] for f in r['functions'].values()), 2)

    def test_same_tick_nested_calls_keep_the_recorded_entry_exit_order(self):
        r = self.decode([(1000, 1, 7, 0, 'A'), (1000, 1, 7, 0, 'B'),
                         (1000, 0, 7, 0, 'B'), (1000, 0, 7, 0, 'A')])
        self.assertEqual(r['unmatched_exits'], 0)
        self.assertEqual(r['unclosed_entries'], 0)
        self.assertEqual(sum(f['calls'] for f in r['functions'].values()), 2)

    def test_a_zero_duration_isr_is_a_call_at_one_tick(self):
        r = self.decode([(1000, 1, 0, 0, 'ISR'), (1000, 0, 0, 0, 'ISR')])
        self.assertEqual(r['unmatched_exits'], 0)
        self.assertEqual(r['functions']['ISR']['calls'], 1)

    def test_a_nonzero_thread_can_migrate_between_entry_and_exit(self):
        r = self.decode([(1000, 1, 7, 0, 'WAIT'), (1400, 0, 7, 1, 'WAIT')])
        self.assertEqual(r['functions']['WAIT']['calls'], 1)
        self.assertEqual(r['functions']['WAIT']['med_us'], 40)


if __name__ == '__main__':
    unittest.main()
