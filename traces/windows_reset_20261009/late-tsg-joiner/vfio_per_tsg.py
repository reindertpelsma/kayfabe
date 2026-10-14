import re,collections
tsgs={}  # (client,h)-> dict
chans={} # (client,h)->tsg or None
for l in open('vfio_timeline.txt'):
    p=l.split()
    t=float(p[0]); kind=p[1]
    if kind=='ALLOC':
        cls=p[2]; cl=p[4]; par=p[6]; h=p[8]
        if cls=='0xa06c':
            tsgs[(cl,h)]=dict(t=t,sched=[],chans=[],late=[])
        else:
            if (cl,par) in tsgs:
                g=tsgs[(cl,par)]
                late = bool(g['sched'])
                g['chans'].append((h,t,late))
                chans[(cl,h)]=(par)
            else: chans[(cl,h)]=None; 
    else:
        cmd=p[2]; cl=p[4]; ob=p[6]
        if cmd=='0xa06c0101' and (cl,ob) in tsgs: tsgs[(cl,ob)]['sched'].append((t,p[-1]))
        if cmd=='0xa06f0103': 
            par=chans.get((cl,ob))
            if par: tsgs[(cl,par)]['chsched']=tsgs[(cl,par)].get('chsched',[])+[(ob,t,p[-1])]
for k,g in tsgs.items():
    late=[c for c in g['chans'] if c[2]]
    print(k,'chans',[c[0] for c in g['chans']],'tsg-sched',g['sched'],'late',late,'chsched',g.get('chsched'))
