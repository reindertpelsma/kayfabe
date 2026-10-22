#!/usr/bin/env python3
"""GPU-free falsifiers for the thin Vast payload wrapper and evidence parsers."""
import importlib.util,subprocess,tempfile,unittest
from pathlib import Path
SHA='a'*40
PROVISION='\n'.join([k+'_RC=0' for k in ['BOX','DRIVER','TREE','LADDER','FG','KF3']])+"\nREADY driver=580.159.04\nEXIT 2026-10-22T20:00:00Z\n"
MERGE='HEAD='+SHA+' CHECKOUT=/workspace/bench/verify-worktrees/fixture\nTESTS passed 2000 failed 0\nGATES_RC=0\nV3_GATES_SUMMARY pass=9 fail=0 gate10=PASS\nKF3_RC=0\nBARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30\nFG_RC=0\nSUITE_RC=0\nFAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30\nBIRTH_CENSUS_OK arms=30\nEXIT rc=0 2026-10-22T20:00:00Z\n'
def ladder(mode='host'):
 return 'CUDA_LADDER_STARTED=now mode='+mode+' reps=1 rungs=cup2,cup3,cup8,cup8bench rev='+SHA[:8]+'\n'+''.join('CL_ROW mode='+mode+' rung='+r+' rep=1 verdict=PASS '+('rc=0 wall_ms=1' if mode=='host' else 'boot_rc=0')+' graded=[PASS]\n' for r in ['cup2','cup3','cup8','cup8bench'])+'CUDA_LADDER_DONE=now\n'
def watch(delta='',before=0,after=0,rc=0):
 return '[xid-watch] host_dmesg_lines=20  (readable)\n[xid-watch] xid_before='+str(before)+'\n[xid-watch] --- command output begins ---\nfixture\n[xid-watch] --- command output ends (rc='+str(rc)+') ---\n[xid-watch] xid_after='+str(after)+'\n'+('[xid-watch] ★★★ NEW HOST Xid ACROSS THE COMMAND — the run is VOID whatever it printed:\n'+''.join('    '+s+'\n' for s in delta.splitlines())+'[xid-watch]   ⊘ An explanatory line\n' if delta else '[xid-watch] ★ HOST Xid CLEAN\n')
XID='[ 20.1] NVRM: Xid (PCI:0000:01:00): 31, pid=123, name=fixture, MMU Fault: faulted @ 0xa_11000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE'
MARK='FAULT_MARK=victim_release va=0x0000000a11000000\n'
class PayloadTests(unittest.TestCase):
 def setUp(self):
  spec=importlib.util.spec_from_file_location('payload',Path(__file__).with_name('vast_payload.py'));self.m=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.m)
 def test_provision_requires_every_exit(self):self.assertEqual(self.m.provision_rows(PROVISION,0),[{'name':'provision','pass':1,'fail':0,'skip':0}])
 def test_provision_red_cannot_be_hidden_by_ready(self):
  with self.assertRaises(self.m.EvidenceError):self.m.provision_rows(PROVISION.replace('DRIVER_RC=0','DRIVER_RC=1'),0)
 def test_provision_missing_build_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.provision_rows(PROVISION.replace('KF3_RC=0\n',''),0)
 def test_merge_exact_revision_and_counts(self):self.assertEqual([r['pass'] for r in self.m.merge_rows(MERGE,0,SHA)],[2000,9,1,30,30,1])
 def test_merge_wrong_revision_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.merge_rows(MERGE.replace(SHA,'b'*40),0,SHA)
 def test_merge_missing_thin_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.merge_rows(MERGE.replace('FAST_SUITE_PASS=30','FAST_SUITE_PASS=29'),0,SHA)
 def test_merge_missing_birth_census_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.merge_rows(MERGE.replace('BIRTH_CENSUS_OK','BIRTH_CENSUS_NOT_OK'),0,SHA)
 def test_merge_nonzero_exit_refuses_even_green_text(self):
  with self.assertRaises(self.m.EvidenceError):self.m.merge_rows(MERGE,1,SHA)
 def test_gate10_fallback_is_existing_allowed_bar(self):self.assertEqual(self.m.merge_rows(MERGE.replace('gate10=PASS','gate10=FALLBACK'),0,SHA)[1]['pass'],9)
 def test_ladder_host_four_unique_correct_rungs(self):self.assertEqual(self.m.ladder_rows(ladder(),0,'host',SHA)[0]['pass'],4)
 def test_ladder_fat_four_unique_correct_rungs(self):self.assertEqual(self.m.ladder_rows(ladder('guest'),0,'guest',SHA)[0]['name'],'cuda_fat')
 def test_ladder_red_row_cannot_hide_behind_exit_zero(self):
  with self.assertRaises(self.m.EvidenceError):self.m.ladder_rows(ladder().replace('rung=cup3 rep=1 verdict=PASS','rung=cup3 rep=1 verdict=FAIL'),0,'host',SHA)
 def test_ladder_duplicate_row_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.ladder_rows(ladder().replace('rung=cup3','rung=cup2'),0,'host',SHA)
 def test_ladder_wrong_revision_refuses(self):
  with self.assertRaises(self.m.EvidenceError):self.m.ladder_rows(ladder().replace(SHA[:8],'b'*8),0,'host',SHA)
 def test_ladder_nonzero_boot_refuses_even_pass(self):
  with self.assertRaises(self.m.EvidenceError):self.m.ladder_rows(ladder('guest').replace('boot_rc=0','boot_rc=1'),0,'guest',SHA)
 def test_first_raw_red_is_detected(self):self.assertTrue(self.m.red_line('BARE_CELL_ARM arm=timer verdict=FAIL secs=1'))
 def test_first_thin_red_is_detected(self):self.assertTrue(self.m.red_line('FAST_CELL_ARM arm=timer verdict=NOTRUN secs=?'))
 def test_provision_red_is_detected(self):self.assertTrue(self.m.red_line('BOX_RC=1'))
 def test_progress_text_is_never_a_verdict(self):self.assertFalse(self.m.red_line('a discussion of FAIL and PASS'))
 def test_xid_named_missing_page_control_allowed(self):self.assertTrue(self.m.xid_ok(watch(XID,0,1),6,'--missing-page-fault',MARK))
 def test_xid_positive_arm_cannot_borrow_control(self):self.assertFalse(self.m.xid_ok(watch(XID,0,1),6,'--timer',MARK))
 def test_xid_wrong_address_refuses(self):self.assertFalse(self.m.xid_ok(watch(XID.replace('0xa_11000000','0xa_12000000'),0,1),6,'--missing-page-fault',MARK))
 def test_xid_wrong_code_refuses(self):self.assertFalse(self.m.xid_ok(watch(XID.replace('): 31,','): 79,'),0,1),6,'--missing-page-fault',MARK))
 def test_xid_excess_count_refuses(self):self.assertFalse(self.m.xid_ok(watch(XID+'\n'+XID.replace('20.1','20.2'),0,2),6,'--missing-page-fault',MARK))
 def test_xid_lost_baseline_refuses(self):self.assertFalse(self.m.xid_ok(watch(XID,3,3),6,'--missing-page-fault',MARK))
 def test_xid_instrument_unreadable_refuses(self):self.assertFalse(self.m.xid_ok('UNWATCHED',7,'--missing-page-fault',MARK))
 def test_xid_cannot_rescue_failed_command(self):self.assertFalse(self.m.xid_ok(watch(XID,0,1,rc=1),6,'--missing-page-fault',MARK))
 def test_xid_clean_readable_control(self):self.assertTrue(self.m.xid_ok(watch(),0,'--timer',''))
 def test_wrapper_delegates_existing_flows(self):
  source=Path(self.m.__file__).read_text();self.assertIn('provision_full.sh',source);self.assertIn('merge_check.sh',source);self.assertIn('cuda_ladder.sh',source);self.assertNotIn('git clone',source);self.assertNotIn('execd',source)
 def test_payload_shell_has_no_reimplemented_build(self):
  source=Path(__file__).with_name('vast_payload.sh').read_text();self.assertIn('vast_payload.py',source);self.assertNotIn('cargo build',source)
if __name__=='__main__':unittest.main()
