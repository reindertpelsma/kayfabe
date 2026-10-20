# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('metric', Path(__file__).with_name('metric.py'))
metric = importlib.util.module_from_spec(spec)
if spec.loader and Path(spec.origin).exists():
    spec.loader.exec_module(metric)


class MetricTests(unittest.TestCase):
    def run_case(self, gaps=(5,), end=None, process_end=None, two=False):
        # 110 presents in 1.09s; then independent long gaps, with activity re-arming.
        points = [i / 100 for i in range(110)]
        for gap in gaps:
            points += [points[-1] + gap + i / 100 for i in range(110)]
        rows = [(round(t * 1000), 'Heaven.exe', 1) for t in points]
        if two:
            rows += [(i * 10, 'Heaven.exe', 2) for i in range(int(points[-1] * 100))]
        return metric.silences(rows, 1000, 0, end or points[-1], {1: process_end} if process_end else {})

    def test_gap_count(self):
        self.assertEqual(self.run_case()['Heaven.exe']['count'], 1)

    def test_total_and_longest(self):
        r = self.run_case((5, 7))['Heaven.exe']
        self.assertAlmostEqual(r['total_s'], 12)
        self.assertAlmostEqual(r['longest_s'], 7)

    def test_exact_four_seconds(self):
        self.assertEqual(self.run_case((4,))['Heaven.exe']['count'], 1)

    def test_subthreshold(self):
        self.assertEqual(self.run_case((3.99,))['Heaven.exe']['count'], 0)

    def test_process_identity(self):
        self.assertEqual(self.run_case(two=True)['Heaven.exe']['count'], 1)

    def test_terminal_censor(self):
        r = self.run_case((), end=8)['Heaven.exe']
        self.assertEqual(r['count'], 1)
        self.assertEqual(r['right_censored'], 1)

    def test_normal_exit_not_terminal_freeze(self):
        self.assertEqual(self.run_case((), end=8, process_end=1.1)['Heaven.exe']['count'], 0)

    def test_insufficient_activity_not_armed(self):
        r = metric.silences([(0, 'a', 1), (5000, 'a', 1)], 1000, 0, 5, {})
        self.assertEqual(r['a']['count'], 0)

    def test_prefixed_csv_uses_present_qpc(self):
        header = 'Application,ProcessID,TimeInQPC\n'
        r, skipped = metric.read_csv((header + '9999,Heaven.exe,7,1000\n').splitlines())
        self.assertEqual(r, [(1000, 'Heaven.exe', 7)])
        self.assertEqual(skipped, 0)

    def test_repeated_header_and_partial_row(self):
        r, skipped = metric.read_csv(['Application,ProcessID,TimeInQPC',
            '9000,a,1,1000', 'Application,ProcessID,TimeInQPC', '9001,a,1,1010', 'bad,row'])
        self.assertEqual(len(r), 2)
        self.assertEqual(skipped, 1)

    def test_nonmonotonic_clock_refused(self):
        with self.assertRaises(ValueError):
            metric.silences([(1000, 'a', 1), (0, 'a', 1)], 1000, 0, 5, {})

    def test_missing_header_refused(self):
        with self.assertRaises(ValueError):
            metric.read_csv(['1,a,7,1000'])


if __name__ == '__main__':
    unittest.main()
