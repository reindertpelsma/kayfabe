"""Lease-preflight self-test ("dry loop") for the BW3 operator (B1g fix 03:30).

Before ANY create the operator requires a current receipt proving that:
  1. every watcher/guard loop body ran once against synthetic input (the loop test suites),
  2. the operator's own timers ran their teardown path against a fake read-only provider,
  3. the static check found no undefined global or unresolvable module attribute,
and that no checked source changed since. Usage: dry_loop.py <workspace> <run_kit>
"""

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
BW = HERE / "guards"
PR = BW
PR_NATIVE = BW
SUITES = [(BW, name) for name in (
    "test_console_pin", "test_console_pin_types", "test_console_pin_settle",
    "test_precreate_watchdog", "test_round_budget",
)] + [(HERE.parent / "rehearsal", "test_bw7"),
     (HERE.parent / "rehearsal", "test_ref_legacy"),
     (HERE.parent / "rehearsal", "test_readiness_replay")]
RECEIPT = "dry_loop_pass.json"
# Workspace-specific suites run only where their module is part of the workspace.
REQUIRES = {"test_budget_b1j": "budget_override.py"}


def default_roots(workspace, run_kit):
    return [BW, Path(workspace), HERE.parent / "rehearsal"]


def sources(roots):
    out = {}
    for root in roots:
        for p in sorted(Path(root).glob("*.py")):
            out[str(p.resolve())] = hashlib.sha256(p.read_bytes()).hexdigest()
    return out


def run_suites(workspace=None, run_kit=None):
    rows = []
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    if workspace and run_kit:
        env.update(KF_BW3_D=str(workspace), KF_BW3_K=str(run_kit))
    for root, suite in SUITES:
        need = REQUIRES.get(suite)
        if need and not (workspace and (Path(workspace) / need).exists()):
            rows.append(
                {
                    "suite": suite,
                    "root": str(root),
                    "ran": 0,
                    "ok": True,
                    "skipped": "workspace has no " + need,
                }
            )
            continue
        p = subprocess.run(
            [sys.executable, "-m", "unittest", suite],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            timeout=600,
        )
        ran = re.search(r"^Ran (\d+) tests?", p.stderr, re.M)
        rows.append(
            {
                "suite": suite,
                "root": str(root),
                "ran": int(ran[1]) if ran else 0,
                "ok": p.returncode == 0 and bool(ran),
            }
        )
    return rows


def _fake_provider(bin_dir):
    fake = Path(bin_dir) / "vastai"
    fake.write_text(
        '#!/bin/sh\necho "$@" >> "$(dirname "$0")/calls.log"\n'
        "case \"$1 $2\" in\n 'show instances') echo '[]';;\n"
        " 'show user') echo '{\"credit\": 28.0}';;\n 'destroy instance') echo '{}';;\n"
        " *) exit 9;;\nesac\n"
    )
    fake.chmod(0o755)
    return {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "PYTHONDONTWRITEBYTECODE": "1"}


def dry_operator(workspace):
    workspace = Path(workspace)
    result = {}
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        env = _fake_provider(tmp)
        ledger = tmp / "owned_instances.json"
        ledger.write_text(json.dumps([{"id": 2, "destroyed": False}]))
        src = (workspace / "budget_watch.py").read_text()
        src = re.sub(r"OWNED = Path\('[^']*'\)", f"OWNED = Path('{ledger}')", src, count=1)
        (tmp / "budget_watch.py").write_text(src)
        p = subprocess.run(
            [sys.executable, str(tmp / "budget_watch.py"), "2", "0.2"],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        result["budget_watch"] = {
            "rc": p.returncode,
            "ready_marker": bool(list(tmp.glob("timer_ready_2_*.json"))),
            "absence_receipt": bool(list(tmp.glob("budget_auto_absence_2_*.json"))),
            "destroy_called": "destroy instance 2" in (tmp / "calls.log").read_text(),
        }
        now = dt.datetime.now(dt.timezone.utc)
        plan = {
            "armed_utc": now.isoformat(),
            "deadline_utc": (now + dt.timedelta(seconds=2)).isoformat(),
            "seconds": 2,
            "quoted_rate": 0.3,
            "rate_bound": 0.4,
            "reservation_usd": 0.6,
            "credit_after_reservation": 27.4,
            "stop_usd": 2.0,
            "cap_usd": 2.5,
            "reserve_usd": 8.0,
            "leases_authorized": 1,
        }
        (tmp / "plan.json").write_text(json.dumps(plan))
        (tmp / "ledger2.json").write_text("[]")
        p = subprocess.run(
            [
                sys.executable,
                str(BW / "precreate_watchdog.py"),
                "--plan",
                str(tmp / "plan.json"),
                "--ledger",
                str(tmp / "ledger2.json"),
                "--ready",
                str(tmp / "ready.json"),
                "--result",
                str(tmp / "result.json"),
            ],
            env=env,
            cwd=BW,
            capture_output=True,
            text=True,
            timeout=60,
        )
        ready = (
            json.loads((tmp / "ready.json").read_text()) if (tmp / "ready.json").exists() else {}
        )
        res = (
            json.loads((tmp / "result.json").read_text()) if (tmp / "result.json").exists() else {}
        )
        result["precreate_watchdog"] = {
            "rc": p.returncode,
            "state": ready.get("state"),
            "result": res.get("reason"),
        }
    return result


def static_problems(roots):
    sys.path.insert(0, str(HERE))
    import static_check

    files = [
        p for r in roots for p in sorted(Path(r).glob("*.py")) if not p.name.startswith("test_")
    ]
    return [list(row) for row in static_check.check(files)]


def _passed(body):
    op = body.get("operator", {})
    operator_ok = not op or (
        op.get("budget_watch", {}).get("absence_receipt") is True
        and op.get("budget_watch", {}).get("destroy_called") is True
        and op.get("precreate_watchdog", {}).get("state") == "ARMED"
        and op.get("precreate_watchdog", {}).get("result") == "no contract before deadline"
    )
    return all(s["ok"] for s in body["suites"]) and not body["static"] and operator_ok


def write_receipt(workspace, body, roots):
    receipt = dict(body)
    receipt.update(
        utc=dt.datetime.now(dt.timezone.utc).isoformat(),
        sources=sources(roots),
        passed=_passed(body),
        experiment_mode="OBSERVATION",
        attestation=False,
    )
    (Path(workspace) / RECEIPT).write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


def require_receipt(workspace, roots):
    path = Path(workspace) / RECEIPT
    if not path.exists():
        raise RuntimeError("dry-loop receipt missing: run dry_loop.py before any create")
    receipt = json.loads(path.read_text())
    if receipt.get("passed") is not True or not _passed(receipt):
        raise RuntimeError("dry-loop receipt is not a pass")
    if receipt.get("sources") != sources(roots):
        raise RuntimeError("a checked source changed after the dry-loop receipt")
    return receipt


def main():
    workspace, run_kit = Path(sys.argv[1]), Path(sys.argv[2])
    started = time.monotonic()
    roots = default_roots(workspace, run_kit)
    body = {
        "suites": run_suites(workspace, run_kit),
        "operator": dry_operator(workspace),
        "static": static_problems(roots),
    }
    body["seconds"] = round(time.monotonic() - started, 1)
    receipt = write_receipt(workspace, body, roots)
    print(
        "DRY_LOOP passed=%s tests=%d static=%d"
        % (receipt["passed"], sum(s["ran"] for s in receipt["suites"]), len(receipt["static"]))
    )
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
