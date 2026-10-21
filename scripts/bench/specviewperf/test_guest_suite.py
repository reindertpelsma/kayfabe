"""The harness must use SPEC's public CLI in the interactive user session."""
import unittest
from guest_suite import cli_arguments, render_task

WORKLOADS = ['blender-01','catia-07','creo-04','energy-04','enscape-01',
             'maya-07','medical-04','solidworks-08','unreal_engine-01']


class SuitePlanTests(unittest.TestCase):
    def test_two_complete_iterations(self):
        args = cli_arguments(WORKLOADS, 2, r'C:\Users\Public\codex-VF1\results')
        self.assertEqual(args[args.index('--iterations')+1], '2')
        self.assertEqual(args[args.index('--workload')+1].split(','), WORKLOADS)

    def test_resolution_is_explicit_fullhd(self):
        args = cli_arguments(WORKLOADS, 2, r'C:\Users\Public\codex-VF1\results')
        self.assertEqual(args[args.index('--resolution')+1], 'FullHD')

    def test_results_stay_in_public_profile(self):
        args = cli_arguments(WORKLOADS, 2, r'C:\Users\Public\codex-VF1\results')
        self.assertEqual(args[args.index('--resultDirectory')+1], r'C:\Users\Public\codex-VF1\results')

    def test_refuse_duplicate_workload(self):
        with self.assertRaises(ValueError):cli_arguments(['blender-01','blender-01'],2,r'C:\Users\Public\x')

    def test_refuse_injected_workload(self):
        with self.assertRaises(ValueError):cli_arguments(['blender-01;exit'],2,r'C:\Users\Public\x')

    def test_refuse_empty_suite(self):
        with self.assertRaises(ValueError):cli_arguments([],2,r'C:\Users\Public\x')

    def test_task_uses_interactive_logon(self):
        text = render_task(WORKLOADS,2,r'C:\Users\Public\codex-VF1')
        self.assertIn('-LogonType Interactive',text)
        self.assertIn("-UserId 'kfadmin'",text)

    def test_no_system_account_benchmark(self):
        text=render_task(WORKLOADS,2,r'C:\Users\Public\codex-VF1')
        self.assertIn('-RunLevel Limited',text)
        self.assertNotIn('-UserId SYSTEM',text)

    def test_foreground_and_result_markers(self):
        text=render_task(WORKLOADS,2,r'C:\Users\Public\codex-VF1')
        for marker in ['SUITE_STARTED','SUITE_EXIT','SPECviewperf-CLI.exe','cli-stdout.log']:
            self.assertIn(marker,text)

    def test_no_timeout_tuning(self):
        text=render_task(WORKLOADS,2,r'C:\Users\Public\codex-VF1')
        self.assertNotIn('--set-timeout',text)
        self.assertNotIn('--workload-gap',text)


if __name__=='__main__':unittest.main()
