#!/usr/bin/env python3
"""Manual guest helper: gx.py ps|int|pull|push ..."""
import importlib.util, json, sys, pathlib
import os
W=pathlib.Path('/workspace/bench/propimage_20261112/work_'+os.environ['KF_WORK'])
spec=importlib.util.spec_from_file_location('pc',pathlib.Path('/workspace/bench/pcnight2_20261115/src/scripts/bench/windows/propimage_check.py'))
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
st=json.loads((W/'state.json').read_text())
g=m.Guest(st,str(W/'id_kf'),st['ids']['net']['ssh_port'])
c=sys.argv[1]
if c=='ps':
    src=sys.argv[2]
    to=int(sys.argv[3]) if len(sys.argv)>3 else 120
    if pathlib.Path(src).is_file():
        n=pathlib.Path(src).name
        g.ps(r"New-Item -Force -ItemType Directory C:\kfhw | Out-Null")
        q=g.scp(src,'C:/kfhw/gx_'+n,True)
        p=g.ssh(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File','C:\\kfhw\\gx_'+n],timeout=to) if not q.returncode else q
    else:
        p=g.ps(src,timeout=to)
elif c=='int':
    p=g.interactive(sys.argv[2],open(sys.argv[3]).read())
elif c=='pull': p=g.scp(sys.argv[2],sys.argv[3],False)
elif c=='push': p=g.scp(sys.argv[2],sys.argv[3],True)
print(p.stdout,end=''); print(p.stderr,end='',file=sys.stderr); sys.exit(p.returncode)
