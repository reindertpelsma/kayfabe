#!/usr/bin/env python3
"""Repeat the same native cases with PID attribution and precise Xid windows.

The two-second observation interval is bench measurement only. No product state
or GPU completion is released by it. CPU-side rc/GPU tail alone are insufficient.
"""
import datetime
import json
from pathlib import Path
import subprocess
import time

ROOT = Path('/var/lib/kf-windows-20261005/codex-native-deferred-isolated-20261011')
PROBE = '/tmp/kf-deferred-update-probe'
CAPTURE = '/tmp/kf-capture-xids.py'

def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M:%S.%f UTC')

assert subprocess.run(['pgrep','qemu-system'],capture_output=True).returncode == 1
assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip() == 'DMA-FQ'
ROOT.mkdir(exist_ok=False)
print('NATIVE_ISOLATED_START',utc(),flush=True)
results=[]
for mode in ['nop','update']:
    case=ROOT/mode
    case.mkdir()
    start=utc()
    with (case/'probe.log').open('w') as log:
        p=subprocess.Popen(['setpriv','--reuid=65534','--regid=65534','--clear-groups',PROBE,mode],stdout=log,stderr=subprocess.STDOUT)
        print('NATIVE_ISOLATED_CASE_START',mode,'pid',p.pid,start,flush=True)
        try:
            rc=p.wait(timeout=30)
        except subprocess.TimeoutExpired:
            p.kill(); p.wait(timeout=10); rc=124
    process_end=utc()
    time.sleep(2)  # Only observe delayed fault delivery; no hardware work or wait release.
    end=utc()
    q=subprocess.run(['python3',CAPTURE,str(case),start,end],capture_output=True,text=True)
    meta=json.loads((case/'kernel-xids.json').read_text()) if q.returncode==0 else {}
    xid=meta.get('xid_records')
    result=dict(mode=mode,pid=p.pid,start_utc=start,process_end_utc=process_end,end_utc=end,
                rc=rc,xid_records=xid,verdict='PASS' if rc==0 and xid==0 else 'FAIL',
                observation_seconds=2,scope='same-config native probe only; not migration or Windows repair')
    (case/'verdict.json').write_text(json.dumps(result,indent=2)+'\n')
    results.append(result)
    print('NATIVE_ISOLATED_CASE_END',json.dumps(result),flush=True)
(ROOT/'verdict.json').write_text(json.dumps(results,indent=2)+'\n')
failed=any(r['verdict']!='PASS' for r in results)
print('NATIVE_ISOLATED_EXIT',int(failed),utc(),flush=True)
raise SystemExit(int(failed))
