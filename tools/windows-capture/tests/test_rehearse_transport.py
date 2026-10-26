#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Rehearse the shell's plain CLI/SSH/scp flow without a network, lease, or Windows."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'rehearse/rehearse-vast.sh'
STUB = '''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
name=Path(sys.argv[0]).name
with open(os.environ['CALL_LOG'],'a') as f:
 f.write(json.dumps([name,*sys.argv[1:]])+'\\n')
if name=='vastai':
 print(json.dumps([{'id':int(os.environ.get('API_ID','123'))}]))
elif name=='ssh':
 if '-G' in sys.argv:
  print('stricthostkeychecking true')
  print('forwardagent '+os.environ.get('FORWARD_AGENT','no'))
 elif 'LastBootUpTime' in sys.argv[-1]:
  p=Path(os.environ['BOOT_COUNTER'])
  n=int(p.read_text())+1 if p.exists() else 1
  p.write_text(str(n))
  print(7 if n<3 else 8)
 elif '-Action Setup' in sys.argv[-1]:
  print('KFWC_REBOOT_REQUIRED='+os.environ.get('REBOOT','0'))
 elif '-Action Collect' in sys.argv[-1]:
  print('collection')
  sys.exit(int(os.environ.get('COLLECT_RC','0')))
 elif '--names' in sys.argv[-1]:
  sys.stdin.read()
  print('warm/cell.json')
 elif 'cat >' in sys.argv[-1] or 'python3 -' in sys.argv[-1]:
  sys.stdin.read()
'''


class TransportTests(unittest.TestCase):
    def run_flow(self, **extra):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for cmd in ('ssh', 'scp', 'vastai', 'sleep'):
                p = root / cmd
                p.write_text(STUB)
                p.chmod(0o755)
            for name in ('kfwc.sys', 'kfwc_collect.exe', 'd3d_minimal.exe', 'SHA256SUMS'):
                (root / name).write_text('fixture')
            log = root / 'calls.jsonl'
            env = dict(os.environ, PATH=str(root)+':'+os.environ['PATH'],
                       CALL_LOG=str(log), BOOT_COUNTER=str(root/'boot-counter'), **extra)
            result = subprocess.run(['bash', str(SCRIPT), '123', 'win', 'trusted',
                                     '/data/test-private', str(root)], env=env,
                                    capture_output=True, text=True, timeout=20, check=False)
            calls = [json.loads(s) for s in log.read_text().splitlines()]
            return result, calls

    def test_success_pulls_and_verifies_on_trusted_host(self):
        result, calls = self.run_flow()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(c[0]=='scp' and '-3' in c for c in calls))
        self.assertTrue(any('python3 - ' in c[-1] for c in calls if c[0]=='ssh'))
        self.assertFalse(any('destroy' in c or 'create' in c for c in calls))

    def test_collector_failure_still_pulls_and_returns_failure(self):
        result, calls = self.run_flow(COLLECT_RC='7')
        self.assertEqual(result.returncode, 7)
        self.assertTrue(any(c[0]=='scp' and '-3' in c for c in calls))
        self.assertTrue(any('python3 - ' in c[-1] for c in calls if c[0]=='ssh'))

    def test_missing_id_does_not_upload(self):
        result, calls = self.run_flow(API_ID='456')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[0]=='scp' for c in calls))

    def test_forwarded_agent_is_refused_before_remote_work(self):
        result, calls = self.run_flow(FORWARD_AGENT='yes')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[0]=='scp' for c in calls))
        self.assertTrue(all(c[0]=='ssh' and '-G' in c for c in calls))

    def test_reboot_requires_changed_boot(self):
        result, calls = self.run_flow(REBOOT='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        reads = [c for c in calls if c[0]=='ssh' and 'LastBootUpTime' in c[-1]]
        self.assertEqual(len(reads), 3)  # before, unchanged old boot, new boot
        self.assertTrue(any(c[-1]=='shutdown.exe /r /t 3' for c in calls))


if __name__ == '__main__':
    unittest.main()
