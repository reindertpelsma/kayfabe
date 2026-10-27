"""Generate PF2 tables from explicit correctness records, never from fps alone."""
import argparse
import json
import math
from pathlib import Path
import statistics

REVISIONS = {
    "A": "2108fb38d7288c82dfb7f5b257228e28fb36a13a",
    "B": "2108fb38d7288c82dfb7f5b257228e28fb36a13a",
    "C": "714ec29d3fdca0a013d9d478755346b0f1af6fa7",
}
APIS = ("direct3d11", "opengl")


def validate(row):
    if row.get("guest_resets") != []:
        return "GUEST_RESET"
    if row.get("host_xids") != []:
        return "HOST_XID"
    if row.get("watch_complete") is not True:
        return "UNWATCHED"
    if row.get("finished") is not True:
        return "INCOMPLETE"
    if row.get("adapter_ok") is not True:
        return "ADAPTER"
    if row.get("settings_ok") is not True:
        return "SETTINGS"
    arm = row.get("arm")
    if arm not in REVISIONS or row.get("revision") != REVISIONS[arm]:
        return "REVISION"
    if row.get("api") not in APIS or row.get("boot") not in [arm + str(n) for n in (1, 2, 3)]:
        return "IDENTITY"
    try:
        fps, low, high, duration = [float(row[k]) for k in
                                    ("fps", "min_fps", "max_fps", "duration_s")]
    except (KeyError, TypeError, ValueError):
        return "NUMERIC"
    if not (all(map(math.isfinite, (fps, low, high, duration))) and 0 <= low <= fps <= high):
        return "NUMERIC"
    if not 250 <= duration <= 275:
        return "DURATION"
    frames = row.get("frames")
    if (type(frames) is not int or frames <= 0 or fps <= 0
            or abs(frames / duration - fps) > max(0.1, fps * 0.005)):
        return "FRAME_COUNT"
    return None


def summarize(rows):
    if any(r.get("arm") not in REVISIONS or r.get("api") not in APIS for r in rows):
        raise ValueError("unplanned arm or API in result records")
    cells = {}
    for arm in REVISIONS:
        for api in APIS:
            group = [r for r in rows if r.get("arm") == arm and r.get("api") == api]
            failures = [{"boot": r.get("boot"), "reason": validate(r)}
                        for r in group if validate(r) is not None]
            valid = [r for r in group if validate(r) is None]
            duplicate = len({r.get("boot") for r in group}) != len(group)
            if failures or duplicate:
                status = "FAIL"
            elif len(valid) != 3:
                status = "INCOMPLETE"
            else:
                status = "PASS"
            cells[f"{arm}/{api}"] = dict(
                status=status, n=len(valid), attempted_n=len(group), median_fps=None,
                fps_range=None, failures=failures, duplicate_boot=duplicate,
                per_run=[{k: r[k] for k in ("boot", "fps", "min_fps", "duration_s")}
                         for r in valid],
            )
    for arm in REVISIONS:
        arm_ok = all(cells[f"{arm}/{api}"]["status"] == "PASS" for api in APIS)
        for api in APIS:
            cell = cells[f"{arm}/{api}"]
            cell["arm_accepted"] = arm_ok
            if arm_ok:
                fps = [r["fps"] for r in cell["per_run"]]
                cell["median_fps"] = statistics.median(fps)
                cell["fps_range"] = [min(fps), max(fps)]
    return cells


def render(cells):
    lines = [
        "| Arm | Revision | API | Status | Valid/attempted | Median fps | FPS range |",
        "|---|---|---|---|---:|---:|---|",
    ]
    for arm in REVISIONS:
        for api in APIS:
            cell = cells[f"{arm}/{api}"]
            median = cell["median_fps"]
            value = f"{median:.2f}" if median is not None else "—"
            span = cell["fps_range"]
            spread = f"{span[0]:.2f}–{span[1]:.2f}" if span else "—"
            status = cell["status"]
            if status == "PASS" and not cell["arm_accepted"]:
                status = "PASS run group; arm not accepted"
            lines.append(f"| {arm} | `{REVISIONS[arm][:9]}` | {api} | {status} | "
                         f"{cell['n']}/{cell['attempted_n']} | {value} | {spread} |")
    lines.extend(["", "Individual accepted runs (failed attempts carry no accepted fps):", "",
                  "| Boot | API | FPS | Min FPS | Engine duration s |",
                  "|---|---|---:|---:|---:|"])
    for key, cell in cells.items():
        api = key.split("/")[1]
        for row in cell["per_run"]:
            lines.append(f"| {row['boot']} | {api} | {row['fps']:.2f} | "
                         f"{row['min_fps']:.2f} | {row['duration_s']:.3f} |")
        for failure in cell["failures"]:
            lines.append(f"| {failure['boot']} | {api} | FAIL: {failure['reason']} | — | — |")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records", type=Path)
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()
    summary = summarize(json.loads(args.records.read_text()))
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (args.output_dir / "table.md").write_text(render(summary))
