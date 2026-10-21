#!/usr/bin/env python3
"""Run only V2's named, CPU-only structural checks from the pinned workflow.

No Cargo, VM, GPU, namespace setup, provisioning, or external host is invoked.
The workflow is read from the audited commit, never from an untrusted download.
Requires the existing environment's PyYAML for workflow parsing.
"""
from pathlib import Path
import json
import subprocess
import yaml

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
SUBJECT = "e64b8e5f6d0f64c41c9c4d40775e74be08de7164"
PREFIXES = (
    "Hexagonal boundary gate", "VMM-vocabulary gate", "Generation-name gate",
    "ABI-quarantine gate", "Unsafe-surface gate", "Host-pointer gate",
    "Bridge-exclusivity gate", "ogkm-version-tag gate", "Claim-ledger gate",
    "Section-number gate", "Unsafe-containment gates",
)


def run(argv):
    p = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, timeout=90)
    return {"rc": p.returncode, "stdout": p.stdout, "stderr": p.stderr}


def main():
    workflow = subprocess.check_output(
        ["git", "show", f"{SUBJECT}:.github/workflows/ci.yml"], cwd=ROOT, text=True)
    jobs = yaml.safe_load(workflow)["jobs"]
    selected = []
    for job in jobs.values():
        for step in job.get("steps", []):
            if any(step.get("name", "").startswith(p) for p in PREFIXES):
                selected.append(step)
    assert len(selected) == len(PREFIXES), "Workflow shape changed; review, do not guess"
    results = []
    for step in selected:
        result = {"name": step["name"], **run(["bash", "-e", "-o", "pipefail", "-c", step["run"]])}
        results.append(result)
        print(f"{result['rc']} {result['name']}", flush=True)
    (OUT / "structural-gates.json").write_text(json.dumps(results, indent=2) + "\n")
    extra = []
    for command in (
        ["python3", "scripts/ci/dependencies.py"],
        ["python3", "scripts/ci/debt.py", "frozen"],
        ["make", "-C", "cuda/walk", "check-invariants"],
    ):
        result = {"command": command, **run(command)}
        extra.append(result)
        print(f"{result['rc']} {' '.join(command)}", flush=True)
    (OUT / "source-checks.json").write_text(json.dumps(extra, indent=2) + "\n")
    raise SystemExit(int(any(r["rc"] for r in results + extra)))


if __name__ == "__main__":
    main()
