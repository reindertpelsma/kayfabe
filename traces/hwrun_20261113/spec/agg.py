#!/usr/bin/env python3
# agg.py LOG [FROM_LINE [TO_LINE]] : aggregate the per-channel "USER_RING_LIFECYCLE (...) Counts {...}" lines of a kf3 log
import re,sys
log=sys.argv[1]; a=int(sys.argv[2]) if len(sys.argv)>2 else 0; b=int(sys.argv[3]) if len(sys.argv)>3 else 10**12
SUM=["segments_streamed","stream_restarts","stream_publish_retries","refused_scan_budget","refused_staged_span_budget","entries_pointer","entries_scan","entries_window"]
MAX=["stream_parts_max","stream_restarts_max","window_entry_max_bytes","pointer_entry_max_bytes"]
s={k:0 for k in SUM}; m={k:0 for k in MAX}; n=0; extra={"USER_RING_REFUSED":0,"SCAN_BUDGET":0,"USER_RING_SUMMARY":0}
for i,l in enumerate(open(log,errors="replace")):
    if i<a or i>=b: continue
    if "USER_RING_REFUSED" in l: extra["USER_RING_REFUSED"]+=1
    if "USER_RING_SCAN_BUDGET" in l: extra["SCAN_BUDGET"]+=1
    if "USER_RING_LIFECYCLE (" in l and "Counts {" in l:
        n+=1
        for k in SUM+MAX:
            mm=re.search(r"\b%s: (\d+)"%k,l)
            if mm:
                v=int(mm.group(1))
                if k in s: s[k]+=v
                else: m[k]=max(m[k],v)
print("channels_reported=%d"%n,s,m,extra)
