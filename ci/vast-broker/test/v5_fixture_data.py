"""Static V5 positive-control metadata for the independent hostile input probes."""


def extra_status(verdict="PASS", pending=False):
    names = {
        "provision": 1,
        "kf_tests": 100,
        "gates": 9,
        "kf3_build": 1,
        "raw_suite": 30,
        "thin_suite": 30,
        "birth_census": 1,
        "cuda_host": 4,
        "cuda_fat": 4,
    }
    rows = [{"name": name, "pass": count, "fail": 0, "skip": 0} for name, count in names.items()]
    if verdict != "PASS":
        rows[0]["pass"] = 0
        rows[0]["fail"] = 1
    return dict(
        profile="check",
        die="AD104",
        compute_cap=890,
        driver_version="580.159.04",
        selection_reason="DEFAULT_ORDER",
        preferences_ignored=0,
        suites=[] if pending else rows,
        boot_deadline_at=2401000,
        hard_deadline_at=10801000,
        soft_deadline_at=10201000,
        last_post_at=2000,
        result_deadline_at=None if pending else 302000,
    )


def availability():
    return {
        "profile": "check",
        "offers": [],
        "thresholds": {
            "reliability": 0.98,
            "cores": 8,
            "cpuGhz": 2,
            "ramMb": 34360,
            "downMbps": 200,
            "upMbps": 50,
            "diskMbps": 100,
            "medianRatio": 0.5,
            "price": 1,
            "bandwidth_down_gb": 15,
            "bandwidth_up_gb": 2,
            "verified": 1,
        },
        "rejected": {},
        "verified_counts": [],
        "cache": {"hit": True, "age_ms": 0},
    }
