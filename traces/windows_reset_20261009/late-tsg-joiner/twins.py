import re, sys

rx_facts = re.compile(r'chanlink: CHAN-FACTS (0x[0-9a-f]+):(0x[0-9a-f]+) parent=(0x[0-9a-f]+) class=(0x[0-9a-f]+) engine=(\S+) .*?ctxShare=(0x[0-9a-f]+).*? tsg=(\S+) device')
rx_born = re.compile(r'act birth passthrough: chan (0x[0-9a-f]+):(0x[0-9a-f]+) BORN Passthrough: token (0x[0-9a-f]+) -> host (0x[0-9a-f]+) .*?engine=(0x[0-9a-f]+)')
rx_ctl = re.compile(r'rpc-trace fn=76 RmControl seq=\d+ cmd=(0xa06c0101|0xa06f0103|0xa06f0104|0xa06c0102|0x906f0102|0xa06f0112|0xa06c0105) client=(0x[0-9a-f]+) object=(0x[0-9a-f]+) result=(\S+)')
rx_sched = re.compile(r'kf3: act schedule: (0x[0-9a-f]+):(0x[0-9a-f]+) GPFIFO_SCHEDULE enable=(\w+) on (\d+) twin')
rx_free = re.compile(r'act free: (0x[0-9a-f]+):(0x[0-9a-f]+): passthrough')
rx_snap = re.compile(r'PT-SNAP tok=(0x[0-9a-f]+) chan (0x[0-9a-f]+):(0x[0-9a-f]+) host=(0x[0-9a-f]+).*? GPGet=(0x[0-9a-f]+) GPPut=(0x[0-9a-f]+)')
facts = {}
ev = {}
mode = sys.argv[2] if len(sys.argv) > 2 else 'all'
for n, l in enumerate(open(sys.argv[1], errors='replace'), 1):
    if 'CHAN-FACTS' in l:
        m = rx_facts.search(l)
        if m:
            facts[(m[1], m[2])] = (m[3], m[6], m[7])
        continue
    m = rx_born.search(l)
    if m:
        c, h = m[1], m[2]
        f = facts.get((c, h), ('?', '?', '?'))
        ev.setdefault(c, []).append((n, 'BORN', h, f"tok={m[3]} host={m[4]} eng={m[5]} parent={f[0]} ctxshare={f[1]} tsg={f[2]}"))
        continue
    m = rx_ctl.search(l)
    if m:
        ev.setdefault(m[2], []).append((n, 'GUEST-CTL', m[3], f"cmd={m[1]} result={m[4]}"))
        continue
    m = rx_sched.search(l)
    if m:
        ev.setdefault(m[1], []).append((n, 'AUTHORED-SCHED', m[2], f"enable={m[3]} twins={m[4]}"))
        continue
    m = rx_snap.search(l)
    if m:
        ev.setdefault(m[2], []).append((n, 'PT-SNAP', m[3], f"tok={m[1]} GPGet={m[5]} GPPut={m[6]}"))
        continue
    m = rx_free.search(l)
    if m:
        ev.setdefault(m[1], []).append((n, 'FREE', m[2], ''))
        continue
for c, e in ev.items():
    born = [x for x in e if x[1] == 'BORN']
    sched_seen = False
    late = False
    for x in e:
        if x[1] == 'GUEST-CTL' and 'cmd=0xa06c0101' in x[3] and x[3].endswith('result=0x0'):
            sched_seen = True
        if x[1] == 'BORN' and sched_seen:
            late = True
    if mode == 'late' and not late:
        continue
    if mode == 'multi' and len(born) < 2:
        continue
    print('CLIENT', c)
    for x in e:
        if x[1] in ('BORN', 'GUEST-CTL', 'AUTHORED-SCHED', 'PT-SNAP', 'FREE'):
            print('  ', x[0], x[1], x[2], x[3])
