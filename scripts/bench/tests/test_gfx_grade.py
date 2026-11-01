#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Native graphics output must not fail when grep -q closes a long producer pipe."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'gfx' / 'gfx_steps.sh'


class GfxGradeTests(unittest.TestCase):
    def earlier_grade(self, stage, fails):
        text = SCRIPT.read_text()
        end = text.index(f'echo "GFX_S{stage}=PASS"')
        start = text.rfind('\nif ', 0, end) + 1
        condition = text[start:text.index('\n\n', end)]
        fixture = ('deviceName NVIDIA fixture\nVKC_BAD={0}\n'
                   'VKR_REF_FAILS={0}\nEGLR_REF_FAILS={0}\n'
                   'EGL_RENDERER=NVIDIA fixture\n').format(fails) + 'detail\n' * 30000
        with tempfile.TemporaryDirectory() as directory:
            info = Path(directory) / 'info'
            info.write_text(fixture)
            cmd = 'set -o pipefail\nVI=$(cat "$1"); VC=$VI; VR=$VI; ER=$VI; VIRC=0\n' + condition
            return subprocess.run(['bash', '-c', cmd, 'bash', str(info)],
                                  capture_output=True, text=True, timeout=5)

    def test_earlier_large_correct_outputs_pass(self):
        for stage in (2, 3, 4):
            with self.subTest(stage=stage):
                r = self.earlier_grade(stage, 0)
                self.assertIn(f'GFX_S{stage}=PASS', r.stdout, r.stderr)

    def test_earlier_failed_references_are_rejected(self):
        for stage in (2, 3, 4):
            with self.subTest(stage=stage):
                r = self.earlier_grade(stage, 1)
                self.assertIn(f'GFX_S{stage}=FAIL', r.stdout, r.stderr)

    def grade(self, fails):
        text = SCRIPT.read_text()
        start = text.index('    if ', text.index('echo "GFX_GLXR_RC='))
        condition = text[start:text.index('\nfi', start)]
        with tempfile.TemporaryDirectory() as directory:
            info = Path(directory) / 'info'
            info.write_text('OpenGL renderer string: NVIDIA fixture\n' + 'extension detail\n' * 30000)
            cmd = 'set -o pipefail\nGI=$(cat "$1")\nGX="GLX_RENDERER=NVIDIA fixture\nGLXR_REF_FAILS=$2"\n' + condition
            return subprocess.run(['bash', '-c', cmd, 'bash', str(info), str(fails)],
                                  capture_output=True, text=True, timeout=5)

    def test_large_correct_hardware_output_passes(self):
        r = self.grade(0)
        self.assertIn('GFX_S5=PASS', r.stdout, r.stderr)

    def test_failed_render_reference_is_still_rejected(self):
        r = self.grade(1)
        self.assertIn('GFX_S5=FAIL', r.stdout, r.stderr)


if __name__ == '__main__':
    unittest.main()
