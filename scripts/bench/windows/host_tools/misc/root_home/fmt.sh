#!/bin/bash
echo "== plane IN_FORMATS as advertised by nvidia-drm =="
sudo drm_info -j 2>/dev/null > /tmp/drm.json || sudo drm_info > /tmp/drm.txt 2>/dev/null
if [ -s /tmp/drm.json ]; then
  python3 - <<'PY'
import json
d=json.load(open('/tmp/drm.json'))
for node,v in d.items():
    for p in v.get('planes',[]):
        print(f"  plane {p.get('id')} type={p.get('type',{}).get('value','?')}")
        fmts=p.get('formats',[])
        print(f"    formats: {' '.join(str(f) for f in fmts[:12])}")
        im=p.get('properties',{}).get('IN_FORMATS',{})
        data=im.get('data',{}) if isinstance(im,dict) else {}
        for e in data.get('formats',[]):
            name=e.get('format') if isinstance(e,dict) else e
            mods=e.get('modifiers',[]) if isinstance(e,dict) else []
            ms=[]
            for m in mods:
                mv=m.get('modifier') if isinstance(m,dict) else m
                ms.append(hex(mv) if isinstance(mv,int) else str(mv))
            print(f"    {name}: {', '.join(ms) if ms else '(none)'}")
PY
else
  grep -iE "plane|format|modifier|AR24|XR24|LINEAR" /tmp/drm.txt 2>/dev/null | head -40
fi
echo "== decode: is AR24 offered with LINEAR(0x0)? =="
python3 - <<'PY'
try:
    import json;d=json.load(open('/tmp/drm.json'))
    hit=False
    for node,v in d.items():
        for p in v.get('planes',[]):
            im=p.get('properties',{}).get('IN_FORMATS',{})
            data=im.get('data',{}) if isinstance(im,dict) else {}
            for e in data.get('formats',[]):
                nm=str(e.get('format') if isinstance(e,dict) else e)
                mods=[ (m.get('modifier') if isinstance(m,dict) else m) for m in (e.get('modifiers',[]) if isinstance(e,dict) else []) ]
                if 'AR24' in nm or '875713089' in nm:
                    print(f"  AR24 modifiers: {[hex(m) if isinstance(m,int) else m for m in mods]}")
                    print(f"  LINEAR(0x0) present for AR24: {0 in mods}")
                    hit=True
    if not hit: print("  (AR24 not found in any IN_FORMATS)")
except Exception as ex: print("  parse failed:",ex)
PY
