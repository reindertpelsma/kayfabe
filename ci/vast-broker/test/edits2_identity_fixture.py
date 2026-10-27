"""No network: offer identity (machine_id, host_id) on the real trusted runner (item 9)."""

import importlib.util
import io
import json
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("runner", Path(__file__).parents[1] / "runner.py")
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
m.time.sleep = lambda _: None
case = sys.argv[1]
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64, "profile": "check"}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
text = "box line\n"
digest = m.chain(m.ZERO, text)
status = {
    **scope["extra_status"](),
    "state": "GRACE",
    "verdict": "PASS",
    "run": ctx["run"],
    "id": "900",
    "machine_id": 100001,
    "host_id": 3001,
    "sha": ctx["sha"],
    "cleanup_pending": True,
    "result_at": 2000,
    "ended_at": None,
    "started_at": 1000,
    "deadline_at": 302000,
    "gpu_model": "RTX_4070",
    "cost_estimate_usd": 0.01,
    "log_cursor": 1,
    "log_tail_sanitised": "",
    "bytes_dropped": 0,
    "tail_bytes_dropped": 0,
    "runner_nonce": ctx["nonce"],
    "log_sha256": digest,
    **manifest,
}
row = {
    "die": "AD104",
    "gpu_name": "RTX_4070",
    "compute_cap": 890,
    "count": 2,
    "effective_hourly": 0.5,
    "estimated_total": 1.51,
    "kvm": True,
    "machine_id": 100001,
    "host_id": 3001,
}
availability = {
    **scope["availability"](),
    "offers": [row],
    "rejected": {"BAD_HOST": 1, "DIE": 2, "PRICE": 1},
    "rejected_offers": [
        {"machine_id": 100002, "host_id": 77, "reasons": ["BAD_HOST", "DIE"]},
        {"machine_id": None, "host_id": None, "reasons": ["DIE", "PRICE"]},
    ],
    "verified_counts": [{"die": "AD104", "total": 4, "verified": 4}],
}


def refused(check, value, label):
    try:
        check(value)
    except m.Invalid:
        return
    raise AssertionError("accepted: " + label)


def valid_status(value):
    return m.validate_status(value, ctx, manifest, 200)


def valid_availability(value):
    return m.validate_availability(value, "check")


if case == "codes":
    print(json.dumps(sorted(m.REJECTION_CODES)))
    raise SystemExit(0)
if case == "validate":
    # The real Worker's /availability response, piped in by the table test.
    m.validate_availability(m.strict_json(sys.stdin.buffer.read()), sys.argv[2])
elif case == "status-ids":
    valid_status(status)
    valid_status(dict(status, machine_id=None, host_id=None))
    for key in ["machine_id", "host_id"]:
        refused(valid_status, {k: v for k, v in status.items() if k != key}, "missing " + key)
        for bad in ["100001", 0, -1, True, 1.5, 2**53, [1]]:
            refused(valid_status, dict(status, **{key: bad}), key + "=" + repr(bad))
elif case == "availability-ids":
    valid_availability(json.loads(json.dumps(availability)))
    empty = dict(availability, rejected={}, rejected_offers=[])
    valid_availability(json.loads(json.dumps(empty)))
    for key in ["machine_id", "host_id"]:
        rows = [{k: v for k, v in row.items() if k != key}]
        refused(valid_availability, dict(availability, offers=rows), "offer without " + key)
        for bad in ["1", 0, True, 2.5]:
            rows = [dict(row, **{key: bad})]
            refused(valid_availability, dict(availability, offers=rows), key + repr(bad))
            bad_rows = [dict(availability["rejected_offers"][0], **{key: bad})]
            bad_rows += availability["rejected_offers"][1:]
            refused(
                valid_availability,
                dict(availability, rejected_offers=bad_rows),
                "rejected " + key + repr(bad),
            )
    # No offer (ask) id, IP or hostname crosses the runner.
    for extra in [{"offer_id": 5}, {"public_ipaddr": "203.0.113.7"}, {"hostname": "h"}]:
        refused(valid_availability, dict(availability, offers=[dict(row, **extra)]), str(extra))
        rows = [dict(availability["rejected_offers"][0], **extra)]
        rows += availability["rejected_offers"][1:]
        refused(valid_availability, dict(availability, rejected_offers=rows), "rej " + str(extra))
    first = availability["rejected_offers"][0]
    for reasons in [[], ["SHELL_COMMAND"], ["DIE", "DIE"], "DIE"]:
        rows = [dict(first, reasons=reasons)] + availability["rejected_offers"][1:]
        refused(valid_availability, dict(availability, rejected_offers=rows), repr(reasons))
    # The counts and the rows come from one Worker pass; any disagreement is refused.
    refused(
        valid_availability,
        dict(availability, rejected=dict(availability["rejected"], DIE=3)),
        "count mismatch",
    )
    refused(
        valid_availability, {k: v for k, v in availability.items() if k != "rejected_offers"}, "-"
    )
    refused(valid_availability, dict(availability, rejected_offers={}), "rows not a list")
elif case == "display":
    # Both ids reach the public log and the step summary, so a bad hoster can be listed.
    printed = []
    summary = io.StringIO()

    def client(route, method="GET", body=None):
        if route.startswith("/availability"):
            data = availability
        elif route.startswith("/logs/"):
            data = {
                "cursor": 0,
                "next_cursor": 1,
                "chunks": [text],
                "log_increment_sanitised": text,
                "log_sha256": digest,
                "bytes_dropped": 0,
            }
        else:
            data = status
        return m.Reply(200, {"content-type": "application/json"}, json.dumps(data).encode())

    result = m.Runner(ctx, manifest, client, emit=printed.append, summary=summary).execute()
    assert result == {"state": "GRACE", "verdict": "PASS", "verified": 1}, result
    log = "".join(printed)
    assert "machine_id host_id" in log, log
    assert "AD104 RTX_4070 890 2 0.500000 1.510000 1 100001 3001" in log, log
    assert "100002/77:BAD_HOST,DIE" in log and "-/-:DIE,PRICE" in log, log
    assert "| Vast machine id | 100001 |" in summary.getvalue(), summary.getvalue()
    assert "| Vast host id | 3001 |" in summary.getvalue(), summary.getvalue()
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
