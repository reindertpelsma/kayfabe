import unittest
from clock_envelope import envelope,certified_interval,certified_inside
class ClockEnvelopeControls(unittest.TestCase):
 def test_union_keeps_the_widest_low_bound(self):self.assertEqual(envelope([(2,6),(3,5)]),(2,6))
 def test_union_keeps_a_later_high_bound(self):self.assertEqual(envelope([(2,3),(4,7)]),(2,7))
 def test_reversed_observation_refuses(self):
  with self.assertRaises(ValueError):envelope([(6,2)])
 def test_no_clock_measurement_refuses(self):
  with self.assertRaises(ValueError):envelope([])
 def test_certified_interval_uses_both_worst_extrema(self):self.assertEqual(certified_interval(10,20,(2,6),1),(17,21))
 def test_center_only_accepts_before_real_start(self):self.assertFalse(certified_inside(15.5,10,20,(2,6),0))
 def test_center_only_accepts_after_real_end(self):self.assertFalse(certified_inside(23.5,10,20,(2,6),0))
 def test_true_common_interior_is_kept(self):self.assertTrue(certified_inside(19,10,20,(2,6),1))
 def test_uncertainty_can_remove_a_whole_short_subtest(self):self.assertIsNone(certified_interval(10,12,(2,6),1))
 def test_upper_boundary_is_not_interior(self):self.assertFalse(certified_inside(21,10,20,(2,6),1))
if __name__=='__main__':unittest.main()
