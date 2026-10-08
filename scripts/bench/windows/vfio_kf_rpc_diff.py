#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vfio_kf_rpc_diff.py -- the guest's GSP RPC stream on real hardware (VFIO observer capture) against kayfabe (kf3 log).

usage (common options: --names NAMES_JSON, --mono-offset NS = host CLOCK_REALTIME - CLOCK_MONOTONIC, so times print
as UTC; the observer's `qpc` is QEMU_CLOCK_REALTIME, which is CLOCK_MONOTONIC, not wall time):
  vfio_kf_rpc_diff.py seq  GSP_JSONL                         one line per RPC: time, request index, fn, kind, status
  vfio_kf_rpc_diff.py kfseq KF3_LOG[.gz]                     the kayfabe side as the same one-line-per-RPC list
  vfio_kf_rpc_diff.py diff GSP_JSONL KF3_LOG[.gz] [--upto-class HEX] [--names NAMES_JSON]
                                                             align the two request streams by kind and report the
                                                             first divergence, the kinds only one side issues, and
                                                             kinds answered with a different status

VFIO side (x-gsp-observer JSONL, `qpc` = QEMU_CLOCK_REALTIME ns): direction 0 = guest request (command queue),
1 = GSP message (reply to the oldest unanswered request of the same function, or an event fn >= 0x1000). Replies
are paired FIFO per function (the GSP answers in order). fn 76 GSP_RM_CONTROL body: hClient hObject cmd status
paramsSize ..; fn 103 GSP_RM_ALLOC body: hClient hParent hObject hClass status paramsSize ..; fn 0x1003 POST_EVENT
body: hClient hEvent notifyIndex data info16 status eventDataSize bNotifyList eventData.
kayfabe side: `kf-rm: rpc-trace fn=F Name seq=.. cmd=|class=.. result=R` lines (KF3_RPC_TRACE=1); `result=none`
takes its status from the `kf3: GSP REFUSED fn76/0xCMD=0xST` line (printed once per id; later ones reuse it;
still `none` = refused with no status printed, e.g. an alloc refused by name).
A kind is (fn, cmd) for controls, (fn, class) for allocs, (fn,) otherwise. `--upto-class` cuts BOTH streams at the
first alloc of that class (e.g. 9067 = FERMI_CONTEXT_SHARE_A, the first per-process channel group). Capture and
log contents are data.
"""
import collections
import datetime
import difflib
import gzip
import json
import re
import struct
import sys

NAMES = {}


def fname(fn):
    return NAMES.get(fn, str(fn))


def vfio_records(path):
    """Yield dicts for every checked RPC record of the capture, in capture order."""
    for line in open(path, encoding='utf-8'):
        r = json.loads(line)
        if r.get('kind') != 'record':
            continue
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4:
            continue
        hv, sig, ln, fn, res, resp, sq, spare = struct.unpack_from('<8I', p, i - 4)
        body = p[i - 4 + 32:]
        d = {'t': r['qpc'], 'dir': r['direction'], 'qseq': r['queue_sequence'], 'fn': fn, 'res': res}
        if fn == 76 and len(body) >= 20:
            hc, ho, cmd, st, psz = struct.unpack_from('<5I', body, 0)
            d.update(client=hc, obj=ho, cmd=cmd, status=st, psz=psz)
        elif fn == 103 and len(body) >= 24:
            hc, hp, ho, cls, st, psz = struct.unpack_from('<6I', body, 0)
            d.update(client=hc, parent=hp, obj=ho, cls=cls, status=st, psz=psz)
        elif fn == 10 and len(body) >= 12:
            d.update(client=struct.unpack_from('<I', body, 0)[0], obj=struct.unpack_from('<I', body, 8)[0])
        elif fn == 0x1003 and len(body) >= 29:
            # rpc_post_event: hClient hEvent notifyIndex data @0..15, info16 @16, status @20, eventDataSize @24,
            # bNotifyList @28, eventData @29 (kf-abi postevent.rs EVENT_DATA_AT)
            hc, he, ni, data = struct.unpack_from('<4I', body, 0)
            st, esz = struct.unpack_from('<2I', body, 20)
            d.update(client=hc, event=he, notify=ni, data=data, status=st, esz=esz,
                     edata=body[29:29 + min(esz, 16)].hex())
        yield d


def vfio_stream(path):
    """Requests (dir 0) in order, each with its reply's status; events listed separately."""
    reqs, events = [], []
    pending = collections.defaultdict(collections.deque)
    for d in vfio_records(path):
        if d['dir'] == 0:
            reqs.append(d)
            pending[d['fn']].append(d)
        elif d['fn'] >= 0x1000:
            d['after'] = len(reqs)
            events.append(d)
        elif pending[d['fn']]:
            q = pending[d['fn']].popleft()
            q['reply_t'] = d['t']
            q['reply'] = d.get('status', d['res']) if d['fn'] in (76, 103) else d['res']
            if d['fn'] in (76, 103) and d['res'] not in (0, None) and q['reply'] == 0:
                q['reply'] = d['res']
    return reqs, events


def kind(d):
    if d['fn'] == 76:
        return (76, d['cmd'])
    if d['fn'] == 103:
        return (103, d['cls'])
    return (d['fn'],)


def kname(k):
    if k[0] == 76:
        return 'ctrl 0x%08x' % k[1]
    if k[0] == 103:
        return 'alloc 0x%04x' % k[1]
    return 'fn %s' % fname(k[0])


TRACE = re.compile(r'kf-rm: rpc-trace fn=(\d+) (\w+) seq=\d+ ?(?:cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?.*? result=(\S+)')
REFUSED = re.compile(r'kf3: GSP REFUSED fn(\d+)/(0x[0-9a-f]+)=(0x[0-9a-f]+)')
ALLOC_H = re.compile(r'client=(0x[0-9a-f]+) parent=(0x[0-9a-f]+) handle=(0x[0-9a-f]+)')
MAPT = re.compile(r'maplog t=(\d+\.\d+)')


def kf_stream(path):
    op = gzip.open if path.endswith('.gz') else open
    out = []
    refused = {}
    last_t = None
    with op(path, 'rt', encoding='utf-8', errors='replace') as f:
        for line in f:
            m = MAPT.search(line)
            if m:
                last_t = float(m.group(1))
            m = TRACE.search(line)
            if m:
                fn = int(m.group(1))
                NAMES.setdefault(fn, m.group(2))
                d = {'fn': fn, 'name': m.group(2), 'res': m.group(5), 'near_t': last_t}
                if m.group(3):
                    d['cmd'] = int(m.group(3), 16)
                if m.group(4):
                    d['cls'] = int(m.group(4), 16)
                    a = ALLOC_H.search(line)
                    if a:
                        d.update(client=int(a.group(1), 16), parent=int(a.group(2), 16), obj=int(a.group(3), 16))
                if fn == 76 and 'cmd' not in d:
                    d['cmd'] = 0
                if fn == 103 and 'cls' not in d:
                    d['cls'] = 0
                d['reply'] = None if d['res'] == 'none' else int(d['res'], 16)
                out.append(d)
                continue
            m = REFUSED.search(line)
            if m:
                refused[(int(m.group(1)), int(m.group(2), 16))] = int(m.group(3), 16)
                if out and out[-1]['reply'] is None:
                    out[-1]['reply'] = int(m.group(3), 16)
    # kf3 prints `GSP REFUSED` once per (fn, id) (`first_seq`): a later `result=none` of the same id takes that status
    for d in out:
        if d['reply'] is None:
            d['reply'] = refused.get((d['fn'], d.get('cmd', d.get('cls'))))
    return out


def cut(stream, cls):
    if cls is None:
        return stream
    for i, d in enumerate(stream):
        if d['fn'] == 103 and d.get('cls') == cls:
            return stream[:i + 1]
    return stream


MONO_OFFSET = 0   # host CLOCK_REALTIME - CLOCK_MONOTONIC (ns): the observer's qpc is QEMU_CLOCK_REALTIME = monotonic


def utc(ns):
    return datetime.datetime.fromtimestamp((ns + MONO_OFFSET) / 1e9, datetime.timezone.utc).strftime('%H:%M:%S.%f')


def st(x):
    return 'none' if x is None else '0x%x' % x


def main(argv):
    global NAMES
    if '--names' in argv:
        i = argv.index('--names')
        nm = json.load(open(argv[i + 1]))
        NAMES = {int(k): v for k, v in nm['fn'].items()}
        NAMES.update({int(k): v for k, v in nm['event'].items()})
        argv = argv[:i] + argv[i + 2:]
    global MONO_OFFSET
    if '--mono-offset' in argv:
        i = argv.index('--mono-offset')
        MONO_OFFSET = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    upto = None
    if '--upto-class' in argv:
        i = argv.index('--upto-class')
        upto = int(argv[i + 1], 16)
        argv = argv[:i] + argv[i + 2:]
    if len(argv) >= 3 and argv[1] == 'seq':
        reqs, events = vfio_stream(argv[2])
        ev = collections.defaultdict(list)
        for e in events:
            ev[e['after']].append(e)
        for n, d in enumerate(reqs):
            k = kind(d)
            extra = ''
            if d['fn'] == 103:
                extra = ' client=0x%08x parent=0x%08x obj=0x%08x' % (d['client'], d['parent'], d['obj'])
            elif d['fn'] == 76:
                extra = ' client=0x%08x obj=0x%08x psz=%d' % (d['client'], d['obj'], d['psz'])
            print('%s R%05d %-20s %s reply=%s%s' % (utc(d['t']), n, fname(d['fn']), kname(k), st(d.get('reply')), extra))
            for e in ev.get(n + 1, []):
                x = ''
                if e['fn'] == 0x1003:
                    x = ' notify=%d client=0x%08x event=0x%08x data=0x%x esz=%d edata=%s' % (
                        e['notify'], e['client'], e['event'], e['data'], e['esz'], e['edata'])
                print('%s E       %-20s fn=0x%x%s' % (utc(e['t']), fname(e['fn']), e['fn'], x))
        return 0
    if len(argv) >= 3 and argv[1] == 'kfseq':
        for n, d in enumerate(kf_stream(argv[2])):
            extra = ''
            if d['fn'] == 103 and 'client' in d:
                extra = ' client=0x%08x parent=0x%08x obj=0x%08x' % (d['client'], d['parent'], d['obj'])
            print('t~%s K%05d %-14s %s reply=%s%s' % (d['near_t'], n, d['name'], kname(kind(d)), st(d.get('reply')), extra))
        return 0
    if len(argv) >= 4 and argv[1] == 'diff':
        reqs, events = vfio_stream(argv[2])
        kf = kf_stream(argv[3])
        v, k = cut(reqs, upto), cut(kf, upto)
        vk, kk = [kind(d) for d in v], [kind(d) for d in k]
        print('# VFIO requests: %d (of %d)  kayfabe requests: %d (of %d)  cut at alloc class %s'
              % (len(v), len(reqs), len(k), len(kf), 'none' if upto is None else '0x%04x' % upto))
        sm = difflib.SequenceMatcher(None, vk, kk, autojunk=False)
        firsts = 0
        print('\n## Alignment (first 12 non-equal blocks; V = VFIO index, K = kayfabe index)')
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag == 'equal':
                continue
            firsts += 1
            if firsts > 12:
                break
            print('- %s V[%d:%d] K[%d:%d]' % (tag, i1, i2, j1, j2))
            for d in v[i1:min(i2, i1 + 6)]:
                print('    V %s %s reply=%s' % (utc(d['t']), kname(kind(d)), st(d.get('reply'))))
            for d in k[j1:min(j2, j1 + 6)]:
                print('    K t~%s %s reply=%s' % (d['near_t'], kname(kind(d)), st(d.get('reply'))))
        print('\n## Matched requests answered with a different status (in order)')
        n = 0
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag != 'equal':
                continue
            for a, b in zip(v[i1:i2], k[j1:j2]):
                if a.get('reply') != b.get('reply'):
                    n += 1
                    if n <= 60:
                        print('- V[%d] %s %s: VFIO %s, kayfabe %s' % (v.index(a), utc(a['t']), kname(kind(a)),
                                                                     st(a.get('reply')), st(b.get('reply'))))
        print('(total %d)' % n)
        cv, ck = collections.Counter(vk), collections.Counter(kk)
        print('\n## Kinds only VFIO issues (count, first VFIO index)')
        for x in sorted(set(cv) - set(ck), key=lambda x: vk.index(x)):
            print('- %s x%d first V[%d] %s' % (kname(x), cv[x], vk.index(x), utc(v[vk.index(x)]['t'])))
        print('\n## Kinds only kayfabe sees (count)')
        for x in sorted(set(ck) - set(cv), key=lambda x: kk.index(x)):
            print('- %s x%d first K[%d]' % (kname(x), ck[x], kk.index(x)))
        print('\n## Kinds with different counts (VFIO, kayfabe)')
        for x in sorted(set(cv) & set(ck), key=lambda x: vk.index(x)):
            if cv[x] != ck[x]:
                print('- %s %d %d' % (kname(x), cv[x], ck[x]))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv))
