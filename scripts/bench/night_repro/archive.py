#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Keep exact compressed capture contents before deleting a redundant guest copy."""
import base64
import gzip
import hashlib
import json
import os
from pathlib import Path
import re


def verify(packet):
    if packet['name']!='pm_tail.bin' and not re.fullmatch(r't_[0-9]{6}\.etl',packet['name']):
        raise ValueError('invalid captured trace name')
    raw=gzip.decompress(base64.b64decode(packet['gzip']))
    if hashlib.sha256(raw).hexdigest()!=packet['sha256']:
        raise ValueError('trace digest mismatch; deletion not authorized')
    return raw


def preserve(out, guest_root, agent):
    names=sorted(set(re.findall(r'-> (t_[0-9]{6}\.etl)',(out/'trig.log').read_text())))
    receipts=[]
    if not names: return receipts
    target=out/'etl'; target.mkdir(exist_ok=True)
    manifest=target/'manifest.json'
    if manifest.exists(): receipts=json.loads(manifest.read_text())
    done={r['name'] for r in receipts if any(f.get('deleted') for f in r.get('delete_receipt',{}).get('files',[]))}
    names=[name for name in names if name not in done]
    for name in names:
        payload=agent(f'& C:\\kf\\n1\\archive.ps1 -Root {guest_root} -Names {name}',90)
        parsed=json.loads(payload)
        for packet in parsed['files']:
            raw=verify(packet)
            dest=target/(packet['name']+'.gz')
            with dest.open('wb') as stream:
                stream.write(base64.b64decode(packet['gzip'])); stream.flush(); os.fsync(stream.fileno())
            directory=os.open(target,os.O_RDONLY)
            try: os.fsync(directory)
            finally: os.close(directory)
            # Read back the durable host file before authorizing a redundant-copy removal.
            if hashlib.sha256(gzip.decompress(dest.read_bytes())).hexdigest()!=packet['sha256']:
                raise ValueError('host trace readback failed')
            receipt={k:v for k,v in packet.items() if k!='gzip'}
            receipt['verified_bytes']=len(raw)
            deletion=agent(f'& C:\\kf\\n1\\archive.ps1 -Root {guest_root} -Names {name} -DeleteHash {packet["sha256"]}',90)
            receipt['delete_receipt']=json.loads(deletion)
            receipts.append(receipt)
            (target/'manifest.json').write_text(json.dumps(receipts,indent=2))
    return receipts


def preserve_pm(out, guest_root, agent):
    """Stop our monitor, preserve its full appended tail, retain the prior prefix."""
    packet=json.loads(agent(f'& C:\\kf\\n1\\pm_tail.ps1 -Root {guest_root}',90))
    raw=verify(packet)
    dest=out/'pm_tail.bin.gz'
    with dest.open('wb') as stream:
        stream.write(base64.b64decode(packet['gzip'])); stream.flush(); os.fsync(stream.fileno())
    directory=os.open(out,os.O_RDONLY)
    try: os.fsync(directory)
    finally: os.close(directory)
    if hashlib.sha256(gzip.decompress(dest.read_bytes())).hexdigest()!=packet['sha256']:
        raise ValueError('PM tail readback failed; no truncate authorized')
    receipt={k:v for k,v in packet.items() if k!='gzip'}
    receipt['verified_bytes']=len(raw)
    receipt['truncate_receipt']=json.loads(agent(f'& C:\\kf\\n1\\pm_tail.ps1 -Root {guest_root} -DeleteHash {packet["sha256"]}',90))
    (out/'pm-tail-manifest.json').write_text(json.dumps(receipt,indent=2))
    # The analysis and immutable compressed transport payload are already saved.
    csv=out/'pm_live.csv'
    if csv.exists():
        original=csv.read_bytes(); compressed=gzip.compress(original,compresslevel=3)
        target=out/'pm_live.csv.gz'; target.write_bytes(compressed)
        if gzip.decompress(target.read_bytes())!=original: raise ValueError('CSV compression mismatch')
        csv.unlink()
    return receipt
