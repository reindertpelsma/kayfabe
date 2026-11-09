#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline image planning arithmetic; UNKNOWN never becomes a zero-sized app.

INFERRED scenario defaults: app compression .55, game/model compression .95,
50 GB workspace/writable delta, 20% headroom, host disk rounded up to 25 GB.
Owner-supplied MEASURED inputs (2026-11-07): Windows+driver+CUDA ~35 GB used,
19.09 GB zstd stream; dev-box R2 55.6 MB/s single and 223 MB/s aggregate eight
streams. These are observations, not guarantees for a future rental.
See docs/design/V3_RELEASE_SOFTWARE_WISHLIST.md for provenance and limitations.
"""

import argparse
import csv
import datetime
import json
import math
import sys
from pathlib import Path

FIELDS = (
    "id", "platform", "app", "nvidia_features", "obtain_route", "automation_route",
    "size_statement", "size_kind", "confidence", "source_url", "retrieved_on",
    "source_low_gb", "source_high_gb", "installed_low_gb", "installed_typical_gb",
    "installed_high_gb", "installed_basis", "compression_class", "free", "minimal",
    "in_base", "status", "owner_notes",
)
INSTALLED = ("installed_low_gb", "installed_typical_gb", "installed_high_gb")
SETS = ("all", "free-only", "minimal")
BASE_USED_GB = 35.0
BASE_COMPRESSED_GB = 19.09
METRICS = ("installed_gb", "compressed_gb", "single_minutes", "eight_stream_minutes",
           "inbound_usd_004", "inbound_usd_020", "disk_request_gb")


def number(text):
    """Blank/UNKNOWN means absent; reject negative, NaN and infinite quantities."""
    if text in ("", "UNKNOWN"):
        return None
    value = float(text)
    if not math.isfinite(value) or value < 0:
        raise ValueError(f"invalid nonnegative finite number: {text!r}")
    return value


def read_catalog(path):
    with Path(path).open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames is None or set(reader.fieldnames) != set(FIELDS) or len(reader.fieldnames) != len(FIELDS):
            raise ValueError("CSV header must contain each documented field exactly once")
        rows = list(reader)
    if not rows:
        raise ValueError("empty catalog")
    seen = set()
    for line, row in enumerate(rows, 2):
        try:
            if None in row or any(value is None for value in row.values()):
                raise ValueError("wrong number of CSV cells")
            if not row["id"] or row["id"] in seen:
                raise ValueError("missing or duplicate id")
            seen.add(row["id"])
            for key in ("app", "nvidia_features", "obtain_route", "automation_route", "size_statement"):
                if not row[key]:
                    raise ValueError(f"missing {key}")
            for key, allowed in (
                ("platform", {"windows", "linux", "shared"}),
                ("compression_class", {"app", "game", "model"}),
                ("size_kind", {"INSTALLED SIZE", "REQUIRED SPACE", "DOWNLOAD SIZE", "UNKNOWN"}),
                ("confidence", {"VENDOR-STATED", "THIRD-PARTY-REPORTED", "UNKNOWN"}),
                ("free", {"0", "1"}), ("minimal", {"0", "1"}), ("in_base", {"0", "1"}),
            ):
                if row[key] not in allowed:
                    raise ValueError(f"invalid {key}")
            if row["minimal"] == "1" and row["free"] != "1":
                raise ValueError("minimal must be free")
            if not row["source_url"].startswith("https://"):
                raise ValueError("missing HTTPS source URL")
            datetime.date.fromisoformat(row["retrieved_on"])
            installed = [number(row[key]) for key in INSTALLED]
            source = [number(row[key]) for key in ("source_low_gb", "source_high_gb")]
            for values in (installed, source):
                if any(v is None for v in values) and not all(v is None for v in values):
                    raise ValueError("size endpoints must all be supplied or all UNKNOWN")
                if values[0] is not None and values != sorted(values):
                    raise ValueError("size endpoints must be ordered")
            if installed[0] is not None and not row["installed_basis"]:
                raise ValueError("installed estimates need an explicit basis")
            if source[0] is not None and (row["size_kind"] == "UNKNOWN" or row["confidence"] == "UNKNOWN"):
                raise ValueError("UNKNOWN source cannot supply numeric source size")
        except (ValueError, TypeError) as error:
            raise ValueError(f"CSV line {line} ({row.get('id', '?')}): {error}") from error
    return rows


def estimate(rows, *, platform="windows", set_name="all", basis="installed",
             app_ratio=.55, asset_ratio=.95, workspace_gb=50.0, headroom=.20):
    if platform not in ("windows", "linux") or set_name not in SETS:
        raise ValueError("unknown platform or set")
    if basis not in ("installed", "required-space-proxy"):
        raise ValueError("unknown projection basis")
    if any(not math.isfinite(v) or not 0 < v <= 1 for v in (app_ratio, asset_ratio)):
        raise ValueError("compression ratios must be finite and in (0, 1]")
    if any(not math.isfinite(v) or v < 0 for v in (workspace_gb, headroom)):
        raise ValueError("workspace and headroom must be finite and nonnegative")
    selected = [r for r in rows if r["platform"] in (platform, "shared")
                and (set_name != "free-only" or r["free"] == "1")
                and (set_name != "minimal" or r["minimal"] == "1")]
    if not selected:
        raise ValueError("selected set is empty")
    installed = [BASE_USED_GB if platform == "windows" else 0.0] * 3
    compressed = [BASE_COMPRESSED_GB if platform == "windows" else 0.0] * 3
    unknown, proxies, covered = [], [], []
    for row in selected:
        if row["in_base"] == "1" and platform == "windows":
            continue  # CUDA is already in the measured Windows base.
        values = [number(row[key]) for key in INSTALLED]
        if values[0] is None and basis == "required-space-proxy" and row["size_kind"] == "REQUIRED SPACE":
            low, high = (number(row[key]) for key in ("source_low_gb", "source_high_gb"))
            if low is not None and high is not None:
                values = [low, low / 2 + high / 2, high]
                proxies.append(row["app"])
        if any(v is None for v in values):
            unknown.append(row["app"])
            continue
        covered.append(row["app"])
        ratio = app_ratio if row["compression_class"] == "app" else asset_ratio
        for i, value in enumerate(values):
            installed[i] += value
            compressed[i] += value * ratio
    metrics = {
        "installed_gb": installed,
        "compressed_gb": compressed,
        "single_minutes": [v * 1000 / 55.6 / 60 for v in compressed],
        "eight_stream_minutes": [v * 1000 / 223.0 / 60 for v in compressed],
        "inbound_usd_004": [v * .004 for v in compressed],
        "inbound_usd_020": [v * .02 for v in compressed],
    }
    # Host retains a compressed archive while expanding a sparse image, plus a
    # writable guest delta/workspace. No virtual-disk-size-as-allocated-size claim.
    disk = [(i + c + workspace_gb) * (1 + headroom) for i, c in zip(installed, compressed)]
    if any(not math.isfinite(v) for values in (*metrics.values(), disk) for v in values):
        raise ValueError("projection overflow")
    metrics["disk_request_gb"] = [math.ceil(v / 25) * 25 for v in disk]
    result = dict(platform=platform, set=set_name, basis=basis, complete=not unknown,
                  unknown_apps=unknown, covered_apps=covered, proxy_apps=proxies,
                  assumptions=dict(app_ratio=app_ratio, asset_ratio=asset_ratio,
                                   workspace_gb=workspace_gb, headroom=headroom,
                                   disk_round_gb=25, evidence_date="2026-11-07"))
    for key, values in metrics.items():
        result["known_" + key] = values
        result[key] = values if not unknown else None
    return result


def render(result):
    scope = "Windows image" if result["platform"] == "windows" else "Linux ADDITIONS ONLY (no OS base)"
    print(f"{result['set']} | {scope} | INFERRED basis={result['basis']}")
    print("low / typical / high; configuration scenarios, not statistical bounds")
    print("TOTAL COMPLETE" if result["complete"] else "TOTAL UNKNOWN; KNOWN COMPONENT SUBTOTAL follows (not a disk request)")
    for key in METRICS:
        values = result[key] if result["complete"] else result["known_" + key]
        label = key if result["complete"] else "known_" + key
        print(f"  {label}: " + " / ".join(f"{value:.3f}" for value in values))
    print("UNKNOWN apps: " + (", ".join(result["unknown_apps"]) or "none"))
    if result["proxy_apps"]:
        print("INFERRED required-space proxies: " + ", ".join(result["proxy_apps"]))
    print("INFERRED compression/workspace/headroom: " + json.dumps(result["assumptions"], sort_keys=True))
    print("Transfer inputs MEASURED 2026-11-07 on dev box: 55.6 MB/s single; 223 MB/s aggregate eight streams.")
    print("Transfer times/costs are INFERRED; inbound rates 0.004 / 0.02 USD/GB are owner-supplied scenarios.\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", type=Path)
    parser.add_argument("--platform", choices=("windows", "linux"), default="windows")
    parser.add_argument("--set", dest="sets", choices=SETS, nargs="+", default=list(SETS))
    parser.add_argument("--basis", choices=("installed", "required-space-proxy"), default="installed")
    parser.add_argument("--app-ratio", type=float, default=.55)
    parser.add_argument("--asset-ratio", type=float, default=.95)
    parser.add_argument("--workspace-gb", type=float, default=50)
    parser.add_argument("--headroom", type=float, default=.20)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        rows = read_catalog(args.csv)
        results = [estimate(rows, platform=args.platform, set_name=name, basis=args.basis,
                            app_ratio=args.app_ratio, asset_ratio=args.asset_ratio,
                            workspace_gb=args.workspace_gb, headroom=args.headroom) for name in args.sets]
    except (OSError, ValueError) as error:
        print(f"image_size_estimate: error: {error}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(results, indent=2, allow_nan=False))
    else:
        for result in results:
            render(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
