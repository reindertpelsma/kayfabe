# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util
from pathlib import Path
import tempfile,unittest
sp=importlib.util.spec_from_file_location('prof_merge',Path(__file__).with_name('merge.py'));m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)
class MergeTests(unittest.TestCase):
 def sample(self,send=100,recv=120):return dict(host_send_ns=send,host_recv_ns=recv,guest_qpc=1000,frequency=1000000000,guest_boot_id=7)
 def cal(self):return m.calibrate_ping_pong([self.sample()],valid_from_ns=100,valid_until_ns=10000,drift_ppm=100)
 def test_diag_bracket_midpoint_retains_uncertainty(self):
  c=self.cal();self.assertEqual(c['host_ns0'],110);self.assertEqual(c['error_ns'],10);self.assertEqual(c['method'],'diag_ping_pong');self.assertEqual(m.align_guest({'qpc':2000,'guest_boot_id':7},c)['ts_ns'],1110)
 def test_missing_calibration_says_unaligned(self):
  r=m.align_guest({'qpc':2000,'guest_boot_id':7},None);self.assertEqual(r['alignment'],'UNALIGNED');self.assertIsNone(r['ts_ns'])
 def test_expired_or_other_boot_is_unaligned(self):
  for r in [{'qpc':20000,'guest_boot_id':7},{'qpc':1000,'guest_boot_id':8}]:self.assertEqual(m.align_guest(r,self.cal())['alignment'],'UNALIGNED')
 def test_minimum_rtt_sample_is_selected_with_its_own_anchor(self):
  c=m.calibrate_ping_pong([self.sample(80,160),self.sample(100,120)],valid_from_ns=100,valid_until_ns=10000,drift_ppm=100);self.assertEqual(c['host_ns0'],110);self.assertEqual(c['sample_count'],2);self.assertEqual(c['error_ns'],10)
 def test_invalid_frequency_or_backwards_bracket_is_rejected(self):
  for key,value in [('frequency',0),('frequency',-1),('host_recv_ns',90),('guest_qpc',2**64)]:
   s=self.sample();s[key]=value
   with self.assertRaises(ValueError):m.calibrate_ping_pong([s],valid_from_ns=100,valid_until_ns=10000,drift_ppm=100)
 def test_clock_drift_error_grows_away_from_anchor(self):
  r=m.align_guest({'qpc':9000,'guest_boot_id':7},self.cal());self.assertEqual(r['clock_error_ns'],11)
 def test_merge_keeps_sources_and_does_not_invent_cross_source_identity(self):
  row={'schema':1,'start_ns':100,'end_ns':200,'threads':[],'ledger':{'oldest':[]},'counters':{}}
  out=m.merge_records([row],[row],[{'qpc':1000,'guest_boot_id':7,'event_id':153}],None);self.assertEqual([r['source']for r in out['records']],['kayfabe','broker']);self.assertEqual(out['guest'][0]['alignment'],'UNALIGNED');self.assertEqual(out['cross_source_identity'],'UNVERIFIED')
 def test_oversize_guest_csv_line_is_rejected(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'guest.csv';p.write_text('qpc,guest_boot_id,event_id\n'+'9'*5000+',7,153\n')
   with self.assertRaises(ValueError):m.read_guest_csv(p)
 def test_guest_csv_prints_only_bounded_integer_fields(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'guest.csv';p.write_text('qpc,guest_boot_id,event_id,pid,tid,guest_text\n1000,7,153,2,3,untrusted\n');rows=m.read_guest_csv(p);self.assertEqual(rows,[{'qpc':1000,'guest_boot_id':7,'event_id':153,'pid':2,'tid':3}])
if __name__=='__main__':unittest.main()
