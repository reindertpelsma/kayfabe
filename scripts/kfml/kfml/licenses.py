"""Derive a wheel's licence and bundled foreign binaries from the wheel itself. Stdlib only.

Unknown / missing / custom licence is NEVER allowed: it resolves to fetch-only.
"""
from __future__ import annotations

import email.parser
import posixpath
import re
import zipfile

ALLOW = frozenset({"MIT", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "ISC", "CC0-1.0", "Unlicense"})

_TEXT_MAP = {
    "mit": "MIT", "mit license": "MIT", "the mit license": "MIT",
    "bsd-2-clause": "BSD-2-Clause", "bsd-3-clause": "BSD-3-Clause", "bsd 3-clause": "BSD-3-Clause",
    "bsd 2-clause": "BSD-2-Clause", "apache-2.0": "Apache-2.0", "apache 2.0": "Apache-2.0",
    "apache license 2.0": "Apache-2.0", "apache license, version 2.0": "Apache-2.0",
    "apache software license": "Apache-2.0", "isc": "ISC", "isc license": "ISC", "cc0-1.0": "CC0-1.0",
    "unlicense": "Unlicense", "public domain": "Unlicense",
}
_CLASSIFIERS = {
    "License :: OSI Approved :: MIT License": "MIT",
    "License :: OSI Approved :: Apache Software License": "Apache-2.0",
    "License :: OSI Approved :: ISC License (ISCL)": "ISC",
    "License :: CC0 1.0 Universal (CC0 1.0) Public Domain Dedication": "CC0-1.0",
    "License :: Public Domain": "Unlicense",
    "License :: OSI Approved :: BSD License": "BSD-?",  # 2 or 3 clause: needs the licence text
    "License :: OSI Approved :: BSD-3-Clause": "BSD-3-Clause",
}
NVIDIA_NAME = re.compile(r"^(nvidia[-_]|triton$|cuda[-_]|nvtx|pynvml$)", re.I)
NVIDIA_LIB = re.compile(r"(^|/)(lib(cu|nv)[a-z0-9_]*\.so[0-9.]*|libcudnn[a-z0-9_]*\.so[0-9.]*|libnccl\.so[0-9.]*|"
                        r"ptxas|cuobjdump|nvdisasm|nvcc|fatbinary|nvlink|bin2c|cicc|nvprune|libdevice[a-z0-9.]*)$")
SO_NAME = re.compile(r"\.so(\.\d+)*$|\.dll$|\.dylib$")


def sniff_text(text: str):
    t = " ".join(text.split())
    if "Permission is hereby granted, free of charge" in t:
        return "MIT"
    if "Apache License" in t and "Version 2.0" in t:
        return "Apache-2.0"
    if "Redistribution and use in source and binary forms" in t:
        return "BSD-3-Clause" if "Neither the name" in t else "BSD-2-Clause"
    if "Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee" in t:
        return "ISC"
    return None


def _norm_expr(expr: str):
    """Evaluate an SPDX-ish expression: returns SPDX string or None if not fully allowlisted."""
    e = expr.strip()
    if not e:
        return None
    if e.lower() in _TEXT_MAP:
        return _TEXT_MAP[e.lower()]
    toks = re.split(r"\s+(OR|AND)\s+|[()]", e)
    ids = [t.strip() for t in toks if t and t.strip() and t.strip() not in ("OR", "AND")]
    if not ids:
        return None
    ok = [i for i in ids if i in ALLOW]
    if " AND " in e and len(ok) == len(ids):
        return e
    if " AND " not in e and " OR " in e and ok:
        return e
    if len(ids) == 1 and ids[0] in ALLOW:
        return ids[0]
    return None


def derive(meta_text: str, license_files: dict) -> dict:
    """-> {'spdx': str|None, 'evidence': str, 'allowed': bool}. license_files: {name: text}."""
    msg = email.parser.Parser().parsestr(meta_text)
    texts = {n: sniff_text(t) for n, t in license_files.items()}
    sniffed = sorted({v for v in texts.values() if v})
    expr = msg.get("License-Expression")
    lic = msg.get("License")
    cls = [c for c in msg.get_all("Classifier") or [] if c.startswith("License ::")]
    spdx, ev = None, ""
    if expr:
        spdx, ev = _norm_expr(expr), "License-Expression: %s" % expr
    elif lic and lic.strip().upper() != "UNKNOWN" and len(lic) < 80:
        spdx, ev = _norm_expr(lic), "License: %s" % lic
    elif cls:
        mapped = {_CLASSIFIERS.get(c) for c in cls}
        if None in mapped:
            ev = "classifiers unmapped/custom: %s" % "; ".join(cls)
        else:
            m = mapped - {"BSD-?"}
            if "BSD-?" in mapped:
                bsd = [s for s in sniffed if s.startswith("BSD")]
                m = m | set(bsd[:1])
            spdx = next(iter(m)) if len(m) == 1 else None
            ev = "classifiers: %s" % "; ".join(cls)
    elif sniffed and not lic:
        spdx, ev = (sniffed[0] if len(sniffed) == 1 else None), "licence file text sniff: %s" % ",".join(sniffed)
    if not ev:
        ev = "no licence metadata" + ("" if not lic else " (License: %.60s)" % lic)
    # a licence file that names a different allowlisted licence than the metadata is "unclear"
    if spdx and sniffed and not any(s in spdx for s in sniffed) and license_files:
        known = [s for s in sniffed]
        if all(k in ALLOW for k in known) and spdx in ALLOW:
            spdx, ev = None, ev + "; conflicts with licence file text (%s)" % ",".join(known)
    allowed = bool(spdx) and _norm_expr(spdx) is not None
    return {"spdx": spdx if allowed else None, "evidence": ev, "allowed": allowed}


def scan_wheel(path: str) -> dict:
    """Read a wheel: licence derivation, shared libs, NVIDIA-origin flags, tags."""
    fn = posixpath.basename(path)
    stem = fn[:-4]
    parts = stem.split("-")
    name, version = parts[0], parts[1]
    tags = "-".join(parts[-3:])
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        meta_n = next((n for n in names if n.endswith(".dist-info/METADATA")), None)
        meta = z.read(meta_n).decode("utf-8", "replace") if meta_n else ""
        lic_files = {}
        for n in names:
            b = posixpath.basename(n).upper()
            if (".dist-info/" in n) and (b.startswith(("LICENSE", "COPYING", "NOTICE")) or "/licenses/" in n) \
                    and not n.endswith("/"):
                lic_files[n] = z.read(n).decode("utf-8", "replace")[:200000]
        libs = sorted({posixpath.basename(n) for n in names if SO_NAME.search(n)})
        nv_libs = sorted({posixpath.basename(n) for n in names if NVIDIA_LIB.search(n)})
    d = derive(meta, lic_files)
    local = version.split("+", 1)[1] if "+" in version else ""
    nvidia = bool(NVIDIA_NAME.match(name)) or bool(nv_libs) or bool(re.match(r"cu\d+", local))
    reasons = []
    if NVIDIA_NAME.match(name):
        reasons.append("NVIDIA-containing package name")
    if nv_libs:
        reasons.append("bundles NVIDIA-origin binaries: " + ", ".join(nv_libs[:8]))
    if re.match(r"cu\d+", local):
        reasons.append("CUDA build (+%s)" % local)
    redist = d["allowed"] and not nvidia
    if not d["allowed"]:
        reasons.append("UNKNOWN or non-allowlisted licence (%s)" % d["evidence"])
    return {"name": name, "version": version, "filename": fn, "tags": tags, "license_spdx": d["spdx"],
            "license_evidence": d["evidence"], "bundled_libs": libs, "nvidia_origin_libs": nv_libs,
            "nvidia_containing": nvidia, "redistributable": redist,
            "fetch_only_reason": "; ".join(reasons) if not redist else ""}
