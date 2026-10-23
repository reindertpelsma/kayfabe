"""No network: the 2026-10-24 owner edits, exercised on the real trusted runner."""

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
manifest = {"protocol_version": 5, "worker_version": "v5", "bundle_sha256": "b" * 64}
text = "box line\n"
digest = m.chain(m.ZERO, text)
final = {
    "state": "GRACE",
    "verdict": "PASS",
    "run": ctx["run"],
    "id": "900",
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
    **scope["extra_status"](),
}
calls = []


def page(log_hash=digest):
    return {
        "cursor": 0,
        "next_cursor": 1,
        "chunks": [text],
        "log_increment_sanitised": text,
        "log_sha256": log_hash,
        "bytes_dropped": 0,
    }


def client(route, method="GET", body=None):
    calls.append((route, method, body))
    if route.startswith("/availability"):
        data = scope["availability"]()
    elif route == "/rent" or route.startswith("/status/"):
        data = dict(final, **overrides)
    elif route.startswith("/logs/"):
        data = page()
    elif route.startswith("/stop/"):
        data = dict(final, state="NONE", cleanup_pending=False, ended_at=3000)
    else:
        raise AssertionError(route)
    return m.Reply(200, {"content-type": "application/json"}, json.dumps(data).encode())


overrides = {}
run = lambda: m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO()).execute()
stops = lambda: [c for c in calls if c[0].startswith("/stop/")]

if case == "rent-body":
    # Item 1: the rent body carries no pr_number; the status object carries no pr.
    result = run()
    rent = [c[2] for c in calls if c[0] == "/rent"]
    assert rent and set(rent[0]) == {"runner_nonce", "tested_sha", "profile"}, rent
    assert result == {"state": "GRACE", "verdict": "PASS", "verified": 1}, result
    for extra in [{"pr": 7}, {"pr": None}]:
        try:
            m.validate_status(dict(final, **extra), ctx, manifest, 200)
        except m.Invalid:
            pass
        else:
            raise AssertionError("status with pr accepted: " + json.dumps(extra))
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
