"""Named per-lease override for round B1e (controller 00:55); defaults are unchanged.

Only this lease: CPU floor 8 effective cores (default 16), the granted offer pinned, and a
3 h lease ceiling (default 150 min, the owner's 3 h box limit). Every arm is labelled
"8-core deviation" so no result is read as the standard profile.
"""

import round_budget as B

DEFAULT_CPU_FLOOR = 16
CPU_FLOOR_OVERRIDE = 8
PINNED_OFFER = 57427095
MAX_SECONDS = 3 * 3600
LABEL = "8-core deviation"


def cpu_floor(override):
    if override is None:
        return DEFAULT_CPU_FLOOR
    if type(override) is not int or override != CPU_FLOOR_OVERRIDE:
        raise ValueError("only the granted CPU floor override (8) is accepted")
    return override


def cores_ok(offer, floor):
    return float(offer.get("cpu_cores_effective", 0) or 0) >= floor


def pinned(offers):
    return [x for x in offers if int(x["id"]) == PINNED_OFFER]


def timer_ceiling_seconds(max_seconds):
    if max_seconds is None:
        return 150 * 60
    if max_seconds != MAX_SECONDS:
        raise ValueError("only the granted 3 h lease ceiling is accepted")
    return MAX_SECONDS


def plan(hourly, credit, armed_at, *, stop_usd, cap_usd, max_seconds):
    ceiling = timer_ceiling_seconds(max_seconds)
    saved = B.MAX_SECONDS
    try:
        B.MAX_SECONDS = ceiling
        return B.plan(hourly, credit, armed_at, stop_usd=stop_usd, cap_usd=cap_usd)
    finally:
        B.MAX_SECONDS = saved


def deviation_fields(cores):
    return {
        "cpu_floor_override": CPU_FLOOR_OVERRIDE,
        "cpu_cores_effective": float(cores),
        "profile": LABEL,
        "lease_max_seconds_override": MAX_SECONDS,
        "authority": "controller 00:55 B1e: this lease only",
    }
