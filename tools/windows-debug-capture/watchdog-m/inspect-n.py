#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Bounded offline inspection of N's later four-entry constructor failure; executes no driver."""
import argparse,hashlib,json,struct,subprocess
from pathlib import Path
SHA='31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c'
RANGES={'prior_window_loop_exit_and_cpu_allocation':(0x16e9755,0x16e97a3),
        'later_four_entry_loop_predicate':(0x16e981d,0x16e986e),
        'later_constructor_call_check_loopback':(0x16e98c7,0x16e9915),
        'later_constructor_failure_assert':(0x16e9962,0x16e9990)}
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--driver',type=Path,required=True);p.add_argument('--disassemble',action='store_true');a=p.parse_args()
with a.driver.open('rb') as f:data=f.read(128*1024*1024+1)
if len(data)>128*1024*1024 or hashlib.sha256(data).hexdigest()!=SHA:raise SystemExit('pinned driver size/hash mismatch')
pe=struct.unpack_from('<I',data,60)[0];opt=pe+24
assert data[pe:pe+4]==b'PE\0\0' and struct.unpack_from('<H',data,opt)[0]==0x20b
base=struct.unpack_from('<Q',data,opt+24)[0];size=struct.unpack_from('<I',data,opt+56)[0]
assert all(0<=start<end<=size and end-start<=256 for start,end in RANGES.values())
print(json.dumps({'driver_sha256':SHA,'ranges':{k:[hex(a),hex(b)] for k,(a,b) in RANGES.items()}},indent=2))
if a.disassemble:
    for name,(start,end) in RANGES.items():
        result=subprocess.run(['objdump','-d','-Mintel',f'--adjust-vma=-{base}',f'--start-address={start}',f'--stop-address={end}',str(a.driver)],check=True,capture_output=True,text=True,timeout=10)
        if len(result.stdout)>65536:raise SystemExit('disassembly exceeds bound')
        print(name+':');print('\n'.join(line for line in result.stdout.splitlines() if 'file format' not in line))
