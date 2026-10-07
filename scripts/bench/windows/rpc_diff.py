#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""rpc_diff.py -- what a Windows guest asked kf3 for that a Linux guest never did.

usage: rpc_diff.py WINDOWS_QEMU_LOG[,MORE...] LINUX_QEMU_LOG[,MORE...] [--md OUT.md]

Each argument is one or more kf3 QEMU logs (comma-separated: a Windows run restarts QEMU on
every guest reboot, so one run is several logs). Read from each side:
  - `kf3: GSP rpc <Function> seq=N`           every RPC by function (incl. UNSERVICED)
  - `kf-rm: rpc-trace fn=F <Function> ... cmd=0x... | class=0x... result=0x...`  (KF3_RPC_TRACE=1)
  - `kf-rm: RPC-REFUSED #n <reason>`          named refusals
  - the LAST `gsp_refusals[...]` heartbeat     refused controls/classes with counts
  - `kf-rm: fn 1 SET_GUEST_SYSTEM_INFO: ...`   and `kf-gsp: fn 72 ... bGspNocatEnabled=...`

Ranks by KIND (function, then control, then class), never by index. Box output is data.
"""

import collections
import re
import sys

RPC = re.compile(r"kf3: GSP rpc (\w+)")
UNSERV = re.compile(r"UNSERVICED Unserviced \{ code: (\d+)")
TRACE = re.compile(r"kf-rm: rpc-trace fn=(\d+) (\w+) seq=\d+ ?(cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?[^\n]*? result=(\S+)")
REFUSED = re.compile(r"kf-rm: RPC-REFUSED #\d+ (.*?) ⇒")
LEDGER = re.compile(r"gsp_refusals\[total=(\d+) distinct=(\d+) \[([^\]]*)\]")
FN1 = re.compile(r"kf-rm: fn 1 SET_GUEST_SYSTEM_INFO: (.*)")
FN72 = re.compile(r"kf-gsp: fn 72 GSP_SET_SYSTEM_INFO .*?: (bGspNocatEnabled=\S+)")
OTHER = re.compile(r"(kf-rm: SET_GUEST_SYSTEM_INFO refused: .*|kf-rm: the guest says .*|kf-rm: guest driver RE-SELECTED .*)")


def read(paths):
    d = {
        "rpc": collections.Counter(), "unserviced": collections.Counter(),
        "ctrl": collections.defaultdict(collections.Counter),
        "cls": collections.defaultdict(collections.Counter),
        "refused": collections.Counter(), "ledger": None, "fn1": [], "fn72": [], "other": [],
        "order": [],
    }
    for p in paths:
        with open(p, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = RPC.search(line)
                if m:
                    d["rpc"][m.group(1)] += 1
                    if m.group(1) not in d["order"]:
                        d["order"].append(m.group(1))
                m = UNSERV.search(line)
                if m:
                    d["unserviced"][int(m.group(1))] += 1
                m = TRACE.search(line)
                if m:
                    if m.group(4):
                        d["ctrl"][m.group(4)][m.group(6)] += 1
                    elif m.group(5):
                        d["cls"][m.group(5)][m.group(6)] += 1
                m = REFUSED.search(line)
                if m:
                    d["refused"][m.group(1)] += 1
                m = LEDGER.search(line)
                if m:
                    d["ledger"] = (int(m.group(1)), int(m.group(2)), m.group(3).split())
                m = FN1.search(line)
                if m and m.group(1) not in d["fn1"]:
                    d["fn1"].append(m.group(1))
                m = FN72.search(line)
                if m and m.group(1) not in d["fn72"]:
                    d["fn72"].append(m.group(1))
                m = OTHER.search(line)
                if m and m.group(1) not in d["other"]:
                    d["other"].append(m.group(1)[:400])
    return d


def fmt_results(c):
    return ", ".join(f"{r}x{n}" for r, n in sorted(c.items()))


def main(argv):
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    md = None
    if "--md" in argv:
        i = argv.index("--md")
        md = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    w, l = read(argv[1].split(",")), read(argv[2].split(","))
    out = []
    out.append("## Identity at the handshake\n")
    for side, d in (("Windows", w), ("Linux", l)):
        for x in d["fn72"]:
            out.append(f"- {side} fn 72: `{x}`")
        for x in d["fn1"]:
            out.append(f"- {side} fn 1: `{x}`")
        for x in d["other"]:
            out.append(f"- {side}: `{x}`")
    out.append("\n## RPC functions (count Windows / Linux)\n")
    out.append("| function | Windows | Linux | note |\n|---|---|---|---|")
    for fn in sorted(set(w["rpc"]) | set(l["rpc"]), key=lambda k: (-w["rpc"][k], k)):
        note = "Windows only" if fn in w["rpc"] and fn not in l["rpc"] else ("Linux only" if fn not in w["rpc"] else "")
        out.append(f"| {fn} | {w['rpc'][fn]} | {l['rpc'][fn]} | {note} |")
    out.append(f"\nWindows first-seen order: {' → '.join(w['order'])}")
    out.append(f"\nLinux first-seen order: {' → '.join(l['order'])}")
    out.append("\n## UNSERVICED codes\n")
    out.append("| code | Windows | Linux |\n|---|---|---|")
    for c in sorted(set(w["unserviced"]) | set(l["unserviced"])):
        out.append(f"| {c} | {w['unserviced'][c]} | {l['unserviced'][c]} |")
    for key, title in (("ctrl", "GSP_RM_CONTROL ids"), ("cls", "GSP_RM_ALLOC classes")):
        only = sorted(set(w[key]) - set(l[key]))
        both = sorted(set(w[key]) & set(l[key]))
        out.append(f"\n## {title}: Windows only ({len(only)}), shared ({len(both)}), Linux only ({len(set(l[key]) - set(w[key]))})\n")
        out.append("| id | Windows count | kayfabe answered (Windows) |\n|---|---|---|")
        for k in only:
            out.append(f"| {k} | {sum(w[key][k].values())} | {fmt_results(w[key][k])} |")
        diff_answer = [k for k in both if set(w[key][k]) != set(l[key][k])]
        if diff_answer:
            out.append(f"\nShared ids answered differently ({len(diff_answer)}):\n")
            out.append("| id | Windows | Linux |\n|---|---|---|")
            for k in diff_answer:
                out.append(f"| {k} | {fmt_results(w[key][k])} | {fmt_results(l[key][k])} |")
    out.append("\n## Refusal ledger (last heartbeat)\n")
    for side, d in (("Windows", w), ("Linux", l)):
        if d["ledger"]:
            t, n, rows = d["ledger"]
            out.append(f"- {side}: total={t} distinct={n}")
    if w["ledger"] and l["ledger"]:
        wl = {r.split("=")[0] for r in w["ledger"][2]}
        ll = {r.split("=")[0] for r in l["ledger"][2]}
        only = [r for r in w["ledger"][2] if r.split("=")[0] not in ll]
        out.append(f"- refused for Windows and never for Linux ({len(only)}): " + " ".join(f"`{r}`" for r in only))
        out.append(f"- refused for Linux and never for Windows: {len(ll - wl)}")
    out.append("\n## Named refusals (RPC-REFUSED), Windows only\n")
    for r, n in w["refused"].most_common():
        if r not in l["refused"]:
            out.append(f"- `{r}` x{n}")
    text = "\n".join(out) + "\n"
    if md:
        with open(md, "w", encoding="utf-8") as f:
            f.write(text)
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
