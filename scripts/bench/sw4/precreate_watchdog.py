"""Arm a fixed cost deadline before create; bind only the round's recorded contract id."""

import argparse
import datetime as dt
import json
import math
import os
from pathlib import Path
import subprocess
import time

import round_budget as B


def watch(plan, *, ledger, ready, now, pause, status, destroy):
    if B.remaining(plan, now()) <= 0:
        raise ValueError("cannot arm an expired allocation plan")
    ready(dict(plan))
    owned = None

    def retire(reason):
        for _ in range(5):
            try:
                destroy(owned)
                proof = status(owned)
                if proof["id"] == owned and proof["absent"] is True:
                    return {"id": owned, "reason": reason, "absence_verified": True}
            except (OSError, ValueError, KeyError, subprocess.SubprocessError):
                pass
            pause(3)
        raise RuntimeError("exact-id teardown did not establish API absence")

    while True:
        try:
            rows = ledger()
            if not isinstance(rows, list) or len(rows) > 1:
                raise ValueError("round must contain at most one owned contract")
            if rows:
                row = rows[0]
                iid = row["id"]
                if type(iid) is not int or iid <= 1 or owned not in (None, iid):
                    raise ValueError("owned contract changed or has invalid id")
                owned = iid
            remaining = B.remaining(plan, now())
            if owned is None:
                if remaining <= 0:
                    return {"id": None, "reason": "no contract before deadline"}
            else:
                current = status(owned)
                if current["id"] != owned:
                    raise ValueError("provider receipt belongs to another id")
                if current["absent"] is True:
                    return {"id": owned, "reason": "API absent", "absence_verified": True}
                rate = B.positive_number(current["rate"], "observed rate")
                credit = current["credit"]
                if type(credit) not in (float, int) or not math.isfinite(credit):
                    raise ValueError("invalid account credit")
                remaining = B.remaining(plan, now())
                if remaining <= 0:
                    return retire("fixed deadline")
                if rate > plan["rate_bound"] or rate > B.MAX_RATE:
                    return retire("price beyond reservation")
                if credit < B.RESERVE_USD:
                    return retire("account reserve")
                spent = plan["credit_before"] - credit
                if spent + B.NONRUNTIME_MARGIN >= plan["stop_usd"]:
                    return retire("billed allocation stop")
                if row.get("destroyed"):
                    return retire("closed ledger but provider still present")
            pause(min(20, remaining))
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            if owned is None:
                raise
            return retire("watchdog error: " + type(error).__name__)


def atomic(path, row):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(row, indent=2) + "\n")
    temporary.replace(path)


def api(args):
    result = subprocess.run(
        ["vastai", *args, "--raw"], capture_output=True, text=True, check=True, timeout=45
    )
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("plan", "ledger", "ready", "result"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())

    def ready(row):
        row.update(
            state="ARMED",
            pid=os.getpid(),
            starttime=int(Path("/proc/self/stat").read_text().rpartition(")")[2].split()[19]),
        )
        atomic(args.ready, row)

    def status(iid):
        row = next((row for row in api(["show", "instances"]) if row["id"] == iid), None)
        if row is None:
            return {"id": iid, "absent": True}
        return {
            "id": iid,
            "absent": False,
            "rate": float(row["dph_total"]),
            "credit": float(api(["show", "user"])["credit"]),
        }

    def destroy(iid):
        subprocess.run(
            ["vastai", "destroy", "instance", str(iid), "-y"],
            capture_output=True,
            check=True,
            timeout=45,
        )

    result = watch(
        plan,
        ledger=lambda: json.loads(args.ledger.read_text()),
        ready=ready,
        now=lambda: dt.datetime.now(dt.timezone.utc),
        pause=time.sleep,
        status=status,
        destroy=destroy,
    )
    result["utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    result["host_verification"] = "UNVERIFIED-HOST"
    atomic(args.result, result)


if __name__ == "__main__":
    main()
