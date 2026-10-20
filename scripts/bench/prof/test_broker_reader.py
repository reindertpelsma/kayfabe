# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util,os,subprocess,tempfile,unittest
from pathlib import Path
sp=importlib.util.spec_from_file_location('reader',Path(__file__).with_name('reader.py'));r=importlib.util.module_from_spec(sp);sp.loader.exec_module(r)
class BrokerReader(unittest.TestCase):
 def fixture(self,d,case):
  exe=os.environ.get('KF_PROF_C_BIN')
  if not exe:self.skipTest('set KF_PROF_C_BIN to the owned broker C fixture')
  subprocess.run([exe,case],env=dict(os.environ,KF_PROF_C_KEEP_DIR=d),check=True,capture_output=True,timeout=5)
 def test_c_writer_record_and_clock_are_understood_by_the_shared_reader(self):
  with tempfile.TemporaryDirectory() as d:
   self.fixture(d,'content');data=r.read_mmap(Path(d)/'prof.mmap');self.assertEqual(data['header']['clock_mode'],'broker_monotonic');row=data['records'][0];self.assertEqual(row['counters']['broker_presented_new_frames'],2);self.assertEqual(row['pacing']['max_gap_ns'],50000000);self.assertEqual(r.read_jsonl(Path(d)/'prof.jsonl'),data['records'])
 def test_c_wire_geometry_and_lifecycle_counters_survive_close(self):
  with tempfile.TemporaryDirectory() as d:
   self.fixture(d,'wire');data=r.read_mmap(Path(d)/'prof.mmap');self.assertEqual(len(data['records']),1);self.assertEqual(data['records'][0]['broker']['received'],1);self.assertEqual(data['records'][0]['broker']['released'],1)
if __name__=='__main__':unittest.main()
