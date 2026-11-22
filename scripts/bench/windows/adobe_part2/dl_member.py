#!/usr/bin/env python3
"""Box-side ranged downloader (stdlib): N parallel connections of one presigned GET into a preallocated file, retries per chunk, then sha256.
usage: dl2.py URLFILE OUT SIZE SHA256 SRC_OFFSET [CONNS=8]  (the stored zip member at SRC_OFFSET of the object, written as OUT at offset 0)   (writes OUT.log start marker + exit line; never prints the URL)"""
import sys, os, time, hashlib, threading, urllib.request, urllib.error
url_file, out, size, want = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
SRC = int(sys.argv[5])
conns = int(sys.argv[6]) if len(sys.argv) > 6 else 8
URL = open(url_file).read().strip()
CH = 128 * 1024 * 1024
log = open(out + ".log", "a", buffering=1)
def L(m): log.write("%s %s\n" % (time.strftime("%FT%TZ", time.gmtime()), m))
L("START size=%d conns=%d src_offset=%d" % (size, conns, SRC))
fd = os.open(out, os.O_RDWR | os.O_CREAT, 0o600)
os.ftruncate(fd, size)
chunks = [(o, min(CH, size - o)) for o in range(0, size, CH)]
done_file = out + ".done"
done = set(int(x) for x in open(done_file).read().split()) if os.path.exists(done_file) else set()
lock = threading.Lock(); nbytes = [0]
q = [c for c in chunks if c[0] not in done]
def worker():
    while True:
        with lock:
            if not q: return
            o, n = q.pop(0)
        for att in range(8):
            try:
                req = urllib.request.Request(URL, headers={"Range": "bytes=%d-%d" % (SRC + o, SRC + o + n - 1)})
                got = 0
                with urllib.request.urlopen(req, timeout=60) as r:
                    if r.status != 206: raise IOError("status %s" % r.status)
                    while got < n:
                        b = r.read(min(1 << 20, n - got))
                        if not b: raise IOError("short")
                        os.pwrite(fd, b, o + got); got += len(b)
                        with lock: nbytes[0] += len(b)
                with lock:
                    open(done_file, "a").write("%d\n" % o)
                break
            except Exception as e:
                L("chunk %d retry %d: %s" % (o, att, type(e).__name__))
                time.sleep(2 + att * 3)
        else:
            L("chunk %d FAILED" % o); os._exit(2)
t0 = time.time()
ts = [threading.Thread(target=worker) for _ in range(conns)]
[t.start() for t in ts]
while any(t.is_alive() for t in ts):
    time.sleep(30); L("progress %.1f GB %.1f MB/s" % (nbytes[0] / 1e9, nbytes[0] / 1e6 / (time.time() - t0)))
os.fsync(fd); L("DOWNLOADED secs=%.0f MB/s=%.1f" % (time.time() - t0, nbytes[0] / 1e6 / (time.time() - t0)))
h = hashlib.sha256(); t1 = time.time()
with open(out, "rb") as f:
    while True:
        b = f.read(8 << 20)
        if not b: break
        h.update(b)
ok = h.hexdigest() == want and os.path.getsize(out) == size
L("SHA256 %s size_ok=%s secs=%.0f" % ("OK" if ok else "MISMATCH", os.path.getsize(out) == size, time.time() - t1))
L("EXIT rc=%d" % (0 if ok else 3)); sys.exit(0 if ok else 3)
