"""Read only fixed, bounded text receipts from our rental; no bodies/dumps/executables."""
from pathlib import Path
import subprocess, sys, re, json, hashlib

run = sys.argv[1]
IID = Path('/workspace/bw3-round-b1-run/own-id').read_text().strip()
assert re.fullmatch('[a-z0-9_]{1,40}', run)
remote = r'''
from pathlib import Path
import json,hashlib,sys
run=sys.argv[1];root=Path('/root/prov');files={}
names=['start_receipt.json','process_receipt.json','pre_enable.txt','enable.txt','diag.txt','diag_after.txt','accept.txt','live_events.txt','cleanup_started.txt','accepted_start.json']
paths=[root/('arm_'+run+'_'+n) for n in names]
paths+=[root/('arm_'+run+'.log'),root/('collect_'+run+'.txt'),root/('collect_'+run+'.log'),root/('public_analysis_'+run+'.txt')]
folder=root/'arms'/run
paths+=[folder/n for n in ['api.tsv','allocations.jsonl','allocations.receipt.json','summary.json','abort_context.txt','kf_line_census.txt','channel_state.tsv','state_receipt.json','rpc_timing.jsonl','rpc_timing.receipt.json','rpc_timing_controller.log','host_kernel.log','keylines.jsonl','xid_policy.json','dump_headers.jsonl','display_driver.json','minidump_sha256.txt','early_code0.txt','init_observer.ready.json','init_observer.log','channel_projection.json','observation_hold.json','observation_permit.json']]
paths+=[root/('wer_times_'+run+'.txt'),root/('dump_hdr_'+run+'.txt')]
paths+=sorted(folder.glob('*_observation_context.json'))
paths+=sorted(folder.glob('*_dmesg_*.txt'))
total=0
for p in paths:
 if not p.exists():continue
 assert p.is_file() and not p.is_symlink(),str(p)
 if p.name=='rpc_timing.jsonl' and p.stat().st_size>4*1024*1024:
  assert p.stat().st_size<=16*1024*1024
  sha=hashlib.sha256(p.read_bytes()).hexdigest()
  note=json.dumps({'complete_stream_exported':False,'source_bytes':p.stat().st_size,
                   'source_sha256':sha,'reason':'bounded public projection exported instead',
                   'windows':'channel_projection.json contains pre-event ordinal windows'})+'\n'
  files['arms/'+run+'/rpc_stream_retained.json']={
   'bytes':len(note.encode()),'sha256':hashlib.sha256(note.encode()).hexdigest(),'text':note}
  continue
 assert p.stat().st_size<=4*1024*1024,str(p)
 b=p.read_bytes();s=b.decode('utf-8');total+=len(b);assert total<=72*1024*1024
 assert not any(ord(c)<32 and c not in '\r\n\t' for c in s),str(p)
 assert 'kf-gsp: reply-body ' not in s and ('BEGIN '+'OPENSSH PRIVATE KEY') not in s,str(p)
 files[str(p.relative_to(root))]={'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest(),'text':s}
print(json.dumps(files))
'''
p = subprocess.run(['timeout','45','ssh','-F','/workspace/bw3-round-b1-run/ssh.conf',Path('/workspace/bw3-round-b1-run/alias').read_text().strip(),'python3 -I - '+run], input=remote, capture_output=True, timeout=50, check=True, text=True)
assert len(p.stdout) <= 100*1024*1024
files = json.loads(p.stdout)
dest = Path('/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/ROUND_B1_1830/arms') / run
dest.mkdir(parents=True, exist_ok=True)
receipt = {}
for name, item in files.items():
    # Map the finite rental-side paths to a flat, reviewable receipt directory.
    target = Path(name).name
    assert re.fullmatch('[a-zA-Z0-9_.-]{1,100}', target)
    assert target not in receipt, target
    raw = item['text'].encode()
    assert len(raw) == item['bytes'] and hashlib.sha256(raw).hexdigest() == item['sha256']
    text = re.sub(r'GPU-[0-9a-fA-F-]+', 'GPU-UUID-REMOVED', item['text'])
    normalized = '\n'.join(line.rstrip() for line in text.splitlines())+'\n' if raw else ''
    (dest/target).write_text(normalized)
    receipt[target] = {'rental_path': '/root/prov/'+name, 'original_bytes': item['bytes'], 'original_sha256':item['sha256'], 'normalization':'trailing whitespace/newline and GPU UUID redaction', 'export_sha256':hashlib.sha256(normalized.encode()).hexdigest()}
(dest/'export_receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({'run':run,'files':len(receipt),'bytes':sum(x['original_bytes'] for x in receipt.values()),'dest':str(dest)}))

from label_evidence import label_exports
label_exports(out if "out" in globals() and isinstance(out,Path) else dest)
