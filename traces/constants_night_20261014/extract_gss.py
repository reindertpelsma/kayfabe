# Extract RM_CONTROL records of the GSS-legacy / cudart-init controls from an nvdiff JSONL trace.
# Prints cmd, status, size, and the request/reply words (u32 LE). Run with python3 -I.
import json, struct, sys

WANT = {0x20809064, 0x20809001, 0x20809009, 0x2080a001, 0x2080a084, 0x2080a026, 0x2080a028, 0x20802209}

def words(h, n=None):
    b = bytes.fromhex(h)
    k = len(b) // 4
    w = list(struct.unpack("<%dI" % k, b[: 4 * k]))
    return w if n is None else w[:n]

for line in open(sys.argv[1]):
    try:
        r = json.loads(line)
    except Exception:
        continue
    if r.get("t") != "ioctl" or r.get("nr") != 0x2a:
        continue
    hpre = bytes.fromhex(r.get("hpre", ""))
    hpost = bytes.fromhex(r.get("hpost", ""))
    if len(hpre) < 32:
        continue
    cmd = struct.unpack_from("<I", hpre, 8)[0]
    if cmd not in WANT:
        continue
    status = struct.unpack_from("<I", hpost, 28)[0] if len(hpost) >= 32 else None
    size = struct.unpack_from("<I", hpre, 24)[0]
    pre = words(r.get("ppre", ""))
    post = words(r.get("ppost", ""))
    nz_pre = [(i, hex(v)) for i, v in enumerate(pre) if v]
    changed = [(i, hex(post[i])) for i in range(min(len(pre), len(post))) if pre[i] != post[i]]
    print("CTRL cmd=%#010x size=%d status=%s" % (cmd, size, hex(status) if status is not None else "?"))
    print("  req_nonzero_words=%s" % nz_pre[:40])
    print("  reply_changed_words=%s" % changed[:40])
