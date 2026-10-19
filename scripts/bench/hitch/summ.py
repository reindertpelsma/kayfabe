import re, kv, statistics as st, sys
import importlib.util
spec=importlib.util.spec_from_file_location('fr','fr.py'); fr=importlib.util.module_from_spec(spec); spec.loader.exec_module(fr)
arms=[('H1 p1 (cold, 0452613b)','H1','p1',None),('H1 p2 (warm)','H1','p2',None),('H2 p1 (cold)','H2','p1','hv_h2_p1.log'),('H2 p2 (warm)','H2','p2','hv_h2_p2.log'),('H3 (WTRACE on)','H3','p1','hv_h3_p1.log'),('H4 (free clock, x11)','H4','p1','hv_h4_p1.log'),('H5 (meters 54266072)','H5','p1','hv_h5_p1.log'),('A2 (54266072)','A2','p1','hv_a2_p1.log'),('A3 (54266072)','A3','p1','hv_a3_p1.log'),('B1 FIX 6c2c7b96','B1','p1','hv_b1_p1.log'),('B2 FIX','B2','p1','hv_b2_p1.log'),('B3 FIX','B3','p1','hv_b3_p1.log')]+[(x,x[:2],'p1','hv_%s_p1.log'%x[:2].lower()) for x in sys.argv[1:]]
heav={'H1p1':(51.4859,29.5193,69.0402),'H1p2':(51.3269,22.3761,68.369)}
def hl(path):
    t=open(path,errors='replace').read().replace('&nbsp;',' ')
    num=r'([0-9]+\.[0-9]{1,4}?)(?=\d\d:\d\d:\d\d|\s|$)'
    m=re.search(r'FPS:\s*'+num+r'\s*(?:\d\d:\d\d:\d\d)?\s*Min FPS:\s*'+num+r'\s*(?:\d\d:\d\d:\d\d)?\s*Max FPS:\s*'+num,t)
    return tuple(float(x) for x in m.groups()) if m else (0,0,0)
print('%-24s %6s %6s %6s | dip-windows(tear<46) dipsec | copies>tick+1 | ETW p99 interval ms / max ms'%('arm','avg','min','max'))
for name,d,p,lg in arms:
    f9=float(open('%s/%s.%s.f9_epoch'%(d,d,p)).read())
    h=hl('%s/%s'%(d,lg)) if lg else heav.get(d+p,(0,0,0))
    rows=[(t,x) for t,k,x in kv.load('%s/%s.dispts.txt'%(d,d)) if k=='fam' and 0<=t-f9<=260 and 'fps.h0.tear' in x]
    dip=sum(1 for _,x in rows if x['fps.h0.tear']<46); ex=sum(1 for _,x in rows if x['fps.copies']-x['fps.h0.tick']>1)
    extra=''
    if lg:
        F=fr.frames('%s/%s'%(d,lg)); t=[fr.to_epoch(a,7200.0)-f9 for a,_ in F]
        idx=[i for i,x in enumerate(t) if 0<=x<=261]; iv=sorted((t[idx[k]]-t[idx[k-1]])*1000 for k in range(1,len(idx)))
        extra='%.1f / %.1f'%(iv[int(len(iv)*.99)],iv[-1])
    print('%-24s %6.2f %6.2f %6.2f | %4d  %4d s          | %3d           | %s'%(name,h[0],h[1],h[2],dip,2*dip,ex,extra))
