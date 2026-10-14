"""align.py HW_GSP_JSONL HW_QEMU_TRACE KF_LOG KF_REPLIES OUTDIR

Ordered alignment of the kayfabe-visible streams: real GPU (VFIO, x-gsp-observer + vfio_region_* trace)
vs a kf3 run log (KF3_RPC_TRACE rpc-trace lines + KF3_DISPLAY_WRITE_TRACE WTRACE lines, in log order).

Items:  C cmd obj        GSP_RM_CONTROL (fn 76)   attrs: status (hw: reply; kf: result=)
        A cls parent obj GSP_RM_ALLOC (fn 103)   attrs: status
        F                FREE (fn 10)             (kf logs no handles for a free)
        O fn             any other guest->GSP function
        W addr           display-range BAR0 write (0x610000-0x6fffff)  attrs: value
Hardware-only (no per-item kf counterpart in any kf trace): E (GSP->guest events), R (display reads),
MSI vectors.  Alignment: difflib on the item keys (handles kept: the guest picks them on both sides).
Window: from the first display-class RPC to the hardware's first window-0 PUT (WRITE 0x690000) after
the modeset, and on kf to the matching point (see main)."""
import collections, difflib, gzip, json, re, struct, sys

OFF = 1791227537.632998
DISP_CLS = {0x0073, 0x5070, 0xc370, 0xc372, 0xc77d, 0xc67e, 0xc67b, 0xc67a, 0xc770, 0xc771, 0xc77f, 0xc37a, 0xc37b, 0xc37d, 0xc37e}


def disp_cmd(c):
    cls = c >> 16
    return cls in (0x0073, 0x5070, 0xc370, 0xc372) or (cls == 0x2080 and (c >> 8) & 0xff == 0x0a and (c & 0xff) <= 0x0f)


def nobj(o):
    # kf-rm logs object=0 for the CPU-RM's internal controls; the hardware stream carries the 0xabcdXXXX
    # internal handles there -- the same control, normalized
    return 0 if o >> 16 == 0xabcd else o


def op(f):
    return gzip.open(f, 'rt', errors='replace') if f.endswith('.gz') else open(f, errors='replace')


def hw_rpcs(path):
    recs = []
    for line in op(path):
        r = json.loads(line)
        if r.get('kind') != 'record':
            continue
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4:
            continue
        hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
        recs.append(((r['qpc'] / 1e9 + OFF) % 86400, r['direction'], fn, p[i - 4 + 32:]))
    pend = collections.defaultdict(collections.deque)
    out = []
    for t, d, fn, b in recs:
        if d == 0:
            e = dict(t=t, fn=fn)
            if fn == 76:
                hc, ho, cmd = struct.unpack_from('<3I', b, 0)
                psz = struct.unpack_from('<I', b, 16)[0]
                e.update(k=('C', cmd, nobj(ho)), client=hc, req=b[40:40 + psz].hex())
            elif fn == 103:
                hc, hp, ho, cls = struct.unpack_from('<4I', b, 0)
                e.update(k=('A', cls, hp, ho), client=hc)
            elif fn == 10:
                e.update(k=('F',))
            else:
                e.update(k=('O', fn))
            pend[fn].append(e)
            out.append(e)
        elif fn >= 0x1000:
            out.append(dict(t=t, fn=fn, k=('E', fn)))
        elif pend[fn]:
            e = pend[fn].popleft()
            if fn == 76:
                e['status'] = struct.unpack_from('<I', b, 12)[0]
                psz = struct.unpack_from('<I', b, 16)[0]
                e['rep'] = b[40:40 + psz].hex()
            elif fn == 103:
                e['status'] = struct.unpack_from('<I', b, 16)[0]
            else:
                e['status'] = None
    return out


ACC = re.compile(r'T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_region_(read|write)\s+\(0000:01:00.0:region0\+(0x[0-9a-f]+), (?:(\d)\) = (0x[0-9a-f]+)|(0x[0-9a-f]+), \d\))')
MSI = re.compile(r'T(\d\d):(\d\d):(\d\d\.\d+)Z vfio_msi_interrupt\s+\(0000:01:00.0\) vector (\d+)')


def hw_bar0(path, t0, t1):
    out = []
    nondisp = collections.Counter()
    for line in op(path):
        m = ACC.search(line)
        if m:
            t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
            if t < t0:
                continue
            if t > t1:
                break
            a = int(m.group(5), 16)
            v = int(m.group(7) or m.group(8), 16)
            rd = m.group(4) == 'read'
            if not 0x610000 <= a < 0x700000:
                nondisp['R' if rd else 'W', a & ~0xfff] += 1
                continue
            out.append(dict(t=t, k=('R' if rd else 'W', a), v=v))
            continue
        m = MSI.search(line)
        if m:
            t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
            if t0 <= t <= t1:
                out.append(dict(t=t, k=('I', int(m.group(4)))))
    return out, nondisp


RPC = re.compile(r'rpc-trace fn=(\d+) \S+ seq=\d+ (.*)result=(\S+)')
REF = re.compile(r'GSP REFUSED fn(\d+)/(0x[0-9a-f]+)=(0x[0-9a-f]+)')
WT = re.compile(r'WTRACE t=([\d.]+) WRITE (0x[0-9a-f]+) <- (0x[0-9a-f]+) w(\d)(.*)')


def kf_items(path):
    out = []
    for ln, line in enumerate(op(path)):
        m = RPC.search(line)
        if m:
            fn = int(m.group(1))
            kv = dict(x.split('=', 1) for x in m.group(2).split() if '=' in x and not x.startswith('device_facts'))
            st = m.group(3)
            st = None if st == 'none' else int(st, 16)
            if fn == 76:
                k = ('C', int(kv['cmd'], 16), nobj(int(kv['object'], 16)))
            elif fn == 103:
                k = ('A', int(kv['class'], 16), int(kv['parent'], 16), int(kv['handle'], 16))
            elif fn == 10:
                k = ('F',)
            else:
                k = ('O', fn)
            out.append(dict(ln=ln + 1, k=k, status=st, client=kv.get('client')))
            continue
        m = REF.search(line)
        if m:
            for e in reversed(out[-6:]):
                if e['k'][0] in 'CA' and e['status'] is None and (e['k'][0] == 'A' or e['k'][1] == int(m.group(2), 16)):
                    e['status'] = int(m.group(3), 16)
                    e['refused'] = True
                    break
            continue
        m = WT.search(line)
        if m:
            out.append(dict(ln=ln + 1, k=('W', int(m.group(2), 16)), v=int(m.group(3), 16), t=float(m.group(1)), note=m.group(5).strip()))
    return out


def fmt(e):
    k = e['k']
    if k[0] == 'C':
        s = 'ctrl 0x%08x obj=0x%08x' % (k[1], k[2])
    elif k[0] == 'A':
        s = 'alloc cls=0x%04x parent=0x%08x h=0x%08x' % k[1:]
    elif k[0] == 'F':
        s = 'free'
    elif k[0] == 'O':
        s = 'fn %d' % k[1]
    elif k[0] == 'E':
        s = 'event fn 0x%x' % k[1]
    elif k[0] == 'I':
        s = 'MSI vector %d' % k[1]
    else:
        s = '%s 0x%06x = 0x%x' % (k[0], k[1], e['v'])
    if 'status' in e and k[0] in 'CA':
        s += ' st=%s' % ('none' if e['status'] is None else '0x%x' % e['status'])
    return s


def main():
    hwp, trp, kfp, krp, od = sys.argv[1:6]
    rp = hw_rpcs(hwp)
    first = next(e for e in rp if e['k'][0] == 'C' and disp_cmd(e['k'][1]) or e['k'][0] == 'A' and e['k'][1] in DISP_CLS)
    t0 = first['t']
    # end of the hardware window: the first window-0 PUT write after the modeset (boot3 12.596 s)
    acc, nondisp = hw_bar0(trp, t0, t0 + 4.0)
    put0 = next(e for e in acc if e['k'] == ('W', 0x690000) and e['t'] % 60 > 12.0)
    t1 = put0['t']
    hw = [e for e in rp if t0 <= e['t'] <= t1] + [e for e in acc if e['t'] <= t1]
    hw.sort(key=lambda e: e['t'])
    # kf replay replies (code), keyed by hardware request index n -> (who, status, reply)
    nidx = {id(e): n for n, e in enumerate(rp)}
    kfrep = {}
    for l in op(krp):
        f = l.split()
        if len(f) >= 4 and f[1].startswith('0x'):
            kfrep[int(f[0])] = (f[2], int(f[3].split('=')[1], 16), f[4] if len(f) > 4 else '-')
    # n in the replay = index among fn76/103 requests only (bodies.py numbering)
    bn = {}
    n = 0
    for e in rp:
        if e['fn'] in (76, 103) and e['k'][0] in 'CA':
            bn[id(e)] = n
            n += 1
    kf_all = kf_items(kfp)
    kfirst = next(i for i, e in enumerate(kf_all) if e['k'][0] == 'C' and disp_cmd(e['k'][1]) or e['k'][0] == 'A' and e['k'][1] in DISP_CLS)
    # kf end: the hardware window ends at the first window PUT; kf3 never writes one, so the kf window ends at
    # the first item after the post-modeset 0x90f10106 that follows 0x20808159 (the last shared RPC on both)
    seen8159 = None
    kend = len(kf_all)
    mset = next(i for i, e in enumerate(kf_all) if i > kfirst and e['k'] == ('W', 0x680000) and e['v'] == 0)
    for i, e in enumerate(kf_all):
        if i < mset:
            continue
        if e['k'][:2] == ('C', 0x20808159):
            seen8159 = i
        if seen8159 is not None and e['k'][:2] == ('C', 0x90f10106):
            kend = i + 1
            break
    kf = kf_all[kfirst:kend]
    hk = [e for e in hw if e['k'][0] in 'CAFOW']
    sm = difflib.SequenceMatcher(None, [e['k'] for e in hk], [e['k'] for e in kf], autojunk=False)
    pos = {id(e): i for i, e in enumerate(hw)}
    rows = []
    with open(od + '/aligned.txt', 'w') as f:
        f.write('# hw window %.6f..%.6f (s of 20:10, boot3)  hw items %d (aligned kinds %d)  kf log lines %d..%d items %d\n' % (
            t0 % 60, t1 % 60, len(hw), len(hk), kf[0]['ln'], kf[-1]['ln'], len(kf)))
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag == 'equal':
                for a, b in zip(hk[i1:i2], kf[j1:j2]):
                    d = []
                    if a['k'][0] in 'CA' and a.get('status') != b.get('status'):
                        d.append('status hw=%s kf=%s' % (a.get('status'), b.get('status')))
                    if a['k'][0] == 'W' and a['v'] != b['v']:
                        d.append('value hw=0x%x kf=0x%x' % (a['v'], b['v']))
                    if a['k'][0] == 'C' and id(a) in bn and bn[id(a)] in kfrep:
                        who, kst, krep = kfrep[bn[id(a)]]
                        if who != 'none' and a.get('status') == 0 and b.get('status') == 0 and kst == 0 and krep != '-' and a.get('rep') and krep != a['rep']:
                            ha, kb = bytes.fromhex(a['rep']), bytes.fromhex(krep)
                            m = max(len(ha), len(kb))
                            ha, kb = ha.ljust(m + 4, b'\0'), kb.ljust(m + 4, b'\0')
                            w = ['+0x%x hw %08x kf %08x' % (o, struct.unpack_from('<I', ha, o)[0], struct.unpack_from('<I', kb, o)[0])
                                 for o in range(0, m, 4) if ha[o:o + 4] != kb[o:o + 4]]
                            if w:
                                d.append('reply[code replay, %s] %d word(s): %s' % (who, len(w), '; '.join(w[:6])))
                    mark = 'DIFF' if d else '    '
                    f.write('%s h%05d %9.6f %-58s | k%06d %-58s %s\n' % (mark, pos[id(a)], a['t'] % 60, fmt(a), b['ln'], fmt(b), ' / '.join(d)))
                    if d:
                        rows.append(('field', pos[id(a)], a['t'] % 60, fmt(a), b['ln'], fmt(b), ' / '.join(d)))
            else:
                for a in hk[i1:i2]:
                    f.write('HWONLY h%05d %9.6f %-58s | %s\n' % (pos[id(a)], a['t'] % 60, fmt(a), tag))
                    rows.append(('hw-only(%s)' % tag, pos[id(a)], a['t'] % 60, fmt(a), None, '', ''))
                prev = hk[i1 - 1] if i1 > 0 else None
                for b in kf[j1:j2]:
                    f.write('KFONLY  after h%05d %-58s | k%06d %s\n' % (pos[id(prev)] if prev else -1, '', b['ln'], fmt(b)))
                    rows.append(('kf-only(%s)' % tag, pos[id(prev)] if prev else -1, prev['t'] % 60 if prev else 0, '', b['ln'], fmt(b), ''))
    with open(od + '/hw-only-kinds.txt', 'w') as f:
        for e in hw:
            if e['k'][0] in 'ERI':
                f.write('h%05d %9.6f %s\n' % (pos[id(e)], e['t'] % 60, fmt(e)))
        f.write('# non-display trapped BAR0 accesses in the window (4K page: count)\n')
        for (rw, pg), c in sorted(nondisp.items()):
            f.write('# %s 0x%06x %d\n' % (rw, pg, c))
    with open(od + '/diffs.tsv', 'w') as f:
        for r in rows:
            f.write('\t'.join(str(x) for x in r) + '\n')
    print('hw t0=%.6f t1=%.6f hw=%d hk=%d kf=%d (lines %d..%d) ratio=%.3f diffs=%d' % (
        t0 % 60, t1 % 60, len(hw), len(hk), len(kf), kf[0]['ln'], kf[-1]['ln'], sm.ratio(), len(rows)))


# kf3's WTRACE and maplog lines share one clock (kf_mem::maplog::t)
VS = re.compile(r'(?:WTRACE|maplog) t=([\d.]+) ')
# channel PUT registers: their values differ by the driver-start push base (record §0 P17), never a first difference
PUTS = {0x680000} | {0x690000 + 0x1000 * w for w in range(8)} | {0x6b0000 + 0x1000 * w for w in range(8)}


def kf_timed(path):
    """kf items (as kf_items) each stamped with the latest WTRACE time at or before it (VSYNC lines
    every ~16 ms bound the error), plus kf3's own engine progress lines (no hardware counterpart)."""
    out, t = [], None
    items = {e['ln']: e for e in kf_items(path)}
    for ln, line in enumerate(op(path), 1):
        m = VS.search(line)
        if m:
            t = float(m.group(1))
        if ln in items:
            e = items[ln]
            e['t'] = e.get('t', t)
            out.append(e)
        elif 'updates completed' in line or 'scanout REFUSED' in line or ('channel ' in line and 'STOPPED' in line):
            out.append(dict(ln=ln, k=('P',), t=t, note=line.strip()[:160]))
    return out


# per-frame acknowledgements: the guest's EVT_STAT_HEAD_TIMING write-1-to-clear once per VSync, on both
FRAME_ACK = {0x611800}


def post(hwp, trp, kfp, od, secs):
    """--post: from the first window-0 PUT after the modeset, `secs` seconds on the hardware, and on kf3
    up to the guest's display teardown (the first RM_INTR_EN_HEAD_TIMING(0) <- 0 followed by FREE RPCs)
    or `secs`. Ordered alignment (difflib) of C/A/F/O items and display writes, frame acks excluded;
    then per-kind counts per second. kf3 traps no display read and logs no MSI: R/I are hardware-only."""
    rp = hw_rpcs(hwp)
    acc, _ = hw_bar0(trp, 0, 1e9)
    put0 = next(e for e in acc if e['k'] == ('W', 0x690000) and e['t'] % 60 > 12.0)
    t0 = put0['t']
    t1 = t0 + secs
    hw = [e for e in rp if t0 <= e['t'] <= t1] + [e for e in acc if t0 <= e['t'] <= t1]
    hw.sort(key=lambda e: e['t'])
    kall = kf_timed(kfp)
    # kf start: the first window-0 PUT after the modeset's core PUT 0 (the post-modeset window programming)
    mset = next(i for i, e in enumerate(kall) if e['k'] == ('W', 0x680000) and e.get('v') == 0)
    ks = next(i for i, e in enumerate(kall) if i > mset and e['k'] == ('W', 0x690000))
    kt0 = kall[ks]['t']
    ke = len(kall)
    for i in range(ks, len(kall)):
        e = kall[i]
        if e['t'] is not None and e['t'] > kt0 + secs:
            ke = i
            break
        if e['k'] == ('W', 0x611d80) and e.get('v') == 0 and any(x['k'] == ('F',) for x in kall[i + 1:i + 6]):
            ke = i + 1
            break
    kf = kall[ks:ke]
    hk = [e for e in hw if e['k'][0] in 'CAFOW' and not (e['k'][0] == 'W' and e['k'][1] in FRAME_ACK)]
    kk = [e for e in kf if e['k'][0] in 'CAFOW' and not (e['k'][0] == 'W' and e['k'][1] in FRAME_ACK)]
    sm = difflib.SequenceMatcher(None, [e['k'] for e in hk], [e['k'] for e in kk], autojunk=False)
    first = None
    with open(od + '/post-aligned.txt', 'w') as f:
        f.write('# hw %.6f..%.6f s-of-day (boot3, from the first window-0 PUT) items %d; kf lines %d..%d '
                't %.6f..%s items %d; frame acks %s excluded\n' % (
                    t0, t1, len(hk), kf[0]['ln'], kf[-1]['ln'], kt0, kf[-1]['t'], len(kk),
                    ','.join('0x%x' % a for a in FRAME_ACK)))
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag == 'equal':
                for a, b in zip(hk[i1:i2], kk[j1:j2]):
                    d = []
                    if a['k'][0] in 'CA' and a.get('status') != b.get('status'):
                        d.append('status hw=%s kf=%s' % (a.get('status'), b.get('status')))
                    if a['k'][0] == 'W' and a['v'] != b['v']:
                        d.append('value hw=0x%x kf=0x%x' % (a['v'], b['v']))
                    f.write('%s +%8.3f %-58s | k%06d +%8.3f %-58s %s\n' % (
                        'DIFF' if d else '    ', a['t'] - t0, fmt(a), b['ln'], (b['t'] or kt0) - kt0, fmt(b), ' / '.join(d)))
                    if d and first is None and not (a['k'][0] == 'W' and a['k'][1] in PUTS):
                        first = ('field', round(a['t'] - t0, 6), fmt(a), b['ln'], fmt(b), d)
            else:
                for a in hk[i1:i2]:
                    f.write('HWONLY +%8.3f %-58s | %s\n' % (a['t'] - t0, fmt(a), tag))
                for b in kk[j1:j2]:
                    f.write('KFONLY  %9s %-58s | k%06d +%8.3f %s\n' % ('', '', b['ln'], (b['t'] or kt0) - kt0, fmt(b)))
                if first is None:
                    first = (tag, round(hk[i1]['t'] - t0, 6) if i1 < i2 else None, fmt(hk[i1]) if i1 < i2 else '',
                             kk[j1]['ln'] if j1 < j2 else None, fmt(kk[j1]) if j1 < j2 else '', [])

    def key(e):
        k = e['k']
        if k[0] == 'C':
            return 'ctrl 0x%08x' % k[1]
        if k[0] in 'WR':
            return '%s 0x%06x' % (k[0], k[1])
        if k[0] == 'P':
            return 'kf3 engine: ' + re.sub(r'\d+', 'N', e['note'].split('display: ', 1)[-1])[:60]
        return fmt(e).split(' st=')[0]
    hc, kc = collections.defaultdict(collections.Counter), collections.defaultdict(collections.Counter)
    for e in hw:
        hc[key(e)][int(e['t'] - t0)] += 1
    for e in kf:
        kc[key(e)][int((e['t'] or kt0) - kt0)] += 1
    with open(od + '/post-counts.txt', 'w') as f:
        f.write('# kind | hw total | kf total | hw per second 0..%d | kf per second\n' % int(secs))
        for k in sorted(set(hc) | set(kc), key=lambda k: -(sum(hc[k].values()) + sum(kc[k].values()))):
            hs = ' '.join(str(hc[k][s]) for s in range(int(secs) + 1))
            ks_ = ' '.join(str(kc[k][s]) for s in range(int(secs) + 1))
            f.write('%-44s | %6d | %6d | %s | %s\n' % (k, sum(hc[k].values()), sum(kc[k].values()), hs, ks_))
    print('post: hw t0=%.6f items=%d kf t0=%.6f items=%d (lines %d..%d, end t=%s) ratio=%.3f first-difference=%s' % (
        t0, len(hk), kt0, len(kk), kf[0]['ln'], kf[-1]['ln'], kf[-1]['t'], sm.ratio(), first))


if len(sys.argv) > 1 and sys.argv[1] == '--post':
    # align.py --post HW_GSP_JSONL HW_QEMU_TRACE KF_LOG OUTDIR [SECONDS]
    post(*sys.argv[2:6], float(sys.argv[6]) if len(sys.argv) > 6 else 20.0)
else:
    main()
