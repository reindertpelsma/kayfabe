import sys, re
t = None
print("tok  engine doorbells  last_doorbell_t  free_t  secs_since_last_doorbell  GPGet GPPut")
for l in open(sys.argv[1], errors='replace'):
    m = re.search(r'PT-SNAP BEGIN at free of tok=(\S+) \(maplog t=([\d.]+)', l)
    if m: t = float(m.group(2)); continue
    m = re.search(r'PT-SNAP tok=(\S+) chan \S+ host=\S+ engine=(\S+) user_work=\S+ doorbells=(\d+) last_doorbell_us=(\d+).*GPGet=(\S+) GPPut=(\S+)', l)
    if m and t is not None:
        ld = int(m.group(4)) / 1e6
        print("%-7s %-5s %-4s %s %.3f %s %s %s" % (m.group(1), m.group(2), m.group(3), ("%.3f" % ld) if ld else '-', t, ("%.1f" % (t - ld)) if ld else '-', m.group(5), m.group(6)))
