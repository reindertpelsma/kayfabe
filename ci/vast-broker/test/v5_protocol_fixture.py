import importlib.util, sys
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "runner", Path(__file__).resolve().parents[1] / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
phase = sys.argv[1]
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64, "profile": "check"}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
names = {
    "provision": 1,
    "kf_tests": 100,
    "gates": 9,
    "kf3_build": 1,
    "raw_suite": 30,
    "thin_suite": 30,
    "birth_census": 1,
    "cuda_host": 4,
    "cuda_fat": 4,
}
data = {
    "state": phase,
    "verdict": "PASS",
    "run": ctx["run"],
    "id": "900",
    "machine_id": 100001,
    "host_id": 3001,
    "sha": ctx["sha"],
    "cleanup_pending": phase != "NONE",
    "result_at": 2000,
    "ended_at": 3000 if phase == "NONE" else None,
    "started_at": 1000,
    "deadline_at": 302000,
    "gpu_model": "RTX_4070",
    "cost_estimate_usd": 0.01,
    "log_cursor": 0,
    "log_tail_sanitised": "",
    "bytes_dropped": 0,
    "tail_bytes_dropped": 0,
    "runner_nonce": ctx["nonce"],
    "log_sha256": "0" * 64,
    "profile": "check",
    "die": "AD104",
    "compute_cap": 890,
    "driver_version": "580.159.04",
    "selection_reason": "DEFAULT_ORDER",
    "preferences_ignored": 0,
    "suites": [{"name": k, "pass": v, "fail": 0, "skip": 0} for k, v in names.items()],
    "boot_deadline_at": 2401000,
    "hard_deadline_at": 10801000,
    "soft_deadline_at": 10201000,
    "last_post_at": 2000,
    "result_deadline_at": 302000,
    **manifest,
}
assert m.validate_status(data, ctx, manifest, 200) == data
for patch in [
    {"suites": []},
    {"preferences_ignored": 999},
    {"driver_version": "::error::"},
    {"profile": "sweep"},
    {"die": "::error::"},
    {"soft_deadline_at": 0},
]:
    try:
        m.validate_status({**data, **patch}, ctx, manifest, 200)
    except m.Invalid:
        pass
    else:
        raise AssertionError("untrusted field accepted: " + str(patch))

import json

scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
calls = []
m.time.sleep = lambda _: None


def client(route, method="GET", body=None):
    calls.append(route)
    value = scope["availability"]() if route.startswith("/availability") else data
    return m.Reply(200, {"content-type": "application/json"}, json.dumps(value).encode())


r = m.Runner(ctx, manifest, client, emit=lambda _: None).execute()
assert r["verified"] == 1 and r["verdict"] == "PASS"
# Owner ruling 2026-10-24 item 4: a verified final result is followed by one stop.
assert [route for route in calls if route.startswith("/stop/")] == ["/stop/123-1"], calls
