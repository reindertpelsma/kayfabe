"""Shared constants for the Blackwell resume lane (own ids only; ledger is this lane's own file)."""
from pathlib import Path
import json, subprocess
W = Path('/workspace/bw-wt')
D = Path('/workspace/bw-run')
TR = W / 'traces/blackwell_resume_20261021'
LEDGER = D / 'owned_instances.json'
LANE_START_CREDIT = 21.38107079596284
STOP = 6.0
RESERVE = 5.0
def api(args):
    return json.loads(subprocess.check_output(['vastai', *args, '--raw'], text=True, stderr=subprocess.DEVNULL))
