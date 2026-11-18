"""Project complete account-scoped inventories; never retry an allocating request."""

import json
import subprocess
import math
import time


class InventoryUnresolved(ValueError):
    """The provider did not establish a complete inventory for this account."""


def query(args, *, timeout=30):
    result = subprocess.run(
        ["vastai", *args, "--retry", "1", "--raw"], capture_output=True, timeout=timeout, check=True
    )
    return json.loads(result.stdout)


def snapshot(owner_id, *, query=query, budget_seconds=30, monotonic=time.monotonic):
    if (
        type(budget_seconds) not in (int, float)
        or not math.isfinite(budget_seconds)
        or budget_seconds <= 0
    ):
        raise InventoryUnresolved("inventory operation budget exhausted")
    deadline = monotonic() + budget_seconds

    def read(args):
        remaining = deadline - monotonic()
        if remaining <= 0:
            raise InventoryUnresolved("inventory operation budget exhausted")
        value = query(args, timeout=remaining)
        if monotonic() >= deadline:
            raise InventoryUnresolved("inventory operation exceeded total budget")
        return value

    try:
        before = read(["show", "user"]).get("id")
        if type(before) is not int or before != owner_id:
            raise InventoryUnresolved("authenticated account differs from intent")
        rows, tokens, expected_total = [], set(), None
        token = None
        for _ in range(400):
            args = ["show", "instances-v1", "--limit", "25"]
            if token is not None:
                args += ["--next-token", token]
            page = read(args)
            if type(page) is not dict or page.get("success") is not True:
                raise InventoryUnresolved("inventory page unsuccessful")
            found, total = page.get("instances_found"), page.get("total_instances")
            chunk = page.get("instances")
            if (
                type(chunk) is not list
                or type(found) is not int
                or found != len(chunk)
                or type(total) is not int
                or not 0 <= total <= 10000
            ):
                raise InventoryUnresolved("inventory page counts invalid")
            if expected_total is None:
                expected_total = total
            if total != expected_total:
                raise InventoryUnresolved("inventory changed during pagination")
            for row in chunk:
                if type(row) is not dict:
                    raise InventoryUnresolved("inventory row invalid")
                safe = {
                    key: row.get(key)
                    for key in ["id", "label", "machine_id", "host_id", "dph_total", "start_date"]
                }
                if "client_id" in row:
                    safe["client_id"] = row["client_id"]
                rows.append(safe)
            token = page.get("next_token")
            if token is None:
                break
            if type(token) is not str or not token or token in tokens:
                raise InventoryUnresolved("inventory cursor invalid")
            tokens.add(token)
        else:
            raise InventoryUnresolved("inventory pagination bound reached")
        after = read(["show", "user"]).get("id")
        ids = [row["id"] for row in rows]
        if (
            type(after) is not int
            or after != owner_id
            or len(rows) != expected_total
            or any(type(i) is not int or i <= 1 for i in ids)
            or len(set(ids)) != len(ids)
        ):
            raise InventoryUnresolved("inventory identity/completeness unverified")
        return dict(owner_id=owner_id, complete=True, rows=rows)
    except InventoryUnresolved:
        raise
    except Exception as error:
        raise InventoryUnresolved(
            "inventory API or schema error: " + type(error).__name__
        ) from None


def create_once(offer_id, label, *, run=subprocess.run):
    # Installed VastClient loops range(retry): 1 means one attempt; 0 sends nothing.
    # Do not inherit its default of three transport attempts for an allocating PUT.
    return run(
        [
            "vastai",
            "create",
            "instance",
            str(offer_id),
            "--retry",
            "1",
            "--raw",
            "--image",
            "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16",
            "--disk",
            "180",
            "--cancel-unavail",
            "--ssh",
            "--direct",
            "--label",
            label,
        ],
        capture_output=True,
        timeout=45,
    )
