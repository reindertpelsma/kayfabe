from pathlib import Path
import re
out=Path('traces/windows_matrix_20261011/gravity-vulkan/user-worker/refusal-audit');out.mkdir(exist_ok=True)
rows=[]
for file in ['userpump.rs','userrelay.rs']:
 p=Path('crates/kf-qemu/src')/file
 for n,line in enumerate(p.read_text().split('#[cfg(test)]')[0].splitlines(),1):
  for name in re.findall(r'"(USER_[A-Z0-9_]+)',line):
   if name=='USER_RING_SUBDEVICE_ID': cls='e'; why='zero rejection is unsupported by public ordinary RM callers; effective zero mask unresolved'
   elif name=='USER_RING_SUBDEVICE_ID_UNMEASURED': cls='b';why='scanner cannot interpret filtered software calls without a matching native configuration'
   elif name in ['USER_RING_SCAN_ALLOC','USER_RING_NATIVE_ATTEMPT_ALLOCATION','USER_RING_NATIVE_ATTEMPT_BUDGET','USER_RING_SW_NUMBER_EXHAUSTED']: cls='d';why='trusted allocation or count budget'
   elif name=='USER_RING_SOURCE_SEQUENCE': cls='c,d';why='host generation cannot wrap into a live source receipt'
   elif name=='USER_RING_CACHE': cls='a,d';why='bounded owned CPU source-view cache setup; propagate typed cache failure'
   elif name in ['USER_RING_GET_WRITE','USER_RING_PUT_READ','USER_RING_MIGRATION_RING','USER_RING_MIGRATION_ADMISSION']: cls='a,c';why='owned live source/USERD access and mapping admission; no stale cursor or reuse authority'
   elif any(t in name for t in ['UNREGISTERED_API','REGISTRY','SOFTWARE_OBJECT','SW_ADMISSION','SW_ALLOCATION','5080','DEFERRED','CONTEXT_FOREIGN','CONTEXT_ACT','FIRST_SOFTWARE','LATE_SOFTWARE']): cls='a,b';why='only channel-owned registered software actions may authorize host CPU/RM work; supported admission/dispatch subset'
   elif name.startswith('USER_RING_SCAN') or any(t in name for t in ['PREFIX_BOUND','SUFFIX_BOUND','PARTIAL_SUFFIX','WORD']): cls='a,b,c';why='unambiguous software trigger framing and immutable segment split/order; CPU bounds remain mandatory'
   elif name=='USER_RING_GP_FORMAT': cls='b,c,d';why='direct/NOP framing for trigger inspection and native fetch attribution; byte budget is resource policy (enum table separates cases)'
   elif any(t in name for t in ['SOURCE_DECLARATION','MIGRATION_DECLARATION']): cls='a,b,d';why='owned memory/channel plus public alignment/entries/length bounds; supported migration shape'
   elif any(t in name for t in ['SOURCE','PB_','GP_ADMISSION','WHOLE_RING','USERD','TOPOLOGY','NO_VASPACE']): cls='a,c';why='bounded owned CPU source/USERD access and live mapping/RAM admission before publication; no stale reuse credit'
   elif any(t in name for t in ['POISONED','GONE','ORPHANED','COLLISION','UNCERTAIN','ABSENT','STOPPED','BUSY','WITHDRAW','TOKEN','QUOTA']): cls='a,c';why='exact native/allocation lifetime or inconsistent/uncertain publication; retain resources rather than forge success'
   elif name=='USER_RING_WAIT_VA_STORAGE': cls='c';why='pending VA-manager ownership; retry is not completion'
   elif any(t in name for t in ['CURSOR','PUT_BOUND','GP_ADDRESS','SNAPSHOT_SIZE','PB_SIZE']): cls='a,c,d';why='bounded CPU arithmetic/index and source/native cursor attribution'
   else: cls='e';why='needs an individual invariant proof before relaxing'
   rows.append((str(p),n,name,cls,why))
with (out/'runtime-labels.tsv').open('w') as f:
 f.write('source\tline\tname\tcategory\tinvariant\n')
 for row in rows:f.write('\t'.join(map(str,row))+'\n')
print('AUDITED_LABEL_OCCURRENCES',len(rows),'UNRESOLVED_LABELS',sorted(set(x[2] for x in rows if x[3]=='e')))
