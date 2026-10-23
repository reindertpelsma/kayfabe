# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Exercise the real arm cleanup with an owned CPU-only process."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ARM = Path(__file__).resolve().parents[1] / 'diesweep/desktop/arm.sh'

class CleanupTests(unittest.TestCase):
    def run_cleanup(self, graceful='timeout', changed=False):
        child = subprocess.Popen(['sleep', '30'])
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td); (root/'run').mkdir()
                (root/'run/qemu.pid').write_text(str(child.pid + int(changed)))
                cleanup = ARM.read_text().split('cleanup(){', 1)[1].split('\ntrap cleanup EXIT', 1)[0]
                cleanup = cleanup.replace('/root/prov/arm_${run}_cleanup_started.txt', '$FIXTURE/cleanup_started.txt')
                script = r'''set +e
VM=$FIXTURE
W=fixture-win-vm
QLOG=$FIXTURE/absent
launched=1
owned_pid=$CHILD
observer_pid=
run=fixture
sleep() { :; }
sha256sum() { return 0; }
timeout() {
 if [ "$1" = 300 ]; then
   echo GRACEFUL_ATTEMPT
   if [ "$GRACEFUL" = success ]; then kill -TERM "$owned_pid"; /bin/sleep .05; return 0; fi
   return 124
 fi
 echo "FALLBACK $*"
 kill -TERM "$owned_pid"
 /bin/sleep .05
 return 0
}
cleanup(){''' + cleanup + '\nfalse\ncleanup\n'
                env = dict(os.environ, FIXTURE=td, CHILD=str(child.pid), GRACEFUL=graceful)
                return subprocess.run(['bash','-c',script], env=env, capture_output=True, text=True, timeout=5)
        finally:
            if child.poll() is None: child.terminate()
            child.wait(timeout=5)

    def test_interrupted_graceful_stop_still_reaches_qmp_quit(self):
        p = self.run_cleanup()
        self.assertIn('FALLBACK 20 python3 scripts/bench/windows/qmp.py', p.stdout)
        self.assertIn('/run/qmp.sock cmd quit', p.stdout)

    def test_qmp_fallback_preserves_original_arm_failure(self):
        p = self.run_cleanup()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertNotIn('OWN_VM_STILL_ALIVE_STOP', p.stdout)

    def test_successful_graceful_stop_does_not_send_quit(self):
        p = self.run_cleanup('success')
        self.assertNotIn('FALLBACK', p.stdout)
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)

    def test_changed_pid_never_receives_qmp_quit(self):
        p = self.run_cleanup(changed=True)
        self.assertNotIn('FALLBACK', p.stdout)
        self.assertIn('OWN_VM_PID_CHANGED_STOP', p.stdout)
