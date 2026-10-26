#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline operator checks: --helper selects an admission helper; no network requests."""
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


@unittest.skipUnless(HELPER, 'Run directly with --helper PATH')
class IdentityCheckpointTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('admission_under_test', HELPER)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'owned-id').write_text('7\n')
        (self.root / 'owned.json').write_text(json.dumps(dict(
            id=7, template='183b10af7fd788d08aca02e31811d70d')))
        (self.root / 'budget.json').write_text(json.dumps(dict(rate_bound=0.2)))
        self.now = 100.0
        wire = struct.pack('>I', 11) + b'ssh-ed25519' + struct.pack('>I', 32) + bytes(32)
        self.public_key = base64.b64encode(wire).decode()

    def fake_run(self, args, **kwargs):
        if args[:3] == ['vastai', 'show', 'instances-v1']:
            row = dict(id=7, dph_total=0.1, public_ipaddr='1.1.1.1',
                       ports={'22/tcp': [{'HostPort': '2222'}]})
            return subprocess.CompletedProcess(args, 0, json.dumps([row]), '')
        if args[0] == 'ssh-keyscan':
            line = '[1.1.1.1]:2222 ssh-ed25519 ' + self.public_key + '\n'
            return subprocess.CompletedProcess(args, 0, line, '')
        if args[:2] == ['vastai', 'logs']:
            self.assertTrue((self.root / 'pin-state.json').exists(),
                            'Checkpoint must precede fallible console I/O')
            raise subprocess.CalledProcessError(1, args, '', 'synthetic console API failure')
        raise AssertionError('Unexpected request: ' + repr(args))

    def invoke(self):
        with patch.object(self.module.subprocess, 'run', side_effect=self.fake_run):
            with patch.object(self.module.time, 'monotonic', side_effect=lambda: self.now):
                self.module.main(str(self.root))

    def test_console_failure_keeps_first_reachable_checkpoint(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.invoke()
        checkpoint = self.root / 'pin-state.json'
        self.assertTrue(checkpoint.exists(), 'First reach must survive failed console I/O')
        state = json.loads(checkpoint.read_text())
        self.assertEqual(state['first_mono'], 100.0)
        self.assertEqual((state['host'], state['port']), ('1.1.1.1', 2222))
        self.assertEqual((self.root / 'console-read-error.txt').read_text(),
                         'synthetic console API failure')

    def test_console_failure_cannot_restart_expired_admission_clock(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.invoke()
        self.assertEqual((self.root / 'console-read-error.txt').read_text(),
                         'synthetic console API failure')
        self.now = 1400.0
        with self.assertRaisesRegex(ValueError, 'REFUSED native identity deadline'):
            self.invoke()

    def seed_verified_progress(self):
        state = dict(first_mono=100.0, first_utc='synthetic', host='1.1.1.1', port=2222,
                     stable_since=100.0, previous=self.module.fp(self.public_key), consecutive=1)
        (self.root / 'pin-state.json').write_text(json.dumps(state))
        self.now = 210.0

    def test_console_failure_invalidates_previous_match_progress(self):
        self.seed_verified_progress()
        with self.assertRaises(subprocess.CalledProcessError):
            self.invoke()
        state = json.loads((self.root / 'pin-state.json').read_text())
        self.assertEqual(state['first_mono'], 100.0)
        self.assertIsNone(state['stable_since'])
        self.assertIsNone(state['previous'])
        self.assertEqual(state['consecutive'], 0)

    def test_console_failure_requires_a_new_full_stability_period(self):
        self.seed_verified_progress()
        with self.assertRaises(subprocess.CalledProcessError):
            self.invoke()
        state = json.loads((self.root / 'pin-state.json').read_text())
        key = self.module.fp(self.public_key)
        admitted, stable = self.module.advance(state, 220.0, key, key, key)
        self.assertFalse(admitted)
        self.assertEqual(stable, 0)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', required=True, type=Path)
    args, remaining = parser.parse_known_args()
    HELPER = args.helper.resolve()
    IdentityCheckpointTests.__unittest_skip__ = False
    unittest.main(argv=[__file__, *remaining])
