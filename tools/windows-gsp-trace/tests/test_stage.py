# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import argparse
import base64
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('stage_qga', Path(__file__).parents[1] / 'stage-qga.py')
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class StageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        bundle = root / 'bundle.zip'
        bundle.write_bytes(b'known trusted payload' * 10000)
        self.args = argparse.Namespace(bundle=bundle, sha256=stage.sha256(bundle), work=root, prepare_helper=root / 'helper.py')
        self.calls, self.blocks = [], []

    def helper(self, short=False, denied=False):
        def rpc(sock, name, args, timeout):
            del sock
            self.assertEqual(timeout, 60)
            self.calls.append(name)
            if name == 'guest-file-open':
                return 17
            if name == 'guest-file-write':
                block = base64.b64decode(args['buf-b64'])
                self.blocks.append(block)
                return {'count': len(block) - int(short)}
            return {}

        def guest_exec(sock, args):
            del sock
            self.calls.append('guest-exec')
            if denied:
                return 1, '', 'native GPU is not deferred'
            if 'stage-result.json' in args[1]:
                return 0, json.dumps({'source_revision': 'test', 'driver_loaded': False}), ''
            return 0, '', ''
        return types.SimpleNamespace(rpc=rpc, guest_exec=guest_exec)

    def run_mocked(self, helper):
        fake_spec = types.SimpleNamespace(loader=types.SimpleNamespace(exec_module=lambda module: None))
        with patch.object(stage.importlib.util, 'spec_from_file_location', return_value=fake_spec), \
             patch.object(stage.importlib.util, 'module_from_spec', return_value=helper), contextlib.redirect_stdout(io.StringIO()):
            stage.stage(self.args)

    def test_checksum_and_hold_guards_precede_guest_changes(self):
        self.args.sha256 = '0' * 64
        with self.assertRaisesRegex(RuntimeError, 'checksum'):
            self.run_mocked(self.helper())
        self.args.sha256 = stage.sha256(self.args.bundle)
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_mocked(self.helper())
        (self.args.work / 'login-test.json').write_text('{}')
        (self.args.work / 'login-test-complete').touch()
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_mocked(self.helper())
        self.assertEqual(self.calls, [])

    def test_native_defer_refusal_precedes_upload(self):
        (self.args.work / 'login-test.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'not deferred'):
            self.run_mocked(self.helper(denied=True))
        self.assertEqual(self.calls, ['guest-exec'])

    def test_chunks_exact_and_hold_remains(self):
        (self.args.work / 'login-test.json').write_text('{}')
        self.run_mocked(self.helper())
        self.assertEqual(b''.join(self.blocks), self.args.bundle.read_bytes())
        self.assertLessEqual(max(map(len, self.blocks)), 65536)
        self.assertFalse((self.args.work / 'login-test-complete').exists())
        self.assertTrue((self.args.work / 'gsp-stage-result.json').exists())
        self.assertIn('guest-file-flush', self.calls)
        self.assertEqual(self.calls[-2:], ['guest-file-close', 'guest-exec'])

    def test_short_write_closes_handle_and_stops(self):
        (self.args.work / 'login-test.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'Short'):
            self.run_mocked(self.helper(short=True))
        self.assertEqual(self.calls[-1], 'guest-file-close')
        self.assertEqual(self.calls.count('guest-exec'), 1)
        self.assertFalse((self.args.work / 'gsp-stage-result.json').exists())

    def test_lost_write_acknowledgement_is_not_retried(self):
        (self.args.work / 'login-test.json').write_text('{}')
        helper = self.helper()
        original = helper.rpc

        def timeout_write(sock, name, args, timeout):
            result = original(sock, name, args, timeout)
            if name == 'guest-file-write':
                raise TimeoutError('lost acknowledgement')
            return result

        helper.rpc = timeout_write
        with self.assertRaisesRegex(RuntimeError, 'guest-file-write.*not retried'):
            self.run_mocked(helper)
        self.assertEqual(self.calls.count('guest-file-write'), 1)
        self.assertEqual(self.calls[-1], 'guest-file-close')
        self.assertFalse((self.args.work / 'gsp-stage-result.json').exists())


if __name__ == '__main__':
    unittest.main()
