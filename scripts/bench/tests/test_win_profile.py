#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Capture the real Windows launcher's argv without launching a GPU or VM."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'diesweep' / 'win_die.sh'


class WindowsProfileTests(unittest.TestCase):
    def start(self, version=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / 'scripts' / 'bench' / 'diesweep' / 'win_die.sh'
            script.parent.mkdir(parents=True)
            shutil.copyfile(SCRIPT, script)
            run = root / 'run'
            run.mkdir()
            (run / 'windows.qcow2').touch()
            bins = root / 'bin'
            bins.mkdir()
            stubs = {
                'git': 'echo 12345678',
                'nvidia-smi': 'echo fixture',
                'dmesg': 'exit 0',
                'setsid': '[[ $1 == bash ]] && exit 0\nexec "$@"',
                'qemu-fixture': 'printf "%s\\n" "$@" > "$CAPTURE"',
            }
            for name, body in stubs.items():
                p = bins / name
                p.write_text('#!/bin/bash\n' + body + '\n')
                p.chmod(0o700)
            capture = root / 'argv'
            env = dict(os.environ, PATH=str(bins) + ':' + os.environ['PATH'],
                       WIN_DIR=str(root / 'win'), WIN_RUN=str(run),
                       QBIN=str(bins / 'qemu-fixture'), CAPTURE=str(capture))
            env.pop('WIN_GUEST_DRIVER', None)
            if version is not None:
                env['WIN_GUEST_DRIVER'] = version
            r = subprocess.run(['bash', str(script), 'start'], env=env,
                               capture_output=True, text=True, timeout=5)
            if r.returncode == 0:
                for _ in range(100):
                    if capture.exists():
                        break
                    time.sleep(0.01)
            args = capture.read_text() if capture.exists() else ''
            return r, args

    def test_default_keeps_the_verified_580_profile(self):
        r, args = self.start()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('guest-driver=580.65.06,', args)

    def test_explicit_profile_reaches_the_device(self):
        r, args = self.start('595.84')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('guest-driver=595.84,', args)

    def test_malformed_or_injected_profiles_never_launch(self):
        for version in ('', '595', '595.84,display=off', '595.84 extra', 'v595.84'):
            with self.subTest(version=version):
                r, args = self.start(version)
                self.assertNotEqual(r.returncode, 0)
                self.assertEqual(args, '')


if __name__ == '__main__':
    unittest.main()
