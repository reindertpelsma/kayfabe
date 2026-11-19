#!/usr/bin/env python3
"""Runs ON the box: builds /root/evid/<tag>/ text evidence for each sweep tag (filtered, scrubbed)."""
import glob, os, re, sys, collections, subprocess
B = "/workspace/bench"
OUT = "/root/evid"
UUID = re.compile(r"GPU-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.I)
IPV4 = re.compile(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b")
def scrub(s):
    s = UUID.sub("GPU-<uuid>", s)
    return s
def rd(p):
    try:
        return open(p, errors="replace").read()
    except OSError:
        return ""
def refusals(files):
    c = collections.Counter()
    for f in files:
        for ln in rd(f).splitlines():
            if "HOST-ABI REFUSED" in ln:
                ln = re.sub(r"^\[[^\]]*\]\s*", "", ln)
                ln = re.sub(r" \[#\d+, rate-limited\]", "", ln)
                c[scrub(ln)[:700]] += 1
    return c
tags = sys.argv[1:]
for tag in tags:
    d = f"{OUT}/{tag}"; os.makedirs(d, exist_ok=True)
    sw = f"{B}/sweep/{tag}"
    log = rd(f"{sw}/summary/sweep_{tag}.log")
    keep = [l for l in log.splitlines() if re.match(r"(SWEEP_|CLIENT_RC|KF3_|SWAP|BARE|GATES|CANARY|MATRIX_ROW|THIN|LADDER|STAGE|FAT|HEALTH|HOSTROW)", l)]
    open(f"{d}/sweep_summary.txt", "w").write(scrub("\n".join(keep)) + "\n")
    for g in glob.glob(f"{sw}/gates/*.log"):
        t = rd(g)
        lines = [l for l in t.splitlines() if re.search(r"V3_GATES_SUMMARY|GATE10|^GATE ?[0-9]+|PASS|FAIL", l)]
        open(f"{d}/gates_{os.path.basename(g)}", "w").write(scrub("\n".join(lines[-60:])) + "\n")
    # suites (thin) and ladders (guest) of this tag
    for s in glob.glob(f"{B}/{tag}_*_suite.out"):
        t = rd(s); lines = [l for l in t.splitlines() if l.startswith(("FAST_SUITE", "FAST_CELL_ARM", "FAST_CELL"))]
        open(f"{d}/{os.path.basename(s)}", "w").write(scrub("\n".join(lines)) + "\n")
    for s in glob.glob(f"{B}/cl_{tag}_*_guest.out") + glob.glob(f"{B}/cl_{tag}_*_host.out"):
        t = rd(s); lines = [re.sub(r"ledger=\[[^\]]*\]", "ledger=[...]", l) for l in t.splitlines() if l.startswith(("CUDA_LADDER", "CL_ROW", "bench sizes", "CL_SUMMARY")) or "PASS=" in l or "VERDICT" in l]
        open(f"{d}/{os.path.basename(s)}", "w").write(scrub("\n".join(lines)) + "\n")
    # HOST-ABI refusals: QEMU logs of the thin suite and of the ladder boots of this tag
    for kind, pat in (("thin", f"{B}/fast_{tag}_*_qemu.log"), ("ladder", f"{B}/run_cl_{tag}_*_qemu.log")):
        fs = glob.glob(pat)
        c = refusals(fs)
        with open(f"{d}/host_abi_refused_{kind}.txt", "w") as o:
            o.write(f"# {kind}: {len(fs)} QEMU logs scanned; {sum(1 for f in fs if 'HOST-ABI REFUSED' in rd(f))} contain HOST-ABI REFUSED\n")
            for ln, n in sorted(c.items(), key=lambda x: -x[1]):
                o.write(f"{n:5d}  {ln}\n")
    # host driver + other kernel lines for the ladder boots
    x = []
    for f in sorted(glob.glob(f"{B}/run_cl_{tag}_*_hostdmesg.log")):
        for ln in rd(f).splitlines():
            if re.search(r"NVRM: Xid|NVRM.*(error|fail)", ln): x.append(os.path.basename(f)[-30:] + ": " + ln[:200])
    open(f"{d}/host_dmesg_xid_lines.txt", "w").write(scrub("\n".join(x[:80])) + "\n")
    # kf3 revision and sha
    rev = rd(f"{sw}/summary/sweep_{tag}.log")
    m = re.search(r"KF3_BUILT (\S+) rev=(\w+)", rev)
    open(f"{d}/kf3_built.txt", "w").write(f"{m.group(0) if m else 'n/a'}\n")
print("collected", tags)
