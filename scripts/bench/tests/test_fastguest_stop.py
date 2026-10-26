#!/usr/bin/env python3
"""GPU-free lifecycle tests with owned simulated processes, never a VM/GPU."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

LANE = Path(__file__).resolve().parents[2] / 'fastguest/run_fast_guest.sh'

class Lifecycle(unittest.TestCase):
    def run_lane(self, existing, slow):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / 'bin'; bin_dir.mkdir()
            fg = root / 'fg'; fg.mkdir()
            for name in ('vmlinuz', 'initrd.cpio.gz'):
                (fg / name).write_text('fixture; no GPU execution')
            def executable(path, text):
                path.write_text(text); path.chmod(0o755)
            executable(bin_dir / 'pkill', '#!/bin/sh\necho "$*" >>"$FIXTURE_ROOT/pkill"\n')
            executable(bin_dir / 'pgrep', '#!/bin/sh\nexit '+('0' if existing else '1')+'\n')
            executable(bin_dir / 'nvidia-smi', '#!/bin/sh\ncase "$*" in *memory.total*) echo 8192;; *) echo 0;; esac\n')
            fake = root / 'fake-guest'
            executable(fake, '''#!/usr/bin/env python3
import os,signal,time
from pathlib import Path
r=Path(os.environ['FIXTURE_ROOT']);(r/'launched').write_text('1')
def stop(signum,frame):
 (r/'term').write_text('1');time.sleep(4);(r/'cleanup').write_text('1');raise SystemExit(0)
signal.signal(signal.SIGTERM,stop)
if os.environ['FIXTURE_SLOW']=='1':
 while True:time.sleep(.02)
''')
            env = os.environ.copy()
            env.update(PATH=str(bin_dir)+':'+env['PATH'], FIXTURE_ROOT=str(root),
                FIXTURE_SLOW='1' if slow else '0', KF_DEVICE='kf3', QEMU_BIN=str(fake),
                BENCH_DIR=str(root), KF_FASTGUEST_DIR=str(fg), KF_LOCK=str(root/'lock'),
                KF3_DEV_EXTRA='guest-driver=580.159.04', KF_REQUIRE_FORWARD='0')
            result = subprocess.run(['bash', str(LANE), 'lifecycle', '1'], env=env,
                text=True, capture_output=True, timeout=15)
            return result, {name: (root/name).exists() for name in ('pkill','launched','term','cleanup')}

    def test_existing_lane_is_refused_without_killing_or_launching(self):
        result, files = self.run_lane(True, False)
        self.assertEqual(result.returncode, 2, result.stdout+result.stderr)
        self.assertFalse(files['pkill'])
        self.assertFalse(files['launched'])

    def test_deadline_remains_a_failure_but_term_cleanup_gets_to_finish(self):
        result, files = self.run_lane(False, True)
        self.assertEqual(result.returncode, 1, result.stdout+result.stderr)
        self.assertIn('FAST_VERDICT=CRASH', result.stdout)
        self.assertTrue(files['term'])
        self.assertTrue(files['cleanup'], 'SIGKILL must not cut off owned process cleanup')

if __name__ == '__main__':
    unittest.main()
