import json,sys
from pathlib import Path
N=sys.argv[1];R=Path("/data/claude-spec-20261024/lane")
d=json.loads((R/"arms"/N/"result.json").read_text());s=d.get("sample",{})
print("RUN",N,d["verdict"],d["reason"],d.get("exception"),"restored",d["restored"],"hoststop",d.get("host_stop",{}).get("reason"),"xids",d.get("host_stop",{}).get("new_host_xids"))
print("subs",[(x[1],x[2]) for x in s.get("subtest_results",[])],"cur",s.get("subtest"))
print("events",len(d.get("events") or []),len(d.get("final_events") or []),"counters",s.get("counters"),"budget",s.get("budget_refusals"),"exits",s.get("viewperf_nonzero_exits"),"ce",s.get("ce_class_refusal"),"start",d.get("ended"))
e=R/"arms"/N/"export.json"
if e.exists():
  x=json.loads(e.read_text(encoding="utf-8-sig"));print("scores",[(y["workload"],y["composite"]) for y in x["scores"]],"errors",x["errors"])
  print("\n".join(l for l in x["table"].splitlines()[1:]))
