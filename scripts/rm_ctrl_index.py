#!/usr/bin/env python3
"""rm_ctrl_index.py — resolve an RM control id to everything `ogkm-580` says about it.

Task #179.  This is the evidence-gathering half of the **data-vs-ACT** classification: it
finds, mechanically and with a citation, the three things a human then has to read.

For each control id it reports

1. the SDK `#define NVxxxx_CTRL_CMD_<NAME> (0x…)` and its file:line
   — `src/common/sdk/nvidia/inc/ctrl/**`;
2. the params `typedef struct` named by the FINN `…_MESSAGE_ID` comment on that `#define`,
   printed in full with its file:line;
3. the **NVOC export table entry** — `methodId` → handler function name, `paramSize`
   expression and the `RMCTRL_FLAGS_*` word — from `src/nvidia/generated/g_*_nvoc.c`.

★ (3) is the load-bearing one and the reason this is a script and not a grep.  NVOC emits

    /*flags=*/      0x5c0c0u,
    /*methodId=*/   0x20800a1cu,
    /*paramSize=*/  sizeof(NV2080_CTRL_INTERNAL_MEMSYS_GET_STATIC_CONFIG_PARAMS),
    /*func=*/       "subdeviceCtrlCmdMemSysGetStaticConfig"

so the id→handler→params mapping is *generated from the same source that dispatches it*,
not transcribed.  Two flags are directly relevant to this classification:

- `RMCTRL_FLAGS_CACHEABLE 0x400` — `ogkm-580:
  src/nvidia/inc/kernel/rmapi/control.h:255-260`, *"the control output does not depend on
  the input parameters and can be cached on the receiving end"*.  That is NVIDIA declaring
  the control to be **DATA**: a pure function of the device, safe to serve from a table.
- `RMCTRL_FLAGS_ROUTE_TO_PHYSICAL 0x40` — `…control.h:230-233`; the control is forwarded
  to GSP rather than serviced by CPU-RM, i.e. it is one a replay must answer at all.

⊘ The **absence** of `CACHEABLE` is not evidence of ACT.  Most of RM predates the flag and
plenty of pure-`[out]` controls do not carry it.  It is a one-way witness: present ⇒ DATA;
absent ⇒ read the handler.  Do not invert it.

⊘ A control's **name** decides nothing.  `CE_UPDATE_CLASS_DB` sounds like an act and
`GET_FAULT_METHOD_BUFFER_SIZE` sounds like data; only the params struct and the CPU-side
consumer settle it.

Usage:
    scripts/rm_ctrl_index.py 0x20800a6c 0xa06f0103 …
    scripts/rm_ctrl_index.py --tsv OUT.tsv 0x…      # one row per id, no struct bodies
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

DEFAULT_OGKM = Path("/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04")

DEF_RE = re.compile(r"^\s*#define\s+(NV\w*_CTRL_CMD_\w+)\s+\(?(0x[0-9a-fA-F]{6,8})U?\)?(.*)$")
MSGID_RE = re.compile(r"\|\s*(\w+)_MESSAGE_ID")
EXPORT_RE = re.compile(
    r"/\*flags=\*/\s*(0x[0-9a-fA-F]+)u,\s*\n"
    r"\s*/\*accessRight=\*/\s*(0x[0-9a-fA-F]+)u,\s*\n"
    r"\s*/\*methodId=\*/\s*(0x[0-9a-fA-F]+)u,\s*\n"
    r"\s*/\*paramSize=\*/\s*([^,]+),\s*\n"
    r"\s*/\*pClassInfo=\*/[^\n]*\n"
    r"(?:[^\n]*\n)*?"
    r"\s*/\*func=\*/\s*\"([^\"]+)\"")

# ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:170-347
RMCTRL_FLAGS = [
    (0x000000001, "NO_GPUS_LOCK"), (0x000000002, "NO_GPUS_ACCESS"),
    (0x000000004, "PRIVILEGED"), (0x000000008, "NON_PRIVILEGED"),
    (0x000000010, "GPU_LOCK_DEVICE_ONLY"),
    (0x000000020, "PRIVILEGED_IF_RS_ACCESS_DISABLED"),
    (0x000000040, "ROUTE_TO_PHYSICAL"), (0x000000080, "INTERNAL"),
    (0x000000100, "API_LOCK_READONLY"), (0x000000200, "ROUTE_TO_VGPU_HOST"),
    (0x000000400, "CACHEABLE"), (0x000000800, "COPYOUT_ON_ERROR"),
    (0x000001000, "ALLOW_WITHOUT_SYSMEM_ACCESS"),
    (0x000004000, "CPU_PLUGIN_FOR_SRIOV"), (0x000008000, "CPU_PLUGIN_FOR_LEGACY"),
    (0x000010000, "GSP_PLUGIN_FOR_VGPU_GSP"), (0x000020000, "CACHEABLE_BY_INPUT"),
    (0x000040000, "PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST"),
    (0x000080000, "DUAL_CLIENT_LOCK"), (0x000100000, "RM_TEST_ONLY_CODE"),
    (0x000200000, "ALL_CLIENT_LOCK"), (0x000400000, "NO_API_LOCK"),
    (0x000800000, "PERSISTENT_CACHEABLE"),
]


def decode_flags(v: int) -> str:
    names = [n for b, n in RMCTRL_FLAGS if v & b]
    left = v & ~sum(b for b, _ in RMCTRL_FLAGS)
    if left:
        names.append("UNKNOWN_0x%x" % left)
    return ",".join(names) or "NONE"


class Index:
    def __init__(self, ogkm: Path):
        self.ogkm = ogkm
        self.sdk = ogkm / "src/common/sdk/nvidia/inc"
        self.gen = ogkm / "src/nvidia/generated"
        self._defs: dict[int, list] | None = None
        self._exports: dict[int, list] | None = None
        self._sdk_text: dict[Path, str] = {}

    def _text(self, p: Path) -> str:
        if p not in self._sdk_text:
            self._sdk_text[p] = p.read_text(errors="replace")
        return self._sdk_text[p]

    @property
    def defs(self):
        if self._defs is None:
            self._defs = {}
            for p in sorted(self.sdk.rglob("*.h")):
                for i, line in enumerate(self._text(p).splitlines(), 1):
                    m = DEF_RE.match(line)
                    if not m:
                        continue
                    mm = MSGID_RE.search(m.group(3))
                    self._defs.setdefault(int(m.group(2), 16), []).append(
                        (m.group(1), str(p.relative_to(self.ogkm)), i,
                         mm.group(1) if mm else None))
        return self._defs

    @property
    def exports(self):
        if self._exports is None:
            self._exports = {}
            for p in sorted(self.gen.glob("g_*_nvoc.c")):
                txt = p.read_text(errors="replace")
                for m in EXPORT_RE.finditer(txt):
                    line = txt[:m.start()].count("\n") + 1
                    self._exports.setdefault(int(m.group(3), 16), []).append(dict(
                        flags=int(m.group(1), 16), access=m.group(2),
                        paramsize=m.group(4).strip(), func=m.group(5),
                        where="%s:%d" % (p.relative_to(self.ogkm), line)))
        return self._exports

    def struct(self, name: str):
        pat = re.compile(r"typedef struct\s+" + re.escape(name) + r"\s*\{.*?\n\}\s*"
                         + re.escape(name) + r"\s*;", re.S)
        for p in sorted(self.sdk.rglob("*.h")):
            m = pat.search(self._text(p))
            if m:
                return (str(p.relative_to(self.ogkm)),
                        self._text(p)[:m.start()].count("\n") + 1, m.group(0))
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="+")
    ap.add_argument("--ogkm", default=str(DEFAULT_OGKM))
    ap.add_argument("--tsv", help="one row per id, no struct bodies")
    args = ap.parse_args()

    idx = Index(Path(args.ogkm))
    rows = []
    for a in args.ids:
        cid = int(a, 16)
        ds = idx.defs.get(cid, [])
        exps = idx.exports.get(cid, [])
        macro, where, msgid = (ds[0][0], "%s:%d" % (ds[0][1], ds[0][2]), ds[0][3]) \
            if ds else ("(none)", "(none)", None)
        st = None
        sname = None
        if msgid:
            for cand in (msgid, msgid + "_PARAMS"):
                st = idx.struct(cand)
                if st:
                    sname = cand
                    break
        e = exps[0] if exps else None
        rows.append((cid, macro, where, sname, st, e))

        if not args.tsv:
            print("=" * 78)
            print("0x%08x  %s" % (cid, macro))
            print("  define  : ogkm-580: %s" % where)
            if e:
                print("  handler : %s" % e["func"])
                print("            ogkm-580: %s" % e["where"])
                print("  flags   : 0x%x  [%s]" % (e["flags"], decode_flags(e["flags"])))
                print("  paramSz : %s" % e["paramsize"])
            else:
                print("  handler : *** NO NVOC EXPORT — not dispatched by CPU-RM ***")
            if st:
                print("  params  : %s   ogkm-580: %s:%d" % (sname, st[0], st[1]))
                for ln in st[2].splitlines():
                    print("      " + ln)
            elif msgid:
                print("  params  : %s  *** struct not found ***" % msgid)
            else:
                print("  params  : *** no FINN MESSAGE_ID on the #define ***")

    if args.tsv:
        with open(args.tsv, "w") as fh:
            fh.write("id\tmacro\tdefine_at\thandler\thandler_at\tflags\tflag_names\t"
                     "params_struct\tparams_at\n")
            for cid, macro, where, sname, st, e in rows:
                fh.write("0x%08x\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" % (
                    cid, macro, where,
                    e["func"] if e else "", e["where"] if e else "",
                    "0x%x" % e["flags"] if e else "",
                    decode_flags(e["flags"]) if e else "",
                    sname or "", "%s:%d" % (st[0], st[1]) if st else ""))
        print("wrote %s" % args.tsv)


if __name__ == "__main__":
    main()
