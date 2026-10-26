"""GPU-free forwarding/marker checks; fixture and observer are compiled on the host."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class PublicGlObserver(unittest.TestCase):
    def check_call(self, mode, size):
        with tempfile.TemporaryDirectory(dir=os.environ['LX2_FIXTURE_TMP']) as tmp:
            out = Path(tmp) / 'observer.txt'
            clean = dict(os.environ)
            clean.pop('LD_PRELOAD', None)
            clean.pop('LX2_RM_OUT', None)
            argv = [os.environ['LX2_GL_FIXTURE'], mode, str(size)]
            reference = subprocess.run(argv, env=clean, capture_output=True, check=True)
            observed = dict(clean, LD_PRELOAD=os.environ['LX2_RM_OBSERVER'],
                            LX2_RM_OUT=str(out))
            result = subprocess.run(argv, env=observed, capture_output=True, check=True)
            self.assertEqual(result.stdout, reference.stdout)
            self.assertEqual(result.stderr, reference.stderr)
            lines = out.read_text().splitlines()
            method = 'glTexImage2D' if mode == 'tex' else 'glRenderbufferStorage'
            self.assertEqual(len(lines), 2)
            self.assertIn('RM_GL_BEGIN ' + method, lines[0])
            self.assertIn('RM_GL_END ' + method, lines[1])
            self.assertIn(f'width={size} height={size + 1}', lines[0])

    def test_texture_small(self):
        self.check_call('tex', 64)

    def test_texture_large(self):
        self.check_call('tex', 3840)

    def test_depth_small(self):
        self.check_call('depth', 64)

    def test_depth_large(self):
        self.check_call('depth', 3840)


if __name__ == '__main__':
    unittest.main()
