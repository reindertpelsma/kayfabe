"""Reserved stability time must remain observable after app-dispatch budget exhaustion."""
from pathlib import Path
import importlib.util
from types import SimpleNamespace
import unittest
PATH=Path(__file__).resolve().parents[1]/'diesweep/desktop/matrix.py'
class BudgetHoldTests(unittest.TestCase):
 def observe(self,seconds=600):
  spec=importlib.util.spec_from_file_location('budget_matrix',PATH);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
  exhausted=getattr(m,'ObservationBudgetExhausted',type('ObservationBudgetExhausted',(RuntimeError,),{}))
  calls=[]
  def rows():
   yield {'id':'driver_status'}
   raise exhausted('lease observation/export budget exhausted')
  def app(a):calls.append(a['id']);return dict(verdict='PASS',restart='')
  def health():calls.append('health')
  def hold():calls.append('hold');return seconds
  self.calls=calls
  return m.run_plan(SimpleNamespace(run_app=app),rows(),lambda r:None,health,hold),calls
 def test_budget_stop_runs_reserved_hold_after_completed_row(self):
  rc,calls=self.observe();self.assertIn('hold',calls);self.assertLess(calls.index('driver_status'),calls.index('hold'))
 def test_healthy_partial_matrix_cannot_claim_complete_pass(self):
  rc,calls=self.observe();self.assertEqual(rc,2);self.assertEqual(calls[-1],'health')
 def test_short_reserved_hold_still_refuses(self):
  with self.assertRaisesRegex(RuntimeError,'incomplete stability hold'):self.observe(seconds=599)
if __name__=='__main__':unittest.main()
