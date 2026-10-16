import gzip,hashlib,json,re,sys
from pathlib import Path
p=Path(sys.argv[1]);assert p.name.endswith('.ppm.gz')
with gzip.open(p,'rb') as f:raw=f.read(32*1024*1024+1)
assert len(raw)<=32*1024*1024
m=re.match(rb'^P6\s+(\d+)\s+(\d+)\s+255\s',raw);assert m
w,h=map(int,m.groups());assert 0<w<=4096 and 0<h<=2160
pixels=raw[m.end():];assert len(pixels)==w*h*3
colors=len(set(pixels[i:i+3] for i in range(0,len(pixels),3)))
print(json.dumps({'width':w,'height':h,'colors':colors,'pixel_sha256':hashlib.sha256(pixels).hexdigest(),'capture':'kf0 QMP console surface; screenshot remains on rental'}))
assert colors>50,'nonblank presented desktop required'
