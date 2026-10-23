import copy,unittest
from stage_verdict import whole_run_pass
BASE={'verdict':'PASS','restored':True,'telemetry_exit':0,'host_stop':{'new_host_xids':0},'proxy':{'verdict':'PASS','phases':12}}
class StageVerdictControls(unittest.TestCase):
 def case(self,**fields):
  result=copy.deepcopy(BASE);result.update(fields);return result
 def test_complete_run_is_accepted(self):self.assertTrue(whole_run_pass(self.case()))
 def test_live_failed_proxy_is_not_a_whole_run_pass(self):self.assertFalse(whole_run_pass(self.case(proxy={'verdict':'FAIL','error':'MODE_CONTROL_FAILED_4'})))
 def test_incomplete_proxy_is_not_a_whole_run_pass(self):self.assertFalse(whole_run_pass(self.case(proxy={'verdict':'INCOMPLETE'})))
 def test_missing_proxy_stage_refuses(self):self.assertFalse(whole_run_pass(self.case(proxy={})))
 def test_only_eleven_primary_phases_refuses(self):self.assertFalse(whole_run_pass(self.case(proxy={'verdict':'PASS','phases':11})))
 def test_zero_primary_phases_refuses(self):self.assertFalse(whole_run_pass(self.case(proxy={'verdict':'PASS','phases':0})))
 def test_unrestored_host_refuses(self):self.assertFalse(whole_run_pass(self.case(restored=False)))
 def test_failed_outer_run_refuses(self):self.assertFalse(whole_run_pass(self.case(verdict='FAIL')))
if __name__=='__main__':unittest.main()
