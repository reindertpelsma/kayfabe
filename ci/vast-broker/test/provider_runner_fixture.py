"""Deterministic provider session controller; no network or rental."""

import base64
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
case = sys.argv[1]
variant = int(sys.argv[2])
limit = 2 + variant
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64}
manifest = {"protocol_version": 7, "worker_version": "v5", "bundle_sha256": "b" * 64}
rents = 0
clock = 0
calls = []
post_seen = False
wait_requests = 0
wait_started = 0
ip = "203.0.113." + str(70 + variant)
port = str(45000 + variant)
raw = (
    "failed endpoint ["
    + ip
    + "]:"
    + port
    + " ssh_port="
    + port
    + " ssh -p "
    + port
    + " ssh "
    + ("-p" if variant == 1 else "-P")
    + port
)
if variant == 2:
    raw += " context-free operand=+000" + port + " alternate=3405803848"
diagnostic = (
    raw
    + " ssh -oPort=+000"
    + port
    + " scp -P000"
    + port
    + " ssh --port=000"
    + port
    + " ssh://user@"
    + ip
    + ":+000"
    + port
)
m.time.monotonic = lambda: clock


def sleep(seconds):
    global clock
    clock += seconds


m.time.sleep = sleep


def status(verdict="UNAVAILABLE", posted=False):
    final = verdict in {"PASS", "FAIL"}
    fault = verdict == "PROVIDER_FAULT"
    result = {
        **scope["extra_status"](verdict=verdict, pending=not final),
        **manifest,
        "run": ctx["run"],
        "sha": ctx["sha"],
        "id": str(899 + rents),
        "instance_id": str(899 + rents),
        "machine_id": 100000 + rents,
        "host_id": 3000 + rents,
        "runner_nonce": ctx["nonce"],
        "state": "GRACE" if final else "NONE" if fault else "TESTING" if posted else "BOOTING",
        "verdict": verdict,
        "cleanup_pending": not fault,
        "result_at": 2000 if final else None,
        "ended_at": 2000 if fault else None,
        "started_at": 1000,
        "deadline_at": 302000,
        "last_post_at": 2000 if final or posted else None,
        "gpu_model": "RTX_4070",
        "country": "US",
        "datacenter": True,
        "ip_prefix": "203.0.113.0",
        "geolocation": raw,
        "dph_total": 0.2,
        "cost_estimate_usd": 0.01,
        "log_cursor": 0,
        "log_tail_sanitised": diagnostic,
        "bytes_dropped": 0,
        "tail_bytes_dropped": 0,
        "log_sha256": "0" * 64,
        "provider_retries": rents - 1,
        "provider_retry_allowed": fault and (rents < limit or case == "reported"),
        "rental_attempts": [
            {
                "instance": str(900 + i),
                "machine": 100001 + i,
                "host": 3001 + i,
                "reason": "GPU_START_ERROR",
                "seconds": 20,
            }
            for i in range(rents if fault else rents - 1)
        ],
    }
    if variant == 2:
        result["endpoint_redactions"] = base64.b64encode(
            json.dumps({"ips": [ip], "ports": [int(port)], "overflow": False}).encode()
        ).decode()
    if case == "failed-hash" and final:
        result["state"] = "NONE"
        result["log_sha256"] = "c" * 64
    return result


def client(route, method="GET", body=None):
    global rents, post_seen, wait_requests, wait_started
    calls.append(route)
    code = 200
    if route.startswith("/availability"):
        data = scope["availability"]()
    elif route == "/rent":
        if case == "waiting" and rents == 1 and wait_requests < 2:
            if wait_requests:
                assert clock >= wait_started + 20, (clock, wait_started, calls)
            wait_started = clock
            wait_requests += 1
            return m.Reply(
                200,
                {"content-type": "application/json"},
                json.dumps(status("PROVIDER_FAULT")).encode(),
            )
        rents += 1
        assert rents <= 8, "controller exceeded safety limit"
        data = status()
    elif route.startswith("/status/"):
        if case == "reported":
            data = status("PROVIDER_FAULT", posted=True)
        elif rents < limit:
            data = status("PROVIDER_FAULT")
        elif case == "outage" and post_seen:
            code = 503
            data = {"private_error": diagnostic}
        elif case == "outage":
            post_seen = True
            data = status(posted=True)
        else:
            data = status(
                "FAIL"
                if case in {"fail", "failed-hash"}
                else "PROVIDER_FAULT" if case == "fault" else "PASS"
            )
    else:
        data = {}
    return m.Reply(code, {"content-type": "application/json"}, json.dumps(data).encode())


messages = []
summary = io.StringIO()
controller = m.Runner(ctx, manifest, client, emit=messages.append, summary=summary)
result = controller.execute()
expected = (
    "FAIL"
    if case in {"fail", "reported", "outage", "failed-hash"}
    else "PROVIDER_FAULT" if case == "fault" else "PASS"
)
assert result["verdict"] == expected, (result, calls)
assert rents == (1 if case == "reported" else limit), (rents, calls)
assert calls.count("/stop/123-1") == 1, calls
text = "".join(messages) + summary.getvalue()
assert ip not in text and port not in text, text
assert "RENTAL instance=" in text, text
assert controller.retry == 0
if case == "waiting":
    assert clock >= 40, (clock, calls)
    assert sum(msg.startswith("RENTAL instance=") for msg in messages) == limit, messages
    assert sum("not a kayfabe verdict" in msg for msg in messages) == limit - 1, messages

if case == "failed-hash":
    assert result["verified"] == 0, result
