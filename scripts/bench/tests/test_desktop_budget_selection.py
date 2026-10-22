"""An unfit app reservation must not discard later affordable coverage."""
import datetime
import importlib.util
from pathlib import Path
import unittest
PATH=Path(__file__).resolve().parents[1]/'diesweep/desktop/matrix.py'
class BudgetSelectionTests(unittest.TestCase):
 def observe(self,apps,seconds=1000):
  spec=importlib.util.spec_from_file_location('budget_select_matrix',PATH);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
  now=datetime.datetime(2026,10,22,tzinfo=datetime.timezone.utc);skipped=[];seen=[];partial=False
  try:
   for row in m.budget_items(apps,now+datetime.timedelta(seconds=seconds),lambda x:skipped.append(x['id']),now=lambda:now):seen.append(row['id'])
  except getattr(m,'ObservationBudgetExhausted',RuntimeError):partial=True
  return seen,skipped,partial
 def test_short_app_after_unfit_long_app_is_retained(self):
  seen,skipped,partial=self.observe([dict(id='long',timeout_s=1800),dict(id='short',timeout_s=10)])
  self.assertEqual(seen,['short']);self.assertEqual(skipped,['long']);self.assertTrue(partial)
 def test_multiple_later_affordable_apps_are_not_discarded(self):
  seen,skipped,partial=self.observe([dict(id='long',timeout_s=1800),dict(id='one',timeout_s=10),dict(id='two',timeout_s=20)])
  self.assertEqual(seen,['one','two']);self.assertTrue(partial)
 def test_unfit_app_is_never_dispatched(self):
  seen,skipped,partial=self.observe([dict(id='long',timeout_s=1800)])
  self.assertEqual(seen,[]);self.assertEqual(skipped,['long']);self.assertTrue(partial)
 def test_all_affordable_apps_can_finish_without_partial(self):
  seen,skipped,partial=self.observe([dict(id='one',timeout_s=10),dict(id='two',timeout_s=20)])
  self.assertEqual(seen,['one','two']);self.assertFalse(partial);self.assertEqual(skipped,[])
 def test_reserved_hold_time_cannot_be_spent_on_app_dispatch(self):
  seen,skipped,partial=self.observe([dict(id='one',timeout_s=10)],seconds=699)
  self.assertEqual(seen,[]);self.assertTrue(partial)
 def test_empty_requested_plan_is_not_budget_refusal(self):
  self.assertEqual(self.observe([]),([],[],False))
if __name__=='__main__':unittest.main()
