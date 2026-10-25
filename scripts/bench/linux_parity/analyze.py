"""Paired, run-level statistics; raw input and incomplete arms remain explicit."""

import argparse
import json
import math
from pathlib import Path
import random
import re
import statistics


def percentile(values, quantile):
    ordered = sorted(values)
    position = (len(ordered) - 1) * quantile
    low = math.floor(position)
    high = math.ceil(position)
    return ordered[low] + (ordered[high] - ordered[low]) * (position - low)


def distribution(values):
    if not values or any(not math.isfinite(x) or x <= 0 for x in values):
        raise ValueError("empty/nonpositive/nonfinite timing; UNMEASURED")
    med = statistics.median(values)
    return {
        "n": len(values),
        "mean_ms": statistics.mean(values),
        "median_ms": med,
        "p90_ms": percentile(values, 0.9),
        "p99_ms": percentile(values, 0.99),
        "p999_ms": percentile(values, 0.999),
        "one_percent_low_per_s": 1000 / percentile(values, 0.99),
        "point_one_percent_low_per_s": 1000 / percentile(values, 0.999),
        "min_ms": min(values),
        "max_ms": max(values),
        "over_33_ms_fraction": sum(x > 33 for x in values) / len(values),
        "over_50_ms_fraction": sum(x > 50 for x in values) / len(values),
        "over_100_ms_fraction": sum(x > 100 for x in values) / len(values),
        "over_2x_median_fraction": sum(x > 2 * med for x in values) / len(values),
    }


def paired(native, guest):
    if len(native) != len(guest) or len(native) < 5:
        raise ValueError("at least five complete matched pairs required")
    if any(not math.isfinite(x) or x <= 0 for x in native + guest):
        raise ValueError("invalid metric")
    ratios = [k / n for n, k in zip(native, guest)]
    rng = random.Random(20261025)
    resamples = [statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000)]
    return {
        "n_pairs": len(ratios),
        "ratio_median": statistics.median(ratios),
        "ratio_range": [min(ratios), max(ratios)],
        "ratio_iqr": [percentile(ratios, 0.25), percentile(ratios, 0.75)],
        "ratio_ci95": [percentile(resamples, 0.025), percentile(resamples, 0.975)],
        "native_median": statistics.median(native),
        "native_range": [min(native), max(native)],
        "guest_median": statistics.median(guest),
        "guest_range": [min(guest), max(guest)],
        "native_iqr": [percentile(native, 0.25), percentile(native, 0.75)],
        "guest_iqr": [percentile(guest, 0.25), percentile(guest, 0.75)],
    }


def parse(text):
    result = {}
    seen = {}
    workload = ""
    for line in text.splitlines():
        if line.startswith("LX2_BEGIN "):
            workload = line.split(" ", 1)[1]
        if line.startswith("SURFACE_FRAME "):
            frame, mono, cpu, render, sample = line.split(" ", 1)[1].split(",")
            frame_id = int(frame)
            if frame_id in seen.setdefault("surface", set()):
                raise ValueError("duplicate surface frame")
            seen["surface"].add(frame_id)
            row = result.setdefault(
                "surface",
                {
                    "cpu_ms": [],
                    "mono": [],
                    "render_gpu_ms": [],
                    "sample_gpu_ms": [],
                },
            )
            row["cpu_ms"].append(float(cpu))
            row["mono"].append(float(mono))
            row["render_gpu_ms"].append(float(render))
            row["sample_gpu_ms"].append(float(sample))
        if line.startswith(("ITER ", "BWITER ")):
            fields = dict(re.findall(r"(\w+)=([^\s]+)", line))
            if int(fields["bad"]) != 0:
                raise ValueError("workload content check failed")
            if int(fields["i"]) == 0:
                continue
            if line.startswith("ITER "):
                key = workload + "-N" + fields["N"]
                latency = float(fields["launch_ms"])
            else:
                key = "stream-" + fields["mib"] + "MiB"
                latency = float(fields["total_ms"])
            iteration_id = int(fields["i"])
            if iteration_id in seen.setdefault(key, set()):
                raise ValueError("duplicate CUDA iteration")
            seen[key].add(iteration_id)
            result.setdefault(key, {"cpu_ms": []})["cpu_ms"].append(latency)
    return result


def process(base):
    runs = []
    for path in sorted((base / "runs").glob("*")):
        if not (path / "result.json").exists():
            continue
        receipt = json.loads((path / "result.json").read_text())
        if receipt["failure"] or not receipt["suite_done"]:
            runs.append({"run": path.name, "status": "excluded-invalid", "receipt": receipt})
            continue
        data = parse((path / "stdout.log").read_text())
        stats = {}
        for name, row in data.items():
            expected = 1000 if name == "surface" else 99
            if len(row["cpu_ms"]) != expected:
                raise ValueError(f"{path.name} {name}: missing timing records")
            stats[name] = {
                "cpu": distribution(row["cpu_ms"]),
                "iterations_per_s": 1000 / statistics.mean(row["cpu_ms"]),
            }
            if name == "surface":
                end = [a + b * 0.001 for a, b in zip(row["mono"], row["cpu_ms"])]
                gaps = [row["cpu_ms"][0]] + [1000 * (b - a) for a, b in zip(end, end[1:])]
                stats[name]["completion_gaps"] = distribution(gaps)
                stats[name]["iterations_per_s"] = 1000 / statistics.mean(gaps)
                stats[name]["render_gpu"] = distribution(row["render_gpu_ms"])
                stats[name]["sample_gpu"] = distribution(row["sample_gpu_ms"])
        runs.append(
            {
                "run": path.name,
                "arm": receipt["arm"],
                "index": receipt["index"],
                "status": "valid",
                "stats": stats,
            }
        )
    by_index = {run["index"]: run for run in runs if run["status"] == "valid"}
    pairs = []
    for index in range(1, 13, 2):
        a, b = by_index.get(index), by_index.get(index + 1)
        if a and b:
            if a["arm"] == b["arm"]:
                raise ValueError("unmatched pair identities")
            pairs.append((a, b) if a["arm"] == "N" else (b, a))
    comparisons = {}
    if len(pairs) >= 5:
        for name in pairs[0][0]["stats"]:
            native = [n["stats"][name]["iterations_per_s"] for n, _ in pairs]
            guest = [k["stats"][name]["iterations_per_s"] for _, k in pairs]
            comparisons[name] = paired(native, guest)
            comparisons[name]["metric"] = "iterations_per_s; K/N higher is better"
    summary = {"runs": runs, "complete_pairs": len(pairs), "comparisons": comparisons}
    (base / "analysis.json").write_text(json.dumps(summary, indent=2) + "\n")
    return summary


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("base", type=Path)
    args = parser.parse_args()
    result = process(args.base)
    print(
        json.dumps(
            {"complete_pairs": result["complete_pairs"], "comparisons": result["comparisons"]},
            indent=2,
        )
    )
