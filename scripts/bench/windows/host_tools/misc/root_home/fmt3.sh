#!/bin/bash
python3 - <<'PY'
import json,struct
d=json.load(open('/tmp/drm.json'))
def fourcc(v):
    try: return struct.pack('<I',v).decode('ascii',errors='replace')
    except Exception: return str(v)
found=None
def walk(o):
    global found
    if isinstance(o,dict):
        for k,v in o.items():
            if k=="IN_FORMATS" and found is None: found=v; return
            walk(v)
    elif isinstance(o,list):
        for v in o: walk(v)
walk(d)
data=found.get("data") if isinstance(found,dict) else None
print("  entries:",len(data) if isinstance(data,list) else "?")
for e in (data or []):
    mod=e.get("modifier"); fl=e.get("formats",[])
    names=[f"{fourcc(f)}({f})" for f in fl]
    print(f"  modifier {hex(mod) if isinstance(mod,int) else mod}: {', '.join(names)}")
print()
AR24=0x34325241; XR24=0x34325258
for e in (data or []):
    if e.get("modifier")==0:
        fl=e.get("formats",[])
        print(f"  LINEAR(0x0) carries: {[fourcc(f) for f in fl]}")
        print(f"  --> AR24 scan-out-able LINEAR? {AR24 in fl}")
        print(f"  --> XR24 scan-out-able LINEAR? {XR24 in fl}")
PY
