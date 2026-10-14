import re, sys
# for each flip (a window-0 PUT run ending in a LATCH), the order of: last PUT, next VSYNC, LATCH, and the
# guest's next RM_INTR_EN_HEAD_TIMING(0) write (0x611d80)
ev = []
for l in open(sys.argv[1], errors='replace'):
    m = re.search(r'WTRACE t=([\d.]+) (VSYNC|LATCH|WRITE (0x[0-9a-f]+) <- (0x[0-9a-f]+))', l)
    if not m:
        continue
    t = float(m.group(1))
    if m.group(2) == 'VSYNC':
        ev.append((t, 'V'))
    elif m.group(2) == 'LATCH':
        ev.append((t, 'L'))
    else:
        a, v = int(m.group(3), 16), int(m.group(4), 16)
        if a == 0x690000:
            ev.append((t, 'P'))
        elif a == 0x611d80:
            ev.append((t, 'E%x' % v))
last_p = None
rows = []
for i, (t, k) in enumerate(ev):
    if k == 'P':
        last_p = t
    if k == 'L' and last_p is not None:
        # the first VSYNC after the last PUT
        v = next((tt for tt, kk in ev if kk == 'V' and tt > last_p), None)
        en = next(((tt, kk) for tt, kk in ev[i + 1:] if kk.startswith('E')), None)
        rows.append((last_p, t, v, en))
        last_p = None
for p, l, v, en in rows:
    order = 'VSYNC-before-LATCH' if v is not None and v < l else 'LATCH-first'
    print('put %.6f latch %+.4f vsync %s %s next_en %s' % (p, l - p, '%+.4f' % (v - p) if v else '-', order,
          '%s@%+.4f' % (en[1], en[0] - l) if en else '-'))
