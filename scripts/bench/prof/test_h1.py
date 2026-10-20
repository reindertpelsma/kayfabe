# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util,json,os,sys,tempfile,unittest
from pathlib import Path
import h1_export
class H1Export(unittest.TestCase):
 def row(self):return dict(schema=1,start_ns=0,end_ns=200000000,threads=[],ledger={'oldest':[]},counters={},coverage={'production_sources':False})
 def test_export_is_context_without_a_fabricated_frame_link(self):
  r=self.row();event=h1_export.context_records([r])[0];self.assertEqual(event['kind'],'prof_snapshot');self.assertEqual(event['ts_ns'],0);self.assertEqual(event['end_ns'],200000000);self.assertEqual(event['prof'],r);self.assertNotIn('frame_seq',event);self.assertNotIn('stage',event);self.assertFalse(event['full_causal_coverage'])
 def test_coverage_cannot_be_promoted_by_a_well_formed_snapshot(self):
  r=self.row();r['coverage']['production_sources']=True;self.assertFalse(h1_export.context_records([r])[0]['full_causal_coverage'])
 def test_pinned_h1_reader_and_analyzer_keep_context_and_unknown_attribution(self):
  root=os.environ.get('KF_PROF_H1_DIR')
  if not root:self.skipTest('set KF_PROF_H1_DIR to the owned pinned H1 fixture')
  old=list(sys.path);sys.path.insert(0,root)
  try:
   spec=importlib.util.spec_from_file_location('pinned_h1',Path(root)/'analyze.py');a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
  finally:sys.path[:]=old
  row=self.row();row['end_ns']=400000000;events=h1_export.context_records([row])
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'context.jsonl';p.write_text(''.join(json.dumps(e)+'\n'for e in events));loaded=a.read_jsonl(p)
  result=a.analyze(loaded,[{'ts_ns':0,'seq':1,'new':True},{'ts_ns':200000000,'seq':2,'new':True}],coverage=False)
  self.assertEqual(len(result['gaps']),1);self.assertEqual(result['gaps'][0]['class'],'UNKNOWN');self.assertTrue(any(e['kind']=='prof_snapshot'for e in result['gaps'][0]['events']));self.assertEqual(next(e['prof']for e in result['gaps'][0]['events']if e['kind']=='prof_snapshot'),row)
 def test_actual_writer_fixture_preserves_the_clock_and_loss_coverage(self):
  import reader
  fixture=Path(__file__).resolve().parents[3]/'traces/kf_prof_20261019/format_fixture.jsonl';rows=reader.read_jsonl(fixture);self.assertEqual(len(rows),1);e=h1_export.context_records(rows)[0];self.assertEqual(e['prof']['clock_mode'],'diagnostic_os_clock');self.assertEqual(e['prof']['coverage'],rows[0]['coverage']);self.assertFalse(e['full_causal_coverage'])
if __name__=='__main__':unittest.main()
