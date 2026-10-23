"""Offline mutations for this round; each restored, syntax valid, >=2 named killers."""

import argparse
import ast
import json
from pathlib import Path
import re
import subprocess

BASE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument("--preflight", action="store_true")
parser.add_argument("--filter", default="")
parser.add_argument("--results", type=Path)
args = parser.parse_args()
cases = json.loads((BASE / "authorization-mutations.json").read_text())["cases"]
rows = []
if not args.preflight:
    baseline = subprocess.run(
        [
            "node",
            "--test",
            *[
                str(BASE / "test" / name)
                for name in sorted({name for case in cases for name in case["tests"]})
            ],
        ],
        cwd=BASE.parent.parent,
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert baseline.returncode == 0, baseline.stdout[-2500:]
for case in cases:
    if args.filter and args.filter not in case["name"]:
        continue
    path = BASE / case["file"]
    original = path.read_text()
    changed = original
    for before, after in case["patches"]:
        assert (
            changed.count(before) >= 1 if case.get("replace_all") else changed.count(before) == 1
        ), ("missing/ambiguous anchor", case["name"], before)
        changed = (
            changed.replace(before, after)
            if case.get("replace_all")
            else changed.replace(before, after, 1)
        )
    if args.preflight:
        continue
    try:
        path.write_text(changed)
        if path.suffix == ".py":
            ast.parse(changed)
        elif path.suffix == ".yml":
            for block in re.findall(r"(?m)^        run: \|\n((?:          .*\n|\n)*)", changed):
                script = "\n".join(
                    line[10:] if line.startswith("          ") else line
                    for line in block.splitlines()
                )
                check = subprocess.run(
                    ["bash", "-n"],
                    input=script,
                    cwd=BASE.parent.parent,
                    capture_output=True,
                    text=True,
                )
                assert check.returncode == 0, check.stderr
        else:
            check = subprocess.run(
                [
                    "sh" if path.suffix == ".sh" else "node",
                    "-n" if path.suffix == ".sh" else "--check",
                    str(path),
                ],
                cwd=BASE.parent.parent,
                capture_output=True,
                text=True,
            )
            assert check.returncode == 0, (case["name"], check.stderr)
        run = subprocess.run(
            ["node", "--test", *[str(BASE / "test" / name) for name in case["tests"]]],
            cwd=BASE.parent.parent,
            capture_output=True,
            text=True,
            timeout=60,
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M)
        assert run.returncode and len(killers) >= 2, (case["name"], killers, run.stdout[-1600:])
        rows.append({"mutant": case["name"], "count": len(killers), "killers": killers})
        print(case["name"] + ": " + str(len(killers)) + " killers", flush=True)
        if args.results:
            args.results.write_text(json.dumps(rows, indent=2) + "\n")
    finally:
        path.write_text(original)
print(("Anchors valid: " if args.preflight else "Mutants killed: ") + str(len(cases)))
