"""Tiny version helpers (no packaging dependency)."""
from __future__ import annotations

import re


def parse(v):
    """'2.7.1+cu128' -> (2, 7, 1); '12.8' -> (12, 8). Non-numeric tail is ignored."""
    if v is None:
        return None
    m = re.match(r"\s*v?(\d+(?:\.\d+)*)", str(v))
    if not m:
        return None
    return tuple(int(x) for x in m.group(1).split("."))


def ge(have, need) -> bool:
    h, n = parse(have), parse(need)
    if h is None or n is None:
        return False
    width = max(len(h), len(n))
    h += (0,) * (width - len(h))
    n += (0,) * (width - len(n))
    return h >= n


def cc_str(cc) -> str:
    return "%d.%d" % tuple(cc) if cc else ""
