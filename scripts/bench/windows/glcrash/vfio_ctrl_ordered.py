import sys,re,collections
# usage: ordered.py gl-window.tsv CLIENT[,CLIENT...]  -> ordered controls (request order) with the paired reply status
want=set(sys.argv[2].split(','))
pend=collections.defaultdict(list); out=[]
for l in open(sys.argv[1]):
    r=l.rstrip('\n').split('\t'); d,t,sq,qs,fn,res,extra=r[:7]
    if fn!='CTRL': continue
    m=dict(re.findall(r'(\w+)=(\w+)',extra))
    k=(m['hClient'],m['hObj'],m['cmd'])
    if d=='REQ':
        e=dict(t=float(t),k=k,psz=m['psz'],req=m.get('params',''),rep=None,st=None); pend[k].append(e); out.append(e)
    else:
        q=pend[k]
        for e in q:
            if e['rep'] is None: e['rep']=m.get('params',''); e['st']=m['status']; e['rt']=float(t); break
for e in out:
    if e['k'][0] in want:
        print('%+.6f'%e['t'],e['k'][0],e['k'][1],e['k'][2],'psz=%s'%e['psz'],'status=%s'%(e['st'] or '??'),'req=%s'%e['req'][:64],'rep=%s'%((e['rep'] or '')[:64]))
