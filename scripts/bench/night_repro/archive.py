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
    if not re.fullmatch(r't_[0-9]{6}\.etl',packet['name']):
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
