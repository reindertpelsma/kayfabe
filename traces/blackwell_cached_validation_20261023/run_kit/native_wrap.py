#!/usr/bin/env python3
from pathlib import Path
import json,os,subprocess,sys
sys.path.insert(0,'/root/sweep2/guard/blackwell')
import phase_xid_guard as G
D=Path('/root/prov'); marker=D/'XID_STOP'; receipt=D/'native_receipt.json'
if marker.exists(): sys.exit(7)
try:
    before=json.loads(receipt.read_text()); old=tuple(before['approved_native_xid_lines'])
    kind=sys.argv[1].removeprefix('--'); log=Path(os.readlink('/proc/self/fd/1'))
    assert str(log).startswith('/workspace/bench/'), 'not our native suite log'
    child=subprocess.Popen(['/root/kayfabe/target/release/kayfabe-rm-ladder',*sys.argv[1:]])
    print('OWNED_NATIVE_STARTED kind='+kind+' pid='+str(child.pid),flush=True)
    rc=child.wait(); journal=G.N.kernel_snapshot(); lines=G.N.xid_lines(journal)
    if lines[:len(old)]!=old: raise G.N.GuardRefused('native frozen history changed')
    text=G.N.bounded(log)
    proof=[{'kind':kind,'pid':child.pid}] if kind in ('defer-liveness','missing-page-fault') else []
    G.verify_native_run('\n'.join(lines[len(old):]),text if kind=='defer-liveness' else '',text if kind=='missing-page-fault' else '',proof,before['pci'])
    if rc: raise G.N.GuardRefused('raw native client failed rc='+str(rc))
    G.atomic_json(receipt,{'approved_native_xid_lines':lines,'pci':before['pci']})
    with (D/'native_processes.jsonl').open('a') as f: f.write(json.dumps({'kind':kind,'pid':child.pid,'rc':rc,'log':str(log)})+'\n')
    if kind in ('defer-liveness','missing-page-fault'): (D/('source_'+kind+'.log')).write_text(text)
except Exception as error:
    G.atomic_json(marker,{'STOP':str(error),'stage':'native raw','kind':sys.argv[1]})
    print('NATIVE_GUARD_STOP '+str(error),file=sys.stderr);sys.exit(7)
