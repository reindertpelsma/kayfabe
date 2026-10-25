"""No network: a configured RENTING_DISABLED refusal (HTTP 503 on /rent) is terminal for the real
trusted runner, with a fake monotonic clock. argv[1] is the case name."""

import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile
from pathlib import Path

broker = Path(__file__).parents[1]
spec = importlib.util.spec_from_file_location("runner", broker / "runner.py")
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
clock = [1000.0]
sleeps = []


def sleep(seconds):
    sleeps.append(seconds)
    clock[0] += seconds


m.time.monotonic = lambda: clock[0]
m.time.sleep = sleep
case = sys.argv[1]
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64, "profile": "check"}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
SENTINEL = "SeNtInEl9f3a7c1d"
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
testing = dict(
    final,
    state="TESTING",
    verdict="UNAVAILABLE",
    result_at=None,
    log_cursor=0,
    log_sha256=m.ZERO,
    result_deadline_at=None,
    suites=[],
)
# Byte-for-byte what the Worker sends (src/index.js, the RENTING_ENABLED test in perform()).
disabled = {
    "state": "RENTING_DISABLED",
    "verdict": "UNAVAILABLE",
    "log_tail_sanitised": "",
    "bytes_dropped": 0,
}
unavailable = dict(disabled, state="UNAVAILABLE")
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
rent_replies = []  # consumed in order; the last one repeats
status_replies = []
log_replies = []


def pop(queue):
    return queue.pop(0) if len(queue) > 1 else queue[0]


def client(route, method="GET", body=None):
    calls.append((route, method, clock[0]))
    if route.startswith("/availability"):
        return answer(200, scope["availability"]())
    if route == "/rent":
        return pop(rent_replies)
    if route.startswith("/status/"):
        return pop(status_replies)
    if route.startswith("/logs/"):
        return pop(log_replies) if log_replies else answer(200, page)
    if route.startswith("/stop/"):
        return answer(200, dict(final, state="NONE", cleanup_pending=False, ended_at=3000))
    raise AssertionError(route)


rents = lambda: [c for c in calls if c[0] == "/rent"]
stops = lambda: [c for c in calls if c[0].startswith("/stop/")]
statuses = lambda: [c for c in calls if c[0].startswith("/status/")]


def run():
    shown = []
    runner = m.Runner(ctx, manifest, client, emit=shown.append, summary=io.StringIO())
    return runner, runner.execute(), "".join(shown)


LINE = "runner: renting is disabled on the Worker"

if case == "one-post":
    # The Worker did not touch Vast: one /rent POST, no waiting, nothing ambiguous, no /stop.
    rent_replies[:] = [answer(503, disabled)]
    runner, result, shown = run()
    assert result == {"state": "RENTING_DISABLED", "verdict": "UNAVAILABLE", "verified": 0}, result
    assert len(rents()) == 1, [c[:2] for c in calls]
    assert sleeps == [], sleeps
    assert clock[0] == 1000.0, clock
    assert runner.rent_ambiguous is False
    assert runner.infrastructure_failure is False
    assert not stops() and not statuses(), [c[:2] for c in calls]
    assert [c[0] for c in calls if c[0] != "/rent"] == ["/availability?profile=check"], [
        c[:2] for c in calls
    ]
elif case == "one-line":
    rent_replies[:] = [answer(503, disabled)]
    runner, result, shown = run()
    assert runner.diagnostic == LINE, runner.diagnostic
elif case == "retry-read":
    # retry_read itself: the first 503 RENTING_DISABLED on /rent is returned at once, flags clean.
    rent_replies[:] = [answer(503, disabled)]
    runner = m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO())
    code, data = runner.retry_read("/rent", "POST", {"runner_nonce": ctx["nonce"]})
    assert (code, data["state"]) == (503, "RENTING_DISABLED"), (code, data)
    assert len(rents()) == 1 and sleeps == [], (calls, sleeps)
    assert runner.rent_ambiguous is False and runner.infrastructure_failure is False
elif case == "main":
    # End to end through main(): exit status, outputs, one displayed line, one /rent POST.
    posts = []

    def https_request(url, method="GET", headers=None, body=None):
        if url.startswith("https://oidc.example"):
            return m.Reply(200, json_header, b'{"value":"a.b.c"}')
        route = "/" + url.split("/", 3)[3].split("/", 1)[1]
        if route == "/rent":
            posts.append(route)
        return client(route, method, None if body is None else json.loads(body))

    m.https_request = https_request
    rent_replies[:] = [answer(503, disabled)]
    stdout, stderr = io.StringIO(), io.StringIO()
    with tempfile.TemporaryDirectory(prefix="kf-rd-") as directory:
        d = Path(directory)
        (d / "manifest").write_text(json.dumps(manifest))
        (d / "output").write_text("")
        (d / "event").write_text(
            json.dumps(
                {
                    "pull_request": {
                        "head": {"sha": "a" * 40, "repo": {"full_name": "o/kayfabe"}},
                        "base": {"ref": "master"},
                    }
                }
            )
        )
        os.environ.clear()
        os.environ.update(
            {
                "WORKER_URL": "https://quiet-tenant.workers.example",
                "WORKER_PATH": "unit-path-only-" + "p" * 32,
                "CF_ACCESS_CLIENT_ID": "runner-a.access",
                "CF_ACCESS_CLIENT_SECRET": "a" * 40,
                "GITHUB_OUTPUT": str(d / "output"),
                "GITHUB_STEP_SUMMARY": str(d / "summary"),
                "GITHUB_EVENT_PATH": str(d / "event"),
                "GITHUB_SHA": "d" * 40,
                "GITHUB_EVENT_NAME": "pull_request_target",
                "GITHUB_REF": "refs/heads/master",
                "GITHUB_REPOSITORY": "o/kayfabe",
                "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "1",
                "ACTIONS_ID_TOKEN_REQUEST_URL": "https://oidc.example/request?test=1",
                "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "mock-request-token",
            }
        )
        sys.argv = ["runner.py", str(d / "manifest")]
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = m.main()
        outputs = (d / "output").read_text()
        summary = (d / "summary").read_text()
    out = stdout.getvalue()
    assert len(posts) == 1, posts
    assert status == 1, status  # same as every other terminal refusal: not a GPU success
    assert outputs == "state=RENTING_DISABLED\nverdict=UNAVAILABLE\nverified=0\n", outputs
    assert out.split("\n").count(LINE) == 1, out
    assert "| State | RENTING_DISABLED |" in summary, summary
    assert not stops(), [c[:2] for c in calls]
    assert sleeps == [], sleeps
elif case == "unavailable-503-retries":
    # A real outage (503 UNAVAILABLE) is still transient and ambiguous: it is retried until the
    # Worker admits the run.
    rent_replies[:] = [answer(503, unavailable), answer(503, unavailable), answer(200, final)]
    status_replies[:] = [answer(200, final)]
    runner, result, shown = run()
    assert len(rents()) == 3, [c[:2] for c in calls]
    assert sleeps, sleeps
    assert result["verdict"] == "PASS" and result["verified"] == 1, result
elif case == "unavailable-503-gives-up":
    rent_replies[:] = [answer(503, unavailable)]
    runner, result, shown = run()
    assert len(rents()) > 6, len(rents())
    assert clock[0] >= runner.until - 1, (clock, runner.until)
    assert runner.rent_ambiguous is True
    assert result == {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}, result
    assert [c[1] for c in stops()] == ["POST"], [c[:2] for c in calls[-3:]]
elif case in {"http500", "http502", "http504"}:
    rent_replies[:] = [answer(int(case[4:]), unavailable), answer(200, final)]
    status_replies[:] = [answer(200, final)]
    runner, result, shown = run()
    assert len(rents()) == 2, [c[:2] for c in calls]
    assert sleeps, sleeps
    assert result["verdict"] == "PASS" and result["verified"] == 1, result
elif case in {"status-503", "logs-503"}:
    # An admitted run can never see RENTING_DISABLED on /status or /logs; if one did arrive it is
    # an ordinary transient failure there (cleanup would still be owed), not a terminal refusal.
    route = "/status/" + ctx["run"] if case == "status-503" else "/logs/" + ctx["run"]
    queue = [
        answer(503, disabled),
        answer(503, disabled),
        answer(200, final if case == "status-503" else page),
    ]
    (status_replies if case == "status-503" else log_replies)[:] = queue
    runner = m.Runner(ctx, manifest, client, emit=lambda _: None, summary=io.StringIO())
    code, data = runner.retry_read(route)
    assert code == 200, (code, data)
    assert len(calls) == 3 and len(sleeps) == 2, (calls, sleeps)
    assert runner.infrastructure_failure is False and runner.diagnostic is None
elif case == "admitted-status-503":
    # Admitted run (/rent -> 200 TESTING), then /status answers 503 RENTING_DISABLED twice.
    rent_replies[:] = [answer(200, testing)]
    status_replies[:] = [answer(503, disabled), answer(503, disabled), answer(200, final)]
    runner, result, shown = run()
    assert len(rents()) == 1 and len(statuses()) == 3, [c[:2] for c in calls]
    assert result["verdict"] == "PASS" and result["verified"] == 1, result
else:
    raise AssertionError("unknown case " + case)
print("ok " + case)
