#!/usr/bin/env python3
"""Offline host-specific identity window checks; --helper selects the operator helper."""
import argparse
import base64
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

HELPER = None
PROFILE = 'wc1-round3e-host79466'


@unittest.skipUnless(HELPER, 'Run directly with --helper PATH')
class HostWindowTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('admission_window_under_test', HELPER)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.owned = dict(id=7, host=79466, template='183b10af7fd788d08aca02e31811d70d')
        self.budget = dict(rate_bound=0.2, host=79466, identity_profile=PROFILE)
        self.row = dict(id=7, host_id=79466, dph_total=0.1, public_ipaddr='1.1.1.1',
                        ports={'22/tcp': [{'HostPort': '2222'}]})
        self.now = 2020.0
        wire = struct.pack('>I', 11) + b'ssh-ed25519' + struct.pack('>I', 32) + bytes(32)
        self.key = base64.b64encode(wire).decode()
        self.fingerprint = self.module.fp(self.key)
        self.state = dict(first_mono=100.0, first_utc='synthetic', host='1.1.1.1', port=2222,
                          identity_window_seconds=3000, stable_since=1900.0,
                          previous=self.fingerprint, consecutive=1)

    def policy(self):
        return self.module.identity_window(self.owned, self.budget, self.row)

    def fake_run(self, args, **kwargs):
        if args[:3] == ['vastai', 'show', 'instances-v1']:
            return subprocess.CompletedProcess(args, 0, json.dumps([self.row]), '')
        if args[0] == 'ssh-keyscan':
            return subprocess.CompletedProcess(args, 0,
                '[1.1.1.1]:2222 ssh-ed25519 ' + self.key + '\n', '')
        if args[:2] == ['vastai', 'logs']:
            console = self.module.MARKER + '\nssh-ed25519 ' + self.key + '\n'
            return subprocess.CompletedProcess(args, 0, console, '')
        raise AssertionError('Unexpected request: ' + repr(args))

    def invoke(self):
        (self.root / 'owned-id').write_text('7\n')
        for name, value in [('owned', self.owned), ('budget', self.budget),
                            ('pin-state', self.state)]:
            (self.root / (name + '.json')).write_text(json.dumps(value))
        with patch.object(self.module.subprocess, 'run', side_effect=self.fake_run):
            with patch.object(self.module.time, 'monotonic', side_effect=lambda: self.now):
                self.module.main(str(self.root))

    def advance(self, now, latest=None, scanned=None, windows=None):
        key = self.fingerprint
        return self.module.advance(self.state, now, latest or key, scanned or key, windows or key)

    def test_explicit_profile_selects3000(self):
        self.assertEqual(self.policy(), 3000)

    def test_other_host_defaults1200(self):
        self.budget.pop('identity_profile')
        self.owned['host'] = self.budget['host'] = self.row['host_id'] = 1
        self.assertEqual(self.policy(), 1200)

    def test_missing_profile_on_target_stays1200(self):
        self.budget.pop('identity_profile')
        self.assertEqual(self.policy(), 1200)

    def test_foreign_provider_host_refused(self):
        self.row['host_id'] = 1
        with self.assertRaisesRegex(ValueError, 'REFUSED'):
            self.policy()

    def test_foreign_owned_host_refused(self):
        self.owned['host'] = 1
        with self.assertRaisesRegex(ValueError, 'REFUSED'):
            self.policy()

    def test_foreign_budget_host_refused(self):
        self.budget['host'] = 1
        with self.assertRaisesRegex(ValueError, 'REFUSED'):
            self.policy()

    def test_unknown_profile_refused(self):
        self.budget['identity_profile'] = 'unapproved'
        with self.assertRaisesRegex(ValueError, 'REFUSED'):
            self.policy()

    def test_refuse_exact3000_boundary(self):
        with self.assertRaisesRegex(ValueError, 'REFUSED native identity deadline'):
            self.advance(3100.0)

    def test_refuse_after3000_boundary(self):
        with self.assertRaisesRegex(ValueError, 'REFUSED native identity deadline'):
            self.advance(3101.0)

    def test119seconds_still_insufficient_after_old_deadline(self):
        admitted, stable = self.advance(2019.0)
        self.assertFalse(admitted)
        self.assertEqual(stable, 119)

    def test120seconds_admits_after_old_deadline(self):
        admitted, stable = self.advance(2020.0)
        self.assertTrue(admitted)
        self.assertEqual(stable, 120)

    def test_mismatched_scan_resets_progress(self):
        admitted, stable = self.advance(2020.0, scanned='different')
        self.assertFalse(admitted)
        self.assertEqual(stable, 0)

    def test_mismatched_windows_key_resets_progress(self):
        admitted, stable = self.advance(2020.0, windows='different')
        self.assertFalse(admitted)
        self.assertEqual(stable, 0)

    def test_main_admits_extended_profile_and_retains_first_time(self):
        self.invoke()
        self.assertTrue((self.root / 'admitted.json').exists())
        state = json.loads((self.root / 'pin-state.json').read_text())
        self.assertEqual(state['first_mono'], 100.0)
        self.assertEqual(state['identity_window_seconds'], 3000)

    def test_main119seconds_does_not_admit(self):
        self.now = 2019.0
        self.invoke()
        self.assertFalse((self.root / 'admitted.json').exists())

    def test_main_refuses_exact3000(self):
        self.now = 3100.0
        with self.assertRaisesRegex(ValueError, 'REFUSED native identity deadline'):
            self.invoke()

    def test_main_refuses_after3000(self):
        self.now = 3101.0
        with self.assertRaisesRegex(ValueError, 'REFUSED native identity deadline'):
            self.invoke()

    def test_main_refuses_existing1200_window_change(self):
        self.now = 500.0
        self.state['identity_window_seconds'] = 1200
        with self.assertRaisesRegex(ValueError, 'REFUSED identity window changed'):
            self.invoke()

    def test_main_refuses_existing_arbitrary_window_change(self):
        self.now = 500.0
        self.state['identity_window_seconds'] = 5000
        with self.assertRaisesRegex(ValueError, 'REFUSED identity window changed'):
            self.invoke()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', required=True, type=Path)
    args, remaining = parser.parse_known_args()
    HELPER = args.helper.resolve()
    HostWindowTests.__unittest_skip__ = False
    unittest.main(argv=[__file__, *remaining])
