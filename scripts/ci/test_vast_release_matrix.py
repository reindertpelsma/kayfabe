"""GPU-free release environment tests. Falsifier: proposed execution silently runs pinned media."""
import base64
import importlib.util
import json
from pathlib import Path
import unittest

def encoded(value):
    return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip('=')

ROOT = Path(__file__).resolve().parents[2]

def load(name):
    spec = importlib.util.spec_from_file_location('j2_' + name, ROOT / 'scripts/ci' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

class ReleaseEnvironment(unittest.TestCase):
    def setUp(self):
        self.report = load('vast_report')
        self.payload = load('vast_payload')
        matrix = json.loads((ROOT / 'ci/vast-broker/release-matrix.json').read_text())
        self.row = matrix['dies'][1]
        self.env = dict(KF_HOST_DRIVER=self.row['drivers']['host_driver'],
            KF_LINUX_GUEST_DRIVER=self.row['drivers']['linux_guest_driver'],
            KF_WINDOWS_GUEST_DRIVER=self.row['drivers']['windows_guest_driver'],
            KF_WINGATE_GUEST_DRIVER=self.row['windows_guest_abi'],
            KF_RELEASE_SUBRUNS=encoded(dict(status='proposed',subruns=self.row['subruns'],cross_subrun=self.row['cross_subrun'])))

    def test_all_driver_fields_belong_to_reporter_contract(self):
        spec = dict((n, check) for n, _, check in self.report.SPEC)
        for name, value in self.env.items():
            self.assertIn(name, spec)
            self.assertTrue(spec[name](value), name)

    def test_reporter_refuses_injection_and_malformed_subrun_json(self):
        spec = dict((n, check) for n, _, check in self.report.SPEC)
        for name in self.env:
            for bad in ['x\nexport BAD=1', '580.88\n', 'x' * 16385, '{}']:
                self.assertFalse(spec[name](bad), (name, bad[:12]))

    def test_payload_maps_host_installer_and_guest_axis_separately(self):
        mapped = self.payload.release_driver_environment(self.env)
        self.assertEqual(mapped['HOST_DRIVER'], self.row['drivers']['host_driver'])
        self.assertEqual(mapped['KF_HOST_DRIVER'], mapped['HOST_DRIVER'])
        self.assertEqual(mapped['KF_LINUX_GUEST_DRIVER'], self.row['drivers']['linux_guest_driver'])
        self.assertEqual(mapped['KF_WINGATE_GUEST_DRIVER'], '580.65.06')
        self.assertEqual(mapped['KF_WINDOWS_GUEST_DRIVER'], '580.88')

    def test_payload_refuses_driver_injection(self):
        for name in self.env:
            with self.assertRaises(self.payload.EvidenceError):
                self.payload.release_driver_environment(dict(self.env, **{name: '580.88\n'}))

    def test_proposed_plan_cannot_run_existing_pinned_media(self):
        with self.assertRaises(self.payload.EvidenceError) as caught:
            self.payload.require_executable_release(self.env)
        self.assertEqual(caught.exception.name, 'release_retry_matrix_proposed')

    def test_every_proposed_die_is_refused_before_stages(self):
        matrix = json.loads((ROOT / 'ci/vast-broker/release-matrix.json').read_text())
        for row in matrix['dies']:
            env = dict(self.env, KF_RELEASE_SUBRUNS=encoded(dict(status=row['status'],subruns=row['subruns'],cross_subrun=row['cross_subrun'])))
            with self.assertRaises(self.payload.EvidenceError):
                self.payload.require_executable_release(env)

    def test_older_worker_without_matrix_keeps_its_environment(self):
        self.assertEqual(self.payload.release_driver_environment({}), {})
        self.assertIsNone(self.payload.require_executable_release({}))

    def test_partial_selection_is_refused(self):
        for name in ['KF_HOST_DRIVER', 'KF_LINUX_GUEST_DRIVER', 'KF_WINDOWS_GUEST_DRIVER', 'KF_RELEASE_SUBRUNS']:
            env = dict(self.env)
            env.pop(name)
            with self.assertRaises(self.payload.EvidenceError):
                self.payload.release_driver_environment(env)

class MappingAcrossDies(unittest.TestCase):
    setUp = ReleaseEnvironment.setUp
    def test_each_die_maps_both_host_selectors(self):
        matrix = json.loads((ROOT / 'ci/vast-broker/release-matrix.json').read_text())
        for row in matrix['dies']:
            env = dict(self.env, KF_HOST_DRIVER=row['drivers']['host_driver'])
            mapped = self.payload.release_driver_environment(env)
            self.assertEqual(mapped['HOST_DRIVER'],row['drivers']['host_driver'])
            self.assertEqual(mapped['KF_HOST_DRIVER'],row['drivers']['host_driver'])

if __name__ == '__main__':
    unittest.main()
