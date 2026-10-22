"""The measurement runner records observations but cannot approve a guest baseline."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('native_sweep_proof', REPO / 'scripts/bench/diesweep/native_sweep_proof.py')


class NativeSweepProofTests(unittest.TestCase):
    def setUp(self):
        self.mod = importlib.util.module_from_spec(SPEC)
        SPEC.loader.exec_module(self.mod)
        self.calls = []

    def execute(self, name, argv, env, dest):
        self.calls.append((name, argv, env))
        text = ''
        if name in ('nobody_table', 'root_table'):
            text = 'PROBE_DIE arch=0x160 impl=0x6 die=TU106\nPROBE subdevice ctl=0x20801112 status=0x29\n'
        if name == 'identity':
            text = 'NVIDIA GeForce RTX 2060 SUPER, 595.91.07\n'
        dest.write_text(text)
        return 101 if name == 'defer' else 102

    def invoke(self, host='595.91.07', arch='TU106', before=''):
        with tempfile.TemporaryDirectory() as d:
            def snapshot(name, argv, env, dest):
                pid = self.execute(name, argv, env, dest)
                if name == 'before':
                    dest.write_text(before)
                return pid
            self.mod.M.measure = lambda *args: {'approved_for_guest': False, 'xid_count': 5}
            return self.mod.run(host, arch, REPO, Path(d), Path('/trusted/ladder'),
                                Path('/trusted/probe'), execute=snapshot)

    def test_successful_observation_is_not_guest_admission(self):
        self.assertEqual(self.invoke(), 3)

    def test_completed_route_measurement_still_refuses_guest(self):
        self.mod.M.measure = lambda *a: {'approved_for_guest': False}
        self.assertNotEqual(self.invoke(), 0)

    def test_driver_mismatch_stops_before_faults(self):
        with self.assertRaises(ValueError):
            self.invoke(host='580.159.04')
        self.assertFalse(any(c[0] == 'defer' for c in self.calls))

    def test_die_mismatch_stops_before_faults(self):
        with self.assertRaises(ValueError):
            self.invoke(arch='GA102')
        self.assertFalse(any(c[0] == 'defer' for c in self.calls))

    def test_preexisting_xid_stops_before_faults(self):
        with self.assertRaises(ValueError):
            self.invoke(before='NVRM: Xid (PCI:0000:01:00): 79, unexpected\n')
        self.assertFalse(any(c[0] == 'defer' for c in self.calls))

    def test_probe_only_asks_correct_size_table(self):
        self.invoke()
        queries = [c for c in self.calls if c[0] in ('root_table', 'nobody_table')]
        self.assertEqual(len(queries), 2)
        for _, _, env in queries:
            self.assertEqual(env['KF_PROBE_EXTRA_ONLY'], '1')
            self.assertEqual(env['KF_PROBE_HEAD'], '3212')

    def test_uses_both_privilege_levels(self):
        self.invoke()
        self.assertEqual(next(c for c in self.calls if c[0] == 'nobody_table')[1][:4],
                         ['runuser', '-u', 'nobody', '--'])
        self.assertEqual(next(c for c in self.calls if c[0] == 'root_table')[2]['KF_PROBE_ALLOW_ADMIN'], '1')

    def test_records_actual_owned_child_pids(self):
        seen = []
        original = self.execute
        def capture(name, argv, env, dest):
            value = original(name, argv, env, dest)
            if name == 'missing':
                import json
                seen.append(json.loads((dest.parent / 'pids.json').read_text()))
            return value
        self.execute = capture
        self.invoke()
        self.assertEqual(seen, [{'defer': 101}])


if __name__ == '__main__':
    unittest.main()
