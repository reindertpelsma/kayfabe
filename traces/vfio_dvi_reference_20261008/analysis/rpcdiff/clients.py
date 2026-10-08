import re, sys, collections
# clients.py SEQ -- per client: first/last time, request index range, engine/channel classes allocated (in order)
INT = {'9067': 'CTXSHARE', 'c56f': 'GPFIFO', 'c997': '3D', 'c9c0': 'COMPUTE', 'c7b5': 'CE', 'a06c': 'TSG',
       '0079': 'EVT0079', '0005': 'EVENT', '5080': 'DEFAPI', '90f1': 'VASPACE', 'c372': 'DISP_SW', 'c77d': 'CORE',
       'c77e': 'WIN', 'c67e': 'WIN?', 'c37a': 'CURS', 'c37b': 'WINIMM', '0073': 'DISP_COMMON', 'c770': 'DISP'}
rows = collections.OrderedDict()
for line in open(sys.argv[1]):
    m = re.search(r'^(\S+) [RK](\d+)\s+\S+\s+alloc 0x([0-9a-f]{4}) reply=(\S+) client=(0x[0-9a-f]+)', line)
    if not m:
        continue
    t, i, cls, rep, cl = m.groups()
    if cls not in INT:
        continue
    r = rows.setdefault(cl, {'t0': t, 'i0': i, 'cls': []})
    r['t1'], r['i1'] = t, i
    r['cls'].append(INT[cls] + ('' if rep == '0x0' else '!' + rep))
for cl, r in rows.items():
    if any(c.startswith(('CTXSHARE', 'GPFIFO', '3D', 'CE', 'TSG')) for c in r['cls']):
        print(cl, r['t0'], r['i0'], '..', r['i1'], ' '.join(r['cls']))
