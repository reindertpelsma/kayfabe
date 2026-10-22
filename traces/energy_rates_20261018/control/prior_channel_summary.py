"""Summarize existing runtime diagnostics; do not export guest payload or command UUIDs."""
import datetime as dt,json,re,hashlib,sys
from pathlib import Path
R=Path('/data/codex-Q3');n=int(sys.argv[1]);source=Path(sys.argv[2]);P=source/'energy-bisect'/str(n);A=source/'windows-20261022'/f'boundary-kayfabe-{n}';S=R/'prior-public'/str(n);S.mkdir(parents=True,exist_ok=True)
r=json.loads((P/'result.json').read_text());lines=(A/'qemu.log').read_text().splitlines();channels=[];extra=[]
for i,line in enumerate(lines):
 m=re.search(r'chan token (0x[0-9a-f]+) \(host (0x[0-9a-f]+)\) RETIRED, forwarded=(\d+) submissions=(\d+)',line)
 if not m:continue
 token,host,forwarded,submissions=m.groups()
 if host!='0x1002d' and 'dead=Some' not in line:continue
 born=None;fact=None;key=None;rpc=[]
 for j in range(i-1,-1,-1):
  b=re.search(r'act birth translated: chan (0x[0-9a-f]+:0x[0-9a-f]+).*?token '+token+r' -> host '+host+r'\b',lines[j])
  if b:
   born={'line':j+1,'text':lines[j]};key=b[1]
   for k in range(j-1,max(-1,j-80),-1):
    if 'CHAN-FACTS '+key+' ' in lines[k]:
     fact={'line':k+1,'text':lines[k]}
     rpc=[{'line':q+1,'text':lines[q]} for q in range(max(0,k-6),min(len(lines),j+4)) if any(v in lines[q] for v in ['GSP rpc RmAlloc','HELD-REPLY fn=RmAlloc','NV50_DEFERRED_API'])]
     break
   break
 gp=re.search(r'gp: (\d+)',line)
 row={'token':token,'host':host,'forwarded':int(forwarded),'submissions':int(submissions),'gp':int(gp[1]) if gp else None,'NoCeObject':'NoCeObject' in line,'retired':{'line':i+1,'text':line},'birth':born,'alloc_facts':fact,'rpc_context':rpc}
 channels.append(row)
 for item in [fact,born,*rpc,{'line':i+1,'text':line}]:
  if item:extra.append(f"{item['line']}: {item['text']}")
status=json.loads((P/'status-final.json').read_text()) if (P/'status-final.json').exists() else json.loads((P/'status-current.json').read_text())
events=[e for e in status['system'] if e['id']==153];first=min((e['time'] for e in events),default=None)
samples=[json.loads(x) for x in (P/'samples.jsonl').read_text().splitlines()]
first_detection=next((x for x in samples if x['guest153']>0),None)
start=json.loads((P/'boot.json').read_text(encoding='utf-8-sig'))
entry={'classification':'MEASURED','run':n,'revision':r['rev'],'arm':r['arm'],'verdict':r['verdict'],'first153_guest':first,'first153_detected_host':first_detection['time'] if first_detection else None,'guest_captured_final':status['captured'],'host_ended':r['ended'],'boot':start,'guest153':len(events),'guest4101':sum(e['id']==4101 for e in status['system']),'new_host_xids':r.get('host_stop',{}).get('new_host_xids'),'restored':r['restored'],'maxima':{key:max(x['counters'][key] for x in samples) for key in samples[0]['counters']},'channels':channels,'rpc_limit':'Production trace logs RmAlloc seq=0 context and CHAN-FACTS/birth linkage, not a unique full RPC transaction replay.'}
(S/'channels.json').write_text(json.dumps(entry,indent=2)+'\n')
(S/'channel-excerpts.txt').write_text(f'MEASURED: run{n}, rev {r["rev"]}; bounded runtime birth/RPC/retirement excerpts, guest payload omitted.\n'+'\n'.join(dict.fromkeys(extra))+'\n')
print(json.dumps({'run':n,'arm':r['arm'],'verdict':r['verdict'],'first153_guest':first,'first153_detected_host':entry['first153_detected_host'],'channels':[{'host':c['host'],'forwarded':c['forwarded'],'gp':c['gp'],'NoCeObject':c['NoCeObject']} for c in channels]}))
