import sys, re
path = sys.argv[1]
born = {}   # (client) -> list of (line, handle, token, host)
sched = []  # (line, client, handle)
for n, l in enumerate(open(path, errors='replace')):
    m = re.search(r'act birth passthrough: chan (0x[0-9a-f]+):(0x[0-9a-f]+) BORN Passthrough: token (0x[0-9a-f]+) -> host (0x[0-9a-f]+)', l)
    if m: born.setdefault(m.group(1), []).append((n, m.group(2), m.group(3), m.group(4)))
    m = re.search(r'act schedule: (0x[0-9a-f]+):(0x[0-9a-f]+) GPFIFO_SCHEDULE enable=true on (\d+) twin', l)
    if m: sched.append((n, m.group(1), m.group(2), int(m.group(3))))
    m = re.search(r'kf3: chan (0x[0-9a-f]+):(0x[0-9a-f]+) GPFIFO_SCHEDULE enable=true \(token (0x[0-9a-f]+)', l)
for c, bl in born.items():
    for (n, h, tok, host) in bl:
        later = [s for s in sched if s[1] == c and s[0] > n]
        nxt = later[0] if later else None
        # is there a schedule between this birth and the next birth of the same client? 
        print('%7d client=%s chan=%s tok=%s host=%s  schedule-after=%s' % (n, c, h, tok, host, ('line %d (%d twin)' % (nxt[0], nxt[3])) if nxt else 'NONE'))
