"""Provider-only pre-SSH retry policy; independent of native and guest fault guards."""

import json
from pathlib import Path

from readiness import policy


def boot_bound(banner_seconds):
    # Before a pinned SSH probe is possible there is no box-speed measurement.
    # Use the cold-start ceiling; then freeze the first actual probe's scale.
    if banner_seconds is None:
        return 1200
    return min(1200, max(600, 120 * policy(banner_seconds)["factor"]))


def failure(status, elapsed, *, banner_seconds=None, ssh_ready=False, work_started=False):
    if ssh_ready or work_started:
        return None
    text = str(status.get("status_msg", "")).lower()
    if (
        text.startswith("error:")
        or "gpu error, unable to start instance" in text
        or str(status.get("actual_status", "")).lower() in {"error", "failed", "offline"}
    ):
        return "provider_host_error"
    if elapsed >= boot_bound(banner_seconds):
        return "provider_boot_timeout"
    return None


def load(path, default):
    path = Path(path)
    if not path.exists():
        return default
    if path.stat().st_size > 65536:
        raise ValueError("provider retry state byte bound")
    return json.loads(path.read_text())


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def bad_machines(path):
    rows = load(path, [])
    if not isinstance(rows, list) or len(rows) > 128:
        raise ValueError("known-bad machine list bound")
    return {int(row["machine_id"]) for row in rows}


def exclude_bad(offers, path):
    bad = bad_machines(path)
    return [row for row in offers if int(row["machine_id"]) not in bad]


def retire(row, owned_ids, reason, bad_path, destroy, absent, utc):
    if (
        int(row["id"]) not in owned_ids
        or row.get("ssh_ready_utc")
        or row.get("work_started")
        or reason not in {"provider_host_error", "provider_boot_timeout"}
    ):
        raise ValueError("not an owned pre-SSH provider failure")
    iid = int(row["id"])
    destroy(iid)
    if absent(iid) is not True:
        raise ValueError("provider retirement API absence not verified")
    rows = load(bad_path, [])
    rows.append(
        {"machine_id": int(row["machine_id"]), "instance_id": iid, "reason": reason, "utc": utc}
    )
    if len(rows) > 128:
        raise ValueError("known-bad machine list bound")
    save(bad_path, rows)


def can_replace(previous, bad_rows):
    if (
        previous.get("ssh_ready_utc")
        or previous.get("work_started")
        or not previous.get("destroyed")
        or not previous.get("absence_verified_utc")
    ):
        return False
    return any(
        int(r["instance_id"]) == int(previous["id"])
        and int(r["machine_id"]) == int(previous["machine_id"])
        and r.get("reason") in {"provider_host_error", "provider_boot_timeout"}
        for r in bad_rows
    )


def reserve_retry(path):
    state = load(path, {"retries_used": 0})
    used = state.get("retries_used")
    if type(used) is not int or not 0 <= used < 2:
        raise ValueError("provider retry limit exhausted or invalid")
    state["retries_used"] = used + 1
    save(path, state)
    return used + 1
