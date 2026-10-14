#!/usr/bin/env python3
"""Bounded kernel Xid capture with microsecond endpoint precision.

Second-truncating --until drops records from the current fractional second.
Optional explicit endpoint limits collection to one already-ended run.
"""
import datetime
import json
from pathlib import Path
import subprocess
import sys

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
start = sys.argv[2]
end = sys.argv[3] if len(sys.argv) > 3 else datetime.datetime.now(
    datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S.%f UTC')
p = subprocess.run(['journalctl', '-k', '--since', start, '--until', end,
                    '--utc', '--no-pager', '-o', 'short-iso-precise'],
                   capture_output=True, text=True)
lines = [s for s in p.stdout.splitlines() if 'NVRM: Xid' in s]
(out / 'kernel-xids.txt').write_text('\n'.join(lines) + ('\n' if lines else ''))
meta = dict(since=start, until=end, source='kernel journal', query_exit=p.returncode,
            xid_records=len(lines) if p.returncode == 0 else None,
            query_stderr=p.stderr.strip(), endpoint_precision='microseconds')
(out / 'kernel-xids.json').write_text(json.dumps(meta, indent=2) + '\n')
print('KERNEL_JOURNAL_XID_RECORDS', meta['xid_records'], 'QUERY_EXIT', p.returncode)
raise SystemExit(p.returncode)
