#!/usr/bin/env python3
"""Per-job and per-step wall times of GitHub Actions runs, to compare CI runs before and after a layout change.

STATUS: LIVE, 2026-11-17 (docs/CI_V3.md, "Job layout, concurrency and caches").

    python3 scripts/ci/ci_run_timings.py 44088759756 44084630157            # jobs of each run, and the slowest steps
    python3 scripts/ci/ci_run_timings.py --steps 25 44088759756             # the 25 slowest steps
    python3 scripts/ci/ci_run_timings.py --jobs-json run.json               # a saved `gh api .../runs/ID/jobs` answer

For each run it prints, per job: the time it waited for a runner (created to started: this is queueing, not work),
the time it ran, and its conclusion; then the run's wall time (first job created to last job completed), the longest
job, and how long the `CI` gate job waited after the last other job finished. It needs the `gh` CLI, logged in, for
run ids; read-only, no secrets beyond what `gh` already holds. The numbers it prints are MEASURED from GitHub's job
records; a layout's expected numbers are not.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime


def when(text):
    return datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ") if text else None


def minutes(a, b):
    return round((b - a).total_seconds() / 60, 1) if a and b else None


def load_jobs(run_id, repo):
    out = subprocess.run(
        ["gh", "api", "--paginate", f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"],
        capture_output=True, text=True, check=True,
    ).stdout
    decoder, pos, jobs = json.JSONDecoder(), 0, []
    while pos < len(out):
        while pos < len(out) and out[pos].isspace():
            pos += 1
        if pos >= len(out):
            break
        page, pos = decoder.raw_decode(out, pos)
        jobs += page["jobs"]
    return jobs


def summarize(jobs):
    """Rows per job, and the run-level numbers. Pure: takes the `jobs` list of the Actions API."""
    rows = []
    for job in jobs:
        created, started, done = when(job["created_at"]), when(job["started_at"]), when(job["completed_at"])
        rows.append(dict(
            name=job["name"], conclusion=job["conclusion"] or job["status"],
            queued=minutes(created, started), ran=minutes(started, done), created=created, started=started, done=done,
            steps=[dict(name=s["name"], ran=minutes(when(s["started_at"]), when(s["completed_at"])), conclusion=s["conclusion"])
                   for s in job.get("steps", []) if s.get("started_at") and s.get("completed_at")],
        ))
    finished = [r for r in rows if r["done"]]
    wall = minutes(min(r["created"] for r in rows), max(r["done"] for r in finished)) if finished else None
    gate = next((r for r in rows if r["name"] == "CI"), None)
    others = [r for r in finished if r is not gate]
    gate_lag = minutes(max(r["done"] for r in others), gate["started"]) if gate and gate["started"] and others else None
    worked = [r for r in rows if r["ran"] is not None and r is not gate and r["conclusion"] != "skipped"]
    longest = max(worked, key=lambda r: r["ran"]) if worked else None
    return dict(rows=rows, wall=wall, gate_lag=gate_lag, longest=longest,
                runner_minutes=round(sum(r["ran"] for r in worked), 1))


def slowest_steps(rows, count):
    steps = [(s["ran"], r["name"], s["name"]) for r in rows for s in r["steps"] if s["ran"] is not None]
    return sorted(steps, reverse=True)[:count]


def report(label, jobs, steps):
    s = summarize(jobs)
    print(f"== {label}")
    for r in sorted(s["rows"], key=lambda r: r["created"]):
        print(f"  {r['name'][:62]:62} queued {r['queued'] if r['queued'] is not None else '-':>7} ran {r['ran'] if r['ran'] is not None else '-':>6}  {r['conclusion']}")
    longest = f"{s['longest']['name'][:40]} ({s['longest']['ran']} min)" if s["longest"] else "-"
    print(f"  wall {s['wall']} min (first job created to last job completed) | runner-minutes {s['runner_minutes']} | "
          f"longest job {longest} | CI gate started {s['gate_lag']} min after the last other job ended")
    if steps:
        print(f"  slowest {steps} steps:")
        for ran, job, step in slowest_steps(s["rows"], steps):
            print(f"    {ran:7.1f}  {job[:34]:34}  {step[:80]}")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("runs", nargs="*", help="workflow run ids")
    ap.add_argument("--repo", default="reindertpelsma/kayfabe")
    ap.add_argument("--steps", type=int, default=0, help="also print the N slowest steps of each run")
    ap.add_argument("--jobs-json", action="append", default=[], help="a saved jobs answer instead of a run id")
    args = ap.parse_args(argv)
    if not args.runs and not args.jobs_json:
        ap.error("give a run id or --jobs-json")
    for path in args.jobs_json:
        data = json.load(open(path))
        report(path, data["jobs"] if isinstance(data, dict) else data, args.steps)
    for run_id in args.runs:
        report(f"run {run_id}", load_jobs(run_id, args.repo), args.steps)
    return 0


if __name__ == "__main__":
    sys.exit(main())
