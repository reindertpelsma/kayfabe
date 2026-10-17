from pathlib import Path
import hashlib,json,re,subprocess
root=Path('/workspace/bench');stem='fast_codex_gb206phase_cmc595_'
per=[];zero=nonzero=0
for path in sorted(root.glob(stem+'*_serial.log')):
 text=path.read_text(errors='replace');samples=[l for l in text.splitlines() if '_kgspIsHeartbeatTimedOut: Heartbeat timed out' in l]
 for line in samples:
  hb=re.search(r' heartbeat (\d+) ',line)
  if hb and int(hb[1])==0:zero+=1
  else:nonzero+=1
 per.append({'arm':path.name[len(stem):-len('_serial.log')],'warnings':len(samples),'samples':samples[:2]})
receipts=[json.loads(p.read_text()) for p in Path('/root/prov/gb206phase-cmc-fast-receipts').glob('*.json')]
xids=[l for l in subprocess.check_output(['dmesg'],text=True).splitlines() if 'NVRM: Xid' in l]
classification=[]
for line in xids:
 m=re.search(r'pid=(\d+)',line);pid=int(m[1]) if m else None
 r=next((r for r in receipts if r['pid']==pid),None)
 arm=next((a for a in ['defer-liveness','missing-page-fault'] if r and '--'+a in ' '.join(r['argv'])),None)
 classification.append({'line':line,'pid':pid,'current_candidate_receipt':r is not None,'intentional_fault_arm':arm})
row={'serial_arms':len(per),'qemu_arms':len(list(root.glob(stem+'*_qemu.log'))),'heartbeat_formula_warnings':sum(p['warnings'] for p in per),'heartbeat_raw_zero_warnings':zero,'heartbeat_raw_nonzero_warnings':nonzero,'per_arm':per,'actual_process_receipts':len(receipts),'host_xids':classification,'host_new_candidate_xids':sum(bool(x['current_candidate_receipt']) for x in classification)}
Path('/root/prov/gb206phase_cmc_heartbeat_census.json').write_text(json.dumps(row,indent=2)+'\n')
print(json.dumps({k:v for k,v in row.items() if k not in ['per_arm','host_xids']}))
print('XID_MAPPING',[(x['pid'],x['intentional_fault_arm']) for x in classification if x['current_candidate_receipt']])
