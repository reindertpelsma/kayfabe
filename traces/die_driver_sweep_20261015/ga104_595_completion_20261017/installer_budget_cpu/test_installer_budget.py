"""Regression from the measured GA104 preparation interval, not a GPU success oracle."""
import ast,datetime,math,re,sys,unittest
from pathlib import Path
controller=Path(sys.argv.pop(1))
stager_path=Path(sys.argv.pop(1))
class InstallerEnvelope(unittest.TestCase):
 def test_parent_allows_measured_preparation_child_wait_and_capture(self):
  t0=datetime.datetime.fromisoformat('2026-10-17T03:17:37.571046+00:00')
  t1=datetime.datetime.fromisoformat('2026-10-17T03:22:52.395945+00:00')
  prep=(t1-t0).total_seconds()
  stager=stager_path.read_text()
  child=int(re.search(r'WaitForExit\((\d+)\)',stager)[1])/1000
  calls=[n for n in ast.walk(ast.parse(controller.read_text())) if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='ps' and len(n.args)>=2 and isinstance(n.args[0],ast.Constant) and n.args[0].value=='ampere59520261017_install58088']
  self.assertEqual(len(calls),1)
  budget=ast.literal_eval(calls[0].args[1])
  required=math.ceil(prep+child+30)
  self.assertGreaterEqual(budget,required,f'parent {budget}s cuts measured {prep:.3f}s preparation + {child}s child wait + capture margin; need >={required}s')
  self.assertLessEqual(budget,900,'bounded QGA helper refuses programs over900s')
unittest.main()
