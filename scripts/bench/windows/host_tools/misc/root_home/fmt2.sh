#!/bin/bash
python3 - <<'PY'
import json
d=json.load(open('/tmp/drm.json'))
def walk(o,path=""):
    if isinstance(o,dict):
        for k,v in o.items():
            if k=="IN_FORMATS":
                print("  RAW IN_FORMATS:", json.dumps(v)[:1200]); return
            walk(v,path+"/"+str(k))
    elif isinstance(o,list):
        for i,v in enumerate(o): walk(v,path+f"[{i}]")
walk(d)
PY
echo "== modetest view (authoritative, human readable) =="
sudo modetest -p 2>/dev/null | sed -n '/planes:/,/^$/p' | head -30
