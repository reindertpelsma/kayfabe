"""Execute the real admission receipt block with isolated files and elapsed budgets."""
import datetime
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ARM=Path(__file__).resolve().parents[1]/'diesweep/desktop/arm.sh'
class ArmAdmissionTests(unittest.TestCase):
 def run_guard(self,attempts=2,minutes=0):
  src=ARM.read_text().split('<<\'PY\'\n',1)[1].split('\nPY\n',1)[0]
  with tempfile.TemporaryDirectory() as d:
   root=Path(d);(root/'root/s2').mkdir(parents=True);(root/'root/prov').mkdir()
   (root/'root/s2/lease.json').write_text(json.dumps({'id':123,'created_utc':(datetime.datetime.now(datetime.timezone.utc)-datetime.timedelta(minutes=minutes)).isoformat(),'auto_destroy_minutes':170}))
   (root/'root/prov/arm_counts.json').write_text(json.dumps({'on':attempts,'off':0}))
   for n in ('arm.sh','rpc_timing.py'):(root/'root/s2'/n).write_text('fixture')
   binary=root/'binary';binary.write_text('fixture')
   x=subprocess.run(['python3','-','on','trial','a'*40,str(binary)],input=src.replace('/root/',str(root)+'/root/'),capture_output=True,text=True,timeout=10)
   receipt=root/'root/prov/arm_trial_start_receipt.json'
   return x,json.loads(receipt.read_text()) if receipt.exists() else None,json.loads((root/'root/prov/arm_counts.json').read_text())
 def test_third_owned_harness_attempt_admitted_within_budget(self):
  x,r,c=self.run_guard();self.assertEqual(x.returncode,0,x.stderr);self.assertEqual(r['cell_attempt'],3)
 def test_fourth_attempt_retains_all_failed_attempt_counts(self):
  x,r,c=self.run_guard(attempts=3);self.assertEqual(x.returncode,0,x.stderr);self.assertEqual(c['on'],4)
 def test_latest_start_guard_still_refuses(self):
  x,r,c=self.run_guard(minutes=146);self.assertNotEqual(x.returncode,0);self.assertIsNone(r);self.assertEqual(c['on'],2)
 def test_expired_lease_cannot_create_receipt(self):
  x,r,c=self.run_guard(minutes=171);self.assertNotEqual(x.returncode,0);self.assertIsNone(r)
if __name__=='__main__':unittest.main()
