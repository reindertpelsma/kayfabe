"""No network: the actual BASE runner handles one provider replacement."""

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
rents = 0
calls = []


def status(verdict="UNAVAILABLE"):
    final = verdict == "PASS"
    return {
        **scope["extra_status"](pending=not final),
        "state": "GRACE" if final else "NONE" if verdict == "PROVIDER_FAULT" else "BOOTING",
        "verdict": verdict,
        "run": ctx["run"],
        "id": str(899 + rents),
        "sha": ctx["sha"],
        "cleanup_pending": verdict == "UNAVAILABLE",
        "result_at": 2000 if final else None,
        "ended_at": 2000 if verdict == "PROVIDER_FAULT" else None,
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
        "provider_retries": rents - 1,
        **manifest,
    }


def client(route, method="GET", body=None):
    global rents
    calls.append(route)
    code = 200
    if route.startswith("/availability"):
        data = scope["availability"]()
    elif route == "/rent":
        rents += 1
        assert rents <= 2, "unbounded provider retry"
        if rents == 2 and case.startswith("budget"):
            code = 409
            data = {
                "state": "UNAVAILABLE_BUDGET",
                "verdict": "UNAVAILABLE",
                "log_tail_sanitised": "",
                "bytes_dropped": 0,
                "reason": "BUDGET:run-usd",
                "budget": {"run_cap_usd": 1, "prior_usd": 0.2, "cost_usd": 0.9},
            }
        else:
            data = status()
    elif route.startswith("/status/"):
        data = status("PASS" if rents == 2 and case.startswith("pass") else "PROVIDER_FAULT")
    else:
        data = {}
    return m.Reply(code, {"content-type": "application/json"}, json.dumps(data).encode())


messages, summary = [], io.StringIO()
result = m.Runner(ctx, manifest, client, emit=messages.append, summary=summary).execute()
assert rents == 2, (result, calls)
assert result["verdict"] == ("PASS" if case.startswith("pass") else "PROVIDER_FAULT"), result
if not case.startswith("pass"):
    assert result["state"] == "PROVIDER_FAULT", result
text = "".join(messages)
assert text.count("RENTAL machine=") == (1 if case.startswith("budget") else 2), text
assert "runner: provider fault" in text.lower(), text
assert calls.count("/stop/123-1") == 1, calls
