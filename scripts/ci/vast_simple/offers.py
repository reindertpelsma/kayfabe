"""Offer rules of the simple CI runner: a port of the Worker's rules, same data file.

CODE READ 2026-10-25: ports ci/vast-broker/src/policy.js (gpuClass, offerPrice,
offerRejections) and src/offers.js (offerCountry, evaluate, select). Numbers come from
ci/vast-broker/defaults.json; nothing is copied into this file except the country spellings.
"""

import json
import math
import re
import unicodedata
from pathlib import Path

DEFAULTS_PATH = Path(__file__).resolve().parents[3] / "ci/vast-broker/defaults.json"
INVISIBLE = re.compile("[­​-‍⁠﻿]")
# Spellings of the denied countries (offers.js DENIED_NAMES); codes come from defaults.json.
NAMES = {
    "CU": "CU|CUB|Cuba|Republic of Cuba",
    "IR": "IR|IRN|Iran|Islamic Republic of Iran|Iran, Islamic Republic of|"
    "Iran (Islamic Republic of)|Persia",
    "KP": "KP|PRK|North Korea|DPRK|DPR Korea|Korea DPR|Korea, North|"
    "Democratic People's Republic of Korea|Korea, Democratic People's Republic of|"
    "Korea (Democratic People's Republic of)|Korea, DPR",
    "SY": "SY|SYR|Syria|Syrian Arab Republic|Syrian Arab Rep",
    "CN": "CN|CHN|China|PRC|People's Republic of China|China, People's Republic of|"
    "Mainland China|China (Mainland)|PR China",
    "HK": "HK|HKG|Hong Kong|Hong Kong SAR|Hong Kong SAR, China|Hong Kong SAR China|"
    "Hong Kong, China|Hong Kong Special Administrative Region|Hong Kong (China)|HongKong",
    "MO": "MO|MAC|Macau|Macao|Macau SAR|Macao SAR|Macau SAR, China|Macao SAR, China|"
    "Macao SAR China|Macau, China|Macao, China|Macau Special Administrative Region|"
    "Macao Special Administrative Region",
}


def load_defaults(path=DEFAULTS_PATH):
    return json.loads(Path(path).read_text())


def make_config(defaults, bad_machines=()):
    cfg = dict(defaults)
    cfg["bad_machines"] = set(defaults["badMachines"]) | set(bad_machines)
    cfg["hard_ms"] = defaults["hardMinutes"] * 60000
    cfg["allow_unknown_country"] = False
    return cfg


def clean(text):
    text = INVISIBLE.sub("", unicodedata.normalize("NFKC", text))
    return re.sub(r"\s+", " ", text).strip()


def has_format(text):
    return any(unicodedata.category(c) in ("Cc", "Cf") for c in text)


def normal(text):
    text = "".join(c for c in clean(text) if unicodedata.category(c) not in ("Cc", "Cf"))
    text = re.sub(r"['‘’`.]", "", text.upper())
    text = re.sub(r"\s*,\s*", ", ", re.sub(r"[()-]", " ", text))
    return re.sub(r"\s+", " ", text).strip()


DENIED = {normal(n): code for code, names in NAMES.items() for n in names.split("|")}


def offer_country(offer):
    geo = offer.get("geolocation")
    if not isinstance(geo, str) or not geo or len(geo) > 256:
        return "UNKNOWN"
    text = clean(geo)
    parts = text.split(",")
    for i in range(len(parts)):
        denied = DENIED.get(normal(",".join(parts[i:])))
        if denied:
            return denied
    if has_format(text):
        return "UNKNOWN"
    at = text.rfind(",")
    region, country = text[: max(0, at)].strip(), text[at + 1 :].strip()
    return country if at > 0 and region and re.fullmatch("[A-Z]{2}", country) else "UNKNOWN"


def num(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def wildcard(pattern, name):
    at = 0
    for part in pattern.split("*"):
        i = name.find(part, at)
        if i < 0 or (at == 0 and not pattern.startswith("*") and i != 0):
            return False
        at = i + len(part)
    return pattern.endswith("*") or at == len(name)


def gpu_class(offer, cfg):
    cc, name = offer.get("compute_cap"), offer.get("gpu_name")
    if (
        offer.get("gpu_arch") != "nvidia"
        or not isinstance(cc, int)
        or isinstance(cc, bool)
        or not 750 <= cc <= 2000
        or cc % 10
        or not isinstance(name, str)
        or len(name) > 80
    ):
        return None
    name = name.upper().replace(" ", "_")
    name = re.sub("^(NVIDIA_)", "", name)
    name = re.sub("^(GEFORCE_)", "", name)
    name = re.sub("^(QUADRO_)", "", name)
    if not re.fullmatch("RTX_[A-Z0-9_]+|GTX_16[A-Z0-9_]*", name):
        return None
    if any(wildcard(p, name) for p in cfg["gpuDeny"]):
        return None
    rules = [
        r for r in cfg["dieRules"] if r["cc"] == cc and any(wildcard(p, name) for p in r["names"])
    ]
    if len(rules) > 1:
        return None
    return rules[0]["die"] if rules else "CC_%d" % (cc // 10)


def offer_price(offer, cfg):
    keys = ("dph_base", "dph_total", "storage_cost", "inet_down_cost", "inet_up_cost")
    if not all(num(offer.get(k)) and 0 <= offer[k] <= 1000 for k in keys):
        return None
    storage = offer["storage_cost"] * cfg["diskGb"] / cfg["storageMonthHours"]
    compute = offer["dph_base"] + storage
    if abs(offer["dph_total"] - compute) > max(1e-7, compute * 1e-6):
        return None
    bandwidth = (
        cfg["bandwidthDownGb"] * offer["inet_down_cost"]
        + cfg["bandwidthUpGb"] * offer["inet_up_cost"]
    )
    effective = compute + bandwidth / (cfg["hard_ms"] / 3600000)
    total = compute * (cfg["hard_ms"] + cfg["cronPeriod"]) / 3600000 + bandwidth
    return {"effective": effective, "total": total}


def rejections(offer, cfg, profile, median):
    q, reasons = cfg["quality"], []
    die, price = gpu_class(offer, cfg), offer_price(offer, cfg)

    def enough(key, floor):
        return num(offer.get(key)) and offer[key] >= floor

    verified = offer.get("verified") is True or (
        "verified" not in offer and offer.get("verification") == "verified"
    )
    if not verified:
        reasons.append("VERIFICATION")
    oid = offer.get("id")
    if (
        not isinstance(oid, int)
        or isinstance(oid, bool)
        or oid <= 0
        or offer.get("vms_enabled") is not True
        or offer.get("rentable") is not True
        or offer.get("rented") is not False
        or offer.get("num_gpus") != 1
        or not enough("disk_space", cfg["diskGb"])
    ):
        reasons.append("CAPABILITY")
    machine = offer.get("machine_id")
    if not isinstance(machine, int) or machine <= 0 or machine in cfg["bad_machines"]:
        reasons.append("MACHINE")
    if offer.get("host_id") in cfg["badHosts"]:
        reasons.append("BAD_HOST")
    dies = cfg["checkDies" if profile == "check" else "sweepDies"]
    if die is None or die not in dies:
        reasons.append("DIE")
    if not enough("reliability", q["reliability"]) or offer["reliability"] > 1:
        reasons.append("RELIABILITY")
    if not (
        enough("cpu_cores_effective", q["cores"])
        and enough("cpu_ghz", q["cpuGhz"])
        and enough("cpu_ram", q["ramMb"])
    ):
        reasons.append("CPU_RAM")
    if (
        not enough("dlperf", 1e-300)
        or not num(median)
        or median <= 0
        or offer["dlperf"] < median * q["medianRatio"]
    ):
        reasons.append("PERFORMANCE")
    if not (enough("inet_down", q["downMbps"]) and enough("inet_up", q["upMbps"])):
        reasons.append("NETWORK")
    if not enough("disk_bw", q["diskMbps"]):
        reasons.append("DISK_BW")
    cap = cfg["price" if profile == "check" else "sweepPrice"]
    if price is None:
        reasons.append("PRICE_FIELDS")
    elif (
        offer["inet_up_cost"] > cfg["maxInetUsdPerGb"]
        or offer["inet_down_cost"] > cfg["maxInetUsdPerGb"]
        or price["effective"] > cap
        or price["total"] > cap * (cfg["hard_ms"] + cfg["cronPeriod"]) / 3600000
    ):
        reasons.append("PRICE")
    country = offer_country(offer)
    if country in cfg["deniedCountries"] or (
        country == "UNKNOWN" and not cfg["allow_unknown_country"]
    ):
        reasons.append("COUNTRY")
    return reasons


def median_of(values):
    values = sorted(values)
    if not values:
        return None
    mid = len(values) // 2
    return values[mid] if len(values) % 2 else (values[mid - 1] + values[mid]) / 2


def candidates(offers, cfg, profile, dies=None, rotate=0):
    """Allowed offers, best first: die order, then preferred country, effective price, id.

    `dies` restricts and orders the dies; `rotate` starts the sweep order at that index (the
    Worker's ledgered rotation, here a caller-chosen index). Check never rotates.
    """
    samples = {}
    for offer in offers:
        die = gpu_class(offer, cfg)
        trusted = offer.get("verified") is True or (
            "verified" not in offer and offer.get("verification") == "verified"
        )
        if trusted and num(offer.get("dlperf")) and offer["dlperf"] > 0:
            samples.setdefault(die, []).append(offer["dlperf"])
    order = list(cfg["checkDies" if profile == "check" else "sweepDies"])
    if dies:
        order = [d for d in dies if d in order]
    elif profile == "sweep" and order:
        at = rotate % len(order)
        order = order[at:] + order[:at]
    allowed = []
    for offer in offers:
        die = gpu_class(offer, cfg)
        median = median_of(samples.get(die, []))
        if die in order and not rejections(offer, cfg, profile, median):
            allowed.append(dict(offer, die=die, pricing=offer_price(offer, cfg)))
    preferred = cfg["preferredCountries"]

    def rank(offer):
        country = offer_country(offer)
        place = preferred.index(country) if country in preferred else len(preferred)
        return order.index(offer["die"]), place, offer["pricing"]["effective"], offer["id"]

    return sorted(allowed, key=rank)
