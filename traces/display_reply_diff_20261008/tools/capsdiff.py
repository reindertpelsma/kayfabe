"""capsdiff.py KFCAPS HWONLY REGNAME -- hardware caps-page reads (8-byte) vs kf3's authored caps page."""
import subprocess, sys
kf = {int(a, 16): int(v, 16) for a, v in (l.split() for l in open(sys.argv[1]))}
hw = {}
for l in open(sys.argv[2]):
    f = l.split()
    if f[0] == '#' or f[2] != 'R':
        continue
    a = int(f[3], 16)
    if 0x640000 <= a < 0x641000:
        v = int(f[5], 16)
        hw[a] = v & 0xffffffff
        hw[a + 4] = v >> 32
rows = [(a, hw.get(a), kf.get(a, 0)) for a in sorted(set(hw) | set(kf)) if hw.get(a) != kf.get(a, 0)]
print(len(rows), 'differing words of', len(hw), 'read')
names = subprocess.run(['python3', '-I', sys.argv[3]] + ['%x' % a for a, _, _ in rows], capture_output=True, text=True).stdout.splitlines()
for (a, h, k), n in zip(rows, names):
    print('0x%06x hw %s kf %08x  %s' % (a, '%08x' % h if h is not None else 'unread', k, n.split(' ', 1)[1][:100]))
