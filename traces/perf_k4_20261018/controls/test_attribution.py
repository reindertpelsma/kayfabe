import unittest
import attribution as A
class AttributionControls(unittest.TestCase):
 def test_unsupported_power_is_not_zero_power(self):self.assertIsNone(A.valid_value({'power_mw':'0','power_mw_rc':'3'},'power_mw'))
 def test_missing_link_status_is_not_x0(self):self.assertIsNone(A.valid_value({'pcie_width':'0'},'pcie_width'))
 def test_real_zero_utilization_is_retained(self):self.assertEqual(A.valid_value({'util_gpu':'0','util_gpu_rc':'0'},'util_gpu'),0)
 def test_nonzero_counter_value_is_retained(self):self.assertEqual(A.valid_value({'pcie_width':'16','pcie_width_rc':'0'},'pcie_width'),16)
 def test_nonfinite_values_refuse(self):
  with self.assertRaises(ValueError):A.valid_value({'x':'NaN','x_rc':'0'},'x')
 def test_eviction_counter_reset_refuses(self):
  with self.assertRaises(ValueError):A.counter_delta([9,3])
 def test_promotion_counter_reset_refuses(self):
  with self.assertRaises(ValueError):A.counter_delta([10,12,1])
 def test_repeated_cumulative_values_not_summed(self):self.assertEqual(A.counter_delta([2,2,5,5]),3)
 def test_zero_eviction_is_positive_observation(self):self.assertEqual(A.counter_delta([0,0,0]),0)
 def test_one_counter_sample_cannot_prove_zero_evictions(self):
  with self.assertRaises(ValueError):A.counter_delta([0])
 def test_guest_local_time_is_not_host_utc(self):self.assertEqual(A.log_epoch('2026-10-23','07:00:00','2026-10-23T07:01:00+02:00'),1792731600)
 def test_guest_epoch_to_host_clock_is_explicit(self):
  lo,hi=A.clock_bounds(1792738800000000000,1792738802000000000,1792731600000);self.assertEqual((lo,hi),(7200,7202))
 def test_invalid_clock_brackets_refuse(self):
  with self.assertRaises(ValueError):A.clock_bounds(20,10,0)
 def test_subtest_interior_omits_load_and_boundary(self):
  self.assertFalse(A.inside(10.5,[{'start':10,'end':20}],margin=1))
 def test_inter_subtest_gap_is_not_gpu_busy_evidence(self):
  self.assertFalse(A.inside(25,[{'start':10,'end':20},{'start':30,'end':40}],margin=1))
 def test_valid_interior_kept(self):self.assertTrue(A.inside(15,[{'start':10,'end':20}],margin=1))
if __name__=='__main__':unittest.main()
