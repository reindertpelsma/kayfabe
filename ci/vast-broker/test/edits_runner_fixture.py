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
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
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
        if case == "stop-after-unreadable-final":
            return m.Reply(503, {"content-type": "application/json"}, b"{}")
        data = page("c" * 64 if case == "stop-after-invalid-final" else digest)
    elif route.startswith("/stop/"):
        data = dict(final, state="NONE", cleanup_pending=False, ended_at=3000)
    else:
        raise AssertionError(route)
    return m.Reply(200, {"content-type": "application/json"}, json.dumps(data).encode())


overrides = {}
run = lambda: m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO()).execute()
stops = lambda: [c for c in calls if c[0].startswith("/stop/")]

if case == "rent-body":
    # Item 1: the rent body carries no pr_number; the status object carries no pr. Edits3 B1
    # added remaining_seconds (the runner's remaining wait budget).
    result = run()
    rent = [c[2] for c in calls if c[0] == "/rent"]
    keys = {"runner_nonce", "tested_sha", "profile", "remaining_seconds"}
    assert rent and set(rent[0]) == keys, rent
    assert result == {"state": "GRACE", "verdict": "PASS", "verified": 1}, result
    for extra in [{"pr": 7}, {"pr": None}]:
        try:
            m.validate_status(dict(final, **extra), ctx, manifest, 200)
        except m.Invalid:
            pass
        else:
            raise AssertionError("status with pr accepted: " + json.dumps(extra))
elif case == "stop-after-verified-final":
    # Item 4: a verified final fetch is followed by exactly one best-effort stop.
    result = run()
    assert result["verified"] == 1 and result["verdict"] == "PASS", result
    assert [(c[0], c[1]) for c in stops()] == [("/stop/123-1", "POST")], calls
    assert calls[-1][0] == "/stop/123-1", calls
elif case == "stop-failure-is-ignored":
    # Item 4: the stop is best effort; its failure never changes the verified result.
    original = client

    def client(route, method="GET", body=None):  # noqa: F811 - the case replaces the client.
        if route.startswith("/stop/"):
            calls.append((route, method, body))
            raise OSError("worker unreachable")
        return original(route, method, body)

    result = run()
    assert result == {"state": "GRACE", "verdict": "PASS", "verified": 1}, result
    assert len(stops()) == 1, calls
elif case in {"stop-after-invalid-final", "stop-after-unreadable-final"}:
    # Edits2 D (supersedes the item-4 reading of 2026-10-24): a result was announced but the
    # final log fetch failed the hash check, or could not be read; the runner stops the run
    # too. Only paths before any result may leave the box to the Worker's timers.
    result = run()
    assert result["verified"] == 0 and result["verdict"] == "UNAVAILABLE", result
    assert [(c[0], c[1]) for c in stops()] == [("/stop/123-1", "POST")], calls
    assert calls[-1][0] == "/stop/123-1", calls
elif case == "worker-not-needed-refused":
    # Review W2: the Worker never decides a skip; a Worker-sent NOT_NEEDED + PASS is refused.
    overrides = dict(state="NOT_NEEDED", id=None, log_cursor=0, log_sha256=m.ZERO, suites=[])
    result = run()
    assert result["verified"] == 0 and result["verdict"] != "PASS", result
    for state in ["NOT_NEEDED", "NEEDS_APPROVAL", "MANUAL"]:
        assert state not in m.STATES, state
        for patch in [{}, {"suites": [], "result_at": None, "verdict": "UNAVAILABLE"}]:
            try:
                m.validate_status(dict(final, state=state, **patch), ctx, manifest, 200)
            except m.Invalid:
                pass
            else:
                raise AssertionError("state accepted: " + state)
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
