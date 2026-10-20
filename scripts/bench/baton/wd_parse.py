#!/usr/bin/env python3
"""wd_parse.py - parse and classify the baton self-watchdog dump (baton --wd-out FILE).

Format (one episode per starvation detection; see baton.c watchdog_dump):
  WD_BEGIN label= pid= t= epoch= idle_ms= iters= launched= sync_active=
  HB label= age_ms= iters=            (other clients' heartbeat files)
  STREAMQ rc= name=                   (cuStreamQuery from the watchdog thread)
  TASK tid= comm= state= wchan= syscall= args=
  STACK tid= ... ENDSTACK             (/proc/self/task/<tid>/stack lines)
  FD n -> target
  PROC path ... ENDPROC               (readable driver stat files)
  WD_END label=
  WD_RECOVER label= t= gap_ms=

Usage: python3 -I wd_parse.py FILE...   prints a summary of every episode.
"""
import re
import sys

_SYSCALLS = {0: "read", 1: "write", 7: "poll", 16: "ioctl", 23: "select", 35: "nanosleep",
             202: "futex", 230: "clock_nanosleep", 232: "epoll_wait", 270: "pselect6",
             271: "ppoll", 281: "epoll_pwait", 441: "epoll_pwait2", 61: "wait4", 34: "pause"}


def syscall_name(nr):
    return _SYSCALLS.get(nr, "sys_%d" % nr)


def _kv(line):
    out = {}
    for m in re.finditer(r"(\w+)=(\S+)", line):
        out[m.group(1)] = m.group(2)
    return out


def _num(v):
    try:
        return int(v)
    except ValueError:
        try:
            return float(v)
        except ValueError:
            return v


def parse(text):
    eps = []
    cur = None
    mode = None  # None | ("stack", tid) | ("proc", path)
    for raw in text.splitlines():
        line = raw.rstrip("\r")
        if mode is not None:
            kind, key = mode
            if (kind == "stack" and line == "ENDSTACK") or (kind == "proc" and line == "ENDPROC"):
                mode = None
            elif cur is not None:
                (cur["stacks"][key] if kind == "stack" else cur["procs"][key]).append(line)
            continue
        if line.startswith("WD_BEGIN "):
            if cur is not None:
                cur["truncated"] = True
                eps.append(cur)
            cur = {k: _num(v) for k, v in _kv(line).items()}
            cur.update(tasks=[], stacks={}, fds={}, procs={}, hb={}, streamq=None, streamq_name=None,
                       truncated=False, recover_gap_ms=None)
        elif line.startswith("WD_RECOVER"):
            kv = _kv(line)
            for e in reversed(eps):
                if e["label"] == kv.get("label"):
                    e["recover_gap_ms"] = float(kv["gap_ms"])
                    break
        elif cur is None:
            continue
        elif line.startswith("WD_END"):
            eps.append(cur)
            cur = None
        elif line.startswith("HB "):
            kv = {k: _num(v) for k, v in _kv(line).items()}
            cur["hb"][kv["label"]] = kv
        elif line.startswith("STREAMQ "):
            kv = _kv(line)
            cur["streamq"] = int(kv["rc"])
            cur["streamq_name"] = kv.get("name")
        elif line.startswith("TASK "):
            kv = {k: _num(v) for k, v in _kv(line).items()}
            cur["tasks"].append(kv)
        elif line.startswith("STACK "):
            tid = int(_kv(line)["tid"])
            cur["stacks"][tid] = []
            mode = ("stack", tid)
        elif line.startswith("PROC "):
            path = line[5:].strip()
            cur["procs"][path] = []
            mode = ("proc", path)
        elif line.startswith("FD "):
            m = re.match(r"FD (\d+) -> (.*)", line)
            if m:
                cur["fds"][int(m.group(1))] = m.group(2)
    if cur is not None:
        cur["truncated"] = True
        eps.append(cur)
    return eps


def classify(ep):
    main = ep["tasks"][0] if ep["tasks"] else {}
    nr = main.get("syscall")
    wait = "tid=%s %s" % (main.get("tid"), syscall_name(nr) if isinstance(nr, int) else "running")
    if main.get("wchan"):
        wait += " wchan=%s" % main["wchan"]
    if isinstance(nr, int) and nr in (16, 0, 1) and main.get("args"):
        try:
            fd = int(str(main["args"]).split(",")[0], 16)
            if fd in ep["fds"]:
                wait += " fd=%d(%s)" % (fd, ep["fds"][fd])
        except ValueError:
            pass
    q = ep["streamq"]
    if q is None:
        kind = "UNKNOWN"
    elif q == 0:
        kind = "LOST_WAKEUP"  # work finished (stream query says so) but the waiter was not woken
    elif q == 600:
        kind = "GPU_NOT_COMPLETE"
    else:
        kind = "QUERY_ERROR"
    return {"kind": kind, "main_wait": wait}


def summarize(eps):
    lines = []
    for e in eps:
        v = classify(e)
        others = ",".join("%s age %.0f ms" % (k, h.get("age_ms", -1)) for k, h in sorted(e["hb"].items()))
        rec = "recovered after %.0f ms" % e["recover_gap_ms"] if e["recover_gap_ms"] is not None else "no recovery logged"
        lines.append("%s t=%.1fs idle=%.0f ms %s | main: %s | streamq=%s | others: %s | %s" % (
            e["label"], e["t"], e["idle_ms"], v["kind"], v["main_wait"], e["streamq_name"], others or "-", rec))
    return "\n".join(lines)


if __name__ == "__main__":
    for p in sys.argv[1:]:
        with open(p, errors="replace") as f:
            print(summarize(parse(f.read())))
