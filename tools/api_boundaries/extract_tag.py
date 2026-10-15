#!/usr/bin/env python3
"""Per-tag API-boundary signatures from an open-gpu-kernel-modules checkout.

Library + CLI.  CLI:  extract_tag.py <ogkm-tree> <tag> <served.json> <out.json>

The tree is a (sparse) checkout of one release tag.  Nothing is compiled and nothing outside the
tree is read.  Signatures are text signatures: comments are stripped and whitespace collapsed, so
a comment-only edit is not a change.  A struct's signature is the hash of its definition together
with every struct / #define it references (transitive closure over identifiers), so a changed
array bound or a changed nested member shows up in the parent.

Output  {"tag": ..., "items": {item: sig12}, "detail": {item: {...}}}  with item namespaces:
  ctrl:0xIIIIIIII      wire signature of an RM control (presence+exporting class, params-struct
                       closure hash, privilege/route flag bits); detail has the components
  ctrlall:0x...        same + the other RMCTRL flag names and access right (locking, caching ...)
  allocstruct:NAME     allocation / NVOS parameter struct closure hash
  classdef:NAME        value of the class define for every class named in any chip class list
  classhdr:0xID        normalised class header cl<id>.h
  chipclasses:CHIP     per-chip engine class list (g_gpu_class_list.c)
  mcarch:NAME          NV2080_CTRL_MC_ARCH_INFO_{ARCHITECTURE,IMPLEMENTATION}_* define
  rpcfn:UNIT:NAME      GSP/RM RPC function / event enum ordinal
  gsp:FILE:def:NAME    #define in a GSP / message-queue / WPR / FSP-facing header
  gsp:FILE:struct:NAME struct / union / enum closure hash in those headers and in the g_rpc files
"""
import hashlib
import json
import os
import re
import sys

# ----------------------------------------------------------------------------- text helpers
_CMT = re.compile(r"/\*.*?\*/|//[^\n]*", re.S)
_WS = re.compile(r"\s+")
_IDENT = re.compile(r"[A-Za-z_]\w*")
_BRACE = re.compile(r"[{}]")


def strip_c(s):
    # replace comments by a space but keep newlines so line-based #define parsing still works
    return _CMT.sub(lambda m: " " + "\n" * m.group(0).count("\n"), s)


def norm(s):
    return _WS.sub(" ", s).strip()


def h12(s):
    if isinstance(s, str):
        s = s.encode()
    return hashlib.sha1(s).hexdigest()[:12]


def read(p):
    with open(p, "rb") as f:
        return f.read().decode("utf-8", "replace")


# ----------------------------------------------------------------------------- C parsing
_SU = re.compile(r"\b(typedef\s+)?(struct|union|enum)\b\s*([A-Za-z_]\w*)?\s*(?:NV_ALIGN\w*\([^)]*\)\s*)?\{")
_DEF = re.compile(r"^[ \t]*#[ \t]*define[ \t]+([A-Za-z_]\w*)(\([^)\n]*\))?[ \t]*(.*)$", re.M)
_FINN = re.compile(
    r"#\s*define\s+(NV\w+_CTRL_CMD_\w+)\s+\(?\s*(0x[0-9a-fA-F]+)[uU]?\s*\)?[^\n]*?\|\s*(NV\w+_MESSAGE_ID)\s*\"")


def parse_c(raw):
    """-> (structs{name: normtext}, defines{name: normbody}, finn{ctrlname: structname}, ids{ctrlname: id})."""
    finn, ids = {}, {}
    for m in _FINN.finditer(raw):
        finn[m.group(1)] = m.group(3)[: -len("_MESSAGE_ID")] if m.group(3).endswith("_MESSAGE_ID") else m.group(3)
        ids[m.group(1)] = int(m.group(2), 16)
    t = strip_c(raw)
    t = re.sub(r"\\[ \t]*\n", " ", t)  # line continuations
    defines = {}
    for m in _DEF.finditer(t):
        defines[m.group(1) + (m.group(2) or "")] = norm(m.group(3))
    structs = {}
    for m in _SU.finditer(t):
        start = m.start()
        depth, pos = 1, m.end()
        while depth:
            b = _BRACE.search(t, pos)
            if not b:
                break
            depth += 1 if b.group(0) == "{" else -1
            pos = b.end()
        if depth:
            continue
        semi = t.find(";", pos)
        if semi < 0:
            continue
        tail = t[pos:semi]
        text = norm(t[start:semi + 1])
        names = set()
        if m.group(3):
            names.add(m.group(3))
        if m.group(1):
            for part in tail.split(","):
                ids_ = _IDENT.findall(part)
                if ids_:
                    names.add(ids_[-1])
        for n in names:
            structs.setdefault(n, text)
    # typedef alias:  typedef NV2080_CTRL_A NV2080_CTRL_B;
    for m in re.finditer(r"\btypedef\s+(?:struct\s+)?([A-Za-z_]\w*)\s+([A-Za-z_]\w*)\s*;", t):
        structs.setdefault(m.group(2), "alias " + m.group(1))
    return structs, defines, finn, ids


# rpc_generic_union lists EVERY rpc payload struct; following it from the message header would make the
# envelope change whenever any payload changes.  The envelope is hashed without the payload union.
NO_FOLLOW = {"rpc_generic_union"}


class Closure:
    def __init__(self, structs, defines):
        self.structs, self.defines = structs, defines
        self.memo = {}

    def sig(self, name):
        if name in self.memo:
            return self.memo[name]
        seen_s, seen_d = set(), set()
        stack = [name]
        parts = []
        while stack:
            n = stack.pop()
            if n in seen_s or n in seen_d:
                continue
            if n in self.structs:
                seen_s.add(n)
                txt = self.structs[n]
                parts.append("S " + n + "=" + txt)
                stack.extend(i for i in _IDENT.findall(txt) if i != n and i not in NO_FOLLOW)
            elif n in self.defines and not n.endswith("_H_") and not n.startswith("_"):
                seen_d.add(n)
                txt = self.defines[n]
                parts.append("D " + n + "=" + txt)
                stack.extend(_IDENT.findall(txt))
        r = (h12("\n".join(sorted(parts))), len(seen_s), len(seen_d)) if seen_s else (None, 0, 0)
        self.memo[name] = r
        return r


_EXPORT = re.compile(
    r"/\*flags=\*/\s*(?P<flags>[^,]+?),\s*/\*accessRight=\*/\s*(?P<ar>[^,]+?),\s*/\*methodId=\*/\s*"
    r"(?P<id>0x[0-9a-fA-F]+)[uU]?,\s*/\*paramSize=\*/\s*(?P<size>[^\n]+?),\s*/\*pClassInfo=\*/\s*"
    r"&\(__nvoc_class_def_(?P<cls>\w+)\.classInfo\)(?:[^\"{}]{0,160}?/\*func=\*/\s*\"(?P<func>\w+)\")?")


def num(s):
    s = s.strip().rstrip("uUlL")
    try:
        return int(s, 0)
    except ValueError:
        return None


def parse_exports(raw):
    out = []
    for m in _EXPORT.finditer(raw):
        out.append((int(m.group("id"), 16), m.group("cls"), num(m.group("flags")),
                    m.group("ar").strip().rstrip("uU"), norm(m.group("size")), m.group("func") or ""))
    return out


# RMCTRL_FLAGS are decoded by NAME with the tag's own control.h: the bit values were renumbered in
# R555 (NON_PRIVILEGED 0x10 -> 0x8, ROUTE_TO_PHYSICAL 0x200 -> 0x40, ...), so raw masks are not comparable.
# Names that change who may call, or where the call is routed (the "wire" view); every other name is
# locking / caching / behaviour (only in the "all flags" view).
# ROUTE_TO_VGPU_HOST is the vGPU-host plugin routing (neither kayfabe nor a GeForce Windows driver is a
# vGPU host), so it is kept with the "other" names.
V_NAMES = {"PRIVILEGED", "NON_PRIVILEGED", "PRIVILEGED_IF_RS_ACCESS_DISABLED", "INTERNAL", "ROUTE_TO_PHYSICAL"}


def decode_flags(fl, fl_map):
    """-> (visibility_names, other_names) for numeric flags `fl` under this tag's RMCTRL_FLAGS_* values."""
    if fl is None:
        return ["?"], ["?"]
    names = sorted(n for n, v in fl_map.items() if fl & v == v)
    rest = fl
    for n in names:
        rest &= ~fl_map[n]
    vis = [n for n in names if n in V_NAMES]
    oth = [n for n in names if n not in V_NAMES]
    if rest:
        oth.append("UNNAMED_0x%x" % rest)
    return vis, oth


GSP_HDRS = [
    "gsp_fw_wpr_meta.h", "gsp_fw_sr_meta.h", "gspifpub.h", "gsp_proxy_reg.h", "gsp_init_args.h",
    "message_queue.h", "message_queue_priv.h", "gsp_static_config.h", "gsp_fw_heap.h", "msgq.h",
    "msgq_priv.h", "libos_init_args.h", "rmgspseq.h", "rmlsfm.h", "rmifrif.h", "rmflcnbl.h",
    "rmRiscvUcode.h", "flcnifcmn.h", "nv-firmware.h", "rpc_headers.h", "g_rpc-message-header.h",
    "g_rpc-structures.h", "kern_fsp.h", "kern_fsp_cot_payload.h", "kern_fsp_retval.h",
    "fsp_emem_channels.h", "fsp_tnvl_rpc.h", "fsp_caps_query_rpc.h", "fsp_clock_boost_rpc.h",
    "nvdm_format.h", "mctp_format.h",
]
GSP_STRUCT_ONLY_C = ["kernel_gsp_fwsec.c", "kernel_gsp_booter.c", "kernel_gsp_booter_tu102.c"]


class Cache:
    """Parse results keyed by content hash, shared across tags in one process."""

    def __init__(self):
        self.c = {}
        self.e = {}

    def parse(self, path):
        raw = read(path)
        k = h12(raw)
        if k not in self.c:
            self.c[k] = parse_c(raw)
        return k, raw

    def exports(self, path):
        raw = read(path)
        k = h12(raw)
        if k not in self.e:
            self.e[k] = parse_exports(raw)
        return self.e[k]


def walk(root, pred):
    for dp, _dn, fn in os.walk(root):
        for f in fn:
            if pred(dp, f):
                yield os.path.join(dp, f)


def extract(tree, tag, served, cache):
    src = os.path.join(tree, "src")
    items, detail = {}, {}
    # ---- parse everything
    structs, defines, finn, ctrl_ids = {}, {}, {}, {}
    sdk = os.path.join(src, "common/sdk/nvidia/inc")
    per_file = {}
    files = list(walk(sdk, lambda dp, f: f.endswith(".h")))
    files += list(walk(src, lambda dp, f: f in GSP_HDRS and not dp.startswith(sdk)))
    files += list(walk(os.path.join(src, "nvidia/generated"), lambda dp, f: f.startswith("g_rpc") and f.endswith(".h")))
    files += list(walk(src, lambda dp, f: f == "control.h" and dp.endswith("rmapi")))
    seen = set()
    for p in sorted(set(files)):
        k, _ = cache.parse(p)
        s, d, fi, ci = cache.c[k]
        base = os.path.basename(p)
        if base in GSP_HDRS:
            key = base if base not in per_file else base + "@" + os.path.relpath(p, tree)
            per_file[key] = (s, d)
        if k in seen:
            # identical content seen already in this tag (duplicate headers): merged once
            continue
        seen.add(k)
        for n, t in s.items():
            structs.setdefault(n, t)
        for n, t in d.items():
            defines.setdefault(n, t)
        finn.update(fi)
        ctrl_ids.update(ci)
    for p in walk(src, lambda dp, f: f in GSP_STRUCT_ONLY_C):
        k, _ = cache.parse(p)
        base = os.path.basename(p)
        per_file[base] = (cache.c[k][0], {})
        for n, t in cache.c[k][0].items():
            structs.setdefault(n, t)
    clo = Closure(structs, defines)
    fl_map = {}
    for n, b in defines.items():
        if n.startswith("RMCTRL_FLAGS_") and re.fullmatch(r"0x[0-9a-fA-F]+", b) and int(b, 16):
            fl_map[n[len("RMCTRL_FLAGS_"):]] = int(b, 16)

    # ---- controls: id -> names (header) and exports (nvoc tables)
    id_names = {}
    for n, body in defines.items():
        if "_CTRL_CMD_" in n and "(" not in n:
            m = re.search(r"0x([0-9a-fA-F]+)", body)
            if m:
                id_names.setdefault(int(m.group(1), 16), []).append(n)
    for n, i in ctrl_ids.items():
        id_names.setdefault(i, [])
        if n not in id_names[i]:
            id_names[i].append(n)
    exports = {}
    for p in sorted(walk(os.path.join(src, "nvidia/generated"), lambda dp, f: f.startswith("g_") and f.endswith("_nvoc.c"))):
        for (i, cls, fl, ar, size, func) in cache.exports(p):
            exports.setdefault(i, []).append((cls, fl, ar, size, func))

    want = set(int(x, 16) for x in served["ctrl_ids"])
    nm2id = {}
    for i, ns in id_names.items():
        for n in ns:
            nm2id.setdefault(n, set()).add(i)
    for n in served["ctrl_names"]:
        want.update(nm2id.get(n, ()))
    for i in sorted(want):
        names = sorted(id_names.get(i, []))
        ex = sorted(set(exports.get(i, [])))
        comps = []
        compsall = []
        det = {"names": names, "exports": []}
        for (cls, fl, ar, size, func) in ex:
            m = re.search(r"sizeof\(\s*(\w+)\s*\)", size)
            sname = m.group(1) if m else None
            sh = clo.sig(sname)[0] if sname else None
            vis, oth = decode_flags(fl, fl_map)
            det["exports"].append({"cls": cls, "flags": fl, "priv": "+".join(vis) or "KERNEL_PRIVILEGED",
                                   "other_flags": "+".join(oth), "ar": ar, "size": size, "struct": sname,
                                   "shash": sh, "func": func})
            comps.append("%s|%s|%s|%s" % (cls, size, sh, "+".join(vis)))
            compsall.append("%s|%s|%s|%s|%s|%s" % (cls, size, sh, "+".join(vis), "+".join(oth), ar))
        if not ex and names:
            sname = finn.get(names[0])
            sh = clo.sig(sname)[0] if sname else None
            det["header_only"] = {"struct": sname, "shash": sh}
            comps.append("HDR|%s" % sh)
            compsall.append("HDR|%s" % sh)
        k = "0x%08x" % i
        if comps:
            items["ctrl:" + k] = h12("\n".join(comps))
            items["ctrlall:" + k] = h12("\n".join(compsall))
            detail["ctrl:" + k] = det
        else:
            items["ctrl:" + k] = "ABSENT"
            items["ctrlall:" + k] = "ABSENT"

    # ---- allocation / NVOS parameter structs
    for n in served["alloc_structs"]:
        items["allocstruct:" + n] = clo.sig(n)[0] or "ABSENT"

    # ---- chip class lists and arch ids
    lp = os.path.join(src, "nvidia/generated/g_gpu_class_list.c")
    classnames = set()
    if os.path.exists(lp):
        t = strip_c(read(lp))
        for m in re.finditer(r"static const (?:CLASSDESCRIPTOR|NvU32)\s+(\w+?)ClassDescriptorList\s*\[\s*\]\s*=\s*\{(.*?)\};", t, re.S):
            chip = m.group(1)
            body = norm(m.group(2))
            typ = "full" if "ENG_" in body else "ids"
            items["chipclasses:%s:%s" % (chip, typ)] = h12(body)
            if typ == "full":
                for cm in re.finditer(r"\{\s*(\w+)\s*,", m.group(2)):
                    classnames.add(cm.group(1))
            else:
                for cm in re.finditer(r"\b([A-Za-z_]\w*)\b", m.group(2)):
                    classnames.add(cm.group(1))
    chdir = os.path.join(sdk, "class")
    hdrs = {}
    for n in sorted(classnames):
        body = defines.get(n)
        m = re.search(r"0x([0-9a-fA-F]+)", body or "")
        if not m:
            continue
        cid = int(m.group(1), 16)
        items["classdef:" + n] = "0x%x" % cid
        hp = os.path.join(chdir, "cl%04x.h" % cid)
        if cid not in hdrs:
            if os.path.exists(hp):
                hdrs[cid] = h12(norm(strip_c(read(hp))))
            else:
                hdrs[cid] = "NOHDR"
        items["classhdr:0x%04x" % cid] = hdrs[cid]
    for n, cid in served["classes"].items():
        if "classdef:" + n not in items and cid > 0x20:
            items["classdef:" + n] = "0x%x" % cid if n in defines else "NODEF"
        hp = os.path.join(chdir, "cl%04x.h" % cid)
        if "classhdr:0x%04x" % cid not in items:
            items["classhdr:0x%04x" % cid] = h12(norm(strip_c(read(hp)))) if os.path.exists(hp) else "NOHDR"
    for n, body in defines.items():
        if n.startswith("NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_") or n.startswith("NV2080_CTRL_MC_ARCH_INFO_IMPLEMENTATION_"):
            items["mcarch:" + n.replace("NV2080_CTRL_MC_ARCH_INFO_", "")] = norm(body)

    # ---- GSP interface
    ep = None
    for p in walk(src, lambda dp, f: f == "rpc_global_enums.h"):
        ep = p
    if ep:
        t = strip_c(read(ep))
        ctr = 0
        for m in re.finditer(r"\bX\(\s*(\w+)\s*,\s*(\w+)\s*(?:,\s*(\d+)\s*)?\)", t):
            if m.group(3) is not None:
                ctr = int(m.group(3))
            items["rpcfn:%s:%s" % (m.group(1), m.group(2))] = str(ctr)
            ctr += 1
    for base, (s, d) in per_file.items():
        short = base.split("@")[0]
        if short.endswith(".h"):
            for n, body in d.items():
                if n.endswith("_H_") or n.endswith("_H") or n.startswith("__"):
                    continue
                items["gsp:%s:def:%s" % (short, n)] = h12(body)
        for n in s:
            if n in NO_FOLLOW:
                continue  # the union of all payload structs: any payload change would move it
            items["gsp:%s:struct:%s" % (short, n)] = clo.sig(n)[0] or "ABSENT"
    return {"tag": tag, "items": items, "detail": detail}


def main():
    tree, tag, servedp, outp = sys.argv[1:5]
    served = json.load(open(servedp))
    res = extract(tree, tag, served, Cache())
    json.dump(res, open(outp, "w"), sort_keys=True)
    print(tag, len(res["items"]), "items")


if __name__ == "__main__":
    main()
