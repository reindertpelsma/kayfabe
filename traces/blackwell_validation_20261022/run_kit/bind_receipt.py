# usage: receipt.py <tag> <expected_exe> <profile:on|off|bind>
from pathlib import Path
import hashlib,json,os,subprocess,sys
v=Path('/workspace/bw-capc-validation/kfwin')
pid=int((v/'run/qemu.pid').read_text()); p=Path('/proc')/str(pid)
argv=p.joinpath('cmdline').read_bytes().split(b'\0')
env=dict(item.split(b'=',1) for item in p.joinpath('environ').read_bytes().split(b'\0') if b'=' in item)
flags=Path('/root/sweep2/p19.flags').read_text().splitlines(); assert len(flags)==19
for flag in flags:
    k,val=flag.encode().split(b'=',1); assert env.get(k)==val,(k,env.get(k))
exe=os.readlink(p/'exe'); assert exe==sys.argv[2],exe
dev=next(x.decode() for x in argv if x.startswith(b'kf3-gpu,'))
opts=dict(s.split('=',1) for s in dev.split(',')[1:] if '=' in s)
prof=sys.argv[3]
if prof in ('on','off'):
    assert opts['display']==prof and opts['gop']==prof,opts
    for key,value in ((b'KF3_RPC_TRACE',b'1'),(b'KF3_RPC_BODY_TRACE',b'1'),(b'KF3_DIAG_PROGRESS',b'1000')):
        assert env.get(key)==value,(key,env.get(key))
assert opts['guest-driver']=='580.65.06',opts
rec=dict(pid=pid,exe=exe,sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),kf3_device=dev,flags={k.decode():x.decode() for k,x in env.items() if k.startswith(b'KF3_')},p19_count=len(flags),harness_checkout=subprocess.check_output(['git','rev-parse','HEAD'],cwd='/root/kayfabe',text=True).strip())
Path('/root/prov/'+sys.argv[1]+'_process_receipt.json').write_text(json.dumps(rec,indent=2)+'\n'); print(json.dumps({k:rec[k] for k in ('pid','exe','sha256','kf3_device')}))
