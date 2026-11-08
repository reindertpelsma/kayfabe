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
        self.assertIn('APP_POLICY name=' + name + ' os=linux result=NOT_RUN note=unsupported (UVM)', done.stdout)
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


class DirectGradePolicy(unittest.TestCase):
    """The public function and real CLI must enforce policy without a profile resolver."""

    def run_grade(self, expected, batched, alone=(), cli=False):
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            for file, entries in (('win.res', batched), ('win_isolated.res', alone)):
                (Path(tmp) / file).write_text(''.join(
                    f'APPRES app={name} verdict={verdict} secs=1 note=synthetic\n'
                    for name, verdict in entries))
            path = ROOT / 'scripts/bench/windows/appmatrix/grade.py'
            if cli:
                return subprocess.run([sys.executable, str(path), tmp, '--expected', ','.join(expected)],
                                      capture_output=True, text=True, timeout=10)
            spec = importlib.util.spec_from_file_location('direct_win_grade', path)
            grader = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(grader)
            return grader.grade(tmp, expected)

    def check_function(self, name):
        rc, kind, counts, per, extra = self.run_grade([name], [(name, 'PASS')], [(name, 'PASS*')])
        self.assertEqual((rc, kind, counts['pass'], counts['harness']), (3, 'RETRY', 0, 1))
        self.assertEqual((per[name]['cls'], per[name]['verdict']), ('harness', 'NOTRUN'))
        self.assertEqual((per[name]['batched'], per[name]['alone']), ('PASS', 'PASS*'))
        self.assertIn('non-gating', per[name]['note'])
        self.assertEqual(extra, [])

    def check_cli(self, name):
        done = self.run_grade([name], [(name, 'PASS')], [(name, 'PASS*')], cli=True)
        self.assertEqual(done.returncode, 3, done.stdout + done.stderr)
        self.assertIn('WINAPPS_GRADE kind=RETRY expected=1 pass=0', done.stdout)
        self.assertIn(f'WINAPPS_APP app={name} class=harness verdict=NOTRUN batched=PASS alone=PASS*', done.stdout)
        self.assertIn('WINAPPS_GRADE_HARNESS ' + name, done.stdout)
        self.assertNotIn(f'WINAPPS_APP app={name} class=pass', done.stdout)

    def test_function_manual_probe_cannot_pass(self):
        self.check_function('overlay_yuv_probe')

    def test_function_manual_soak_cannot_pass(self):
        self.check_function('overlay_yuv_soak')

    def test_function_unsupported_uvm_cannot_pass(self):
        self.check_function('managed_t')

    def test_cli_manual_probe_cannot_pass(self):
        self.check_cli('overlay_yuv_probe')

    def test_cli_manual_soak_cannot_pass(self):
        self.check_cli('overlay_yuv_soak')

    def test_cli_unsupported_uvm_cannot_pass(self):
        self.check_cli('managed_t')

    def test_other_manual_heaven_cannot_pass(self):
        self.check_function('heaven_d3d11')

    def test_other_manual_valley_cannot_pass(self):
        self.check_function('valley_opengl')

    def test_mixed_cli_manual_keeps_only_supported_pass_credit(self):
        done = self.run_grade(['nvidia_smi', 'overlay_yuv_probe'],
                              [('nvidia_smi', 'PASS'), ('overlay_yuv_probe', 'PASS')], cli=True)
        self.assertEqual(done.returncode, 3, done.stdout + done.stderr)
        self.assertIn('WINAPPS_GRADE kind=RETRY expected=2 pass=1 guest_fail=0 boot_fail=0 harness=1', done.stdout)

    def test_mixed_cli_uvm_keeps_only_supported_pass_credit(self):
        done = self.run_grade(['nvidia_smi', 'managed_t'],
                              [('nvidia_smi', 'PASS'), ('managed_t', 'PASS')], cli=True)
        self.assertEqual(done.returncode, 3, done.stdout + done.stderr)
        self.assertIn('WINAPPS_GRADE kind=RETRY expected=2 pass=1 guest_fail=0 boot_fail=0 harness=1', done.stdout)

    def check_supported_failure(self, name):
        done = self.run_grade(['nvidia_smi', name], [('nvidia_smi', 'FAIL'), (name, 'PASS')], cli=True)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn('WINAPPS_GRADE kind=FAIL expected=2 pass=0 guest_fail=1 boot_fail=0 harness=1', done.stdout)

    def test_supported_failure_with_manual_stays_fail(self):
        self.check_supported_failure('overlay_yuv_probe')

    def test_supported_failure_with_uvm_stays_fail(self):
        self.check_supported_failure('managed_t')

    def test_policy_failures_are_not_guest_failures(self):
        for name in ('overlay_yuv_soak', 'managed_t'):
            with self.subTest(name=name):
                rc, kind, counts, *_ = self.run_grade([name], [(name, 'FAIL')])
                self.assertEqual((rc, kind, counts['pass'], counts['guest_fail']), (3, 'RETRY', 0, 0))

    def test_supported_cli_pass_control(self):
        done = self.run_grade(['nvidia_smi'], [('nvidia_smi', 'PASS')], cli=True)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn('WINAPPS_GRADE kind=PASS expected=1 pass=1', done.stdout)

    def test_policy_observations_outside_expected_stay_extra(self):
        rc, kind, counts, _, extra = self.run_grade(['nvidia_smi'],
            [('nvidia_smi', 'PASS'), ('overlay_yuv_probe', 'PASS'), ('managed_t', 'FAIL')])
        self.assertEqual((rc, kind, counts['pass']), (0, 'PASS', 1))
        self.assertEqual(extra, ['managed_t', 'overlay_yuv_probe'])


class NativeOverlayEvidence(unittest.TestCase):
    """Cited setup refusal must not become a completed overlay correctness result."""

    def generator(self):
        spec = importlib.util.spec_from_file_location('native_overlay_gen', ROOT / 'scripts/bench/windows/appmatrix/gen_apps.py')
        generator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(generator)
        return generator.build()

    def assert_setup(self, text):
        self.assertIn('SETUP/refused (code 5)', text)
        self.assertIn('no scenario frames', text)
        self.assertNotIn('probe is FAIL', text)
        self.assertNotIn('probe FAIL', text)

    def ledger_row(self):
        import csv
        with (ROOT / 'docs/RELEASE_CLAIMS.tsv').open() as handle:
            rows = list(csv.DictReader(handle, delimiter='\t'))
        return next(row for row in rows if row['claim_id'].startswith('native-overlay-probe-'))

    def test_catalog_preserves_native_setup_class(self):
        catalog = {a['id']: a for a in G.load('apps.json')['apps']}
        for name in OVERLAYS:
            with self.subTest(name=name):
                self.assert_setup(catalog[name]['note'])

    def test_generator_preserves_native_setup_class(self):
        for app in self.generator()['apps']:
            if app['id'] in OVERLAYS:
                with self.subTest(name=app['id']):
                    self.assert_setup(app['note'])

    def test_generator_and_committed_catalog_agree(self):
        self.assertEqual(self.generator(), G.load('apps.json'))

    def test_manual_templates_preserve_native_setup_class(self):
        for item in G.load('gate_profiles.json')['manual']['items']:
            if item['id'] in OVERLAYS:
                with self.subTest(name=item['id']):
                    self.assert_setup(item['evidence_template'])

    def test_manual_profiles_and_catalog_agree_on_refusal(self):
        catalog = {a['id']: a for a in G.load('apps.json')['apps']}
        for item in G.load('gate_profiles.json')['manual']['items']:
            if item['id'] in OVERLAYS:
                self.assert_setup(item['evidence_template'])
                self.assert_setup(catalog[item['id']]['note'])

    def test_ledger_class_matches_raw_native_evidence(self):
        raw = (ROOT / 'traces/windows_overlay_native_20261010/raw/t3.txt').read_text()
        self.assertIn('KFOVL RESULT SETUP', raw)
        self.assertIn('(code=5)', raw)
        self.assertIn('flags=0x0 direct=0', raw)
        record = (ROOT / 'traces/windows_overlay_native_20261010/README.md').read_text()
        self.assertIn('scenario never started (no frames)', record)
        row = self.ledger_row()
        self.assert_setup(row['claim'])
        self.assertIn('not a completed correctness failure', row['note'])
        self.assertIn('raw/t3.txt', row['evidence'])

    def test_readme_and_ledger_agree_on_native_setup(self):
        readme = (ROOT / 'README.md').read_text()
        self.assert_setup(readme)
        self.assert_setup(self.ledger_row()['claim'])
        self.assertNotIn('probe failed on a Vast', readme)

    def test_readme_keeps_native_attempt_distinct_from_manual_qualification(self):
        record = (ROOT / 'traces/windows_overlay_native_20261010/README.md').read_text()
        self.assertIn('scenario never started (no frames)', record)
        readme = (ROOT / 'README.md').read_text()
        line = next(line for line in readme.splitlines() if line.startswith('**Manual: needs real display.**'))
        self.assert_setup(line)
        self.assertIn('no real-monitor native record is committed', line)
        self.assertIn('#mc-14-windows-yuv-overlays-on-a-real-display', line)

    def test_generated_matrix_preserves_catalog_native_class(self):
        doc = (ROOT / 'docs/design/V3_WINDOWS_APP_MATRIX.md').read_text()
        catalog = {a['id']: a for a in G.load('apps.json')['apps']}
        for name in OVERLAYS:
            line = next(line for line in doc.splitlines() if line.startswith('| `' + name + '` |'))
            with self.subTest(name=name):
                self.assert_setup(line)
                self.assertIn(catalog[name]['note'], line)

    def test_generated_probe_doc_matches_native_record(self):
        raw = (ROOT / 'traces/windows_overlay_native_20261010/raw/t3.txt').read_text()
        self.assertIn('KFOVL RESULT SETUP', raw)
        doc = (ROOT / 'docs/design/V3_WINDOWS_APP_MATRIX.md').read_text()
        line = next(line for line in doc.splitlines() if line.startswith('| `overlay_yuv_probe` |'))
        self.assert_setup(line)

if __name__ == '__main__':
    unittest.main()
