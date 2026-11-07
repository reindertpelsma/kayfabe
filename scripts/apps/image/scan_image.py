#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fail-closed image scan. Findings are evidence, not a legal certification.

Scan the mounted guest filesystem from inside the guest, excluding pseudo and
transient mounts. No host mount or host traversal is used by the builder.
"""

import os
from pathlib import Path
import re
import stat

PROPRIETARY = re.compile(
    rb"(?im)^[ \t]*(?:[/#*]+[ \t]*)?NVIDIA[^\r\n]{0,100}(?:SOFTWARE LICENSE AGREEMENT|END USER LICENSE AGREEMENT|PROPRIETARY AND CONFIDENTIAL)"
)
NOTICE = re.compile(rb"(?im)^[ \t]*(?:[/#*]+[ \t]*)?Copyright[^\r\n]{0,180}NVIDIA")
VENDOR = re.compile(
    r"(?i)^(?:lib(?:cuda(?:rt)?|nvidia|cublas|cufft|curand|cusolver|cusparse|nvrtc|nvjitlink|cudnn|nccl)(?:[.\-_]|$)|nvidia[-_].*|nvidia\.ko(?:\..*)?$|ptxas$|nvcc$|libdevice\..*)"
)
SKIP = {"proc", "sys", "dev", "run"}


def proprietary_header(text):
    normalized = b" ".join(
        re.sub(rb"(?m)^[ \t]*[/#*]+[ \t]*", b"", text).lower().split()
    )
    return bool(
        PROPRIETARY.search(text)
        or b" ".join((b"proprietary and confidential", b"to nvidia")) in normalized
        or (
            b" ".join((b"notice to", b"licensee:")) in normalized
            and b" ".join((b"nvidia intellectual", b"property rights")) in normalized
        )
    )


def oss_evidence(text):
    normalized = b" ".join(
        re.sub(rb"(?m)^[ \t]*[/#*]+[ \t]*", b"", text).lower().split()
    )
    spdx = re.search(
        rb"(?im)^[ \t]*(?:[/#*]+[ \t]*)?SPDX-License-Identifier:[ \t]*([^\r\n]+)", text
    )
    if spdx:
        expression = spdx[1].decode("ascii", errors="replace").strip(" */")
        terms = set(re.findall(r"[A-Za-z0-9.+-]+", expression)) - {"AND", "OR"}
        if terms and terms <= {
            "MIT",
            "BSD-2-Clause",
            "BSD-3-Clause",
            "Apache-2.0",
            "GPL-2.0",
            "GPL-2.0-only",
            "GPL-2.0-or-later",
            "GPL-3.0-only",
            "GPL-3.0-or-later",
        }:
            return "SPDX: " + expression
    if (
        b"redistribution and use in source and binary forms, with or without"
        in normalized
        and b"modification, are permitted provided that the following conditions are met"
        in normalized
        and b"this software is provided" in normalized
    ):
        return "BSD redistribution terms in file"
    if (
        b"permission is hereby granted, free of charge, to any person obtaining a copy"
        in normalized
    ):
        return "MIT permission terms in file"
    if b"licensed under the apache license, version 2.0" in normalized:
        return "Apache-2.0 header in file"
    return None


def scan(
    root,
    dpkg,
    notice_policy="documented-oss-only",
    max_bytes=32 * 1024**3,
    max_files=250000,
):
    if notice_policy != "documented-oss-only":
        raise ValueError("unsupported NVIDIA scan policy")
    root = Path(root).resolve()
    findings = []
    notices = []
    count = 0
    total = 0

    def finding(path, reason):
        findings.append({"path": path, "reason": reason})

    for row in dpkg:
        parts = row.split("\t")
        if len(parts) < 2:
            finding("/var/lib/dpkg/status", "invalid package row")
            continue
        package = parts[1].split(":")[0]
        if re.match(
            r"(?i)^(?:lib)?nvidia|^cuda-|^libcu(?:dnn|blas|fft|rand|solver|sparse)",
            package,
        ):
            finding("/var/lib/dpkg/status", "NVIDIA package: " + package)

    def walk_error(error):
        path = Path(error.filename or root)
        display = (
            "/" + path.relative_to(root).as_posix()
            if path.is_relative_to(root)
            else "/"
        )
        finding(display, "directory could not be traversed")

    for current, dirs, files in os.walk(root, followlinks=False, onerror=walk_error):
        current = Path(current)
        if current == root:
            dirs[:] = [d for d in dirs if d not in SKIP]
        for name in list(dirs) + files:
            p = current / name
            relative = p.relative_to(root).as_posix()
            display = "/" + relative
            try:
                mode = p.lstat().st_mode
                if (
                    VENDOR.match(name)
                    or relative.startswith(("usr/local/cuda/", "usr/local/cuda-"))
                    or "/nvidia/" in "/" + relative
                ):
                    finding(display, "NVIDIA payload path")
                if stat.S_ISLNK(mode):
                    # Absolute links inside a guest are rooted at the scanned image, not the host.
                    value = os.readlink(p)
                    target = (
                        (root / value.lstrip("/"))
                        if value.startswith("/")
                        else (p.parent / value)
                    )
                    if not target.resolve().is_relative_to(root):
                        finding(display, "symlink escapes image")
                    if name in dirs:
                        dirs.remove(name)
                    continue

                if not stat.S_ISREG(mode):
                    continue
                count += 1
                if count > max_files:
                    finding(display, "scan file budget exceeded")
                    return {
                        "clean": False,
                        "files": count,
                        "bytes": total,
                        "findings": findings,
                        "notices": notices,
                    }
                tail = b""
                has_notice = False
                licence = None
                with p.open("rb") as stream:
                    while block := stream.read(1024 * 1024):
                        total += len(block)
                        if total > max_bytes:
                            finding(display, "scan byte budget exceeded")
                            return {
                                "clean": False,
                                "files": count,
                                "bytes": total,
                                "findings": findings,
                                "notices": notices,
                            }
                        text = tail + block
                        if proprietary_header(text):
                            finding(display, "NVIDIA proprietary licence header")
                            break
                        if NOTICE.search(text):
                            has_notice = True
                        if has_notice and licence is None:
                            licence = oss_evidence(text)
                        tail = text[-512:]
                if has_notice:
                    notices.append(
                        {
                            "path": display,
                            "reason": "NVIDIA copyright notice",
                            "licence_evidence": licence,
                        }
                    )
                    if licence is None:
                        finding(display, "NVIDIA notice without documented OSS licence")
            except OSError:
                finding(display, "file could not be completely scanned")
    return {
        "clean": not findings,
        "files": count,
        "bytes": total,
        "findings": findings,
        "notices": notices,
    }


def inventory_result(report):
    """Scan all files each boot, but compare findings and notices, not log sizes.

    File/byte counts measure scan work; cloud-init logs and instances change on
    each boot. The app file hashes are checked by the separate app inventory.
    """
    return {
        "clean": report["clean"],
        "findings": sorted(
            report["findings"], key=lambda row: (row["path"], row["reason"])
        ),
        "notices": sorted(report["notices"], key=lambda row: row["path"]),
    }
