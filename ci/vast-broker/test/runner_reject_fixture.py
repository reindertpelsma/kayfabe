"""No network: the real Runner.execute() against a mocked Worker whose /rent (or /status) 200 body
is valid except for the fields argv[1] mutates. argv[1] is a JSON object:
  mutate - fields to overwrite in the 200 body (the marker may be inside a value)
  where  - "rent" (default) or "status": which reply carries the mutation
Prints one JSON object: result, shown (everything the runner emitted), the route list, and the
length of what had been emitted when each /stop was sent."""

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
args = json.loads(sys.argv[1])
m.time.sleep = lambda _: None
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
page = {
    "cursor": 0,
    "next_cursor": 1,
    "chunks": [text],
    "log_increment_sanitised": text,
    "log_sha256": digest,
    "bytes_dropped": 0,
}
json_header = {"content-type": "application/json"}
answer = lambda status, data: m.Reply(status, json_header, json.dumps(data).encode())
mutate = args.get("mutate", {})
where = args.get("where", "rent")
shown = []
calls = []
stop_marks = []


def client(route, method="GET", body=None):
    calls.append(route)
    if route.startswith("/availability"):
        return answer(200, scope["availability"]())
    if route == "/rent":
        return answer(200, testing if where == "status" else dict(final, **mutate))
    if route.startswith("/status/"):
        return answer(200, dict(final, **mutate))
    if route.startswith("/logs/"):
        return answer(200, page)
    if route.startswith("/stop/"):
        stop_marks.append(len("".join(shown)))
        return answer(200, dict(final, state="NONE", cleanup_pending=False, ended_at=3000))
    raise AssertionError(route)


runner = m.Runner(ctx, manifest, client, emit=shown.append, summary=io.StringIO())
result = runner.execute()
print(
    json.dumps(
        {
            "result": result,
            "shown": "".join(shown),
            "calls": calls,
            "stop_marks": stop_marks,
            "source": (broker / "runner.py").read_text().splitlines(),
        }
    )
)
