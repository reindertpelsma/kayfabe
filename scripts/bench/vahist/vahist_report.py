#!/usr/bin/env python3
"""Post-hoc reader for the KF3_DIAG_VAHIST measurement (kf_mem::vahist), 2026-10-24.

STATUS: LIVE, 2026-10-24. Measurement tooling only; reads logs, writes nothing but stdout.

Usage:
  python3 -I vahist_report.py QEMU_LOG [--xids NEW_XIDS_JSON] [--va 0x... --chid 0x...]
  python3 -I vahist_report.py --selftest

What it answers, per host Xid (from new-xids.json, or --va/--chid):
  1. which guest channel held the host channel id (CHAN_LIFE lines: the newest birth whose
     host_token & 0xffff == chid, alive at the end of the log or freed after the fault), its
     process id, class set and VA space, and every EARLIER owner of the same host id;
  2. kayfabe's own answer, when the lane triggered a lookup (VAHIST LOOKUP .. END): the verdict
     per candidate space (PRESENT / UNMAPPED / REFUSED / WITHHELD / HELD / UNMAP_REFUSED /
     ROW_GONE / NOT_NAMED / RETIRED_MAPPED / RETIRED_NOT_MAPPED), the events covering the VA, the
     guest PTE probe, and the last runs of the primary space;
  3. the VAHIST AUTO dumps (refused maps / walk refusals) of the faulting space, if any.
It prints a one-line VERDICT per Xid and which hypothesis that reading supports (see
/root/enscape08_hypotheses_20261024.md: H1 absence, H2 order, H3 reuse, H4 lost map, H5 priv).

All values are read as text; nothing is executed or evaluated from the log.
"""
import json
import re
import sys

XID_RE = re.compile(r"channel 0x([0-9a-fA-F]+).*?faulted @ 0x([0-9a-fA-F_]+)")
BIRTH_RE = re.compile(r"kf3: CHAN_LIFE birth #(\d+) t_ms=(\d+) key=\((0x[0-9a-f]+),(0x[0-9a-f]+)\) "
                      r"guest_tok=(0x[0-9a-f]+) host_token=(0x[0-9a-f]+) pid=(\S+) engine=(0x[0-9a-f]+) "
                      r"gp=(\d+) gpfifo=(0x[0-9a-f]+) vas=\((0x[0-9a-f]+),(0x[0-9a-f]+)\)")
FREE_RE = re.compile(r"kf3: CHAN_LIFE free #(\d+) t_ms=(\d+) key=\((0x[0-9a-f]+),(0x[0-9a-f]+)\) "
                     r"host_token=(0x[0-9a-f]+) lived_ms=(\d+) classes=\[([^\]]*)\] ok=(\w+)")
LOOKUP_RE = re.compile(r"kf3: VAHIST LOOKUP #(\d+) va=(0x[0-9a-f]+) chid=(\S+)")
END_RE = re.compile(r"kf3: VAHIST END #(\d+)")
AUTO_RE = re.compile(r"kf3: VAHIST AUTO #(\d+) key=\((0x[0-9a-f]+),(0x[0-9a-f]+)\)")
VAS_RE = re.compile(r"kf3: VAHIST VAS key=(\S+) role=(\S+) verdict=(\S+) age_ms=(\S+)")

SUPPORT = {
    "REFUSED": "H1 (kayfabe absence: a refused MAP) SUPPORTED",
    "WITHHELD": "H5 (privileged leaf withheld from a user twin) SUPPORTED",
    "UNMAPPED": "H2 (the guest unmapped the VA before the GPU write: order) SUPPORTED; check age_ms",
    "UNMAP_REFUSED": "unexpected: an UNMAP was refused (host still holds the old mapping)",
    "HELD": "HELD: the host already held something else at this VA (stale host mapping)",
    "ROW_GONE": "H4-like: a MAP was recorded but no row covers the VA now (lost row)",
    "PRESENT": "H1/H2/H4 FALSIFIED for this VA: our row covers it; look host-side (wrong space bound, TLB)",
    "NOT_NAMED": "no recorded event: read the PTE probe (guest PTE valid => H4 lost map; invalid => foreign VA / H3)",
    "RETIRED_MAPPED": "H3 (VA of a PREVIOUS owner's space) SUPPORTED if the current owner's verdict is NOT_NAMED",
}


def parse_log(lines):
    births, frees, lookups, autos = [], [], [], []
    cur = None
    for ln in lines:
        m = BIRTH_RE.search(ln)
        if m:
            births.append(dict(n=int(m.group(1)), t=int(m.group(2)), key=(m.group(3), m.group(4)),
                               host_token=int(m.group(6), 16), pid=m.group(7), gp=int(m.group(9)),
                               vas=(m.group(11), m.group(12))))
            continue
        m = FREE_RE.search(ln)
        if m:
            frees.append(dict(n=int(m.group(1)), t=int(m.group(2)), key=(m.group(3), m.group(4)),
                              host_token=int(m.group(5), 16), lived=int(m.group(6)),
                              classes=m.group(7), ok=m.group(8)))
            continue
        m = LOOKUP_RE.search(ln) or AUTO_RE.search(ln)
        if m and "VAHIST LOOKUP capped" not in ln:
            cur = dict(kind="LOOKUP" if "LOOKUP" in ln else "AUTO", head=ln.strip(), lines=[])
            (lookups if cur["kind"] == "LOOKUP" else autos).append(cur)
            continue
        if cur is not None and "kf3: VAHIST " in ln:
            cur["lines"].append(ln.strip())
            if END_RE.search(ln):
                cur = None
    return births, frees, lookups, autos


def owners(births, frees, chid):
    """Every recorded channel whose host token names `chid`, oldest first, with its free."""
    out = []
    for b in births:
        if b["host_token"] & 0xFFFF != chid:
            continue
        f = next((f for f in frees if f["key"] == b["key"] and f["n"] > b["n"]), None)
        out.append((b, f))
    return out


def report(lines, xids):
    births, frees, lookups, autos = parse_log(lines)
    print(f"log: {len(births)} CHAN_LIFE births, {len(frees)} frees, {len(lookups)} lookup(s), "
          f"{len(autos)} auto dump(s)")
    if not births and not lookups:
        print("NOTE: no KF3_DIAG_VAHIST lines: the run was not made with KF3_DIAG_VAHIST=1")
    for (chid, va) in xids:
        print(f"\n== Xid: chid={chid:#x} va={va:#x}")
        own = owners(births, frees, chid)
        for i, (b, f) in enumerate(own):
            state = f"freed t_ms={f['t']} lived_ms={f['lived']} classes=[{f['classes']}]" if f else "ALIVE at end"
            print(f"  owner {i + 1}/{len(own)}: key=({b['key'][0]},{b['key'][1]}) pid={b['pid']} gp={b['gp']} "
                  f"vas=({b['vas'][0]},{b['vas'][1]}) born t_ms={b['t']} {state}")
        if not own:
            print("  no CHAN_LIFE birth names this host id")
        lk = [L for L in lookups if f"va={va:#x}" in L["head"]]
        if not lk:
            print("  no VAHIST LOOKUP for this VA in the log (did the lane write the trigger before the stop?)")
        for L in lk:
            print("  " + L["head"])
            verdicts = []
            for ln in L["lines"]:
                if any(t in ln for t in (" VAS ", " EV ", " PTE ", " CHAN ")):
                    print("    " + ln[:400])
                m = VAS_RE.search(ln)
                if m:
                    verdicts.append((m.group(1), m.group(2), m.group(3), m.group(4)))
            if verdicts:
                key, role, v, age = verdicts[0]
                print(f"  VERDICT: primary space {key} ({role}): {v} (age_ms={age}) -> {SUPPORT.get(v, 'see the lines above')}")
                if v == "NOT_NAMED" and any(x[2] == "RETIRED_MAPPED" for x in verdicts[1:]):
                    print("  VERDICT: " + SUPPORT["RETIRED_MAPPED"])
        owner_vas = {b["vas"] for b, _ in own}
        for A in autos:
            m = AUTO_RE.search(A["head"])
            if m and (m.group(2), m.group(3)) in owner_vas:
                print("  AUTO dump of an owner's space: " + A["head"][:300])
                for ln in A["lines"][:12]:
                    print("    " + ln[:300])


def xids_from(path):
    out = []
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        data = json.load(fh)
    for s in data if isinstance(data, list) else []:
        m = XID_RE.search(str(s))
        if m:
            out.append((int(m.group(1), 16), int(m.group(2).replace("_", ""), 16)))
    return out


SELFTEST = """\
kf3: CHAN_LIFE birth #1 t_ms=100 key=(0xc1d00284,0xff040039) guest_tok=0x3d host_token=0x67 pid=0x37a0 engine=0x1 gp=32768 gpfifo=0x722b000 vas=(0xc1d00276,0xff000850)
kf3: CHAN_LIFE free #2 t_ms=900 key=(0xc1d00284,0xff040039) host_token=0x67 lived_ms=800 classes=[0xc7b5,0xc997] ok=true vas=(0xc1d00276,0xff000850)
kf3: CHAN_LIFE birth #3 t_ms=1000 key=(0xc1d0029b,0xff040039) guest_tok=0x3d host_token=0x67 pid=0x325c engine=0x1 gp=32768 gpfifo=0x722b000 vas=(0xc1d0028c,0xff000850)
kf3: VAHIST LOOKUP #1 va=0xb5a225000 chid=0x67 t_ms=1500 t0_unix_ms=0 spaces=2 retired=1 chans=2 chan_matches=2 spaces_untracked=0
kf3: VAHIST CHAN host_token=0x67 key=(0xc1d0029b,0xff040039) guest_tok=0x3d pid=0x325c engine=0x1 gp=32768 vas=(0xc1d0028c,0xff000850) born_ms=1000 alive classes=[]
kf3: VAHIST VAS key=(0xc1d0028c,0xff000850) role=chan verdict=UNMAPPED age_ms=12 history_complete=yes host_space=0xcaff4b20 recycled=true born_ms=990 runs=10 walk_refusals=0
kf3: VAHIST EV key=(0xc1d0028c,0xff000850) walk#9 t_ms=1488 op=UNMAP va=0xb5a200000 len=0x40000 ap=0 kind=0x0 ack=1
kf3: VAHIST END #1 lines=5
"""


def selftest():
    import io
    import contextlib
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        report(SELFTEST.splitlines(),
               xids_from_text(["... channel 0x00000067, MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0xb_5a225000. ..."]))
    out = buf.getvalue()
    assert "owner 1/2: key=(0xc1d00284,0xff040039)" in out, out
    assert "owner 2/2: key=(0xc1d0029b,0xff040039)" in out and "ALIVE at end" in out, out
    assert "VERDICT: primary space (0xc1d0028c,0xff000850) (chan): UNMAPPED (age_ms=12)" in out, out
    assert "H2" in out, out
    print("selftest ok")


def xids_from_text(items):
    out = []
    for s in items:
        m = XID_RE.search(s)
        if m:
            out.append((int(m.group(1), 16), int(m.group(2).replace("_", ""), 16)))
    return out


def main(argv):
    if "--selftest" in argv:
        selftest()
        return 0
    if not argv or argv[0].startswith("-"):
        print(__doc__)
        return 2
    log = argv[0]
    xids = []
    if "--xids" in argv:
        xids += xids_from(argv[argv.index("--xids") + 1])
    if "--va" in argv:
        va = int(argv[argv.index("--va") + 1].replace("_", ""), 16)
        chid = int(argv[argv.index("--chid") + 1], 16) if "--chid" in argv else 0
        xids.append((chid, va))
    with open(log, "r", encoding="utf-8", errors="replace") as fh:
        report(fh, xids)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
