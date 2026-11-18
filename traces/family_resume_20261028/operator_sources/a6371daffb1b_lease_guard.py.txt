"""Bound this allocation's cost and capture before exact-id safety teardown."""

import datetime as dt
import math
import subprocess


def own_cost(plan, now):
    """Conservative liability; other users' account debits are not this lease's cost."""
    armed = dt.datetime.fromisoformat(plan["armed_utc"])
    if now.utcoffset() is None or armed.utcoffset() is None or now < armed:
        raise ValueError("invalid allocation clock")
    values = [plan[key] for key in ("rate_bound", "transfer_usd", "nonruntime_margin_usd")]
    if any(type(v) not in (int, float) or not math.isfinite(v) or v < 0 for v in values):
        raise ValueError("invalid allocation liability")
    return (now - armed).total_seconds() * values[0] / 3600 + sum(values[1:])


def retire(iid, *, capture, destroy, status, pause):
    """Capture once, then require matching API absence even if capture failed."""
    result = dict(id=iid, capture_verified=False)
    try:
        proof = capture(iid)
        if proof.get("id") != iid or proof.get("capture_verified") is not True:
            raise ValueError("final capture does not attest this allocation")
        result["capture_verified"] = True
    except Exception as error:
        result["capture_error"] = type(error).__name__
    for _ in range(5):
        try:
            destroy(iid)
            proof = status(iid)
            if proof["id"] == iid and proof["absent"] is True:
                return dict(result, absence_verified=True)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError):
            pass
        pause(3)
    raise RuntimeError("exact-id teardown did not establish API absence")
