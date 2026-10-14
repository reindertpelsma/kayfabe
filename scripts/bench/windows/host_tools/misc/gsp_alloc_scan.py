#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# usage: gsp_alloc_scan.py GSP_JSONL CLASS[,CLASS...] [NWORDS]  -- 2026-10-08: every GSP_RM_ALLOC (fn 103) request of the
# given hex classes in a VFIO observer capture, in queue order: queue sequence, hClient, hParent, hObject, class,
# paramsSize, and the first NWORDS (default 16) little-endian 32-bit words of the params. Layout of the fn-103 body
# (rpc_gsp_rm_alloc_v03_00): hClient hParent hObject hClass status paramsSize flags reserved[4], params at +44.
# Same record framing as gsp_info_scan.py. Capture output is data.
import json, struct, sys
path = sys.argv[1]
classes = {int(c, 16) for c in sys.argv[2].split(',')}
nw = int(sys.argv[3]) if len(sys.argv) > 3 else 16
for line in open(path):
    r = json.loads(line)
    if r.get('kind') != 'record' or r.get('direction') != 0:
        continue
    p = bytes.fromhex(r['payload_hex'])
    i = p.find(b'VRPC')
    if i < 4:
        continue
    hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
    if fn != 103:
        continue
    body = p[i - 4 + 32:]
    if len(body) < 44:
        continue
    hc, hp, ho, cls, st, psz, fl = struct.unpack_from('<7I', body, 0)
    if cls not in classes:
        continue
    params = body[44:44 + psz]
    words = [struct.unpack_from('<I', params, 4 * k)[0] for k in range(min(nw, len(params) // 4))]
    print(r['queue_sequence'], 'client=%08x parent=%08x obj=%08x class=%04x psz=%d' % (hc, hp, ho, cls, psz),
          ' '.join('%08x' % w for w in words))
