"""DX1 policy tests, GPU-free. Falsifier: a policy app runs in a Vast profile,
is absent from a report, or becomes PASS/FAIL rather than an explicit non-gating row.
Raw measurements must remain available in the note when a historical result exists.
"""
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / 'scripts/bench/windows/appmatrix'))
import gate_profiles as G
import vast_run_report as R
import app_policy as P

OVERLAYS = {'overlay_yuv_' + x for x in ('probe', 'hide', 'occlude', 'resize', 'move', 'recreate', 'fullscreen', 'soak')}
UVM = {'UnifiedMemoryStreams', 'UnifiedMemoryPerf', 'conjugateGradientUM', 'attach_verify'}

class Profiles(unittest.TestCase):
    def test_all_overlays_are_manual_with_real_display_reason(self):
        p = G.load('gate_profiles.json')
        self.assertTrue(OVERLAYS <= set(p['manual']['apps']))
        items = {x['id']: x for x in p['manual']['items']}
        for name in OVERLAYS:
            self.assertEqual(items[name]['reason'], 'needs real display')
            self.assertIn('native', items[name]['evidence_template'])
            self.assertNotIn(name, G.resolve('full'))

    def test_manual_and_unsupported_are_never_gate_members(self):
        for profile in ('small', 'mid', 'full', 'release'):
            self.assertFalse((OVERLAYS | {'managed_t'}) & set(G.resolve(profile)))
        for name in sorted(OVERLAYS | {'managed_t'}):
            with self.subTest(name), self.assertRaises(SystemExit):
                G.resolve('ids:' + name)

    def test_managed_reason_records_the_owner_hold(self):
        reason = G.load('gate_profiles.json')['known_unsupported']['managed_t']
        self.assertIn('UVM', reason)
        self.assertIn('GR hold', reason)
        self.assertEqual(set(P.linux_unsupported()), UVM)

    def test_supported_failure_is_not_reclassified(self):
        self.assertIn('nbody_demo_gl', G.resolve('full'))
        self.assertNotIn('nbody_demo_gl', {x['name'] for x in P.rows()})

class Reports(unittest.TestCase):
    def test_empty_run_still_lists_every_policy_app_without_a_result(self):
        doc = R.build_report([], {})
        by = {(x['os'], x['name']): x for x in doc['apps']}
        for name in OVERLAYS:
            self.assertEqual(by['windows', name]['note'], 'manual: needs real display')
        for name in UVM:
            self.assertEqual(by['linux', name]['note'], 'unsupported (UVM)')
        self.assertEqual(by['windows', 'managed_t']['note'], 'unsupported (UVM)')
        self.assertTrue(all(x['result'] == 'NOT_RUN' for x in by.values()))
        self.assertEqual(doc['tests'], [])

    def test_unrequested_windows_still_reports_manual_entries(self):
        doc = R.build_report(['KF_PAYLOAD_TIER windows=0'], {})
        self.assertTrue(OVERLAYS <= {x['name'] for x in doc['apps'] if x['note'] == 'manual: needs real display'})
        self.assertEqual(doc['subruns'][2]['verdict'], 'NOT_RUN')

    def test_unsupported_historical_failure_is_named_with_observation(self):
        doc = R.build_report(['APP_ROW name=conjugateGradientUM os=linux result=FAIL secs=3 guest_xid=0 note=wrong answer'], {})
        row = next(x for x in doc['apps'] if x['name'] == 'conjugateGradientUM')
        self.assertEqual(row['result'], 'NOT_RUN')
        self.assertIn('unsupported (UVM)', row['note'])
        self.assertIn('observed FAIL', row['note'])
        self.assertEqual(next(x for x in doc['tests'] if x['name'] == row['name'])['result'], 'SKIP')

    def test_manual_historical_failure_is_not_a_gate_failure(self):
        line = 'WINAPPS_APP app=overlay_yuv_probe class=guest_fail verdict=FAIL batched=FAIL alone=- secs=4 tdr=0 wer=0 gsp=0 note=no DIRECT'
        doc = R.build_report([line], {})
        row = next(x for x in doc['apps'] if x['name'] == 'overlay_yuv_probe')
        self.assertEqual(row['result'], 'NOT_RUN')
        self.assertIn('manual: needs real display', row['note'])
        self.assertIn('observed FAIL', row['note'])
        self.assertEqual(doc['tests'][0]['result'], 'SKIP')

    def test_policy_rows_survive_the_app_count_cap(self):
        lines = [f'APP_ROW name=app{i} os=linux result=PASS secs=1 note=-' for i in range(600)]
        doc = R.build_report(lines, {})
        self.assertLessEqual(len(doc['apps']), 500)
        self.assertTrue(OVERLAYS <= {x['name'] for x in doc['apps']})
        self.assertTrue(UVM <= {x['name'] for x in doc['apps']})

    def test_policy_never_changes_a_supported_failure(self):
        doc = R.build_report(['APP_ROW name=vectorAdd os=linux result=FAIL secs=3 note=wrong answer'], {})
        row = next(x for x in doc['apps'] if x['name'] == 'vectorAdd')
        self.assertEqual(row['result'], 'FAIL')
        self.assertEqual(next(x for x in doc['tests'] if x['name'] == 'vectorAdd')['result'], 'FAIL')

class SpecPin(unittest.TestCase):
    def test_both_pin_values_equal_the_recorded_checksums_and_downloads(self):
        pins = dict(line.split(' sha256=') for line in (ROOT / 'scripts/bench/box/specviewperf.pin').read_text().splitlines() if ' sha256=' in line and not line.startswith('#'))
        checks = dict(line.split() for line in (ROOT / 'traces/specviewperf_vast_20261105/spec/CHECKSUMS_pins.txt').read_text().splitlines())
        downloads = (ROOT / 'docs/DOWNLOADS.md').read_text()
        for suffix, key in (('.tar.zst', 'zst_sha256'), ('.tar', 'tar_sha256')):
            self.assertEqual(pins['specviewperf-15.0.0' + suffix], checks[key])
            self.assertIn(checks[key], downloads)

class PolicyValidation(unittest.TestCase):
    def check_with(self, mutate):
        import copy
        prof = copy.deepcopy(G.load('gate_profiles.json'))
        mutate(prof)
        original = G.load
        with patch.object(G, 'load', side_effect=lambda name: prof if name == 'gate_profiles.json' else original(name)):
            return G.check()

    def test_manual_probe_without_a_template_is_rejected(self):
        def change(p):
            next(x for x in p['manual']['items'] if x['id'] == 'overlay_yuv_probe').pop('evidence_template')
        self.assertTrue(self.check_with(change))

    def test_manual_soak_without_a_reason_is_rejected(self):
        def change(p):
            next(x for x in p['manual']['items'] if x['id'] == 'overlay_yuv_soak')['reason'] = ''
        self.assertTrue(self.check_with(change))

    def test_unsupported_without_a_reason_is_rejected(self):
        self.assertTrue(self.check_with(lambda p: p['known_unsupported'].update(managed_t='')))

    def test_unsupported_with_an_uninformative_reason_is_rejected(self):
        self.assertTrue(self.check_with(lambda p: p['known_unsupported'].update(managed_t='UVM')))

    def test_unknown_unsupported_app_is_rejected(self):
        self.assertTrue(self.check_with(lambda p: p['known_unsupported'].update(not_an_app='unsupported (UVM): missing definition')))

    def test_unsupported_cannot_overlap_owner_decision(self):
        self.assertTrue(self.check_with(lambda p: p['owner_decision']['apps'].append('managed_t')))

    def test_unsupported_cannot_overlap_manual(self):
        self.assertTrue(self.check_with(lambda p: p['manual']['apps'].append('managed_t')))

    def test_explicit_manual_probe_cannot_enter_a_vast_gate(self):
        with self.assertRaises(SystemExit):
            G.resolve('ids:overlay_yuv_probe')

    def test_explicit_managed_cannot_enter_a_vast_gate(self):
        with self.assertRaises(SystemExit):
            G.resolve('ids:managed_t')

    def test_managed_never_enters_the_supported_open_set(self):
        self.assertNotIn('managed_t', G.open_apps(G.load('apps.json'), G.load('manifest.json'), G.load('gate_profiles.json')))

    def test_release_cannot_contain_a_manual_probe(self):
        p = G.load('gate_profiles.json')
        p['release_software']['apps'].append('overlay_yuv_probe')
        with self.assertRaises(SystemExit):
            G.resolve('release', prof=p)

    def test_release_cannot_contain_managed(self):
        p = G.load('gate_profiles.json')
        p['release_software']['apps'].append('managed_t')
        with self.assertRaises(SystemExit):
            G.resolve('release', prof=p)

class Summaries(unittest.TestCase):
    def test_windows_summary_lists_policy_even_without_results(self):
        import importlib.util
        import tempfile
        spec = importlib.util.spec_from_file_location('win_summary', ROOT / 'scripts/bench/windows/appmatrix/summarize.py')
        summary = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(summary)
        with tempfile.TemporaryDirectory() as tmp:
            text = summary.summarize(tmp, G.load('apps.json'))
        for name in OVERLAYS:
            self.assertIn('| ' + name + ' |', text)
        self.assertIn('manual: needs real display', text)
        self.assertIn('unsupported (UVM)', text)

    def test_linux_summary_lists_unsupported_even_without_results(self):
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            done = subprocess.run([sys.executable, '-I', str(ROOT / 'scripts/apps/summarize.py'), tmp], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        for name in UVM:
            self.assertIn('| ' + name + ' |', done.stdout)
        self.assertEqual(done.stdout.count('unsupported (UVM)'), 8)  # status column and observation detail

class LinuxGatePolicy(unittest.TestCase):
    def run_gate(self, name):
        import os
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            (work / 'list').write_text(name + '\n')
            done = subprocess.run(['bash', str(ROOT / 'scripts/bench/box/apps_gate.sh'), 'policy', str(work / 'prov')],
                                  env={'PATH': os.environ['PATH'], 'KF_APPS_LIST': str(work / 'list'),
                                       'KF_APPS_ROOT': str(work / 'unprovisioned')},
                                  capture_output=True, text=True, timeout=20)
        self.assertEqual(done.returncode, 3, done.stdout + done.stderr)
        self.assertIn('APPS_STAGE_RETRY class=BAD_LIST', done.stdout)
        self.assertNotIn('APPS_STAGE_START', done.stdout)

    def test_gate_refuses_a_streams_override_before_setup(self):
        self.run_gate('UnifiedMemoryStreams')

    def test_gate_refuses_an_attach_override_before_setup(self):
        self.run_gate('attach_verify')

class GradePolicy(unittest.TestCase):
    def output(self):
        import contextlib
        import io
        import tempfile
        spec = importlib.util.spec_from_file_location('win_grade', ROOT / 'scripts/bench/windows/appmatrix/grade.py')
        grade = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(grade)
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp, contextlib.redirect_stdout(out):
            self.assertEqual(grade.main([tmp, '--expected', 'nvidia_smi']), 3)
        return out.getvalue()

    def test_grade_artifact_lists_manual_entries_without_calling_them_passes(self):
        text = self.output()
        for name in OVERLAYS:
            self.assertIn('APP_POLICY name=' + name + ' os=windows result=NOT_RUN note=manual: needs real display', text)
        self.assertIn('WINAPPS_GRADE kind=RETRY expected=1 pass=0', text)

    def test_grade_artifact_lists_the_unsupported_counterpart(self):
        text = self.output()
        self.assertIn('APP_POLICY name=managed_t os=windows result=NOT_RUN note=unsupported (UVM)', text)
        self.assertNotIn('WINAPPS_APP app=managed_t', text)

if __name__ == '__main__':
    unittest.main()
