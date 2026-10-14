#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read the source-defined 4KiB display capabilities page via a local QMP socket.

Offline research observation, not a product table. The fixed page is defined by
NV_PDISP_FE_SW in OGKM and kf-disp/caps.rs. Never reads an arbitrary BAR range.
"""
import argparse
import hashlib
import json
from pathlib import Path
import qmp


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('work', type=Path)
    a = p.parse_args()
    work = a.work.resolve()
    out = work/'display-caps.bin'
    if out.exists():
        raise RuntimeError('Refuse to overwrite a prior observation')
    s, f = qmp.qmp_open(str(work/'qmp.sock'))
    try:
        result = qmp.qmp_cmd(f, 'query-pci')
        if 'error' in result:
            raise RuntimeError(result['error'])
        devices = [dev for bus in result['return'] for dev in bus['devices']
                   if dev['slot'] == 6 and dev['function'] == 0]
        if len(devices) != 1 or devices[0]['id']['vendor'] != 0x10de:
            raise RuntimeError('Expected one NVIDIA GPU in PCI slot 6.0')
        dev = devices[0]
        bars = [r for r in dev['regions'] if r.get('bar') == 0 and r['type'] == 'memory']
        if len(bars) == 1 and bars[0]['address'] == -1:
            meta = dict(schema=1, available=False, pci=dev,
                        note='BAR0 decoding disabled at sampling time; no page read was attempted')
            (work/'display-caps-unavailable.json').write_text(json.dumps(meta, indent=2)+'\n')
            print(json.dumps({'available': False, 'reason': meta['note']}))
            return
        if len(bars) != 1 or bars[0]['address'] <= 0 or bars[0]['size'] < 0x641000:
            raise RuntimeError('Display capability page is outside assigned BAR0')
        address = bars[0]['address']+0x640000
        result = qmp.qmp_cmd(f, 'pmemsave', {'val': address, 'size': 4096, 'filename': str(out)})
        if 'error' in result:
            raise RuntimeError(result['error'])
        data = out.read_bytes()
        if len(data) != 4096:
            raise RuntimeError('Incomplete capability snapshot')
        meta = dict(schema=1, pci=dev, bar0_offset='0x640000', bytes=len(data),
                    sha256=hashlib.sha256(data).hexdigest(),
                    note='One snapshot after startup; no claim of observing every guest read')
        (work/'display-caps.json').write_text(json.dumps(meta, indent=2)+'\n')
        print(json.dumps({'available': True, 'sha256': meta['sha256'], 'bytes': len(data)}))
    finally:
        s.close()


if __name__ == '__main__':
    main()
