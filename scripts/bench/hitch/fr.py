#!/usr/bin/env python3
"""fr.py GUESTLOG F9_EPOCH [OFFSET_S] : frame-time analysis from 'FR <start FILETIME> <dur 100ns>' lines of a heaven_etw job log.
Host epoch of a frame = FILETIME/1e7 - 11644473600 + OFFSET_S (default 7200: the guest clock is 2 h behind true UTC).
Prints: frame count, intervals histogram (in ms buckets / vsync quanta), per-2s fps series relative to F9, top longest intervals with time."""
import sys, collections

def frames(path):
    out = []
    for ln in open(path, errors='replace'):
        if ln.startswith('FR '):
            p = ln.split()
            try:
                out.append((int(p[1]), int(p[2])))
            except (IndexError, ValueError):
                pass
    return out

def to_epoch(ft, off):
    return ft / 1e7 - 11644473600 + off

def main():
    path, f9 = sys.argv[1], float(sys.argv[2])
    off = float(sys.argv[3]) if len(sys.argv) > 3 else 7200.0
    fr = frames(path)
    t = [to_epoch(a, off) - f9 for a, _ in fr]
    dur = [d / 1e4 for _, d in fr]  # ms inside Present
    # benchmark window: the first 262 s after F9 (F9 happens ~ at t=0)
    idx = [i for i, x in enumerate(t) if 0 <= x <= 261]
    n = len(idx)
    iv = [(t[idx[k]] - t[idx[k - 1]]) * 1000 for k in range(1, n)]
    print('frames in window: %d  (mean fps %.2f)  intervals: min %.2f max %.2f ms' % (n, n / 261.0, min(iv), max(iv)))
    # fps instantaneous = 1/interval; Heaven min FPS ~ 1000/max(iv)
    print('instantaneous min fps = %.1f ; p1 interval %.1f ms ; p50 %.1f ; p99 %.1f ; p99.9 %.1f' % (
        1000.0 / max(iv), sorted(iv)[int(len(iv) * .01)], sorted(iv)[len(iv) // 2], sorted(iv)[int(len(iv) * .99)], sorted(iv)[int(len(iv) * .999)]))
    q = collections.Counter()
    for x in iv:
        k = int(x / 8.333 + 0.5)  # in half-vsync quanta
        q[min(k, 12)] += 1
    print('interval histogram (quanta of 8.33 ms, i.e. 2=one 60 Hz vblank):')
    for k in sorted(q):
        print('  %5.1f ms (%2d): %5d %5.1f%%' % (k * 8.333, k, q[k], 100.0 * q[k] / len(iv)))
    # in-Present durations
    dd = [dur[i] for i in idx]
    sd = sorted(dd)
    print('time inside Present(): p50 %.2f p90 %.2f p99 %.2f max %.2f ms ; share of frame time %.0f%%' % (
        sd[len(sd) // 2], sd[int(len(sd) * .9)], sd[int(len(sd) * .99)], sd[-1], 100.0 * sum(dd) / (sum(iv) or 1)))
    top = sorted(range(1, n), key=lambda k: -iv[k - 1])[:15]
    print('top 15 longest frame intervals (t rel F9, interval ms, Present ms):')
    for k in sorted(top):
        print('  t=%7.2f  %6.1f ms  present=%6.1f' % (t[idx[k]], iv[k - 1], dur[idx[k]]))
    print('per-2s fps:')
    b = collections.Counter(int(t[i] // 2) for i in idx)
    print(' '.join('%d:%.0f' % (k * 2, b[k] / 2.0) for k in sorted(b)))

if __name__ == '__main__':
    main()
