# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import reader,unittest
class PacingTests(unittest.TestCase):
 def row(self):return dict(start_ns=100,end_ns=2000000100,counters={'broker_presented_new_frames':100},broker={'repeated':2},pacing={'p99_gap_ns':20000000,'p999_gap_ns':50000000})
 def test_interval_exposure_sets_fps_and_repeat_rate(self):
  p=reader.pacing(self.row());self.assertEqual(p['new_fps'],50);self.assertEqual(p['repeated_per_second'],1)
 def test_another_interval_is_not_treated_as_one_second(self):
  r=self.row();r['end_ns']=500000100;p=reader.pacing(r);self.assertEqual(p['new_fps'],200);self.assertEqual(p['repeated_per_second'],4)
 def test_lows_use_the_99th_and_999th_gap_percentiles(self):
  p=reader.pacing(self.row());self.assertEqual(p['low_1pct_fps'],50);self.assertEqual(p['low_01pct_fps'],20)
 def test_missing_counts_and_zero_gaps_are_unknown(self):
  p=reader.pacing({'start_ns':0,'end_ns':100,'counters':{},'pacing':{'p99_gap_ns':0}});self.assertIsNone(p['new_fps']);self.assertIsNone(p['low_1pct_fps']);self.assertIsNone(p['low_01pct_fps'])
if __name__=='__main__':unittest.main()
