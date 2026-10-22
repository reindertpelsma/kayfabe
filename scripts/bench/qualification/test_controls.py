"""Exact process-observer controls, including Windows' serialized array wrapper."""
import unittest
from controls import controls_ok

class Controls(unittest.TestCase):
    def setUp(self):
        self.controls=[{'pid':100+i,'expected':n,'actual':n} for i,n in enumerate([0,7,9])]
        self.records=[{'pid':x['pid'],'name':'cmd.exe','exit':x['actual']} for x in self.controls]
    def test_exact_three_records_pass(self):self.assertTrue(controls_ok(self.controls,self.records))
    def test_explicit_windows_array_wrapper_passes(self):self.assertTrue(controls_ok({'value':self.controls,'Count':3},self.records))
    def test_missing_seven_fails(self):self.assertFalse(controls_ok(self.controls,[self.records[0],self.records[2]]))
    def test_missing_nine_fails(self):self.assertFalse(controls_ok(self.controls,self.records[:2]))
    def test_wrong_first_pid_fails(self):self.records[0]['pid']=999;self.assertFalse(controls_ok(self.controls,self.records))
    def test_wrong_second_pid_fails(self):self.records[1]['pid']=999;self.assertFalse(controls_ok(self.controls,self.records))
    def test_wrong_seven_exit_fails(self):self.records[1]['exit']=0;self.assertFalse(controls_ok(self.controls,self.records))
    def test_wrong_nine_exit_fails(self):self.records[2]['exit']=0;self.assertFalse(controls_ok(self.controls,self.records))
    def test_wrong_process_name_fails(self):self.records[1]['name']='other.exe';self.assertFalse(controls_ok(self.controls,self.records))
    def test_incomplete_control_definition_fails(self):self.assertFalse(controls_ok(self.controls[:2],self.records))
    def test_duplicate_pid_fails(self):self.controls[1]['pid']=100;self.assertFalse(controls_ok(self.controls,self.records))
    def test_failed_control_command_fails(self):self.controls[1]['actual']=0;self.assertFalse(controls_ok(self.controls,self.records))
    def test_invalid_array_count_fails(self):self.assertFalse(controls_ok({'value':self.controls,'Count':1},self.records))

if __name__=='__main__':unittest.main()
