"""No network: edits3 B1 (the BUSY wait ends before the hard limit no longer fits) on the
real trusted runner, with a fake monotonic clock."""

import importlib.util
import io
import json
import sys
from pathlib import Path

broker = Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location("runner", broker / "runner.py")
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
clock = [1000.0]
m.time.monotonic = lambda: clock[0]
m.time.sleep = lambda s: clock.__setitem__(0, clock[0] + s)
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
busy = {"state": "BUSY", "verdict": "UNAVAILABLE", "log_tail_sanitised": "", "bytes_dropped": 0}
page = {
    "cursor": 0,
    "next_cursor": 1,
    "chunks": [text],
    "log_increment_sanitised": text,
    "log_sha256": digest,
    "bytes_dropped": 0,
}
calls = []
answer = lambda status, data: m.Reply(
    status, {"content-type": "application/json"}, json.dumps(data).encode()
)


def rent_reply(remaining):
    return answer(409, busy)


def client(route, method="GET", body=None):
    calls.append((route, method, None if body is None else dict(body), clock[0]))
    if route.startswith("/availability"):
        return answer(200, scope["availability"]())
    if route == "/rent":
        return rent_reply(body["remaining_seconds"])
    if route.startswith("/status/"):
        return answer(200, final)
    if route.startswith("/logs/"):
        return answer(200, page)
    if route.startswith("/stop/"):
        return answer(200, dict(final, state="NONE", cleanup_pending=False, ended_at=3000))
    raise AssertionError(route)


def run():
    runner = m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO())
    return runner, runner.execute()


rents = lambda: [c for c in calls if c[0] == "/rent"]
need = getattr(m, "ADMISSION_NEED_SECONDS", None)

if case == "need-constant":
    # The runner needs the default hard limit plus the Worker's admission margin, and its own
    # budget must leave room for a short wait.
    defaults = json.loads((broker / "defaults.json").read_text())
    assert need == (defaults["hardMinutes"] + defaults["admissionMarginMinutes"]) * 60, need
    assert need < 600 * 20, need
elif case == "busy-wait-ends":
    # A BUSY run stops asking once its remaining budget is below what a run needs. No rental,
    # no stop, a clean BUSY outcome; it does not burn the whole budget.
    runner, result = run()
    assert result == {"state": "BUSY", "verdict": "UNAVAILABLE", "verified": 0}, result
    bodies = [c[2] for c in rents()]
    assert len(bodies) > 1, calls
    for c in rents():
        assert set(c[2]) == {"runner_nonce", "tested_sha", "profile", "remaining_seconds"}, c
        assert type(c[2]["remaining_seconds"]) is int, c
        assert c[2]["remaining_seconds"] == int(runner.until - c[3]), (c, runner.until)
        assert c[2]["remaining_seconds"] >= need, c
    assert bodies[-1]["remaining_seconds"] < need + 60, bodies[-1]
    assert not [c for c in calls if c[0].startswith("/stop/")], calls
    assert clock[0] < runner.until, (clock, runner.until)
elif case == "ambiguous-keeps-asking":
    # After an ambiguous answer (a 503: the Worker may have created a box before it failed) the
    # runner cannot know whether it was admitted, so it keeps asking even below the threshold;
    # the Worker returns the prior record or refuses (RUNNER_TIME).
    state = {"phase": "busy"}
    unavailable = dict(busy, state="UNAVAILABLE")

    def rent_reply(remaining):  # noqa: F811 - the case replaces the reply.
        # BUSY well above the threshold (the next send is at most 20 s later), then ambiguous
        # replies across it, then the admitted record.
        if state["phase"] == "busy" and remaining >= need + 25:
            return answer(409, busy)
        state["phase"] = "ambiguous"
        if remaining >= need:
            return answer(503, unavailable)
        return answer(200, final)

    runner, result = run()
    assert result == {"state": "GRACE", "verdict": "PASS", "verified": 1}, result
    assert any(c[2]["remaining_seconds"] < need for c in rents()), rents()
elif case == "validate-refusal":
    # The Worker's RUNNER_TIME refusal passes the runner's strict validator and is final.
    data = json.loads(sys.stdin.read())
    m.validate_status(data, ctx, manifest, 409)
    assert data["state"] == "NOT_ELIGIBLE" and data["reason"] == "RUNNER_TIME", data
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
