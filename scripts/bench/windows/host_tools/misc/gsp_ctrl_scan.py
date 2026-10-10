#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# usage: gsp_ctrl_scan.py GSP_JSONL PREFIX[,PREFIX...]  -- 2026-10-08: every fn-76 RmControl whose command id starts
# with one of the hex PREFIXes (e.g. 0073,5070,c37) in a VFIO observer capture, in queue order: direction (0 = guest
# request, 1 = GSP reply), queue sequence, command, status, params size, and the first 32 params bytes in hex.
# Same record layout as gsp_info_scan.py. Capture output is data.
import json, struct, sys
path, prefixes = sys.argv[1], [p.lower() for p in sys.argv[2].split(',')]
for line in open(path):
    r = json.loads(line)
    if r.get('kind') != 'record':
        continue
    p = bytes.fromhex(r['payload_hex'])
    i = p.find(b'VRPC')
    if i < 4:
        continue
    hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
    if fn != 76:
        continue
    body = p[i - 4 + 32:]
    if len(body) < 44:
        continue
    hc, ho, cmd, st, psz, fl, rf = struct.unpack_from('<7I', body, 0)
    c = '%08x' % cmd
    if not any(c.startswith(x.rjust(4, '0')[:len(x)]) or c.startswith(x) for x in prefixes):
        continue
    print(r['direction'], r['queue_sequence'], '0x' + c, 'status=%x' % st, 'psz=%d' % psz, body[40:72].hex())
