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


class StampedCliTests(unittest.TestCase):
    """The CLI never writes an unstamped table (owner decision 2026-10-23)."""
    RELEASE = 'SPECviewperf 15.0.1'
    CONFIG = 'host RTX 4070 Ti; VM guest Windows 11; kayfabe virtual GPU'

    def run_cli(self, *extra):
        import contextlib
        import io
        import json
        import pathlib
        import tempfile
        import result_table
        import spec_stamp
        data = {'root': 'result_one',
                'files': [{'path': 'result_one/run-log.txt',
                           'text': '[Workload:blender-01] Completed'},
                          {'path': 'result_one/blender-01/stdout.log', 'text': XML}],
                'scores': [{'run': 'result_one', 'workload': 'blender-01', 'composite': 61.61}]}
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            export = tmp / 'export.json'
            export.write_text(json.dumps(data))
            argv = [str(export), '--arm', 'vfio', '--output', str(tmp / 'fps.csv'),
                    '--scores-output', str(tmp / 'scores.csv'), *extra]
            with contextlib.redirect_stderr(io.StringIO()):
                result_table.main(argv)
            return ((tmp / 'fps.csv').read_text(), (tmp / 'scores.csv').read_text(), spec_stamp)

    def test_release_and_configuration_are_mandatory(self):
        for extra in ([], ['--release', self.RELEASE], ['--config', self.CONFIG]):
            with self.assertRaises(SystemExit):
                self.run_cli(*extra)

    def test_guessed_release_is_refused(self):
        with self.assertRaises(ValueError):
            self.run_cli('--release', '15', '--config', self.CONFIG)

    def test_outputs_are_stamped_and_pass_the_checker(self):
        fps, scores, spec_stamp = self.run_cli('--release', self.RELEASE, '--config', self.CONFIG)
        self.assertIn('27.36 (nc)', fps)
        self.assertIn('61.61 (nc)', scores)
        self.assertEqual(spec_stamp.check_text(fps, 'csv'), [])
        self.assertEqual(spec_stamp.check_text(scores, 'csv'), [])
        self.assertIn(spec_stamp.DISCLAIMER, fps)


if __name__=='__main__':unittest.main()
