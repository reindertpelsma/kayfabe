import sys,re
import Evtx.Evtx as ev
f=sys.argv[1]; since=sys.argv[2]
with ev.Evtx(f) as log:
    for r in log.records():
        x=r.xml()
        t=re.search(r"SystemTime=\"([^\"]+)\"",x)
        if not t or t.group(1)<since: continue
        p=re.search(r"Provider Name=\"([^\"]+)\"",x).group(1)
        i=re.search(r"<EventID[^>]*>(\d+)<",x).group(1)
        d=re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",x.split("</System>")[-1]))[:int(sys.argv[3]) if len(sys.argv)>3 else 300]
        print(t.group(1)[:23],p,i,d)
