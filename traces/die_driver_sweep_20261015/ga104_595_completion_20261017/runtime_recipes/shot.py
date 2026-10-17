"""Bounded QMP screenshot statistics; the image stays on this owned rental."""
from pathlib import Path
import hashlib, json, re, socket

vm=Path('/workspace/ampere-winvm/ampere580')
assert Path('/root/prov/OWN_SWEEP_ID').read_text().strip()=='56326220'
p=Path('/root/prov/ampere595_windows_desktop.ppm')
s=socket.socket(socket.AF_UNIX);s.settimeout(15);s.connect(str(vm/'run/qmp.sock'))
f=s.makefile('rb')
def recv():
    b=f.readline(65537)
    if len(b)>65536 or not b.endswith(b'\n'):raise RuntimeError('QMP reply bound')
    r=json.loads(b)
    if not isinstance(r,dict):raise RuntimeError('QMP invalid reply')
    return r
def call(name,args=None):
    request=dict(execute=name,id=name)
    if args:request['arguments']=args
    s.sendall(json.dumps(request).encode()+b'\n')
    for _ in range(64):
        r=recv()
        if r.get('id')==name:
            if 'error' in r:raise RuntimeError('QMP command failed')
            return r['return']
    raise RuntimeError('QMP event bound')
try:
    assert 'QMP' in recv()
    call('qmp_capabilities')
    call('screendump',dict(filename=str(p)))
finally:f.close();s.close()
assert p.stat().st_size<=32*1024*1024
raw=p.read_bytes()
m=re.match(rb'^P6\s+(\d+)\s+(\d+)\s+255\s',raw);assert m
w,h=map(int,m.groups());assert 0<w<=4096 and 0<h<=2160
pixels=raw[m.end():];assert len(pixels)==w*h*3
colors=len(set(pixels[i:i+3] for i in range(0,len(pixels),3)))
print(json.dumps(dict(width=w,height=h,colors=colors,pixel_sha256=hashlib.sha256(pixels).hexdigest(),capture='QMP kf0 surface; screenshot stays on rental')))
assert colors>50,'nonblank presented desktop required'
