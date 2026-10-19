"""Rental-side text export: API metadata and explicitly public posted display bodies only."""
import collections,csv,hashlib,io,json,re,sys
from pathlib import Path

source=Path(sys.argv[1]);destination=Path(sys.argv[2]);destination.mkdir(exist_ok=True)
assert source.is_file() and source.name.endswith('qemu.log')
assert source.stat().st_size<=64*1024*1024,'bounded export input'
fields=['ordinal','function','kind','rpc_sequence','control_or_class','trace_result','effective_result']
commands=[];posted=[];omitted=0;quota=0;digest=hashlib.sha256();size=0
pattern=re.compile(r'kf-rm: rpc-trace fn=(\d+) (.*?) seq=(\d+) (.*?) result=(\S+)')
with source.open('rb') as incoming:
    for raw in incoming:
        digest.update(raw);size+=len(raw)
        if len(raw)>128*1024:
            continue
        line=raw.decode('utf-8',errors='replace').rstrip('\r\n')
        quota+='rpc-trace lifetime limit reached' in line
        match=pattern.search(line)
        if match:
            assert len(commands)<16384,'metadata quota exceeded'
            fn,kind,sequence,what,result=match.groups()
            selector=re.search(r'(?:cmd|class)=(0x[0-9a-fA-F]+)',what)
            commands.append({'ordinal':len(commands),'function':int(fn),'kind':kind,
                'rpc_sequence':int(sequence),'control_or_class':selector[1].lower() if selector else '-',
                'trace_result':result,'effective_result':'unknown'})
        if 'kf-gsp: reply-body ' in line:
            fn=re.search(r' function=(0x[0-9a-fA-F]+)',line)
            selector=re.search(r' selector=(0x[0-9a-fA-F]+)',line)
            key=int(selector[1],16) if selector else None
            public=fn and int(fn[1],16)==76 and key is not None and (key>>16==0x73 or key==0x20800a58)
            if public:
                assert len(posted)<512,'posted body record quota exceeded'
                posted.append(line)
            else:
                omitted+=1

def tsv(rows):
    out=io.StringIO();writer=csv.DictWriter(out,fields,delimiter='\t',lineterminator='\n')
    writer.writeheader();writer.writerows(rows);return out.getvalue()

full=tsv(commands);prefix=tsv(commands[:1000]);bodies='\n'.join(posted)+'\n'
(destination/'api-full.tsv').write_text(full)
(destination/'api-prefix1000.tsv').write_text(prefix)
(destination/'posted-public-display-bodies.txt').write_text(bodies)
critical={}
for key in [0x73010c,0xc3700201,0x20800a89,0x20800ab2,0x20800ab4,0xcbcb,0x20802801,0x20802806,0xca70,0xca7a]:
    rows=[c for c in commands if c['control_or_class']!='-' and int(c['control_or_class'],16)==key]
    critical[hex(key)]={'count':len(rows),'first_ordinals':[c['ordinal'] for c in rows[:16]],
        'policy_results':dict(collections.Counter(c['trace_result'] for c in rows)),
        'scope':'all captured policy metadata; after the quota notice is unknown'}
summary={'source_log':source.name,'source_bytes':size,'source_sha256':digest.hexdigest(),
    'policy_metadata_commands':len(commands),'prefix_commands_exported':min(len(commands),1000),
    'metadata_quota_notices':quota,'metadata_truncated':bool(quota),
    'public_posted_body_records':len(posted),'opaque_or_alloc_body_records_omitted':omitted,
    'critical':critical,'status_limit':'trace_result is the policy envelope before held actions resolve; effective_result stays unknown. Actual posted display statuses are in the separate screened body log.',
    'body_limit':'Only function76 NV0073 and20800a58 bodies exported. Alloc and Other4 bytes remain on rental.',
    'alignment_limit':'Prefix0..999 is an explicit comparable initialization phase; full screened API metadata is exported, raw bodies stay on rental.'}
(destination/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({'summary':summary}))
