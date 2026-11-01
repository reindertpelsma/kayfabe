STATUS: LIVE, 2026-10-23. MEASURED cell outcomes only; planned revisions do not imply execution.

| Lease / offer die | Host / guest driver | Source actually exercised | Native | Windows | Tier3 PRECOMP |
|---|---|---|---|---|---|
|57273215 / measuredGB206|host595.91.07, Windows580.88 planned|product6e3fd3fd6; original native fixture|gates/raw/CUDA pass; GR14/15 fail|UNRUN|UNRUN|
|57280677 / measuredGB202|host595.91.07, Windows580.88 planned|product6e3fd3fd6; corrected native618f04318|gates9/9+10,USER12,raw30,CUDA4,GR15 pass|UNRUN: official VirtIO download curl7 before VM|UNRUN|
|57285340 / measuredGB205|host595.91.07; signed Windows32.0.15.8088|product6e3fd3fd6, fixture618f04318, guard1ac2b51d5|gates9/9+10,USER12,raw30,CUDA4,GR15 pass|LTSC/check + disabledCode22 preparation pass; enable/Code0/Code43/TDR/hold/apps/B1 UNRUN after first CIM10s timeout|UNRUN|
|2026-10-23 57296911 / candidateGB203 only|no measured driver; planned595.91.07/580.88|none deployed; planned6e3fd3fd6 / readiness8e623d92e / kit a6456796e|UNRUN: provider memory timeout before SSH|UNRUN|UNRUN|

2026-10-23 Tier2 PRECOMP returned0x1f with the kernel non-admin0x640780 denial on the three actually probed595.91.07 dies (GB206/202/205); no physical page was obtained there. The privileged kit has hardware n=0. GPU-free readiness195/195 and measurement host100/100 + tool6/6 do not count as hardware cells. All prior-data exclusions are retained. A future non-GB202 result cannot pass GB202's historical fourth-cursor0x50; that question remains open.
