"""Independent exact-own-run safety timer. No process shared with the driver."""
import json,os,sys,time,urllib.request,urllib.parse
from pathlib import Path
from vastai._base import _resolve_api_key,_APIKEY_SENTINEL
root=Path(sys.argv[1]);key=_resolve_api_key(_APIKEY_SENTINEL)
run=json.loads((root/'run.json').read_text());label=run['label']
def api(path,method='GET'):
    with urllib.request.urlopen(urllib.request.Request('https://console.vast.ai'+path,method=method,headers={'Authorization':'Bearer '+key}),timeout=20) as r:return json.load(r)
def owned():
    p=root/'owned.json'
    if p.exists():
        item=json.loads(p.read_text());detail=api('/api/v0/instances/'+str(item['id'])+'/');assert 'instances' in detail;i=detail['instances']
        if i is None:return []
        assert i.get('label')==label and i.get('id')==item['id']
        return [i['id']]
    q=urllib.parse.urlencode({'limit':2,'select_filters':json.dumps({'label':{'eq':label}})})
    d=api('/api/v1/instances/?'+q)
    assert d.get('success') is True and isinstance(d.get('instances'),list) and not d.get('next_token')
    matches=[i for i in d['instances'] if i.get('label')==label];assert len(matches)<=1
    return [i['id'] for i in matches]
deadline=time.time()+900
(root/'reaper-armed.json').write_text(json.dumps({'pid':os.getpid(),'deadline':deadline,'label':label}))
print('Independent own-run reaper armed.',flush=True)
time.sleep(max(0,deadline-time.time()))
for attempt in range(12):
    try:
        ids=owned()
        if not ids:print('Verified own run absent.',flush=True);break
        for i in ids:assert api('/api/v0/instances/'+str(i)+'/','DELETE').get('success') is True
    except Exception:print('Reaper provider retry.',flush=True)
    time.sleep(5)
else:print('REAPER_EXHAUSTED',flush=True)
