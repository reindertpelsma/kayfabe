import unittest
from arm_history import may_run_apps

class ArmHistory(unittest.TestCase):
    def test_a_clean_b1_arm_after_the_prior_off_pair_may_run_apps(self):self.assertTrue(may_run_apps(['native31'],['native31','off32a','off32b'],['native31','off32a','off32b']))
    def test_a_clean_sibling_after_one_prior_diagnostic_fault_may_run_apps(self):self.assertTrue(may_run_apps(['native31'],['native31','old32'],['native31','old32']))
    def test_one_new_fault_in_this_arm_refuses_workloads(self):self.assertFalse(may_run_apps(['native31'],['native31'],['native31','this32']))
    def test_two_new_faults_in_this_arm_refuse_workloads(self):self.assertFalse(may_run_apps(['native31'],['native31'],['native31','this32a','this32b']))
    def test_a_lost_prior_record_cannot_turn_this_arm_green(self):self.assertFalse(may_run_apps(['native31'],['native31','old32'],['native31']))
    def test_a_replaced_record_of_equal_length_is_not_clean(self):self.assertFalse(may_run_apps(['native31'],['native31','old32'],['native31','different32']))
    def test_a_corrupt_native_prefix_is_not_a_clean_snapshot(self):self.assertFalse(may_run_apps(['native31'],['wrong'],['wrong']))
    def test_the_first_fault_free_arm_still_runs_apps(self):self.assertTrue(may_run_apps(['native31'],['native31'],['native31']))

if __name__=='__main__':unittest.main()
