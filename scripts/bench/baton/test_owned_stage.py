import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


class StageLease(unittest.TestCase):
    def check_abort(self, xid):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lock, ready, report, result = [root / s for s in ("lease", "ready", "report", "result")]
            counter = root / "counter"
            counter.write_text("")
            fake = root / "dmesg"
            fake.write_text('#!/bin/sh\ncat "$STAGE_TEST_COUNTER"\n')
            fake.chmod(0o755)
            body = '''import json,pathlib,signal,subprocess,sys,time
lock,ready,report=sys.argv[1:]
def stop(*_):
 time.sleep(0.15) # cleanup is still in progress after its parent might exit
 released=subprocess.run(['flock','-n',lock,'true']).returncode==0
 pathlib.Path(report).write_text(json.dumps({'lease_released':released}))
 time.sleep(0.15)
 raise SystemExit(0)
signal.signal(signal.SIGTERM,stop)
pathlib.Path(ready).write_text('ready')
while True:time.sleep(0.05)
'''
            command = [sys.executable, str(Path(__file__).with_name("owned_stage.py")),
                       "--lock", str(lock), "--seconds", "5" if xid else "0.5",
                       "--baseline", "0", "--result", str(result), "--",
                       sys.executable, "-c", body, str(lock), str(ready), str(report)]
            proc = subprocess.Popen(command, env=os.environ | dict(PATH=str(root) + ":" + os.environ["PATH"], STAGE_TEST_COUNTER=str(counter)))
            try:
                until = time.monotonic() + 5
                while not ready.exists() and proc.poll() is None and time.monotonic() < until:
                    time.sleep(0.01)
                self.assertTrue(ready.exists(), "owned child must actually start")
                if xid:
                    counter.write_text("Xid\n")
                proc.wait(timeout=10)
                self.assertEqual(json.loads(result.read_text())["reason"], "new_xid" if xid else "deadline")
                self.assertFalse(json.loads(report.read_text())["lease_released"], "lease dropped while owned child was cleaning up")
                self.assertEqual(subprocess.run(["flock", "-n", str(lock), "true"]).returncode, 0)
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    proc.wait(timeout=10)

    def test_deadline_keeps_lease_through_child_cleanup(self):
        self.check_abort(False)

    def test_xid_keeps_lease_through_child_cleanup(self):
        self.check_abort(True)


if __name__ == "__main__":
    unittest.main()
