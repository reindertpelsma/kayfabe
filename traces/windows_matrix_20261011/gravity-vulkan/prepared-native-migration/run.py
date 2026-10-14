#!/usr/bin/env python3
# Trusted host, caller holds global GPU flock -o. No VM, no product change.
import datetime,json,os,re,subprocess,sys,time
from pathlib import Path
binary,out,revision,*profile=sys.argv[1:]
assert profile in ([], ['cursors'])
cursors=profile==['cursors']
out=Path(out);out.mkdir(exist_ok=False)
def now():return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='microseconds')
assert subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode!=0
assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip()=='DMA-FQ'
(out/'START').write_text(now()+'\n')
(out/'source.txt').write_text(revision+'\n')
(out/'binary.sha256').write_bytes(subprocess.check_output(['sha256sum',binary]))
(out/'gpu.txt').write_bytes(subprocess.check_output(['nvidia-smi','--query-gpu=name,driver_version','--format=csv,noheader']))
print('HYPOTHESIS a fully prepared owned replacement GP/USERD can execute through V2 on this host; previous same-config Xid does not establish permission rejection.',flush=True)
print('FALSIFIER native marker2/newGET2 absent, registration refusal, cleanup failure or hostXid. Any negative still has UNKNOWN inner status. ONE VARIABLE per pair: 5080 NOP vs API trigger; matched128 post-tail doorbells. UID pairs are separately attributed.',flush=True)
if cursors:
 print('H-RESET: replacement seeded GET0 selects slot0 then slot1. FALSIFIER slot0 marker0 with slot1 marker2/newGET2. Slot0 marker10 instead falsifies a start-at-slot1 hypothesis. Observation scope: this fixture/driver, not general migration cursor semantics.',flush=True)
rows=[]
for uid in [65534,0]:
 for mode in (['migrate-cursors-nop','migrate-cursors'] if cursors else ['migrate-nop','migrate']):
  label=f'uid{uid}-{mode}';start=now()
  command=([binary,mode] if uid==0 else ['setpriv','--reuid=65534','--regid=65534','--clear-groups',binary,mode])
  with (out/(label+'.log')).open('wb') as log:
   proc=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT)
   try:rc=proc.wait(timeout=30)
   except subprocess.TimeoutExpired:
    proc.kill();proc.wait();rc=124
  # Bounded diagnostic observation only, never a product state-release timeout.
  time.sleep(2)
  end=now()
  j=subprocess.run(['journalctl','-k','--since',start,'--until',end,'--no-pager'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  xids=[line for line in j.stdout.decode(errors='replace').splitlines() if 'NVRM: Xid' in line]
  (out/(label+'-journal-xid.log')).write_text('\n'.join(xids)+'\n')
  s=(out/(label+'.log')).read_text()
  nop=mode.endswith('-nop')
  proof=('DEFERRED_MIGRATION_NOP_CONTROL replacement_marker=0 replacement_get=0' if nop else 'DEFERRED_MIGRATION_NATIVE_EFFECT marker=2 new_get=2')
  cleanup='channel=Ok(Some(())) view=Ok(()) unmap=Ok(()) memory=Ok(())' in s
  match=re.search(r'DEFERRED_MIGRATION_CURSOR_OBSERVATION trigger=(true|false) slot0=(\d+) slot1=(\d+) new_get=(\d+) new_put=2 seeded_get=0 old_get=(\d+)',s)
  observation=({'trigger':match[1]=='true','slot0':int(match[2]),'slot1':int(match[3]),'new_get':int(match[4]),'old_get':int(match[5])} if match else None)
  cursor_ok=(not cursors or (observation is not None and observation['trigger']==(not nop) and (observation['slot0'],observation['slot1'],observation['new_get']) in ([(0,0,0)] if nop else [(0,2,2),(10,2,2)])))
  caller_ok=uid==0 or ('Uid:\t65534\t65534\t65534\t65534' in s and 'CapEff:\t0000000000000000' in s)
  passed=rc==0 and j.returncode==0 and not xids and cleanup and proof in s and cursor_ok and caller_ok
  row=dict(uid=uid,mode=mode,pid=proc.pid,start=start,end=end,rc=rc,journal_rc=j.returncode,xids=len(xids),cleanup=cleanup,observable_proof=proof in s,verdict='PASS' if passed else 'FAIL',inner_status='UNMEASURED',nop_post_kick_hold_s=2)
  if cursors:row['cursor_observation']=observation;row['reset_hypothesis']=('CONTROL' if nop else ('SUPPORTED_FOR_FIXTURE' if passed and observation['slot0']==10 else ('FALSIFIED' if passed else 'UNRESOLVED')))
  rows.append(row);print(json.dumps(row),flush=True)
  (out/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
  health=subprocess.run(['timeout','10','nvidia-smi','--query-gpu=name,driver_version','--format=csv,noheader'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  (out/(label+'-health.log')).write_bytes(health.stdout)
  if rc==124 or health.returncode!=0 or j.returncode!=0:
   (out/'EXIT').write_text('1\n');raise SystemExit(1)
(out/'EXIT').write_text('0\n')
print('COLLECTION_EXIT=0 (per-mode verdicts separate)',flush=True)
