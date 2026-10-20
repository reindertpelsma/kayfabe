#!/usr/bin/env python3
"""userd_probe: READ-ONLY view of the guest-visible GP_PUT / GP_GET of every live Windows
USER-ring (passthrough) channel of a running kayfabe VM.  Standard library only.

What it reads (see README.md for the limits -- they matter):
  * the VM's qemu.log (read-only, tailed): channel births (CLASSIFY / CHAN-FACTS / "Windows USER ring
    chan .. BORN"), frees ("act free: .. passthrough"), and "act disable channels" acts;
  * the QEMU process's guest-RAM memfd mapping through /proc/<pid>/mem, opened O_RDONLY, pread only.

The word pair it prints is the GUEST'S USERD SLOT (sysmem): GP_PUT is written by the guest's driver,
GP_GET is written back by kayfabe's relay from the PRIVATE (kayfabe-owned, vidmem) USERD the GPU actually
uses.  So  PUT != GET  means "the guest's view of the channel has unreturned work", not by itself "the GPU
never fetched it": the private words are not in guest RAM.  A pair that stays unequal AND frozen is the
stuck candidate.  Nothing is ever written to the VM, its memory, its log or any lock.
"""
import argparse
import os
import re
import struct
import sys
import time

USERD_GP_GET = 0x88  # NV_RAMUSERD_GP_GET, dword 34 (kf_abi::submit::USERD_GP_GET)
USERD_GP_PUT = 0x8C  # NV_RAMUSERD_GP_PUT, dword 35
USERD_BYTES = 0x200
STORE_BYTES = 1 << 20          # kf_chan::user_storage::BYTES: one private storage object per USER-ring channel
STORE_USERD = 0xF0000          # kf_chan::user_storage::USERD_OFFSET: the GPU's own (private) USERD inside it
PRIV_ENTRIES = 512             # kf_chan::user_storage::ENTRIES: the private GPFIFO size
PAGE = 0x1000
GIB = 1 << 30
LOWMEM_CANDIDATES = (3 * GIB, 2 * GIB, 0xB0000000)  # measured on this host: 3 GiB (12288 MiB guest)

RE_CLASSIFY = re.compile(r"CLASSIFY (0x[0-9a-f]+:0x[0-9a-f]+) USER WORK -> Passthrough")
RE_FACTS = re.compile(r"CHAN-FACTS (0x[0-9a-f]+:0x[0-9a-f]+) ")
RE_ENTRIES = re.compile(r"gpFifoEntries=(\d+)")
RE_PID = re.compile(r"ProcessID=Some\((\d+)\)")
RE_USERD = re.compile(r"userd=Some\((\w+) \{ base: ([0-9a-f]+), size: ([0-9a-f]+) \}\)")
RE_BORN = re.compile(r"Windows USER ring chan (0x[0-9a-f]+:0x[0-9a-f]+) BORN USER .*? token (0x[0-9a-f]+) host (0x[0-9a-f]+)")
RE_FREE = re.compile(r"act free: (0x[0-9a-f]+:0x[0-9a-f]+): passthrough ")
RE_DIS = re.compile(r"act disable channels: (0x[0-9a-f]+) DISABLE_CHANNELS\(bDisable=(true|false)")
RE_DIS_LIST = re.compile(r"(0x[0-9a-f]+:0x[0-9a-f]+)->host (0x[0-9a-f]+)")
RE_UMODE = re.compile(r"chan DISABLE_USERMODE_CHANNELS bDisable=(true|false)")


class Chan:
    __slots__ = ("key", "token", "host", "entries", "pid", "aperture", "gpa", "size", "born_line",
                 "put", "get", "nz", "t_change", "t_behind", "last", "ok")

    def __init__(self, key):
        self.key = key
        self.token = self.host = None
        self.entries = 0
        self.pid = None
        self.aperture = None
        self.gpa = None
        self.size = 0
        self.born_line = 0
        self.put = self.get = self.nz = None
        self.t_change = None   # monotonic time of the last PUT/GET change
        self.t_behind = None   # monotonic time since PUT != GET continuously
        self.last = (None, None)
        self.ok = True


class LogState:
    """Incremental, read-only parse of qemu.log."""

    def __init__(self, path):
        self.path = path
        self.f = open(path, "rb")
        self.off = 0
        self.partial = b""
        self.line_no = 0
        self.live = {}       # key -> Chan
        self.classified = set()
        self.facts = {}      # key -> dict
        self.by_host = {}    # host token -> key
        self.act = {}        # host -> dict(state, t, n_dis, n_en)  state in 'D','E'
        self.events = []     # (wall time or None, text) produced since last drain
        self.history = True  # lines read before the first drain have unknown times
        self.umode = None

    def poll(self):
        size = os.fstat(self.f.fileno()).st_size
        if size < self.off:  # truncated / replaced: start over
            self.off = 0
            self.partial = b""
        self.f.seek(self.off)
        data = self.f.read(min(size - self.off, 64 << 20))
        self.off += len(data)
        if not data:
            return
        data = self.partial + data
        lines = data.split(b"\n")
        self.partial = lines.pop()
        now = None if self.history else time.time()
        for raw in lines:
            self.line_no += 1
            if (b"CLASSIFY" not in raw and b"CHAN-FACTS" not in raw and b"USER ring chan" not in raw
                    and b"act free:" not in raw and b"act disable channels" not in raw
                    and b"DISABLE_USERMODE" not in raw):
                continue
            self.line(raw.decode("utf-8", "replace"), now)

    def line(self, s, now):
        m = RE_CLASSIFY.search(s)
        if m:
            self.classified.add(m.group(1))
            return
        m = RE_FACTS.search(s)
        if m:
            key = m.group(1)
            if key in self.classified:
                self.classified.discard(key)
                d = {"entries": 0, "pid": None, "ap": None, "gpa": None, "size": 0}
                e = RE_ENTRIES.search(s)
                d["entries"] = int(e.group(1)) if e else 0
                p = RE_PID.search(s)
                d["pid"] = int(p.group(1)) if p else None
                u = RE_USERD.search(s)
                if u:
                    d["ap"] = u.group(1)
                    d["gpa"] = int(u.group(2), 16)
                    d["size"] = int(u.group(3), 16)
                self.facts[key] = d
            return
        m = RE_BORN.search(s)
        if m:
            key = m.group(1)
            d = self.facts.pop(key, None)
            c = Chan(key)
            c.token, c.host = int(m.group(2), 16), int(m.group(3), 16)
            c.born_line = self.line_no
            if d:
                c.entries, c.pid, c.aperture, c.gpa, c.size = d["entries"], d["pid"], d["ap"], d["gpa"], d["size"]
            old = self.live.get(key)
            if old is not None:
                self.by_host.pop(old.host, None)
            self.live[key] = c
            self.by_host[c.host] = key
            self.act.pop(c.host, None)
            if now:
                self.events.append((now, f"BIRTH {key} host {c.host:#x} pid {c.pid} userd {c.aperture} {c.gpa and hex(c.gpa)}"))
            return
        m = RE_FREE.search(s)
        if m:
            c = self.live.pop(m.group(1), None)
            if c is not None:
                self.by_host.pop(c.host, None)
                self.act.pop(c.host, None)
                if now:
                    self.events.append((now, f"FREE {c.key} host {c.host:#x}"))
            return
        m = RE_DIS.search(s)
        if m:
            disable = m.group(2) == "true"
            for k, h in RE_DIS_LIST.findall(s):
                h = int(h, 16)
                a = self.act.setdefault(h, {"state": None, "t": None, "n_dis": 0, "n_en": 0, "line": 0})
                a["state"] = "D" if disable else "E"
                a["t"] = now
                a["line"] = self.line_no
                a["n_dis" if disable else "n_en"] += 1
                if now:
                    self.events.append((now, f"{'DISABLE' if disable else 'ENABLE'} {k} host {h:#x}"))
            return
        m = RE_UMODE.search(s)
        if m:
            self.umode = (m.group(1) == "true", now)
            if now:
                self.events.append((now, f"DISABLE_USERMODE_CHANNELS bDisable={m.group(1)}"))


def find_qemu(name):
    hits = []
    for d in os.listdir("/proc"):
        if not d.isdigit():
            continue
        try:
            cmd = open(f"/proc/{d}/cmdline", "rb").read().split(b"\0")
        except OSError:
            continue
        if not cmd or b"qemu-system" not in cmd[0]:
            continue
        args = [a.decode("utf-8", "replace") for a in cmd]
        if "-name" in args and args[args.index("-name") + 1] == name:
            hits.append((int(d), args))
    if len(hits) != 1:
        sys.exit(f"userd_probe: expected exactly one qemu with -name {name}, found {len(hits)}; use --pid/--log")
    return hits[0]


def guest_ram(pid, args):
    size = None
    for a in args:
        m = re.search(r"memory-backend-memfd.*?size=(\d+)([MG])", a)
        if m:
            size = int(m.group(1)) * (1 << 20 if m.group(2) == "M" else GIB)
    best = None
    for l in open(f"/proc/{pid}/maps"):
        f = l.split(None, 5)
        if len(f) < 6 or "memory-backend-memfd" not in f[5]:
            continue
        a, b = (int(x, 16) for x in f[0].split("-"))
        if size is None or b - a == size:
            best = (a, b - a)
    if best is None:
        sys.exit("userd_probe: guest RAM memfd mapping not found in /proc/<pid>/maps")
    return best


class GuestMem:
    def __init__(self, pid, base, size, lowmem):
        self.fd = os.open(f"/proc/{pid}/mem", os.O_RDONLY)  # read-only by construction
        self.base, self.size, self.lowmem = base, size, lowmem

    def hva(self, gpa, n):
        if gpa + n <= self.lowmem:
            off = gpa
        elif gpa >= 4 * GIB:
            off = gpa - 4 * GIB + self.lowmem
        else:
            return None
        return self.base + off if off + n <= self.size else None

    def read(self, gpa, n):
        h = self.hva(gpa, n)
        if h is None:
            return None
        try:
            b = os.pread(self.fd, n, h)
        except OSError:
            return None
        return b if len(b) == n else None


def usable(ch):
    return ch.aperture == "Sysmem" and ch.gpa is not None and ch.size >= USERD_BYTES and ch.gpa & 0x1FF == 0


def detect_lowmem(pid, base, size, chans):
    """Pick the GPA->memfd split under which the logged USERD slots look like USERD pages
    (GP_PUT/GP_GET below the declared ring size, almost every other word zero)."""
    best, scores = None, {}
    for lm in LOWMEM_CANDIDATES:
        gm = GuestMem(pid, base, size, lm)
        good = n = 0
        for c in chans:
            if not usable(c) or not c.entries:
                continue
            raw = gm.read(c.gpa, USERD_BYTES)
            n += 1
            if raw is None:
                continue
            w = struct.unpack("<128I", raw)
            nz = sum(1 for x in w if x)
            if w[USERD_GP_GET // 4] < c.entries and w[USERD_GP_PUT // 4] < c.entries and nz <= 8:
                good += 1
        os.close(gm.fd)
        scores[lm] = (good, n)
        if best is None or good > scores[best][0]:
            best = lm
    return best, scores


class Priv:
    """One 1 MiB nvidia0 mapping of the QEMU process that looks like a private USER-ring storage object."""
    __slots__ = ("addr", "get", "put", "ref", "last", "t_change", "t_behind", "n", "chan", "j")

    def __init__(self, addr):
        self.addr = addr
        self.get = self.put = self.ref = None
        self.last = None
        self.t_change = None
        self.t_behind = None
        self.n = 0.0
        self.chan = None
        self.j = 0.0


def store_maps(pid):
    """Start addresses of the 1 MiB rw-s /dev/nvidia0 mappings (candidate private storages)."""
    out = []
    try:
        for l in open(f"/proc/{pid}/maps"):
            f = l.split(None, 5)
            if len(f) > 5 and f[1] == "rw-s" and f[5].startswith("/dev/nvidia0"):
                a, b = (int(x, 16) for x in f[0].split("-"))
                if b - a == STORE_BYTES:
                    out.append(a)
    except OSError:
        pass
    return out


def read_priv(fd, addr):
    """(reference, GP_GET, GP_PUT) of the GPU's private USERD, 0x50 bytes read-only; None if unreadable/implausible."""
    try:
        b = os.pread(fd, 0x50, addr + STORE_USERD + 0x48)
    except OSError:
        return None
    if len(b) != 0x50:
        return None
    w = struct.unpack("<20I", b)
    get, put = w[(USERD_GP_GET - 0x48) // 4], w[(USERD_GP_PUT - 0x48) // 4]
    if get >= PRIV_ENTRIES or put >= PRIV_ENTRIES:
        return None
    return w[0], get, put


def fmt_age(t, now):
    return "?" if t is None else f"{now - t:.0f}s"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--name", default="boundary-kayfabe-3092", help="qemu -name of the VM")
    ap.add_argument("--pid", type=int, help="qemu pid (skips discovery)")
    ap.add_argument("--log", help="qemu.log path (default: next to the -qmp socket)")
    ap.add_argument("--lowmem", help="bytes of guest RAM below 4 GiB in the memfd (default: auto-detect)")
    ap.add_argument("--watch", nargs="?", const=0, type=float, metavar="SECONDS",
                    help="refresh forever (or for SECONDS)")
    ap.add_argument("--interval", type=float, default=1.0, help="seconds between printed ticks")
    ap.add_argument("--fast", type=float, default=0.02, help="seconds between the fast samples that feed the matcher")
    ap.add_argument("--tsv", help="append rows to this TSV (behind channels each tick, all channels every 10 s, events)")
    ap.add_argument("--all", action="store_true", help="print every live channel each tick (default: behind/flagged/changed only)")
    ap.add_argument("--stuck-after", type=float, default=2.0, help="seconds of PUT!=GET with no change to flag it")
    ap.add_argument("--lazy-bound", type=int, default=32,
                    help="a frozen guest PUT!=GET gap up to this many entries is 'lazy?' (only meaningful with "
                         "KF3_LAZY_OBSERVE on), larger gaps STUCK")
    ap.add_argument("--no-real", action="store_true", help="do not read the private (GPU) USERDs")
    a = ap.parse_args()

    if a.pid:
        pid, args = a.pid, [x.decode() for x in open(f"/proc/{a.pid}/cmdline", "rb").read().split(b"\0")]
    else:
        pid, args = find_qemu(a.name)
    log = a.log
    if not log:
        m = next((re.search(r"unix:(.*?)/qmp\.sock", x) for x in args if re.search(r"unix:(.*?)/qmp\.sock", x)), None)
        if not m:
            sys.exit("userd_probe: cannot derive the run dir; use --log")
        log = m.group(1) + "/qemu.log"
    base, size = guest_ram(pid, args)
    st = LogState(log)
    st.poll()
    st.history = False
    chans = list(st.live.values())
    if a.lowmem:
        lowmem, scores = int(a.lowmem, 0), {}
    else:
        lowmem, scores = detect_lowmem(pid, base, size, chans)
    gm = GuestMem(pid, base, size, lowmem)
    pfd = gm.fd  # the same read-only /proc/<pid>/mem descriptor also reads the nvidia0 (BAR1) mappings
    print(f"# userd_probe pid={pid} log={log} ram={size >> 20}MiB@{base:#x} lowmem={lowmem >> 30}GiB "
          f"scores={ {hex(k): v for k, v in scores.items()} } live_by_log={len(chans)} "
          f"non_sysmem={[c.key for c in chans if c.aperture != 'Sysmem']}", flush=True)
    tsv = open(a.tsv, "a") if a.tsv else None
    if tsv and tsv.tell() == 0:
        tsv.write("kind\tiso\tepoch\tchannel\tpid\thost\ttoken\tgpa\tput\tget\tdelta\tbehind_s\tstatic_s\tact\tflag"
                  "\treal_get\treal_put\treal_delta\treal_static_s\tmatch_j\n")
    privs = {}                      # addr -> Priv
    co = {}                         # (chan key, addr) -> decayed co-change count
    gn = {}                         # chan key -> decayed change count
    prev_flags = {}                 # last fast sample's change flags
    t_end = None if not a.watch else time.time() + a.watch
    last_full = last_decay = last_maps = 0.0
    first = True
    next_print = 0.0
    while True:
        tick = time.time()
        mono = time.monotonic()
        # ---- bookkeeping that is cheap enough for every fast sample
        st.poll()
        if not a.no_real and tick - last_maps >= 1.0:
            last_maps = tick
            cur = set(store_maps(pid))
            for addr in list(privs):
                if addr not in cur:
                    del privs[addr]
            for addr in cur:
                privs.setdefault(addr, Priv(addr))
        live = [c for c in st.live.values() if usable(c)]
        # ---- fast sample of every guest slot and every private USERD
        gchg, pchg = set(), set()
        for c in live:
            raw = gm.read(c.gpa + USERD_GP_GET, 8)
            if raw is None:
                continue
            get, put = struct.unpack("<II", raw)
            if (put, get) != c.last:
                c.t_change = mono
                c.last = (put, get)
                gchg.add(c.key)
            c.put, c.get = put, get
            if put != get:
                if c.t_behind is None:
                    c.t_behind = mono
            else:
                c.t_behind = None
        for pr in privs.values():
            r = read_priv(pfd, pr.addr)
            if r is None:
                pr.get = pr.put = pr.ref = None
                continue
            ref, get, put = r
            if (get, put) != pr.last:
                pr.t_change = mono
                pr.last = (get, put)
                pchg.add(pr.addr)
            pr.ref, pr.get, pr.put = ref, get, put
            if put != get:
                if pr.t_behind is None:
                    pr.t_behind = mono
            else:
                pr.t_behind = None
        # ---- matcher: a guest slot and a private USERD "co-change" when both changed within two consecutive samples
        g_act = gchg | prev_flags.get("g", set())
        p_act = pchg | prev_flags.get("p", set())
        for k in g_act:
            gn[k] = gn.get(k, 0.0) + 1.0
        for ad in p_act:
            if ad in privs:
                privs[ad].n += 1.0
        for k in g_act:
            for ad in p_act:
                co[(k, ad)] = co.get((k, ad), 0.0) + 1.0
        prev_flags = {"g": gchg, "p": pchg}
        if tick - last_decay >= 1.0:
            last_decay = tick
            for key in list(co):
                co[key] *= 0.97
                if co[key] < 0.05:
                    del co[key]
            for key in list(gn):
                gn[key] *= 0.97
            for pr in privs.values():
                pr.n *= 0.97
        # ---- once per print interval: assignment, flags, output
        if tick >= next_print:
            next_print = tick + a.interval
            ev, st.events = st.events, []
            iso = time.strftime("%H:%M:%S", time.localtime(tick))
            alive = {c.key for c in live}
            for pr in privs.values():
                if pr.chan is not None and pr.chan not in alive:
                    pr.chan = None
            cand = {}
            for (k, ad), v in co.items():
                if k not in alive or ad not in privs:
                    continue
                j = v / max(1e-9, gn.get(k, 0) + privs[ad].n - v)
                cand.setdefault(k, []).append((j, ad))
            bound = {}
            for k, lst in cand.items():
                lst.sort(reverse=True)
                j1, a1 = lst[0]
                j2 = lst[1][0] if len(lst) > 1 else 0.0
                if gn.get(k, 0) >= 5 and j1 >= 0.30 and j1 >= 1.5 * j2:
                    bound[k] = (a1, j1)
            taken = {}
            for k, (ad, j) in sorted(bound.items(), key=lambda kv: -kv[1][1]):
                if ad not in taken:
                    taken[ad] = k
            full = tick - last_full >= 10
            rows, behind, stuck, frozen_n = [], 0, 0, 0
            for c in sorted(live, key=lambda c: c.born_line):
                if c.put is None:
                    continue
                put, get = c.put, c.get
                nz = None
                behind_s = None if c.t_behind is None else mono - c.t_behind
                static_s = None if (c.t_change is None or put == get) else mono - c.t_change
                if first:
                    static_s = 0.0 if put != get else None
                if put != get:
                    behind += 1
                delta = (put - get) % c.entries if c.entries else put - get
                frozen = put != get and static_s is not None and static_s >= a.stuck_after
                is_stuck = frozen and delta > a.lazy_bound
                stuck += is_stuck
                frozen_n += frozen
                ad_j = bound.get(c.key)
                pr = privs.get(ad_j[0]) if ad_j and taken.get(ad_j[0]) == c.key else None
                rg = rp = rd = rs = None
                if pr is not None and pr.get is not None:
                    rg, rp = pr.get, pr.put
                    rd = (rp - rg) % PRIV_ENTRIES
                    rs = None if (pr.t_change is None or rg == rp) else mono - pr.t_change
                a_ = st.act.get(c.host)
                actcol = "-"
                if a_ and a_["state"] == "D":
                    actcol = f"DIS({fmt_age(a_['t'], tick)})"
                elif a_ and a_["state"] == "E":
                    actcol = f"EN({fmt_age(a_['t'], tick)})"
                    if a_["t"] is not None and tick - a_["t"] <= 30:
                        actcol += "<30s"
                flag = "STUCK" if is_stuck else ("frozen" if frozen else ("behind" if put != get else ""))
                if frozen and pr is not None:
                    # the classification the coordinator asked for (only meaningful with a trusted match)
                    if rd == 0:
                        flag += " H-GET?(real==put,guest lags)"
                    else:
                        flag += " REAL-BEHIND(gpu has not consumed)" if rs is not None and rs >= a.stuck_after else ""
                rows.append((c, put, get, delta, behind_s, static_s, actcol, flag, c.key in gchg, rg, rp, rd, rs,
                             ad_j[1] if pr is not None else None))
                if tsv and (put != get or full):
                    tsv.write("S\t%s\t%.3f\t%s\t%s\t%#x\t%#x\t%#x\t%d\t%d\t%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" % (
                        time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(tick)), tick, c.key, c.pid, c.host, c.token,
                        c.gpa, put, get, delta, "" if behind_s is None else "%.1f" % behind_s,
                        "" if static_s is None else "%.1f" % static_s, actcol, flag.strip(),
                        "" if rg is None else rg, "" if rp is None else rp, "" if rd is None else rd,
                        "" if rs is None else "%.1f" % rs, "" if pr is None else "%.2f" % ad_j[1]))
            # mapping-free view: private storages whose GPU GET has not reached PUT for a while
            real_behind = [pr for pr in privs.values() if pr.get is not None and pr.put != pr.get and pr.t_change is not None
                           and mono - pr.t_change >= a.stuck_after]
            for pr in real_behind:
                if tsv:
                    tsv.write("R\t%s\t%.3f\t%#x\t\t\t\t\t\t\t\t\t\t\t\t%d\t%d\t%d\t%.1f\t\n" % (
                        time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(tick)), tick, pr.addr, pr.get, pr.put,
                        (pr.put - pr.get) % PRIV_ENTRIES, mono - pr.t_change))
            if full:
                last_full = tick
            if tsv:
                for t, text in ev:
                    tsv.write("E\t%s\t%.3f\t%s\n" % (time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(t)), t, text))
                tsv.flush()
            disabled = sum(1 for h, x in st.act.items() if x["state"] == "D" and h in st.by_host)
            print(f"--- {iso} live={len(st.live)} privs={len(privs)} matched={len(taken)} behind={behind} "
                  f"frozen={frozen_n} STUCK(>{a.lazy_bound})={stuck} real_behind_frozen={len(real_behind)} "
                  f"disabled_unmatched={disabled}", flush=True)
            shown = [r for r in rows if a.all or r[7] or r[8] or r[6] != "-"]
            if shown:
                print(f"{'channel':24} {'pid':>5} {'host':>8} {'gpa':>11} {'put':>6} {'guest_get':>9} {'dlt':>5} "
                      f"{'behind':>7} {'static':>7} {'real_get':>8} {'real_put':>8} {'rstatic':>7} {'J':>4} {'act':>10} flag")
            for c, put, get, delta, bs, ss, actcol, flag, changed, rg, rp, rd, rs, j in shown:
                print(f"{c.key:24} {str(c.pid):>5} {c.host:>#8x} {c.gpa:>#11x} {put:>6} {get:>9} {delta:>5} "
                      f"{'' if bs is None else '%.1fs' % bs:>7} {'' if ss is None else '%.1fs' % ss:>7} "
                      f"{'' if rg is None else rg:>8} {'' if rp is None else rp:>8} {'' if rs is None else '%.1fs' % rs:>7} "
                      f"{'' if j is None else '%.2f' % j:>4} {actcol:>10} {flag}")
            for pr in real_behind:
                print(f"    REAL-BEHIND private storage {pr.addr:#x}: GPU GET={pr.get} PUT={pr.put} frozen {mono - pr.t_change:.1f}s"
                      f"{'  (matched ' + pr.chan + ')' if pr.chan else ''}")
            for t, text in ev:
                print(f"    event {time.strftime('%H:%M:%S', time.localtime(t))} {text}")
            sys.stdout.flush()
            first = False
            for ad, k in taken.items():
                privs[ad].chan = k
        if a.watch is None or (t_end and time.time() >= t_end):
            break
        if not os.path.exists(f"/proc/{pid}"):
            print("# qemu exited", flush=True)
            break
        try:
            time.sleep(max(0.0, a.fast - (time.time() - tick)))
        except KeyboardInterrupt:
            break


if __name__ == "__main__":
    main()
