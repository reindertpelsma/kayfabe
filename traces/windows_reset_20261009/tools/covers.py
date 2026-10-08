import re, sys
# covers.py LOG VASKEY VA T0 T1 : every MAP/UNMAP row of the space that covers VA between kf times T0..T1
f, key, va, t0, t1 = sys.argv[1], sys.argv[2], int(sys.argv[3], 16), float(sys.argv[4]), float(sys.argv[5])
for n, l in enumerate(open(f, errors='replace'), 1):
    if key not in l:
        continue
    m = re.search(r'maplog t=([\d.]+) walk#(\d+) \S+ root \S+ (UNMAP|MAP) va=(0x[0-9a-f]+) len=(0x[0-9a-f]+) at=(0x[0-9a-f]+)', l)
    if not m:
        continue
    t = float(m.group(1))
    s, ln = int(m.group(4), 16), int(m.group(5), 16)
    if t0 <= t <= t1 and s <= va < s + ln:
        print(n, t, 'walk#' + m.group(2), m.group(3), m.group(4), m.group(5), 'at=' + m.group(6))
