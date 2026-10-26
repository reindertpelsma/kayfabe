from pathlib import Path
import hashlib,json,os,subprocess,sys
v=Path('/workspace/sweep-win-56788212/kfwin')
pid=int((v/'run/qemu.pid').read_text())
p=Path('/proc')/str(pid)
argv=p.joinpath('cmdline').read_bytes().split(b'\0')
env=dict(item.split(b'=',1) for item in p.joinpath('environ').read_bytes().split(b'\0') if b'=' in item)
flags=Path('/root/sweep-p19.flags').read_text().splitlines()
assert len(flags)==19
for flag in flags:
    k,val=flag.encode().split(b'=',1)
    assert env.get(k)==val,(k,env.get(k))
exe=os.readlink(p/'exe')
assert exe=='/workspace/bench/kf3-bins/24906bf3/qemu-system-x86_64',exe
assert b'-vga' in argv and argv[argv.index(b'-vga')+1]==b'none'
dev=next(x.decode() for x in argv if x.startswith(b'kf3-gpu,'))
profile=sys.argv[2]
assert profile in ('on','off')
opts=dict(s.split('=',1) for s in dev.split(',')[1:] if '=' in s)
assert opts['display']==profile and opts['gop']==profile
assert opts['guest-driver']=='580.65.06'
record=dict(pid=pid,source=subprocess.check_output(['git','rev-parse','HEAD'],cwd='/root/kayfabe',text=True).strip(),exe=exe,sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest(),argv=[a.decode() for a in argv if a],flags={k.decode():val.decode() for k,val in env.items() if k.startswith(b'KF3_')},p19_count=len(flags))
dest=Path('/root/prov')/(sys.argv[1]+'_process_receipt.json')
dest.write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record))
