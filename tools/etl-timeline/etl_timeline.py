#!/usr/bin/env python3
"""Offline reader for a DxgKrnl ETW trace (.etl) captured from a Windows guest.

Pure stdlib, read-only, no network. Treat the input files as untrusted data: nothing from them is
executed, imported or eval'd. Run it as `python3 -I etl_timeline.py ...`.

Sub-commands (see README.md):
  extract-wevt <dxgkrnl.sys> <out.wevt>   pull the WEVT_TEMPLATE resource out of a PE file
  summary <etl> <wevt>                    time span, buffer count, event-id histogram
  stall <etl> <wevt> [--ens3 FILE]        the engine-timeout report: node, outstanding DMA packets,
                                          scheduler tail, optional host apply-walk cadence match

Field names and types come from the manifest in the supplied dxgkrnl.sys (the same thing
tracerpt/wevtutil use). Event labels below are the descriptor names that Microsoft's PUBLIC symbols
give to the event descriptors; they are a convenience, never an authority.
"""
import argparse, collections, re, struct, sys, uuid

PROVIDER = uuid.UUID('802EC45A-1E99-4B83-9920-87C98277BA9D')   # Microsoft-Windows-DxgKrnl

# (id, version-agnostic) -> label. From public-symbol descriptor names (EventDmaSubmit etc.).
LABEL = {
    18: 'EnterWorkerThread', 19: 'ExitWorkerThread', 20: 'UpdateContextStatus', 22: 'AttemptPreemption',
    113: 'UpdateContextRunningTime', 165: 'ExternalEvent', 175: 'DmaSubmit', 176: 'DmaDpcComplete',
    177: 'DmaIsrComplete', 178: 'QueueSubmit', 179: 'QueueSelect', 180: 'QueueComplete',
    262: 'CalibrateGpuClock', 322: 'PagingQueueSubmit', 324: 'PagingQueueSelect', 325: 'PagingQueueComplete',
    436: 'SelectContext2', 450: 'DmaReleaseToGpu', 451: 'DmaCompleteByGpu', 467: 'VidSchMarkDeviceAsError',
    540: 'TdrPayloadEngineTimeout', 541: 'TdrPayloadVSyncTimeout', 547: 'TdrCaptureDumpStart',
    548: 'TdrCaptureDumpFinish',
}

# ------------------------------------------------------------------ PE resource (WEVT_TEMPLATE)

def extract_wevt(path):
    d = open(path, 'rb').read()
    pe = struct.unpack_from('<I', d, 0x3c)[0]
    if d[pe:pe + 4] != b'PE\0\0':
        raise SystemExit('not a PE file')
    nsec, = struct.unpack_from('<H', d, pe + 6)
    optsz, = struct.unpack_from('<H', d, pe + 20)
    opt = pe + 24
    magic, = struct.unpack_from('<H', d, opt)
    ddoff = opt + (112 if magic == 0x20b else 96)
    rsrc_rva, _ = struct.unpack_from('<II', d, ddoff + 2 * 8)
    secs = []
    for i in range(nsec):
        o = opt + optsz + i * 40
        vsz, va, rsz, rptr = struct.unpack_from('<IIII', d, o + 8)
        secs.append((va, max(vsz, rsz), rptr))

    def off(rva):
        for va, sz, ptr in secs:
            if va <= rva < va + sz:
                return rva - va + ptr
        raise SystemExit('bad rva')

    base = off(rsrc_rva)

    def entries(dir_off):
        nn, ni = struct.unpack_from('<HH', d, dir_off + 12)
        for i in range(nn + ni):
            nm, tgt = struct.unpack_from('<II', d, dir_off + 16 + i * 8)
            yield nm, tgt

    def name(nm):
        if not nm & 0x80000000:
            return nm
        o = base + (nm & 0x7fffffff)
        n, = struct.unpack_from('<H', d, o)
        return d[o + 2:o + 2 + 2 * n].decode('utf-16le')

    for nm, tgt in entries(base):
        if name(nm) != 'WEVT_TEMPLATE':
            continue
        sub = base + (tgt & 0x7fffffff)
        for _, t2 in entries(sub):
            sub2 = base + (t2 & 0x7fffffff)
            for _, t3 in entries(sub2):
                rva, size = struct.unpack_from('<II', d, base + t3)
                return d[off(rva):off(rva) + size]
    raise SystemExit('no WEVT_TEMPLATE resource')

# ------------------------------------------------------------------ manifest (CRIM) + payload decode

def u16(d, o): return struct.unpack_from('<H', d, o)[0]
def u32(d, o): return struct.unpack_from('<I', d, o)[0]
def u64(d, o): return struct.unpack_from('<Q', d, o)[0]

def name_at(d, o):
    n = u32(d, o)
    return d[o + 4:o + n].decode('utf-16le', 'replace').rstrip('\0')

def load_manifest(blob):
    """-> {(event id, version): [(field name, in-type, flags, count, length-field)]}"""
    if blob[:4] != b'CRIM':
        raise SystemExit('not a CRIM manifest')
    prov = None
    for i in range(u32(blob, 12)):
        if uuid.UUID(bytes_le=blob[16 + i * 20:32 + i * 20]) == PROVIDER:
            prov = u32(blob, 32 + i * 20)
    if prov is None:
        raise SystemExit('DxgKrnl provider not in manifest')
    nd = u32(blob, prov + 12)
    events, templates = [], {}
    for i in range(nd):
        eo = u32(blob, prov + 20 + i * 8)
        sig = blob[eo:eo + 4]
        if sig == b'EVNT':
            for k in range(u32(blob, eo + 8)):
                p = eo + 16 + k * 48
                events.append((u16(blob, p), blob[p + 2], u32(blob, p + 20)))
        elif sig == b'TTBL':
            p = eo + 12
            for _ in range(u32(blob, eo + 8)):
                size, n = u32(blob, p + 4), u32(blob, p + 8)
                items_at = u32(blob, p + 16)
                items = []
                for k in range(n):
                    q = items_at + k * 20
                    items.append((name_at(blob, u32(blob, q + 16)), blob[q + 4], u32(blob, q), u16(blob, q + 12),
                                  u16(blob, q + 14)))
                templates[p] = items
                p += size
    return {(i, v): templates.get(t, []) for i, v, t in events}

_SZ = {3: 1, 4: 1, 5: 2, 6: 2, 7: 4, 8: 4, 9: 8, 10: 8, 11: 4, 12: 8, 13: 4, 15: 16, 17: 8, 20: 4, 21: 8}

def decode(items, ud, ptr=8):
    """Decode a user-data blob. Returns a dict; '_bad' is set when the template did not fit."""
    out, p, vals = {}, 0, []
    try:
        for nm, t, fl, cnt, ln in items:
            n = cnt if fl & 16 else (vals[cnt] if fl & 8 and cnt < len(vals) else 1)
            if t == 1:
                e = p
                while ud[e:e + 2] != b'\0\0':
                    e += 2
                v, p = ud[p:e].decode('utf-16le', 'replace'), e + 2
            elif t == 2:
                e = ud.index(b'\0', p)
                v, p = ud[p:e].decode('latin1'), e + 1
            elif t == 14:
                k = vals[ln] if fl & 4 else ln
                v, p = ud[p:p + k], p + k
            else:
                sz = ptr if t == 16 else _SZ[t]
                sg = t in (3, 5, 7, 9)
                if fl & 24:
                    v = [int.from_bytes(ud[p + i * sz:p + (i + 1) * sz], 'little', signed=sg) for i in range(n)]
                    p += n * sz
                else:
                    v = int.from_bytes(ud[p:p + sz], 'little', signed=sg)
                    p += sz
            vals.append(v)
            out[nm] = v
        if p != len(ud):
            out['_bad'] = 'leftover %d' % (len(ud) - p)
    except Exception as ex:                     # noqa: BLE001 - untrusted input, never raise
        out['_bad'] = str(ex)
    return out

# ------------------------------------------------------------------ ETL reading

def read_etl(path):
    """All DxgKrnl events, sorted by timestamp, plus the logger header facts.
    Event record = EVENT_HEADER (80 bytes) + user data; buffers carry a 0x48 byte WMI_BUFFER_HEADER."""
    data = open(path, 'rb').read()
    bsz = struct.unpack_from('<I', data, 0)[0]
    if bsz not in (0x1000, 0x2000, 0x4000, 0x8000, 0x10000, 0x20000):
        raise SystemExit('unexpected buffer size %#x' % bsz)
    evs, flush_ts = [], 0
    for b in range(len(data) // bsz):
        base = b * bsz
        saved, = struct.unpack_from('<I', data, base + 4)
        flush_ts = max(flush_ts, struct.unpack_from('<Q', data, base + 0x10)[0])
        p, end = base + 0x48, base + min(saved, bsz)
        while p + 16 <= end:
            sz, ht, mk = struct.unpack_from('<HBB', data, p)
            if sz < 8:
                break
            if mk == 0xC0 and ht in (0x12, 0x13) and sz >= 80:
                fl, _, tid, pid, ts = struct.unpack_from('<HHIIQ', data, p + 4)
                if data[p + 24:p + 40] == PROVIDER.bytes_le:
                    eid, ver, ch, lvl, opc, task, kw = struct.unpack_from('<HBBBBHQ', data, p + 40)
                    q = p + 80
                    if fl & 1:
                        while q + 8 <= p + sz:
                            _, _, r2, ds = struct.unpack_from('<HHHH', data, q)
                            q = (q + 8 + ds + 7) & ~7
                            if not r2 & 1:
                                break
                        q = min(q, p + sz)
                    evs.append(dict(ts=ts, id=eid, ver=ver, opc=opc, tid=tid, pid=pid, ud=data[q:p + sz]))
            p += (sz + 7) & ~7
    evs.sort(key=lambda e: e['ts'])
    hdr = {}
    # TRACE_LOGFILE_HEADER lives in the first record: PerfFreq / BootTime / StartTime are FILETIME-ish.
    for off in range(0x58, 0x300, 4):
        v = struct.unpack_from('<q', data, off)[0]
        if 133_000_000_000_000_000 < v < 140_000_000_000_000_000:
            hdr.setdefault('filetimes', []).append((off, v))
    hdr['last_flush_ts'] = flush_ts
    hdr['buffers'] = len(data) // bsz
    return evs, hdr

def filetime(v):
    import datetime
    return datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=v / 10)

# ------------------------------------------------------------------ sub-commands

def cmd_summary(a):
    man = load_manifest(open(a.wevt, 'rb').read())
    evs, hdr = read_etl(a.etl)
    t0, t1 = evs[0]['ts'], evs[-1]['ts']
    print('events %d  buffers %d  span %.3f s (10 MHz QPC ticks %d..%d)' % (len(evs), hdr['buffers'], (t1 - t0) / 1e7, t0, t1))
    print('last buffer flush (≈ session stop) is %.3f s after the last event' % ((hdr['last_flush_ts'] - t1) / 1e7))
    for off, v in hdr.get('filetimes', []):
        print('  header filetime @%#x = %s' % (off, filetime(v)))
    c = collections.Counter((e['id'], e['ver']) for e in evs)
    for (i, v), n in c.most_common(a.top):
        print('%5d.%d %-28s n=%d fields=%s' % (i, v, LABEL.get(i, ''), n, ','.join(x[0] for x in man.get((i, v), []))[:90]))

def cmd_stall(a):
    man = load_manifest(open(a.wevt, 'rb').read())
    evs, hdr = read_etl(a.etl)
    tend = evs[-1]['ts']
    def D(e): return decode(man.get((e['id'], e['ver']), []), e['ud'])
    def rel(e): return (e['ts'] - tend) / 1e7
    timeouts = [e for e in evs if e['id'] == 540]
    if not timeouts:
        print('no TdrPayloadEngineTimeout (event 540) in this trace')
        return
    for to in timeouts:
        d = D(to)
        cut = to['ts']
        print('== TdrPayloadEngineTimeout at rel %.4f s: node %s engine %s  LastHwCompleted %#x  LastHwSubmitted %#x  pending-suspend %s'
              % (rel(to), d.get('NodeOrdinal'), d.get('EngineOrdinal'), d.get('LastHwCompletedFenceId', 0),
                 d.get('LastHwSubmittedFenceId', 0), d.get('NumberOfPendingSuspendRequests')))
        node_ctx = {}
        sub, done = {}, set()
        last_done = {}
        for e in evs:
            if e['ts'] > cut:
                break
            if e['id'] == 436:
                x = D(e); node_ctx[x['hContext']] = x['NodeOrdinal']
            elif e['id'] == 175:
                x = D(e); sub[(x['hContext'], x['uliSubmissionId'])] = (e['ts'], x)
            elif e['id'] == 176:
                x = D(e); done.add((x['hContext'], x['uliCompletionId'])); last_done[x['hContext']] = e['ts']
        print('   DMA packets submitted (DmaSubmit) but not completed at the timeout, by context:')
        for k, (ts, x) in sorted(sub.items(), key=lambda kv: kv[1][0]):
            if k in done:
                continue
            ctx = k[0]
            hw_age = (cut - last_done.get(ctx, ts)) / 1e7
            print('   ctx %#x node %s fence %#x type %d seq %#x submitted rel %.3f (age %.3f s; %.3f s since that context last completed)'
                  % (ctx, node_ctx.get(ctx, '?'), k[1], x['PacketType'], x['ulQueueSubmitSequence'], rel(dict(ts=ts)),
                     (cut - ts) / 1e7, hw_age))
        pre = collections.Counter()
        for e in evs:
            if cut < e['ts'] <= cut + 30_000_000:
                pre[LABEL.get(e['id'], e['id'])] += 1
        print('   events in the 3 s after the timeout:', dict(pre.most_common(10)))
        for e in evs:
            if e['id'] in (547, 548, 165) and e['ts'] >= cut:
                x = D(e)
                if e['id'] == 165 and isinstance(x.get('Buffer'), bytes):
                    x['Buffer'] = x['Buffer'].decode('utf-16le', 'replace')
                print('   rel %.4f  %s %s' % (rel(e), LABEL[e['id']], x))
    if a.ens3:
        host_walk_match(evs, man, a.ens3, tend)

def host_walk_match(evs, man, path, tend):
    """Compare guest DMA-completion cadence with the host apply walks of a kayfabe ENS3 dump."""
    def D(e): return decode(man.get((e['id'], e['ver']), []), e['ud'])
    walks = []
    for l in open(path, errors='replace'):
        m = re.search(r'ENS3 APPLY space=(0x[0-9a-f]+) walk=(\d+) start_ns=(\d+) end_ns=(\S+) finished=(\w+) runs=(\d+) '
                      r'omitted=\d+ wall_ns=(\d+)', l)
        if m and m.group(5) == 'true' and int(m.group(6)) > 10:
            st = int(m.group(3)) / 1e9
            walks.append((int(m.group(2)), st, st + int(m.group(7)) / 1e9, int(m.group(7)) / 1e9))
    comp = sorted((e['ts'] - tend) / 1e7 for e in evs if e['id'] == 176)
    bursts = []
    for t in comp:
        if not bursts or t - bursts[-1] > 0.15:
            bursts.append(t)
    # only the last 40 s of node completions before the stall: that is where the cadence is slow enough
    # (one completion per walk) for the interval-to-duration match to be meaningful
    lastc = max((b for b in bursts if b < -5.0), default=bursts[-1])
    ints = [(bursts[i + 1] - bursts[i], bursts[i + 1]) for i in range(len(bursts) - 1)
            if lastc - 40 < bursts[i + 1] <= lastc]
    print('== guest completion cadence vs host apply walks (%d walks, %d intervals in the last 40 s before the stall)'
          % (len(walks), len(ints)))
    best = None
    for s in range(-3, len(walks)):
        pairs = [(iv, t, walks[k + s]) for k, (iv, t) in enumerate(ints) if 0 <= k + s < len(walks)]
        pairs = [p for p in pairs if abs(p[0] - p[2][3]) < 0.003]
        if best is None or len(pairs) > len(best):
            best = pairs
    if not best:
        print('   no interval matches a walk duration')
        return
    offs = [w[2] - t for _, t, w in best]
    hist = collections.Counter(round(o / 0.005) for o in offs)
    mode, cnt = hist.most_common(1)[0]
    print('   %d guest intervals equal a host walk duration to <3 ms; %d of them share one clock offset '
          '(host_end - guest_rel = %.3f s +-5 ms), i.e. one completion per walk, phase-locked' % (len(best), cnt, mode * 0.005))

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest='cmd', required=True)
    x = sp.add_parser('extract-wevt'); x.add_argument('sys'); x.add_argument('out')
    s = sp.add_parser('summary'); s.add_argument('etl'); s.add_argument('wevt'); s.add_argument('--top', type=int, default=25)
    t = sp.add_parser('stall'); t.add_argument('etl'); t.add_argument('wevt'); t.add_argument('--ens3')
    a = ap.parse_args()
    if a.cmd == 'extract-wevt':
        blob = extract_wevt(a.sys)
        open(a.out, 'wb').write(blob)
        print('wrote %s (%d bytes)' % (a.out, len(blob)))
    elif a.cmd == 'summary':
        cmd_summary(a)
    else:
        cmd_stall(a)

if __name__ == '__main__':
    main()
