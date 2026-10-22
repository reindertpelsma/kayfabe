# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The sign-in verifier must identify the configured console account."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('s4_signin', Path(__file__).resolve().parents[1] / 'windows/signin.py')
signin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signin)


class SigninUserTests(unittest.TestCase):
    def verify(self, configured, observed):
        env = dict(KF_GUEST_PW='fixtureOnly17', KF_GUEST_USER=configured)
        with patch.dict(os.environ, env), patch.object(signin.sys, 'argv', ['signin.py', 'qmp', 'qga']), \
             patch.object(signin.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, observed, '')), \
             patch.object(signin.socket, 'socket', side_effect=RuntimeError('would type credentials')), \
             contextlib.redirect_stdout(io.StringIO()) as out:
            signin.main()
            return out.getvalue()

    def test_kf_session_is_verified_without_typing(self):
        self.assertIn('already signed in', self.verify('kf', 'FIXTURE\\kf\n'))

    def test_configured_account_match_is_case_insensitive(self):
        self.assertIn('already signed in', self.verify('KF', 'FIXTURE\\kf\n'))

    def test_different_account_is_never_accepted(self):
        with self.assertRaisesRegex(RuntimeError, 'would type'):
            self.verify('kf', 'FIXTURE\\vast\n')

    def test_existing_vast_caller_still_works(self):
        self.assertIn('already signed in', self.verify('vast', 'FIXTURE\\vast\n'))
