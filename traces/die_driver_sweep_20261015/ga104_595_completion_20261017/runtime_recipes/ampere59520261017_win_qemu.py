#!/usr/bin/env python3
"""Private bench adapter: persist this VM's UUID and select only the exact built QEMU."""
import os,sys,uuid
from pathlib import Path
root=Path('/workspace/ampere-winvm/ampere580')
root.mkdir(parents=True,exist_ok=True,mode=0o700)
p=root/'vm.uuid'
if not p.exists():
    fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as f:f.write(str(uuid.uuid4())+'\n')
identity=uuid.UUID(p.read_text().strip())
if not identity.int:raise SystemExit('REFUSED zero VM UUID')
args=sys.argv[1:]
if '-uuid' in args:raise SystemExit('REFUSED duplicate VM identity')
args+=['-uuid',str(identity)]
binary='/workspace/bench/kf3-bins/9d6f210b/qemu-system-x86_64'
env=os.environ.copy()
# --no-rpc-trace writes zero. Absence has the same disabled meaning and keeps P19 exact.
if env.get('KF3_RPC_TRACE')=='0':env.pop('KF3_RPC_TRACE')
os.execve(binary,[binary]+args,env)
