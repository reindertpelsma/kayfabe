import sys, re, collections
path = sys.argv[1]
cur = None
events = []   # (lineno, kind, text)
snap = None
sem_texts = collections.Counter()
def flush():
    global snap
    if snap:
        events.append(snap); snap = None
for n, l in enumerate(open(path, errors='replace')):
    if 'PT-SNAP BEGIN' in l:
        flush()
        m = re.search(r'stall #(\d+): no doorbell to any of (\d+) live Passthrough twin\(s\) for (\d+) ms \(doorbells so far (\d+)\) \(maplog t=([\d.]+)', l)
        if m:
            snap = dict(kind='SNAP', line=n, n=int(m.group(1)), twins=int(m.group(2)), db=int(m.group(4)), t=m.group(5), chans=[], sems=[])
        else:
            m = re.search(r'at free of tok=(\S+) \(maplog t=([\d.]+)', l)
            snap = dict(kind='SNAP', line=n, n='free:'+m.group(1), twins=0, db=0, t=m.group(2), chans=[], sems=[])
    elif snap is not None and 'PT-SNAP tok=' in l:
        m = re.search(r'PT-SNAP tok=(0x[0-9a-f]+) chan (\S+) host=(\S+) engine=(\S+) user_work=(\S+) doorbells=(\d+).*?Put=(0x[0-9a-f]+) Get=(0x[0-9a-f]+).*?GPGet=(0x[0-9a-f]+) GPPut=(0x[0-9a-f]+)(.*)', l)
        if m:
            snap['chans'].append((m.group(1), m.group(4), int(m.group(6)), m.group(7)==m.group(8), m.group(9)==m.group(10), m.group(9), m.group(10), m.group(11)[:60]))
        elif ' SEM ' in l:
            m2 = re.search(r'tok=(0x[0-9a-f]+) SEM (.*?) va=(\S+) payload=(\S+) memory=(\S+) — (.*)', l)
            if m2: snap['sems'].append(m2.groups())
    elif 'PT-SNAP END' in l:
        flush()
    elif 'BORN Passthrough' in l or 'UnloadingGuestDriver seq' in l and 'rpc-trace' in l or 'ChannelFreed' in l or 'birth REFUSED' in l:
        flush()
        events.append(dict(kind='EV', line=n, text=l[:140].strip()))
flush()
for e in events:
    if e['kind'] == 'EV':
        print('%7d EV   %s' % (e['line'], e['text'][:120]))
    else:
        ne = [c for c in e['chans'] if not (c[3] and c[4])]
        bad = collections.Counter(s[5] for s in e['sems'])
        print('%7d SNAP #%s t=%s twins=%d db=%d  chans=%d GPGet!=GPPut:%d Put!=Get:%d  sem verdicts=%s' % (e['line'], e['n'], e['t'], e['twins'], e['db'], len(e['chans']), sum(1 for c in e['chans'] if not c[4]), sum(1 for c in e['chans'] if not c[3]), dict(bad)))
        for c in e['chans']:
            if not (c[3] and c[4]): print('        NONEQ', c)
