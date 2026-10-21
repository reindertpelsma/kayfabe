import re, statistics as st, importlib.util, os
def load(n,p):
    spec=importlib.util.spec_from_file_location(n,p); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m
fr=load('fr','fr.py'); pt=load('pt','plot_timeline.py'); sm=load('sm','summ.py') if False else None
def hl(path):
    t=open(path,errors='replace').read().replace('&nbsp;',' ')
    num=r'([0-9]+\.[0-9]{1,4}?)(?=\d\d:\d\d:\d\d|\s|$)'
    m=re.search(r'FPS:\s*'+num+r'\s*(?:\d\d:\d\d:\d\d)?\s*Min FPS:\s*'+num+r'\s*(?:\d\d:\d\d:\d\d)?\s*Max FPS:\s*'+num,t)
    return tuple(float(x) for x in m.groups())
groups={'baseline (candidate-4 0452613b / meters 54266072)':[('H2','p1'),('H2','p2'),('H5','p1'),('A2','p1'),('A3','p1'),('D1','p1'),('D3','p1')],
        'FIX (colour path by the mailbox rule, 6c2c7b96 / 388f3289)':[('B1','p1'),('B2','p1'),('B3','p1'),('C1','p1'),('C3','p1')]}
def arm(d,p):
    lg='%s/hv_%s_%s.log'%(d,d.lower(),p)
    if not os.path.exists(lg): return None
    f9=float(open('%s/%s.%s.f9_epoch'%(d,d,p)).read())
    avg,mn,mx=hl(lg)
    F=fr.frames(lg); t=sorted(fr.to_epoch(a,7200.0)-f9 for a,_ in F); t=[x for x in t if 0<=x<=260]
    iv=[(t[i]-t[i-1])*1000 for i in range(1,len(t))]; s=sorted(iv); mins=260/60.0
    r=dict(n_pres=len(t)/260.0,avg=avg,mn=mn,mx=mx,g33=sum(x>33 for x in iv)/mins,g50=sum(x>50 for x in iv)/mins,g100=sum(x>100 for x in iv)/mins,gmax=s[-1],low1=1000/s[int(len(s)*.99)],low01=1000/s[int(len(s)*.999)])
    fam=[(tt,x) for tt,x in pt.load_family('%s/%s.dispts.txt'%(d,d)) if 'fps.h0.tear' in x]
    W=[x for tt,x in fam if 0<=tt-f9<=260]
    r['copies']=st.mean(x['fps.copies'] for x in W); r['ticks']=st.mean(x['fps.h0.tick'] for x in W)
    r['repeat_derived']=r['ticks']-r['copies']
    ck='%s/%s.clk'%(d,d)
    if os.path.exists(ck):
        clk=dict(x.split('=') for x in open(ck).read().split()); off=float(clk['epoch'])-float(clk['uptime'])
        B=[]
        for ln in open('%s/%s.broker.log'%(d,d),errors='replace'):
            m=re.search(r'stats t=(\d+) (.*)',ln)
            if m:
                dd=pt.parse_kv(m.group(2)); dd['t']=int(m.group(1))/1000.0+off-f9; B.append(dd)
        Bw=[b for b in B if 0<=b['t']<=260]
        if Bw and 'new' in Bw[0]:
            r['b_new']=sum(b['new'] for b in Bw)/len(Bw); r['b_rep']=sum(b['repeat'] for b in Bw)/len(Bw); r['b_merged']=sum(b['merged'] for b in Bw)/len(Bw)
            r['b_cg33']=sum(b['cg33'] for b in Bw)/mins; r['b_cg50']=sum(b['cg50'] for b in Bw)/mins; r['b_cg100']=sum(b['cg100'] for b in Bw)/mins; r['b_cmax']=max(b['cgap_max_us'] for b in Bw)/1000
        r['b_timeouts_session']=sum(b['timeouts'] for b in B); r['b_timeouts_bench']=sum(b['timeouts'] for b in Bw)
    return r
def ms(v):
    v=[x for x in v if x is not None]
    return '%.2f [%.2f..%.2f]'%(st.mean(v),min(v),max(v)) if v else 'n/a'
for g,arms in groups.items():
    R=[(d,p,arm(d,p)) for d,p in arms]; R=[(d,p,r) for d,p,r in R if r]
    print('\n== %s : n=%d passes (%s)'%(g,len(R),', '.join(d+p for d,p,_ in R)))
    for k,lab in (('avg','(e) Heaven avg fps'),('mn','    Heaven min fps'),('mx','    Heaven max fps'),('n_pres','(a) guest presents/s (ETW) [derived stage]'),('copies','    kayfabe copies/s [derived]'),('repeat_derived','(d) repeated presents/s = ticks - copies [derived]'),('low1','(b) 1% low fps = 1000/p99 of per-present gap (guest ETW)'),('low01','    0.1% low fps (guest ETW)'),('g33','(c) gaps >33 ms per min (guest)'),('g50','    gaps >50 ms per min'),('g100','    gaps >100 ms per min'),('gmax','    max gap ms (guest)'),('b_new','(a) BROKER new-content presents/s'),('b_rep','(d) BROKER repeat presents/s'),('b_merged','    BROKER commits merged/s'),('b_cg33','(c) BROKER new-content gaps >33 /min'),('b_cg50','    >50 /min'),('b_cg100','    >100 /min'),('b_cmax','    BROKER max new-content gap ms'),('b_timeouts_session','    broker request timeouts, whole session'),('b_timeouts_bench','    broker request timeouts, benchmark window')):
        print('   %-62s %s'%(lab,ms([r.get(k) for _,_,r in R])))
