import hashlib, importlib.util, io, json, sys
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "runner", Path(__file__).resolve().parents[1] / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
extra_status = scope["extra_status"]
availability = scope["availability"]
case = sys.argv[1]
ctx = {"sha": "a" * 40, "run": "123-1", "nonce": "f" * 64}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
if case.startswith("transport-"):
    import time

    class Fake:
        status = 200

        def __init__(self, *_, **__):
            pass

        def request(self, *_, **__):
            pass

        def getresponse(self):
            if case == "transport-header-timeout":
                time.sleep(0.1)
            return self

        def getheader(self, *_):
            return "2"

        def getheaders(self):
            return [("content-type", "application/json")]

        def read(self, *_):
            if case == "transport-body-timeout":
                time.sleep(0.1)
            return b"{}"

        def close(self):
            pass

    m.http.client.HTTPSConnection = Fake
    original = m.deadline
    m.deadline = lambda _: original(0.02)
    try:
        m.https_request("https://broker.example")
        raise AssertionError("absolute transport deadline missing")
    except TimeoutError:
        pass
    raise SystemExit(0)
if case.startswith("framing-"):
    printed = []
    r = m.Runner(ctx, manifest, lambda *_: None, emit=printed.append)
    for _ in range(1000):
        r.show("safe\n" * 10)
    output = "".join(printed)
    if case == "framing-bytes":
        assert len(output.encode()) < 70000
    else:
        assert sum(line.startswith("[untrusted]") for line in output.splitlines()) <= 200
    raise SystemExit(0)
m.time.sleep = lambda _: None
text = {
    "controls": "safe\x1b[31mRED\x1b[0m\x1b]0;hidden\x07\u202eEVIL\u200b\x00\r\x7f\u0085é\n",
    "commands": "::set-output name=x::oops\n::add-mask::secret\n::error::x\n##[group]\n",
    "caps": "L" * 100000 + "\n" + "line\n" * 10000,
    "display-only": "PASS\nstate=COMPLETE\nverdict=PASS\n",
}.get(case, "ordinary bounded log\n")
encoded = text.encode()
h = hashlib.sha256(bytes(32) + len(encoded).to_bytes(4, "big") + encoded).hexdigest()
verdict = "FAIL" if case in {"display-only", "cursor-race-fail"} else "PASS"
status = {
    "state": "GRACE",
    "verdict": verdict,
    "run": ctx["run"],
    "id": "900",
    "sha": ctx["sha"],
    "cleanup_pending": True,
    "result_at": 2000,
    "ended_at": None,
    "started_at": 1000,
    "deadline_at": 302000,
    "gpu_model": "RTX 4070",
    "cost_estimate_usd": 0.01,
    "log_cursor": 1,
    "log_tail_sanitised": text[:100],
    "bytes_dropped": 0,
    "tail_bytes_dropped": 0,
    "runner_nonce": ctx["nonce"],
    "log_sha256": h,
    **manifest,
}
status.update(extra_status(verdict))
if case == "extra":
    status["evil"] = "unexpected"
if case == "extra-object":
    status["unexpected"] = {"command": "no"}
if case == "wrong-sha":
    status["sha"] = "c" * 40
if case == "wrong-nonce":
    status["runner_nonce"] = "0" * 64
if case == "wrong-run":
    status["run"] = "124-1"
if case == "bad-state":
    status["state"] = "::error::"
if case == "bad-verdict":
    status["verdict"] = "PAss"
if case == "wrong-hash":
    status["log_sha256"] = "0" * 64
if case == "wrong-version":
    status["bundle_sha256"] = "0" * 64
if case == "wrong-worker-version":
    status["worker_version"] = "v4-other"
if case.startswith("cursor-race-"):
    h = m.chain(h, "newer log page\n")
    status.update(log_cursor=2, log_sha256=h)
calls = []
printed = []
summary = io.StringIO()


def client(route, method="GET", body=None):
    calls.append((route, method, body))
    if route.startswith("/availability?"):
        return m.Reply(
            200, {"content-type": "application/json"}, json.dumps(availability()).encode()
        )
    if route == "/rent":
        assert body["runner_nonce"] == ctx["nonce"] and body["tested_sha"] == ctx["sha"]
        if case == "timeout":
            raise TimeoutError()
        first = (
            {
                **status,
                "state": "TESTING",
                "verdict": "UNAVAILABLE",
                "result_at": None,
                "log_cursor": 1,
                **extra_status(pending=True),
            }
            if case.startswith("cursor-race-")
            else status
        )
        raw = json.dumps(first, ensure_ascii=False).encode()
        if case == "duplicate":
            raw = raw[:-1] + b',"verdict":"PASS"}'
        if case == "duplicate-sha":
            raw = raw[:-1] + b',"sha":"' + ctx["sha"].encode() + b'"}'
        if case == "invalid-utf8":
            raw = b'{"state":"\xff"}'
        if case == "huge":
            raw = b"X" * 10000000
        if case == "html":
            raw = b"<html>200</html>"
        return m.Reply(
            302 if case == "redirect" else 307 if case == "redirect-307" else 200,
            {"content-type": "text/html" if case == "html" else "application/json"},
            raw,
        )
    if route.startswith("/status/"):
        return m.Reply(200, {"content-type": "application/json"}, json.dumps(status).encode())
    if route.startswith("/logs/"):
        chunks = (
            [text[i : i + 4000] for i in range(0, len(text), 4000)] if case == "caps" else [text]
        )
        if case.startswith("cursor-race-"):
            chunks = [text, "newer log page\n"]
        # The caps probe is intentionally one overlarge page: bounded fail-closed.
        data = {
            "cursor": 0,
            "next_cursor": len(chunks),
            "chunks": chunks,
            "log_increment_sanitised": "".join(chunks),
            "log_sha256": "0" * 64 if case == "wrong-page-hash" else h,
            "bytes_dropped": 0,
        }
        return m.Reply(
            200, {"content-type": "application/json"}, json.dumps(data, ensure_ascii=False).encode()
        )
    return m.Reply(200, {"content-type": "application/json"}, b"{}")


result = m.Runner(ctx, manifest, client, emit=printed.append, summary=summary).execute()
output = "".join(printed) + summary.getvalue()
if case in {"valid", "controls", "commands", "cursor-race-pass"}:
    assert result["verdict"] == "PASS", result
elif case in {"display-only", "cursor-race-fail"}:
    assert result["verdict"] == "FAIL", result
else:
    assert result["verdict"] == "UNAVAILABLE", result
assert result["state"] in m.STATES and result["verdict"] in m.VERDICTS
assert type(result["verified"]) is int and result["verified"] in {0, 1}
assert (
    "\x1b" not in output
    and "\x00" not in output
    and "\r" not in output
    and "\u202e" not in output
    and "\u200b" not in output
    and "\x7f" not in output
    and "\u0085" not in output
)
assert (
    "::set-output" not in output
    and "::add-mask" not in output
    and "::error" not in output
    and "##[" not in output
)
assert len(output.encode()) < 150000
if case == "caps":
    safe, dropped = m.scrub(encoded, max_bytes=4096, max_lines=20, max_line=100)
    assert dropped > 0 and "bytes dropped" in safe and len(safe.encode()) < 5000
if case in {"controls", "commands"}:
    assert "UNTRUSTED" in output
assert next(c for c in calls if c[0] == "/rent")[2]["runner_nonce"] == ctx["nonce"]
