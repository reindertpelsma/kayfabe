#!/usr/bin/env python3
"""List GSP RPCs of a VFIO observer trace touching DISABLE_CHANNELS (0x2080110b) and POST_EVENT 139, in order."""
import json, sys
recs = []
for ln in open(sys.argv[1]):
    try:
        d = json.loads(ln)
    except Exception:
        continue
    if d.get("kind") != "record":
        continue
    recs.append(d)
t0 = recs[0]["qpc"]
for d in recs:
    h = d["payload_hex"]
    fn = d["rpc_function"]
    tag = None
    if "0b118020" in h:
        tag = "DISABLE_CHANNELS"
        # bDisable is the first byte of params; find params after cmd
        i = h.find("0b118020")
    elif fn == 4099 or fn == 0x1004:
        tag = "POST_EVENT?"
    if fn in (4099, 4100) or tag:
        print(f"{(d['qpc']-t0)/1e9:10.6f} dir={d['direction']} fn={fn} seq={d['rpc_sequence']} res={d['rpc_result']:#x} {tag} len={d['payload_bytes']}")
