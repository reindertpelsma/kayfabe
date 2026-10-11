#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vfio_ctrl_window.py -- list the RM control (fn 76), alloc (fn 103) and free (fn 10) RPCs of a VFIO GSP-observer capture
(gsp.jsonl) whose observer timestamp (qpc = host CLOCK_MONOTONIC ns) lies in [T0, T1], with the GSP's reply paired by the RPC
sequence number. Layout: rpc_message_header_v (32 bytes) + rpc_gsp_rm_control_v03_00 (hClient, hObject, cmd, status,
paramsSize, rmapiRpcFlags, rmctrlFlags, rmctrlAccessRight, reserved0[8], params[]) from open ogkm g_rpc-structures.h.
usage: vfio_ctrl_window.py GSP_JSONL T0_MONO_NS T1_MONO_NS [MAXHEX=512]   (prints tab-separated lines; capture output is data)"""
import json, struct, sys
path, t0, t1 = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
maxhex = int(sys.argv[4]) if len(sys.argv) > 4 else 512
FN = {76: 'CTRL', 103: 'ALLOC', 10: 'FREE'}
reqs = {}
rows = []
for line in open(path):
    try:
        r = json.loads(line)
    except ValueError:
        continue   # the live file's last line may be partial
    if r.get('kind') != 'record':
        continue
    q = r['qpc']
    if q < t0 - 2_000_000_000 or q > t1 + 2_000_000_000:
        continue
    p = bytes.fromhex(r['payload_hex'])
    i = p.find(b'VRPC')
    if i < 4:
        continue
    hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
    if fn not in FN:
        continue
    body = p[i - 4 + 32:i - 4 + ln]
    d = r['direction']
    rec = dict(dir=d, q=q, sq=sq, fn=fn, rpc_res=res, body=body, qs=r['queue_sequence'], miss=r.get('missing_before', 0))
    if d == 0:
        reqs[sq] = rec
    rows.append(rec)
for rec in rows:
    if not (t0 <= rec['q'] <= t1):
        continue
    b = rec['body']; fn = rec['fn']
    if fn == 76 and len(b) >= 40:
        hc, ho, cmd, st, psz, f1, f2, f3 = struct.unpack_from('<8I', b, 0)
        extra = 'hClient=%08x hObj=%08x cmd=%08x status=%08x psz=%d params=%s' % (hc, ho, cmd, st, psz, b[40:40 + maxhex].hex())
    elif fn == 103 and len(b) >= 16:
        hc, hp, hn, cl = struct.unpack_from('<4I', b, 0)
        extra = 'hClient=%08x hParent=%08x hNew=%08x class=%08x body=%s' % (hc, hp, hn, cl, b[:64].hex())
    else:
        extra = b[:32].hex()
    print('%s\t%+.6f\tsq=%d\tqs=%d\t%s\trpc_res=%08x\t%s' % ('REQ' if rec['dir'] == 0 else 'REP', (rec['q'] - t0) / 1e9, rec['sq'], rec['qs'], FN[fn], rec['rpc_res'], extra))
