#!/usr/bin/env python3
"""vncw_analyze.py EVDIR — what a VNC client saw around the guest pointer (vnc_watch.py's timeline)
against the side script's marks. The ROI picture shown at `hover_ref` (+3 s) is the cursor-free
reference (in hover the frames carry no cursor); a later ROI picture differing from it in more than
20 pixels (any channel > 24) is "composed" (the cursor drawn into the frame). The client sees TWO
cursors while a visible cursor is defined AND the shown ROI is composed; NONE while neither."""
import os, sys
ev = sys.argv[1]; d = os.path.join(ev, "vncw")
marks = [(float(l.split()[0]), l.split()[1]) for l in open(os.path.join(ev, "marks.txt")) if l.strip()]
tl = []
for l in open(os.path.join(d, "timeline.txt")):
    p = l.split()
    if len(p) >= 2:
        tl.append((float(p[0]), p[1], p[2:]))
def ppm(h):
    b = open(os.path.join(d, "roi_%s.ppm" % h), "rb").read().split(b"\n", 3)
    return b[3]
ref_t = dict((n, t) for t, n in marks).get("hover_ref")
if ref_t is None:
    print("VNCW_ANALYZE no hover_ref mark"); sys.exit(0)
ref = None
for t, k, a in tl:
    if k == "FB" and t <= ref_t + 3:
        ref = a[1].split("=")[1][2:]
if ref is None:
    print("VNCW_ANALYZE no ROI before hover_ref"); sys.exit(0)
R = ppm(ref)
def diff(h):
    X = ppm(h)
    return sum(1 for i in range(0, len(R), 3) if max(abs(R[i] - X[i]), abs(R[i+1] - X[i+1]), abs(R[i+2] - X[i+2])) > 24)
cache = {}
vis = None; comp = None; state_t = None; two = []; none = []
events = []
for t, k, a in tl:
    if k == "CURSOR":
        v = int([x for x in a if x.startswith("visible_px=")][0].split("=")[1]); vis = v > 0
        events.append((t, "cursor %s" % ("visible(%d)" % v if v else "hidden")))
    elif k == "FB":
        h = a[1].split("=")[1][2:]
        if h not in cache:
            cache[h] = diff(h)
        comp = cache[h] > 20
        events.append((t, "frame %s(%d px off ref)" % ("COMPOSED" if comp else "clean", cache[h])))
    else:
        continue
    if state_t is not None and prev is not None:
        pass
    cur = (vis, comp)
    if state_t is not None:
        if prev == (True, True):
            two.append((state_t, t))
        if prev == (False, False):
            none.append((state_t, t))
    state_t, prev = t, cur
end = tl[-1][0] if tl else 0
if state_t is not None:
    if prev == (True, True): two.append((state_t, end))
    if prev == (False, False): none.append((state_t, end))
def fmt(iv):
    return ", ".join("%.3f s at %s" % (b - a, next((n for mt, n in reversed(marks) if mt <= a), "start")) for a, b in iv)
print("VNCW_ANALYZE ref=%s rois=%d cursors=%d frames=%d" % (ref, len(cache), sum(1 for e in events if e[1].startswith("cursor")), sum(1 for e in events if e[1].startswith("frame"))))
print("VNCW_TWO_CURSORS intervals=%d total_s=%.3f [%s]" % (len(two), sum(b - a for a, b in two), fmt(two)))
print("VNCW_NO_CURSOR intervals=%d total_s=%.3f [%s]" % (len(none), sum(b - a for a, b in none), fmt(none)))
allev = sorted([(t, "MARK " + n) for t, n in marks] + events)
t0 = allev[0][0] if allev else 0
with open(os.path.join(ev, "vncw_sequence.txt"), "w") as f:
    for t, e in allev:
        f.write("%9.3f %s\n" % (t - t0, e))
print("VNCW_SEQUENCE -> %s (%d lines)" % (os.path.join(ev, "vncw_sequence.txt"), len(allev)))
