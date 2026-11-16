import os, re, time, datetime
OUT = "/workspace/stage_kernel_offsets.tsv"
STD = "/workspace/payload_stdout.log"
KL = "/workspace/kernel.log"
seen = 0
last_hb = 0
def now(): return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
def ksize():
    try:
        b = open(KL, "rb").read()
        return len(b), b.count(b"\n"), len(re.findall(rb"NVRM: Xid", b))
    except OSError:
        return -1, -1, -1
with open(OUT, "a") as f:
    f.write("utc\tevent\tkernel_bytes\tkernel_lines\txid_lines\n")
    while True:
        try:
            lines = open(STD, errors="replace").read().splitlines()
        except OSError:
            lines = []
        for ln in lines[seen:]:
            if ln.startswith("KF_PAYLOAD_STAGE_START") or ln.startswith("KF_PAYLOAD_STAGE_END"):
                s = ksize()
                f.write("%s\t%s\t%d\t%d\t%d\n" % (now(), ln, *s)); f.flush()
        seen = len(lines)
        if time.time() - last_hb > 60:
            s = ksize()
            f.write("%s\theartbeat\t%d\t%d\t%d\n" % (now(), *s)); f.flush()
            last_hb = time.time()
        if os.path.exists("/workspace/payload_exit.line") and time.time() - os.path.getmtime("/workspace/payload_exit.line") > 5:
            s = ksize(); f.write("%s\tfinal\t%d\t%d\t%d\n" % (now(), *s)); f.flush(); break
        time.sleep(2)
