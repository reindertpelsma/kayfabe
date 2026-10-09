import sys, re
from Evtx.Evtx import Evtx
import xml.etree.ElementTree as ET
ns = '{http://schemas.microsoft.com/win/2004/08/events/event}'
path = sys.argv[1]; since = sys.argv[2] if len(sys.argv) > 2 else ''
rows = []
with Evtx(path) as log:
    for rec in log.records():
        try:
            x = ET.fromstring(rec.xml())
        except Exception:
            continue
        s = x.find(ns + 'System')
        prov = s.find(ns + 'Provider').get('Name')
        eid = s.find(ns + 'EventID').text
        t = s.find(ns + 'TimeCreated').get('SystemTime')
        lvl = s.find(ns + 'Level').text
        ed = x.find(ns + 'EventData')
        data = []
        if ed is not None:
            for d in ed:
                data.append((d.get('Name') or '') + '=' + (d.text or ''))
        ud = x.find(ns + 'UserData')
        if ud is not None:
            data.append(ET.tostring(ud, encoding='unicode')[:300])
        rows.append((t, prov, eid, lvl, ' '.join(data)[:400]))
rows.sort()
for r in rows:
    if r[0] >= since:
        print(*r, sep=' | ')
