"""No network: exercise the actual trusted runner's paid-run failure path."""

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
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
base = {
    "state": "TESTING",
    "verdict": "UNAVAILABLE",
    "run": ctx["run"],
    "id": "900",
    "sha": ctx["sha"],
    "cleanup_pending": True,
    "result_at": None,
    "ended_at": None,
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
    **manifest,
    **scope["extra_status"](pending=True),
}
calls = []
reads = 0


def client(route, method="GET", body=None):
    global reads
    calls.append(route)
    code = 200
    if route.startswith("/availability"):
        data = scope["availability"]()
    elif route == "/rent":
        data = base
    elif route.startswith("/status/"):
        reads += 1
        if case.startswith("malformed-") and reads <= 2:
            return m.Reply(
                int(case.rsplit("-", 1)[1]),
                {"content-type": "text/html"},
                b"private upstream diagnostics must never be echoed",
            )
        if reads <= 2 or case.startswith("exhausted"):
            state = (
                ("RATE_LIMITED" if case == "exhausted-rate" else "UNAVAILABLE")
                if case.startswith("exhausted")
                else case
            )
            code = {"BUSY": 409, "RATE_LIMITED": 429, "UNAVAILABLE": 503}[state]
            data = {
                "state": state,
                "verdict": "UNAVAILABLE",
                "log_tail_sanitised": "",
                "bytes_dropped": 0,
            }
        else:
            data = {
                **base,
                "state": "GRACE",
                "verdict": "PASS",
                "result_at": 2000,
                **scope["extra_status"](),
            }
    else:
        data = {}
    return m.Reply(code, {"content-type": "application/json"}, json.dumps(data).encode())


result = m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO()).execute()
stops = [x for x in calls if x.startswith("/stop/")]
# Owner ruling 2026-10-24 item 4: a verified final result is followed by one stop.
assert stops == ([] if case.startswith("exhausted") else ["/stop/123-1"]), calls
assert reads >= 3, reads
assert result["verdict"] == ("UNAVAILABLE" if case.startswith("exhausted") else "PASS"), result
