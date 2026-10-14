# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import sys,re,collections
rows=[l.rstrip('\n').split('\t') for l in open(sys.argv[1])]
cl=collections.OrderedDict()
for r in rows:
    d,t,sq,qs,fn,res,extra=r[:7]
    t=float(t)
    m=re.search(r'hClient=(\w+)',extra); hc=m.group(1) if m else None
    c=cl.setdefault(hc,dict(first=t,last=t,classes=collections.Counter(),cmds=collections.Counter(),free=None,nfree=0))
    c['last']=t
    if fn=='ALLOC' and d=='REQ':
        c['classes'][re.search(r'class=(\w+)',extra).group(1)]+=1
    if fn=='CTRL' and d=='REQ':
        c['cmds'][re.search(r'cmd=(\w+)',extra).group(1)]+=1
    if fn=='FREE' and d=='REQ': c['free']=t; c['nfree']+=1
for hc,c in cl.items():
    print(hc,'first=%.3f last=%.3f free=%s'%(c['first'],c['last'],c['free']),'nctrl=%d'%sum(c['cmds'].values()),'classes=',dict(c['classes']) if len(c['classes'])<14 else len(c['classes']))
