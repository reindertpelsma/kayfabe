# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Report the installed nginx version and the repo candidate. Read-only: never installs or updates anything."""
import re
import shutil
import subprocess


def _run(argv):
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return (p.stdout + p.stderr) if p.returncode == 0 else None


def _upstream(v):
    """'1:1.28.3-1ubuntu1' -> '1.28.3'; '1.26.2-r0' -> '1.26.2'."""
    m = re.search(r"(\d+(?:\.\d+)+)", v or "")
    return m.group(1) if m else None


def _apt():
    inst = cand = None
    for pkg in ("nginx-light", "nginx", "nginx-core"):
        out = _run(["apt-cache", "policy", pkg])
        if out is None:
            continue
        i = re.search(r"^\s*Installed:\s*(\S+)", out, re.M)
        c = re.search(r"^\s*Candidate:\s*(\S+)", out, re.M)
        if c and c.group(1) != "(none)" and cand is None:
            cand = c.group(1)
        if i and i.group(1) != "(none)":
            return i.group(1), (c.group(1) if c and c.group(1) != "(none)" else cand)
    return inst, cand


def _dnf():
    out = _run(["dnf", "-q", "repoquery", "--latest-limit=1", "--qf", "%{version}-%{release}", "nginx"])
    return None, (out.strip() or None) if out else None


def _apk():
    out = _run(["apk", "policy", "nginx"])
    m = re.search(r"^\s+(\d[\w.\-]*):", out or "", re.M)
    return None, m.group(1) if m else None


def check():
    """Return (ok, lines). ok is False only when nginx is missing."""
    nginx = shutil.which("nginx")
    if not nginx:
        return False, ["nginx: MISSING"]
    out = _run([nginx, "-v"]) or ""
    m = re.search(r"nginx/(\S+)", out)
    binver = m.group(1) if m else None
    lines = ["nginx installed: %s" % (binver or "version unknown")]
    pkg_inst = cand = None
    for tool, fn in (("apt-cache", _apt), ("dnf", _dnf), ("apk", _apk)):
        if shutil.which(tool):
            pkg_inst, cand = fn()
            break
    if not cand:
        lines.append("nginx candidate: candidate unknown")
        return True, lines
    lines.append("nginx candidate: %s" % cand)
    if pkg_inst:
        drift = pkg_inst != cand
        have = pkg_inst
    else:
        have = binver
        drift = _upstream(binver) != _upstream(cand)
    if drift:
        lines.append("WARNING: installed nginx (%s) != repo candidate (%s); update it (kfseed never updates packages)" % (have, cand))
    return True, lines
