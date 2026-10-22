"""Owned rental only: export bounded channel state, never an RPC/pushbuffer body."""
from pathlib import Path
import sys,re,json,hashlib
run=sys.argv[1];assert re.fullmatch('[a-z0-9_]{1,40}',run)
vm=Path('/root/s2/common.sh').read_text();m=re.search(r'WINVM_ROOT=(\S+)',vm);assert m
log=Path(m[1])/'kfwin/logs'/f'{run}_b1_qemu.log';out=Path('/root/prov/arms')/run;out.mkdir(parents=True,exist_ok=True)
b=log.read_bytes();assert len(b)<=512*1024*1024
selected=[];counts={}
for n,line in enumerate(b.decode(errors='replace').splitlines()):
 if len(line)>4096 or 'reply-body' in line or 'body=' in line:continue
 kind=None
 if 'PROGRESS_STRAND' in line:kind='strand_diagnostic'
 elif 'PROBE-DUMP' in line:kind='translated_probe'
 elif 'PT-SNAP' in line and ('USERD' in line or 'engine=' in line):kind='passthrough_header'
 elif 'kf3: chan token' in line and any(k in line for k in ['forwarded=', 'dead=', 'FENCE-AT-RELEASE']):kind='channel_census'
 elif 'kf3' in line and any(k in line for k in ['NONSTALL', 'HOST-EVENT', 'host event', 'engine=', 'REFUSED', 'GRmethods=', 'GUEST_PANICKED']):kind='engine_or_refusal'
 if kind:
  counts[kind]=counts.get(kind,0)+1
  if len(selected)<8000:selected.append(f'{n}\t{kind}\t{line.rstrip()}')
(out/'channel_state.tsv').write_text('\n'.join(selected)+'\n')
(out/'state_receipt.json').write_text(json.dumps({'qemu_log_sha256':hashlib.sha256(b).hexdigest(),'source_bytes':len(b),'exported_lines':len(selected),'matching_lines':counts,'max_exported_lines':8000,'limit':'Existing probes are conditional/try-lock snapshots, not a complete timeout-time census. Raw command/RPC bodies stay on rental.'},indent=2)+'\n')
