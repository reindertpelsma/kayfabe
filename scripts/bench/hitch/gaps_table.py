import sys, re, importlib.util
spec=importlib.util.spec_from_file_location('fr','fr.py'); fr=importlib.util.module_from_spec(spec); spec.loader.exec_module(fr)
spec=importlib.util.spec_from_file_location('pt','plot_timeline.py'); pt=importlib.util.module_from_spec(spec); spec.loader.exec_module(pt)
arms=[('H2p1','H2','p1','hv_h2_p1.log',None),('H2p2','H2','p2','hv_h2_p2.log',None),('H5','H5','p1','hv_h5_p1.log',None),('A2','A2','p1','hv_a2_p1.log',None),('A3','A3','p1','hv_a3_p1.log',None),('D1','D1','p1','hv_d1_p1.log','D1'),('D3','D3','p1','hv_d3_p1.log','D3'),
('B1 FIX','B1','p1','hv_b1_p1.log',None),('B2 FIX','B2','p1','hv_b2_p1.log',None),('B3 FIX','B3','p1','hv_b3_p1.log',None),('C1 FIX','C1','p1','hv_c1_p1.log','C1'),('C3 FIX','C3','p1','hv_c3_p1.log','C3')]
print('Per-minute counts (benchmark window 0..260 s = 4.33 min). GUEST = gap between consecutive DXGI Present starts (ETW, per present). BROKER = gap between consecutive NEW-content presents (compositor present_ns) where the line exists.')
print('%-8s | guest >33  >50  >100 per min, max ms, p99, p99.9 ms, 1%%low/0.1%%low fps | broker new fps, repeat/s, merged/s, cg>33 >50 >100 per min, max ms'%'arm')
for name,d,p,lg,bk in arms:
    try: f9=float(open('%s/%s.%s.f9_epoch'%(d,d,p)).read())
    except OSError: continue
    try: F=fr.frames('%s/%s'%(d,lg))
    except OSError: continue
    t=sorted(fr.to_epoch(a,7200.0)-f9 for a,_ in F); t=[x for x in t if 0<=x<=260]
    iv=[(t[i]-t[i-1])*1000 for i in range(1,len(t))]
    if not iv: continue
    s=sorted(iv); mins=260/60.0
    c=lambda th: sum(1 for x in iv if x>th)/mins
    p99=s[int(len(s)*.99)]; p999=s[int(len(s)*.999)]
    row='%-8s | %7.1f %5.1f %5.2f   %6.1f %6.1f %6.1f   %5.1f/%5.1f' % (name,c(33),c(50),c(100),s[-1],p99,p999,1000/p99,1000/p999)
    if bk:
        import os
        clk=dict(x.split('=') for x in open('%s/%s.clk'%(bk,bk)).read().split()); off=float(clk['epoch'])-float(clk['uptime'])
        B=[]
        for ln in open('%s/%s.broker.log'%(bk,bk),errors='replace'):
            m=re.search(r'stats t=(\d+) (.*)',ln)
            if m:
                dd=pt.parse_kv(m.group(2)); dd['t']=int(m.group(1))/1000.0+off-f9; B.append(dd)
        W=[b for b in B if 0<=b['t']<=260]
        if W and 'new' in W[0]:
            row+=' | %5.1f  %5.1f  %5.2f   %5.1f %5.1f %5.2f  %6.1f'%(sum(b['new'] for b in W)/len(W),sum(b['repeat'] for b in W)/len(W),sum(b['merged'] for b in W)/len(W),sum(b['cg33'] for b in W)/mins,sum(b['cg50'] for b in W)/mins,sum(b['cg100'] for b in W)/mins,max(b['cgap_max_us'] for b in W)/1000)
        elif W:
            row+=' | (v1 broker line: no new-content fields) timeouts in window=%d'%sum(b['timeouts'] for b in W)
    print(row)
