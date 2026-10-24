"""2026-10-23: bounded lease-relative timer; exact own-id destruction and API absence."""
from pathlib import Path
import datetime
import json
import os
import subprocess
import sys
import time

D = Path(__file__).resolve().parent
OWNED = Path('/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/ROUND_B1m_0645/attempt1_rtx5090/owned_instances.json')
iid = int(sys.argv[1])
seconds = float(sys.argv[2])
rows = json.loads(OWNED.read_text())
if not (0 < seconds <= 170 * 60):
    raise ValueError('timer must be positive and at most 170 minutes')
if not any(int(row['id']) == iid and not row.get('destroyed') for row in rows):
    raise ValueError('timer id is not a live own lease')
ready = {
    'host_verification':'UNVERIFIED-HOST', 'id': iid, 'pid': os.getpid(), 'seconds': seconds,
    'armed_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
(D / f'timer_ready_{iid}_{os.getpid()}.json').write_text(json.dumps(ready) + '\n')
print("UNVERIFIED-HOST timer armed",iid,seconds,flush=True)
time.sleep(seconds)
for attempt in range(5):
    try:
        subprocess.run(['vastai','destroy','instance',str(iid),'-y'],
                       capture_output=True, timeout=45)
        live = json.loads(subprocess.check_output(
            ['vastai','show','instances','--raw'], stderr=subprocess.DEVNULL,
            text=True, timeout=45,
        ))
        if not isinstance(live, list):
            raise ValueError('instance list is not an array')
        if not any(int(row['id']) == iid for row in live):
            receipt = {'host_verification':'UNVERIFIED-HOST', 'id': iid, 'pid': os.getpid(), 'absence_verified_utc':
                       datetime.datetime.now(datetime.timezone.utc).isoformat()}
            (D / f'budget_auto_absence_{iid}_{os.getpid()}.json').write_text(
                json.dumps(receipt) + '\n')
            break
    except (subprocess.SubprocessError, ValueError, TypeError, KeyError) as error:
        print('OWN_TIMER_RETRY', attempt + 1, type(error).__name__, flush=True)
    time.sleep(3)
else:
    raise RuntimeError('own timer exact-id API absence unverified after five attempts')
