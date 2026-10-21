"""Trusted GPU-free boundary tests; fixtures never contact a GPU or cloud account."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def guard():
    path = ROOT / 'scripts/ci/vast_workflow.py'
    spec = importlib.util.spec_from_file_location('vast_workflow', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class VastWorkflowTests(unittest.TestCase):
    def test_repository_workflows_obey_the_boundary(self):
        self.assertEqual(guard().check_repository(ROOT), [])

    def test_run_permissions_are_read_only(self):
        self.assertEqual(guard().gpu_workflow(ROOT)['jobs']['run']['permissions'], {'contents': 'read'})

    def test_extra_run_permissions_fail(self):
        for permission in ('id-token', 'attestations', 'actions', 'packages'):
            with self.subTest(permission=permission):
                wf = guard().gpu_workflow(ROOT)
                wf['jobs']['run']['permissions'][permission] = 'write'
                self.assertTrue(guard().check({'gpu-vast.yml': wf}))

    def test_run_write_contents_fails(self):
        wf = guard().gpu_workflow(ROOT)
        wf['jobs']['run']['permissions'] = {'contents': 'write'}
        self.assertTrue(guard().check({'gpu-vast.yml': wf}))

    def test_direct_attestation_and_release_dependencies_fail(self):
        for name in ('attest', 'release'):
            with self.subTest(name=name):
                wf = guard().gpu_workflow(ROOT)
                wf['jobs'][name] = {'needs': ['run'], 'permissions': {'id-token': 'write'},
                                    'steps': [{'uses': 'actions/attest-build-provenance@v2'}]}
                self.assertTrue(guard().check({'gpu-vast.yml': wf}))

    def test_transitive_attestation_dependency_fails(self):
        wf = guard().gpu_workflow(ROOT)
        wf['jobs']['bridge'] = {'needs': ['rent'], 'steps': []}
        wf['jobs']['publish'] = {'needs': 'bridge', 'permissions': {'attestations': 'write'}, 'steps': []}
        self.assertTrue(guard().check({'gpu-vast.yml': wf}))

    def test_final_check_always_runs(self):
        wf = guard().gpu_workflow(ROOT)
        self.assertEqual(wf['jobs']['gpu-required']['if'], '${{ always() }}')
        self.assertIn('run', wf['jobs']['gpu-required']['needs'])

    def test_gpu_workflow_cannot_attest_or_call_an_opaque_workflow(self):
        for patch in ({'uses': './.github/workflows/release.yml'},
                      {'steps': [{'uses': 'actions/attest-build-provenance@v2'}]}):
            with self.subTest(patch=patch):
                wf = guard().gpu_workflow(ROOT)
                wf['jobs']['run'].update(patch)
                self.assertTrue(guard().check({'gpu-vast.yml': wf}))

    def test_downstream_workflow_run_artifact_consumer_fails(self):
        downstream = {'on': {'workflow_run': {'workflows': ['Vast GPU'], 'types': ['completed']}},
                      'jobs': {'release': {'steps': [{'uses': 'actions/attest@v2'}]}}}
        self.assertTrue(guard().check({'gpu-vast.yml': guard().gpu_workflow(ROOT), 'publish.yml': downstream}))


class HostileOutputTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue((ROOT / "ci/vast-broker/verdict.sh").is_file(), "verdict implementation absent")

    def verdict(self, data, code=0):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'output'
            path.write_bytes(data)
            return subprocess.run(['bash', str(ROOT / 'ci/vast-broker/verdict.sh'), str(path), str(code), 'a' * 32],
                                  capture_output=True, timeout=5)

    def test_valid_nonce_final_line_passes(self):
        self.assertEqual(self.verdict(b'KF_GPU_PASS ' + b'a' * 32 + b'\n').returncode, 0)

    def test_a_failure_exit_rejects_even_a_valid_marker(self):
        self.assertNotEqual(self.verdict(b'KF_GPU_PASS ' + b'a' * 32 + b'\n', 1).returncode, 0)

    def test_wrong_nonce_and_duplicate_markers_fail(self):
        for data in (b'KF_GPU_PASS wrong\n', (b'KF_GPU_PASS ' + b'a' * 32 + b'\n') * 2):
            with self.subTest(data=data):
                self.assertNotEqual(self.verdict(data).returncode, 0)

    def test_trailing_bytes_and_missing_newline_fail(self):
        marker = b'KF_GPU_PASS ' + b'a' * 32
        for data in (marker, marker + b'\nextra\n', marker + b'\r\n', marker + b'\x00\n'):
            with self.subTest(data=data):
                self.assertNotEqual(self.verdict(data).returncode, 0)

    def test_workflow_commands_are_printed_under_an_unpredictable_stop_token(self):
        result = self.verdict(b'::error::hostile\nKF_GPU_PASS ' + b'a' * 32 + b'\n')
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.splitlines()
        token = lines[0].removeprefix(b'::stop-commands::')
        self.assertEqual(lines[-1], b'::' + token + b'::')
        self.assertEqual(lines[1], b'::error::hostile')

    def test_output_at_cap_cannot_pass(self):
        data = b'x' * (1048576 - 45) + b'KF_GPU_PASS ' + b'a' * 32 + b'\n'
        self.assertGreaterEqual(len(data), 1048576)
        self.assertNotEqual(self.verdict(data).returncode, 0)
