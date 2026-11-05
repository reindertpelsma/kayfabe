"""Public580.65.06 reply projections; no raw headers, pointers or unknown fields."""
from pathlib import Path
import json,re,hashlib,sys,collections

def project(line,schema):
    key=int(re.search(r' selector=(0x[0-9a-f]+)',line)[1],16)
    out={'record':int(re.search(r' record=(\d+)',line)[1]),'message_seq':int(re.search(r' message_seq=(\d+)',line)[1]),'control':hex(key)}
    b=bytes.fromhex(re.search(r' body=([0-9a-f]+)$',line)[1])
    out['posted_payload_sha256']=hashlib.sha256(b).hexdigest()
    out['envelope_result']=re.search(r' rpc_result=(\S+)',line)[1]
    out['envelope_private_result']=re.search(r' rpc_result_private=(\S+)',line)[1]
    spec=schema.get(str(key))
    if len(b)<40 or ' truncated=0 ' not in line or not spec:
        out['omitted']='unknown/truncated header or schema';return out
    word=lambda off:int.from_bytes(b[off:off+4],'little')
    size=word(16)
    if word(8)!=key or word(20)&2 or size!=spec['size'] or len(b)!=40+size:
        out['omitted']='selector/serialized/size guard';return out
    out.update(public_type=spec['type'],body_status=word(12),params_size=size,fields={})
    p=b[40:]
    for name,f in spec['fields'].items():
        start,width=f['offset'],f['bytes']
        assert 0<=start<=size and 0<width<=size-start
        v=p[start:start+width]
        if f['ctype'] in ['NvU64','NvP64','NvHandle'] or re.search('addr|offset|pointer|base|limit|reserved',name,re.I):
            out['fields'][name]={'redacted':True,'nonzero':any(v)}
        elif width in [1,2,4]:out['fields'][name]=int.from_bytes(v,'little',signed=f['ctype']=='NvS32')
        elif name=='capsTbl' and f['ctype']=='NvU8':out['fields'][name]={'public_caps_hex':v.hex()}
        elif name=='edidBuffer' and f['ctype']=='NvU8':
            fsize=spec['fields']['bufferSize'];n=int.from_bytes(p[fsize['offset']:fsize['offset']+4],'little')
            if 128<=n<=len(v) and n%128==0 and all(sum(v[i:i+128])%256==0 for i in range(0,n,128)):
                out['fields'][name]={'public_edid_hex':v[:n].hex(),'unused_tail_omitted':len(v)-n}
            else:out['fields'][name]={'omitted':'invalid/nonstandard EDID projection'}
        else:out['fields'][name]={'omitted':'non-scalar field','bytes':width}
    return out

if __name__=='__main__':
    schema=json.loads(Path(sys.argv[1]).read_text());p=Path(sys.argv[2]);assert p.stat().st_size<=2*1024*1024
    lines=p.read_text().splitlines();assert len(lines)<=512 and all(len(l)<=16384 for l in lines)
    rows=[project(l,schema) for l in lines if l.strip()]
    Path(sys.argv[3]).write_text(''.join(json.dumps(r,sort_keys=True)+'\n' for r in rows))
    print(json.dumps({'records':len(rows),'projected':sum('fields' in r for r in rows),'omitted':dict(collections.Counter(r['omitted'] for r in rows if 'omitted' in r)),'raw_unknown_or_pointer_values_exported':False}))
