"""No network: edits4 F1/T5 (an ambiguous /rent reply is sticky) on the real trusted runner,
with a fake monotonic clock."""

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
unavailable = dict(busy, state="UNAVAILABLE")
page = {
    "cursor": 0,
    "next_cursor": 1,
    "chunks": [text],
    "log_increment_sanitised": text,
    "log_sha256": digest,
    "bytes_dropped": 0,
}
calls = []
json_header = {"content-type": "application/json"}
answer = lambda status, data: m.Reply(status, json_header, json.dumps(data).encode())
need = m.ADMISSION_NEED_SECONDS
ambiguity = {"done": False}


def ambiguous_once(kind):
    """BUSY while there is clearly time, then ONE ambiguous reply of `kind` as soon as the
    remaining budget is within 25 s of the threshold, then BUSY for ever (the review's case:
    the Worker may have created a box before the reply was lost)."""

    def reply(remaining):
        if not ambiguity["done"] and remaining < need + 25:
            ambiguity["done"] = True
            if kind == "503":
                return answer(503, unavailable)
            if kind == "html":
                return m.Reply(502, {"content-type": "text/html"}, b"<html>bad gateway</html>")
            raise TimeoutError()
        return answer(409, busy)

    return reply


rent_reply = lambda remaining: answer(409, busy)


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
stops = lambda: [c for c in calls if c[0].startswith("/stop/")]

if case == "clean-busy":
    # Nothing was admitted: the run stops waiting before its budget is gone, no /stop needed.
    runner, result = run()
    assert result == {"state": "BUSY", "verdict": "UNAVAILABLE", "verified": 0}, result
    assert rents()[-1][2]["remaining_seconds"] < need + 60, rents()[-1]
    # At most about 15 minutes of BUSY are waited; the rest of the budget is kept.
    assert clock[0] < runner.until - need + 60, (clock, runner.until)
    assert not stops(), [c[:2] for c in calls[-3:]]
elif case in {"ambiguous-503-then-busy", "ambiguous-timeout-then-busy", "ambiguous-html-then-busy"}:
    rent_reply = ambiguous_once(case.split("-")[1])
    runner, result = run()
    assert ambiguity["done"], "the ambiguous reply was never sent"
    assert result == {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}, result
    if case != "ambiguous-html-then-busy":
        # The runner cannot know it was not admitted: it keeps asking below the threshold
        # until the budget is exhausted ...
        assert any(c[2]["remaining_seconds"] < need for c in rents()), len(rents())
        assert clock[0] >= runner.until - 1, (clock, runner.until)
    # (A garbled gateway page is not retried at all: the run ends at once.) In every case the
    # run is then stopped (best effort): the Worker may have created a box for it.
    assert [c[1] for c in stops()] == ["POST"], [c[:2] for c in calls[-3:]]
    assert calls[-1][0].startswith("/stop/"), calls[-1][:2]
elif case == "busy-flag-reset":
    # T5: the BUSY flag is reset before every /rent send, so an ambiguous reply (a lost one
    # leaves no BUSY behind) never counts as a definite BUSY.
    runner = m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO())
    runner.until = clock[0] + need + 0.5
    runner.rent_busy = True  # a definite BUSY on the previous request
    seen = []

    def rent_reply(remaining):  # noqa: F811
        seen.append(runner.rent_busy)
        if len(seen) == 1:
            raise TimeoutError()
        return answer(200, final)

    code, data = runner.retry_read("/rent", "POST", {"runner_nonce": ctx["nonce"]})
    assert code == 200 and seen == [False, False], (code, seen)
    assert runner.rent_busy is False
elif case == "refusal-shapes":
    # T4: a refusal's `reason` and `selection` are checked, not just allowed: wrong types or
    # values are Invalid (and never a TypeError that depends on the value's type).
    ok_selection = {"die": "AD104", "ignored": 0, "reason": "DEFAULT_ORDER"}
    refusal = lambda **extra: dict(busy, state="NOT_ELIGIBLE", **extra)
    good = [
        refusal(),
        refusal(reason="RUNNER_TIME"),
        refusal(reason="NO_ALLOWED_OFFERS", selection=ok_selection),
        refusal(selection=dict(ok_selection, reason="PREFERENCE", ignored=16)),
    ]
    bad = [
        refusal(reason="BUSY"),
        refusal(reason="runner_time"),
        refusal(reason=""),
        refusal(reason=None),
        refusal(reason=1),
        refusal(reason=True),
        refusal(reason=["RUNNER_TIME"]),
        refusal(reason={"RUNNER_TIME": 1}),
        refusal(selection=None),
        refusal(selection="DEFAULT_ORDER"),
        refusal(selection=[ok_selection]),
        refusal(selection={}),
        refusal(selection=dict(ok_selection, extra=1)),
        refusal(selection={k: v for k, v in ok_selection.items() if k != "die"}),
        refusal(selection=dict(ok_selection, die="ad104")),
        refusal(selection=dict(ok_selection, die=7)),
        refusal(selection=dict(ok_selection, ignored="0")),
        refusal(selection=dict(ok_selection, ignored=-1)),
        refusal(selection=dict(ok_selection, ignored=17)),
        refusal(selection=dict(ok_selection, ignored=True)),
        refusal(selection=dict(ok_selection, reason="BOGUS")),
        refusal(selection=dict(ok_selection, reason=["NONE"])),
        refusal(reason="RUNNER_TIME", selection={"die": "AD104"}),
        dict(refusal(reason="RUNNER_TIME"), verdict="PASS"),
        dict(refusal(reason="RUNNER_TIME"), extra=1),
    ]
    for data in good:
        m.validate_status(dict(data), ctx, manifest, 409)
    for data in bad:
        try:
            m.validate_status(dict(data), ctx, manifest, 409)
        except m.Invalid:
            continue
        except Exception as error:  # noqa: BLE001 - the type is the finding
            raise AssertionError("not Invalid: %r -> %s" % (data, type(error).__name__))
        raise AssertionError("accepted: %r" % (data,))
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
