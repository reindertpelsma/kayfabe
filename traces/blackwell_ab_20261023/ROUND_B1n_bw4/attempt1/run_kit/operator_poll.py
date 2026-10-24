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
    'sweep_windows_install.log', 'install_life', 'measured_build.log', 'bwr_bind.log',
    'arm_on_urt.log', 'collect_on_urt.log', 'on_urt_panic',
    'arm_on_relay_off.log', 'collect_on_relay_off.log', 'on_relay_off_panic',
    'arm_on_tdr60.log', 'collect_on_tdr60.log', 'on_tdr60_panic',
    'arm_lx_reload.log', 'collect_lx_reload.log', 'lx_reload_panic',
    'arm_on_diag_pnp.log', 'collect_on_diag_pnp.log', 'on_diag_pnp_panic',
    'fatal_collect.log',
]
if not row.get('controller_alive'):
    keys += ['sweep_20261020_provision.log']
print(json.dumps({k: row[k] for k in keys if k in row}))
