"""Named per-lease profile for round B1f: "7-visible-core deviation"; defaults unchanged.

Derived from the offline inventory (ROUND_REF_bw4/attempt1/cpu_ram_inventory.json): offer floor and
in-box visible-core floor 7 (rejects 6), Windows guest vCPUs 6 (one host CPU kept for QEMU I/O,
the kf drainer/relay and the controller), build parallelism 7, native fuzz unchanged at
3 pinned cores (hostable on 7), no time bound shortened, 3 h lease ceiling under the cost stop.
"""

import round_budget as B

PROFILE = "7-visible-core deviation"
DEFAULT_OFFER_CPU_FLOOR = 16
DEFAULT_VISIBLE_FLOOR = 8
DEFAULT_GUEST_SMP = 8
PROFILE_CPU_FLOOR = 7
PROFILE_GUEST_SMP = 6
PROFILE_BUILD_JOBS = 7
FUZZ_CORES = 3
PREFERRED_MACHINE = 99805
MAX_SECONDS = 3 * 3600


def _check(profile):
    if profile is not None and profile != PROFILE:
        raise ValueError("unknown lease profile")
    return profile


def offer_floor(profile):
    return DEFAULT_OFFER_CPU_FLOOR if _check(profile) is None else PROFILE_CPU_FLOOR


def visible_floor(profile):
    return DEFAULT_VISIBLE_FLOOR if _check(profile) is None else PROFILE_CPU_FLOOR


def guest_smp(profile):
    return DEFAULT_GUEST_SMP if _check(profile) is None else PROFILE_GUEST_SMP


def build_jobs(profile):
    if _check(profile) is None:
        raise ValueError("default build parallelism is per script; no single default")
    return PROFILE_BUILD_JOBS


def visible_ok(nproc, profile):
    return type(nproc) is int and nproc >= visible_floor(profile)


def offer_ok(offer, profile):
    return float(offer.get("cpu_cores_effective", 0) or 0) >= offer_floor(profile)


def pinned_fuzz_hostable(visible):
    return visible >= FUZZ_CORES


def prefer(offers):
    chosen = [x for x in offers if int(x["machine_id"]) == PREFERRED_MACHINE]
    return chosen or offers


def plan(hourly, credit, armed_at, *, stop_usd, cap_usd, max_seconds):
    if max_seconds is None:
        ceiling = 150 * 60
    elif max_seconds == MAX_SECONDS:
        ceiling = MAX_SECONDS
    else:
        raise ValueError("only the granted 3 h lease ceiling is accepted")
    saved = B.MAX_SECONDS
    try:
        B.MAX_SECONDS = ceiling
        return B.plan(hourly, credit, armed_at, stop_usd=stop_usd, cap_usd=cap_usd)
    finally:
        B.MAX_SECONDS = saved


def timer_ceiling_seconds(max_seconds):
    if max_seconds is None:
        return 150 * 60
    if max_seconds != MAX_SECONDS:
        raise ValueError("only the granted 3 h lease ceiling is accepted")
    return MAX_SECONDS


def deviation_fields(visible, offer_cores):
    return {
        "profile": PROFILE,
        "cpu_visible": visible,
        "cpu_profile_floor": PROFILE_CPU_FLOOR,
        "cpu_cores_effective_offer": float(offer_cores),
        "guest_smp": PROFILE_GUEST_SMP,
        "build_jobs": PROFILE_BUILD_JOBS,
        "fuzz_cores": FUZZ_CORES,
        "lease_max_seconds": MAX_SECONDS,
        "authority": "controller B1f grant: this lease only",
    }
