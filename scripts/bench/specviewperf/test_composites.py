"""Workload composite qualification is independent of individual FPS rows."""
import unittest
from result_table import scores_from_export


class CompositeTests(unittest.TestCase):
    def data(self,status='Completed',value=61.61):
        log=f'[Workload:blender-01] {status}'
        return {'root':'results','files':[{'path':'result_one/run-log.txt','text':log}],
                'scores':[{'run':'result_one','workload':'blender-01','composite':value,
                           'source':'result.json','version':'1.0.1'}]}

    def test_complete_workload_keeps_vendor_composite(self):
        row=scores_from_export(self.data(),'vfio')[0]
        self.assertEqual(row['composite'],61.61)
        self.assertEqual(row['source'],'result.json')

    def test_failed_workload_has_no_qualified_composite(self):
        row=scores_from_export(self.data('Failed'),'vfio')[0]
        self.assertIsNone(row['composite'])
        self.assertEqual(row['reported_composite'],61.61)

    def test_unfinished_workload_has_no_qualified_composite(self):
        row=scores_from_export(self.data('Starting'),'vfio')[0]
        self.assertIsNone(row['composite'])
        self.assertEqual(row['workload_status'],'Incomplete')

    def test_missing_scores_are_not_fabricated(self):
        self.assertEqual(scores_from_export({'root':'results','files':[]},'vfio'),[])

    def test_nonfinite_complete_score_is_refused(self):
        with self.assertRaises(ValueError):scores_from_export(self.data(value='NaN'),'vfio')


if __name__=='__main__':unittest.main()
