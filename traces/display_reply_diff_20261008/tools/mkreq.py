"""mkreq.py BODIES TMAX > requests.txt -- display requests of a hardware capture in display_replay's input format."""
import json, sys
DISP_ALLOC = {0xc77d, 0xc67e, 0xc67b, 0xc67a, 0xc770, 0x0073, 0xc372}
tmax = float(sys.argv[2])
for l in open(sys.argv[1]):
    e = json.loads(l)
    if e['t'] > tmax:
        continue
    if e['fn'] == 76:
        c = e['cmd']
        cls = c >> 16
        if cls in (0x73, 0x5070, 0xc370, 0xc372) or c == 0x20808159 or (cls == 0x2080 and (c >> 8) & 0xff == 0x0a):
            print('ctrl %d 0x%08x 0x%08x 0x%08x %s' % (e['n'], c, e['client'], e['obj'], e['req'] or '-'))
    elif e['cls'] in DISP_ALLOC:
        req = e['req']
        if not req and e['cls'] in (0xc77d, 0xc67e, 0xc67b, 0xc67a):
            # the observer kept only the alloc header: SYNTHESIZED params (channelInstance from the
            # handle's low nibble, everything else 0) so the model registers the channel
            inst = e['obj'] & 0xf if e['cls'] != 0xc77d else 0
            size = 16 if e['cls'] == 0xc67a else 40
            req = (inst.to_bytes(4, 'little') + bytes(size - 4)).hex()
            print('# synthesized params for alloc n=%d' % e['n'])
        print('alloc %d 0x%04x 0x%08x 0x%08x 0x%08x %s' % (e['n'], e['cls'], e['client'], e['parent'], e['obj'], req or '-'))
