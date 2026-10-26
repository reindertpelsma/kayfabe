"""No network: the actual controller across silent failure phases."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile

spec = importlib.util.spec_from_file_location(
    "silent_runner", Path(__file__).parents[1] / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
case = sys.argv[1]
sentinel = "PRIVATE-SENTINEL"
m.time.sleep = lambda _: None
calls = []
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
error = {
    "state": "UNAVAILABLE",
    "verdict": "UNAVAILABLE",
    "log_tail_sanitised": "",
    "bytes_dropped": 0,
}


def client(url, method="GET", headers=None, body=None):
    if url.startswith("https://oidc.example"):
        if case == "oidc-json":
            return m.Reply(200, {"content-type": "application/json"}, b"bad-json")
        return m.Reply(200, {"content-type": "application/json"}, b'{"value":"a.b.c"}')
    route = (
        "availability" if "/availability" in url else "rent" if url.endswith("/rent") else "stop"
    )
    calls.append(route)
    data, status = scope["availability"](), 200
    if route == "availability":
        if case.startswith("availability-5"):
            data, status = error, int(case.rsplit("-", 1)[1])
        elif case == "availability-shape":
            data = {"PRIVATE-SENTINEL": 1}
        elif case == "availability-json":
            return m.Reply(200, {"content-type": "application/json"}, b"PRIVATE-SENTINEL")
        elif case == "availability-runtime":
            raise RuntimeError(sentinel)
    elif route == "rent":
        data, status = error, 409
        if case == "no-offers":
            data = {**error, "reason": "NO_ALLOWED_OFFERS"}
        elif case == "rent-refusal":
            data = {**error, "state": "NOT_ELIGIBLE"}
        elif case.startswith("rent-5"):
            status = int(case.rsplit("-", 1)[1])
        elif case in ("bundle", "failed-result"):
            nonce = json.loads(body)["runner_nonce"]
            data = {
                "state": "NONE",
                "verdict": "FAIL",
                "run": "123-1",
                "id": "900",
                "sha": "a" * 40,
                "cleanup_pending": False,
                "result_at": 2000,
                "ended_at": 3000,
                "started_at": 1000,
                "deadline_at": 301000,
                "gpu_model": "RTX_4070",
                "cost_estimate_usd": 0.01,
                "log_cursor": 0,
                "log_tail_sanitised": "",
                "bytes_dropped": 0,
                "tail_bytes_dropped": 0,
                "runner_nonce": nonce,
                "log_sha256": "0" * 64,
                **manifest,
                **scope["extra_status"]("FAIL"),
            }
            if case == "bundle":
                data["bundle_sha256"] = "c" * 64
            status = 200
    else:
        data, status = error, 404
    return m.Reply(status, {"content-type": "application/json"}, json.dumps(data).encode())


m.https_request = client
out, err = io.StringIO(), io.StringIO()
with tempfile.TemporaryDirectory(prefix="wk1-silent-") as directory:
    root = Path(directory)
    (root / "manifest").write_text(json.dumps(manifest) if case != "manifest" else "bad")
    event = {
        "pull_request": {
            "head": {"sha": "a" * 40, "repo": {"full_name": "o/k"}},
            "base": {"ref": "master"},
        }
    }
    if case == "preferences":
        event["inputs"] = {"preferences": [sentinel]}
    (root / "event").write_text(json.dumps(event))
    os.environ.clear()
    os.environ.update(
        {
            "GITHUB_OUTPUT": str(root / ("missing/output" if case == "output" else "output")),
            "GITHUB_STEP_SUMMARY": str(root / "summary"),
            "GITHUB_EVENT_PATH": str(root / "event"),
            "GITHUB_SHA": "d" * 40,
            "GITHUB_EVENT_NAME": "pull_request_target",
            "GITHUB_REF": "refs/heads/master",
            "GITHUB_REPOSITORY": "o/k",
            "GITHUB_RUN_ID": "123",
            "GITHUB_RUN_ATTEMPT": "1",
            "WORKER_URL": "https://broker.example",
            "WORKER_PATH": "p" * 32,
            "ACTIONS_ID_TOKEN_REQUEST_URL": "https://oidc.example/?a=1",
            "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "mock-token",
        }
    )
    sys.argv = ["runner.py", str(root / "manifest")]
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            status = m.main()
        except Exception:
            status = 1
print(
    json.dumps(
        {"status": status, "stdout": out.getvalue(), "stderr": err.getvalue(), "calls": calls}
    )
)
