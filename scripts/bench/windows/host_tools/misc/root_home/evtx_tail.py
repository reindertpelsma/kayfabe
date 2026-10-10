import sys, re, Evtx.Evtx as e
since = sys.argv[2]
with e.Evtx(sys.argv[1]) as log:
    for r in log.records():
        x = r.xml()
        m = re.search(r'SystemTime="([^"]+)"', x)
        if not m or m.group(1) < since: continue
        prov = re.search(r'Provider Name="([^"]+)"', x).group(1)
        eid = re.search(r'<EventID[^>]*>(\d+)<', x).group(1)
        data = re.findall(r'<Data Name="([^"]+)">([^<]*)<', x)
        print(m.group(1), prov, eid, "; ".join("%s=%s" % d for d in data)[:300])
