import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from target_preload import target_preload


class TargetPreload(unittest.TestCase):
    def capture(self):
        with tempfile.TemporaryDirectory(dir=os.environ['LX2_FIXTURE_TMP']) as tmp:
            log = Path(tmp) / 'rm.txt'
            argv = ['taskset', '-c', '0-7', os.environ['LX2_RM_FIXTURE']]
            args, env = target_preload(argv, os.environ, 3, {
                'LD_PRELOAD': os.environ['LX2_RM_OBSERVER'], 'LX2_RM_OUT': str(log),
            })
            result = subprocess.run(args, env=env, capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout, 'fixture rc=-1 errno=25\n')
            self.assertEqual(result.stderr, '')
            return log.read_text()

    def test_target_memory_request_is_observed_after_affinity_wrapper(self):
        text = self.capture()
        self.assertIn('RM_MEMORY in ', text)
        self.assertIn('compression=2 page=2 huge=1 size=1048576', text)

    def test_target_return_and_errno_are_observed_after_affinity_wrapper(self):
        text = self.capture()
        self.assertIn('RM_MEMORY out ', text)
        self.assertIn('RM_RETURN command=2b rc=-1 errno=25', text)


if __name__ == '__main__':
    unittest.main()
