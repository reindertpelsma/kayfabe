"""bodies.py GSP_JSONL OUT_JSONL -- every fn76 (GSP_RM_CONTROL) and fn103 (GSP_RM_ALLOC) request of an x-gsp-observer
capture paired (FIFO per function) with its reply, with the request and reply PARAMS bytes.
rpc header 32 bytes (VRPC at +4); rpc_gsp_rm_control_v03_00: hClient hObject cmd status paramsSize rmapiRpcFlags
rmctrlFlags rmctrlAccessRight reserved0(u64) params[] -> params at body+40 (ogkm-595.84 g_rpc-structures.h:1545).
rpc_gsp_rm_alloc_v03_00: hClient hParent hObject hClass status paramsSize flags reserved[4] params[] (checked below).
Times: host UTC seconds of day (qpc monotonic + 1791227537.632998 s, README clock note)."""
import collections, json, struct, sys
OFF = 1791227537.632998
recs = []
for line in open(sys.argv[1]):
    r = json.loads(line)
    if r.get('kind') != 'record':
        continue
    p = bytes.fromhex(r['payload_hex'])
    i = p.find(b'VRPC')
    if i < 4:
        continue
    hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
    body = p[i - 4 + 32:]
    t = (r['qpc'] / 1e9 + OFF) % 86400
    recs.append(dict(t=t, dir=r['direction'], fn=fn, res=res, body=body, ln=ln))
pending = collections.defaultdict(collections.deque)
out = []
for d in recs:
    if d['fn'] not in (76, 103):
        continue
    b = d['body']
    if d['dir'] == 0:
        e = dict(t=d['t'], fn=d['fn'])
        if d['fn'] == 76:
            hc, ho, cmd, st, psz, rf, cf, ar = struct.unpack_from('<8I', b, 0)
            e.update(client=hc, obj=ho, cmd=cmd, psz=psz, rpcflags=rf, ctrlflags=cf, req=b[40:40 + psz].hex())
        else:
            hc, hp, ho, cls, st, psz = struct.unpack_from('<6I', b, 0)
            e.update(client=hc, parent=hp, obj=ho, cls=cls, psz=psz, hdr=b[:32].hex(), req=b[32:32 + psz].hex())
        pending[d['fn']].append(e)
        out.append(e)
    elif pending[d['fn']]:
        e = pending[d['fn']].popleft()
        e['reply_t'] = d['t']
        e['rpc_result'] = d['res']
        if d['fn'] == 76:
            hc, ho, cmd, st, psz = struct.unpack_from('<5I', b, 0)
            e['status'] = st
            e['reply_cmd'] = cmd
            e['rep'] = b[40:40 + psz].hex()
        else:
            hc, hp, ho, cls, st, psz = struct.unpack_from('<6I', b, 0)
            e['status'] = st
            e['rep'] = b[32:32 + psz].hex()
with open(sys.argv[2], 'w') as f:
    for n, e in enumerate(out):
        e['n'] = n
        f.write(json.dumps(e) + '\n')
print(len(out), sum(1 for e in out if 'rep' not in e), 'unpaired', sum(1 for e in out if e.get('reply_cmd', e.get('cmd')) != e.get('cmd')), 'cmd-mismatch')
