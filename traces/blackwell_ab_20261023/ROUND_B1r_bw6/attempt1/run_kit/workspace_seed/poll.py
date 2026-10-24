from pathlib import Path
import json
import re
import subprocess
import sys

D = Path(__file__).resolve().parent
r = subprocess.run(
    [sys.executable, str(D / 'remote.py'), str(D / 'milestone_probe.py')],
    capture_output=True, text=True, timeout=55,
)
if r.returncode:
    print('PROBE_RC', r.returncode, r.stderr[-500:])
    raise SystemExit(r.returncode)
row = json.loads(r.stdout)
clean = re.sub(r'GPU-[0-9a-fA-F-]+', 'GPU-UUID-REMOVED', json.dumps(row))
with (D / 'milestones.jsonl').open('a') as f:
    f.write(clean + '\n')
keys = ['utc', 'controller_alive', 'fatal', 'native_ready', 'native_ack']
keys += [
    'sweep_windows_install.log', 'install_life', 'measured_build.log', 'observed_fix_build.log', 'bwr_bind.log',
    'arm_on_fix.log', 'collect_on_fix.log', 'on_fix_panic',
    'fatal_collect.log',
]
if not row.get('controller_alive'):
    keys += ['sweep_20261020_provision.log']
print(json.dumps({k: row[k] for k in keys if k in row}))
