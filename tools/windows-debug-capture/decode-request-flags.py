#!/usr/bin/env python3
"""Compile public flag definitions and decode observed ALLOC_MEMORY/channel scalars.

Header scanning selects macro names only; C supplies every mask, shift and enum value.
Only our generated C probe is executed. No NVIDIA binary or guest pointer is executed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', type=Path, required=True)
    p.add_argument('--memory-flags', type=lambda s: int(s, 0), required=True)
    p.add_argument('--channel-flags', type=lambda s: int(s, 0), required=True)
    p.add_argument('--device-flags', type=lambda s: int(s, 0), default=0x38)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    if any(not 0 <= value <= 0xffffffff for value in [a.memory_flags, a.channel_flags, a.device_flags]):
        p.error('Flags must fit u32')
    inc = a.source / 'src/common/sdk/nvidia/inc'
    headers = [inc/'nvos.h', inc/'alloc/alloc_channel.h']
    lines = ['#include <stdio.h>', '#include "nvtypes.h"', '#include "nvmisc.h"',
             '#include "nvos.h"', '#include "alloc/alloc_channel.h"', 'int main(void) {']
    for prefix, header in zip(['NVOS02_FLAGS_', 'NVOS04_FLAGS_'], headers):
        source = header.read_text()
        fields = re.findall(r'^#define\s+('+prefix+r'\w+)\s+\d+:\d+\s*$', source, re.M)
        enums = re.findall(r'^#define\s+('+prefix+r'\w+)\s+\(?0x[0-9a-fA-F]+\)?', source, re.M)
        for name in fields:
            lines.append(f'printf("F {name} %llu %u\\n", (unsigned long long)DRF_SHIFTMASK({name}), (unsigned)DRF_SHIFT({name}));')
        for name in enums:
            lines.append(f'printf("E {name} %llu\\n", (unsigned long long){name});')
    for name in re.findall(r'^#define\s+(NV_DEVICE_ALLOCATION_(?:FLAGS|VAMODE)_\w+)\s+\(?0x[0-9a-fA-F]+\)?', headers[0].read_text(), re.M):
        lines.append(f'printf("E {name} %llu\\n", (unsigned long long){name});')
    lines.extend(['return 0;', '}'])
    with tempfile.TemporaryDirectory(prefix='kf-request-flags-') as work:
        c = Path(work)/'probe.c'; exe = Path(work)/'probe'
        c.write_text('\n'.join(lines)+'\n')
        subprocess.run(['cc', '-std=gnu11', '-I'+str(inc), str(c), '-o', str(exe)], check=True)
        raw = subprocess.check_output([str(exe)], text=True)
    constants = {}; fields = []
    for line in raw.splitlines():
        row = line.split()
        if row[0] == 'E': constants[row[1]] = int(row[2])
        else: fields.append((row[1], int(row[2]), int(row[3])))
    decoded = {}
    for name, mask, shift in fields:
        flag = a.memory_flags if name.startswith('NVOS02_') else a.channel_flags
        value = (flag & mask) >> shift
        decoded[name] = dict(mask=hex(mask), shift=shift, value=value,
                            enum_matches=[k for k, v in constants.items() if k.startswith(name+'_') and v == value])
    result = dict(schema='kayfabe-compiled-request-flags/1',
                  source_commit=subprocess.check_output(['git','-C',str(a.source),'rev-parse','HEAD'],text=True).strip(),
                  header_sha256={str(h.relative_to(a.source)):hashlib.sha256(h.read_bytes()).hexdigest() for h in headers},
                  compiler=subprocess.check_output(['cc','--version'],text=True).splitlines()[0],
                  memory_flags=hex(a.memory_flags),channel_flags=hex(a.channel_flags),fields=decoded,
                  device_flags=hex(a.device_flags),
                  device_flag_names=[k for k, v in constants.items() if k.startswith("NV_DEVICE_ALLOCATION_FLAGS_") and v and v & (v-1) == 0 and a.device_flags & v],
                  va_modes={k:v for k,v in constants.items() if k.startswith("NV_DEVICE_ALLOCATION_VAMODE_")},
                  limits=['Scalar bitfield decoding; not permission to forward requests or claim hardware support.'])
    a.out.write_text(json.dumps(result, indent=2)+'\n')


if __name__ == '__main__': main()
