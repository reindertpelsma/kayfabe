"""A failed or partial workload must never become a composite success."""
import unittest
from result_table import parse_stdout, parse_statuses, rows_from_export

XML = '<Results Name="blender-01"><Test Index="2" Frames="594" Seconds="21.71" FPS_2="27.36" /></Results>'


class ResultTests(unittest.TestCase):
    def test_parse_embedded_xml_amid_console_noise(self):
        rows=parse_stdout('console\n'+XML+'\ncleanup warning')
        self.assertEqual(rows[0]['fps'],27.36)
        self.assertEqual(rows[0]['subtest'],2)

    def test_keep_all_subtests_not_just_last_xml_file(self):
        rows=parse_stdout(XML+XML.replace('Index="2"','Index="3"').replace('FPS_2','FPS_3'))
        self.assertEqual([r['subtest'] for r in rows],[2,3])

    def test_no_result_is_not_zero_fps(self):
        self.assertEqual(parse_stdout('Failed to load'),[])

    def test_nonfinite_fps_is_refused(self):
        with self.assertRaises(ValueError):parse_stdout(XML.replace('27.36','NaN'))

    def test_negative_fps_is_refused(self):
        with self.assertRaises(ValueError):parse_stdout(XML.replace('27.36','-1'))

    def test_statuses_are_run_level_only(self):
        text='[Workload: blender-01] Failed -- check logs\n[Workload:energy-04] Completed\n[workload: blender-01 / subtest: 02_Foo] Completed'
        self.assertEqual(parse_statuses(text),{'blender-01':'Failed','energy-04':'Completed'})

    def test_failed_workload_retains_numeric_row_and_failure(self):
        data={'root':'result_native','files':[{'path':'result_one/run-log.txt','text':'[Workload:blender-01] Failed -- errors'}, {'path':'result_one/blender-01/stdout.log','text':XML}]}
        row=rows_from_export(data,'vfio')[0]
        self.assertEqual(row['workload_status'],'Failed')
        self.assertEqual(row['run'],'result_one')
        self.assertEqual(row['fps'],27.36)

    def test_unfinished_workload_is_explicitly_incomplete(self):
        data={'root':'result_native','files':[{'path':'blender-01/stdout.log','text':XML}]}
        row=rows_from_export(data,'vfio')[0]
        self.assertEqual(row['workload_status'],'Incomplete')
        self.assertEqual(row['run'],'result_native')

    def test_windows_paths_normalize(self):
        data={'root':'C:\\Results\\result_x','files':[{'path':'blender-01\\stdout.log','text':XML}]}
        self.assertEqual(rows_from_export(data,'kayfabe')[0]['run'],'result_x')


if __name__=='__main__':unittest.main()
