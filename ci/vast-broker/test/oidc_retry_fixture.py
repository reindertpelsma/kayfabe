"""The real controller entry point against a fake GitHub token endpoint; no network, fake clock.

usage: oidc_retry_fixture.py '<json>'      (KF_RUNNER_PATH overrides the runner under test)
  script   replies of the token endpoint, one per request; after the list (or with "repeat": the
           last entry for ever) the endpoint answers 200 with a good token. An entry is an HTTP
           status, {"status", "retry_after"}, "timeout", "reset", "garbled", "shape", "long" or
           "redirect".
  skip     good token replies before the script starts (a failure in the middle of a run)
  seconds  fake seconds one token request takes (default 0)
  frozen   the fake sleep does not advance the clock (the attempt cap alone must end the loop)
Prints one JSON object. A runaway loop (more than 60 token requests) or a negative sleep exits 3.
"""

import contextlib, hashlib, importlib.util, io, json, os, sys, tempfile, urllib.parse
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "runner_main",
    os.environ.get("KF_RUNNER_PATH") or Path(__file__).resolve().parents[1] / "runner.py",
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner_main"] = m
spec.loader.exec_module(m)
scope = {}
exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
extra_status = scope["extra_status"]
availability = scope["availability"]
args = json.loads(sys.argv[1])
origin = "https://quiet-tenant.workers.example"
prefix = args.get("path", "unit-path-only-" + "p" * 32)
manifest = {"protocol_version": 7, "worker_version": "v5", "bundle_sha256": "b" * 64}
manifest["protocol_version"] = args.get("manifest_protocol", manifest["protocol_version"])
status_protocol = args.get("status_protocol", manifest["protocol_version"])
sha = "a" * 40
base = "d" * 40
calls = []
SENTINEL = "SeNtInEl7b2e"
TOKEN = "hdr" + SENTINEL + ".payload" + SENTINEL + ".sig" + SENTINEL
REQUEST_TOKEN = "request-token-" + SENTINEL
script = list(args.get("script", []))
now, sleeps, oidc = [5000.0], [], []


def fake_sleep(seconds):
    if seconds < 0:
        raise SystemExit(3)
    sleeps.append([len(oidc), seconds])
    if not args.get("frozen"):
        now[0] += seconds


def token_endpoint(url, headers):
    oidc.append({"url": url, "authorization": headers.get("Authorization"), "at": now[0] - 5000.0})
    if len(oidc) > 60:
        raise SystemExit(3)
    now[0] += args.get("seconds", 0)
    index = len(oidc) - 1 - args.get("skip", 0)
    good = m.Reply(200, {"content-type": "application/json"}, json.dumps({"value": TOKEN}).encode())
    if index < 0:
        return good
    if index >= len(script):
        if not args.get("repeat"):
            return good
        index = len(script) - 1
    entry = script[index]
    if entry == "timeout":
        raise TimeoutError(SENTINEL)
    if entry == "reset":
        raise ConnectionResetError(SENTINEL)
    if entry == "redirect":
        raise m.Diagnosed(m.reply_line(302, "text/html"))
    if entry == "garbled":
        return m.Reply(200, {"content-type": "text/html"}, ("<html>" + SENTINEL).encode())
    if entry == "shape":
        body = json.dumps({"value": "not a jwt " + SENTINEL}).encode()
        return m.Reply(200, {"content-type": "application/json"}, body)
    if entry == "long":
        body = json.dumps({"value": "a.b." + "c" * 8192}).encode()
        return m.Reply(200, {"content-type": "application/json"}, body)
    if isinstance(entry, int):
        entry = {"status": entry}
    headers = {"content-type": "text/plain"}
    if "retry_after" in entry:
        headers["retry-after"] = entry["retry_after"]
    return m.Reply(entry["status"], headers, ("refused " + SENTINEL).encode())


stdout = io.StringIO()
summary = ""
outputs = ""
nonce = None
with tempfile.TemporaryDirectory(prefix="v3-r3-main-") as directory:
    d = Path(directory)
    (d / "manifest").write_text(json.dumps(manifest))
    (d / "output").write_text("")
    event = {
        "number": 7,
        "pull_request": {
            "head": {
                "sha": sha,
                "repo": {
                    "full_name": "fork/kayfabe" if args.get("fork") else "reindertpelsma/kayfabe"
                },
            },
            "base": {"ref": "master"},
        },
    }
    (d / "event").write_text(json.dumps(event))
    os.environ.clear()
    os.environ.update(
        {
            "WORKER_URL": args.get("url", origin),
            "WORKER_PATH": prefix,
            "GITHUB_OUTPUT": str(d / "output"),
            "GITHUB_STEP_SUMMARY": str(d / "summary"),
            "GITHUB_EVENT_PATH": str(d / "event"),
            "GITHUB_SHA": base,
            "GITHUB_EVENT_NAME": "pull_request_target",
            "GITHUB_REF": "refs/heads/master",
            "GITHUB_REPOSITORY": "reindertpelsma/kayfabe",
            "GITHUB_RUN_ID": "123",
            "GITHUB_RUN_ATTEMPT": "1",
            "ACTIONS_ID_TOKEN_REQUEST_URL": "https://oidc.example/request?test=1",
            "ACTIONS_ID_TOKEN_REQUEST_TOKEN": REQUEST_TOKEN,
        }
    )
    if args.get("manual"):
        os.environ.update(
            GITHUB_EVENT_NAME="workflow_dispatch",
            GITHUB_SHA=sha,
            GITHUB_REF=args.get("ref", "refs/heads/master"),
        )
    if args.get("push"):
        os.environ.update(GITHUB_EVENT_NAME="push", GITHUB_SHA=sha)
    if "selected_sha" in args:
        event["inputs"] = {"sha": args["selected_sha"]}
        (d / "event").write_text(json.dumps(event))
        if isinstance(args["selected_sha"], str) and len(args["selected_sha"]) == 40:
            sha = args["selected_sha"]
    if args.get("access"):
        os.environ["CF_ACCESS_CLIENT_ID"] = "runner-a.access"
        os.environ["CF_ACCESS_CLIENT_SECRET"] = "a" * 64
    text = "[box] ::error::hostile\n"
    if args.get("leak"):
        text += origin + " " + prefix + " " + TOKEN + "\n"
    raw = text.encode()
    h = hashlib.sha256(bytes(32) + len(raw).to_bytes(4, "big") + raw).hexdigest()
    count = 0

    def request(url, method="GET", headers=None, body=None):
        global nonce, count
        masks = [
            line[12:] for line in stdout.getvalue().splitlines() if line.startswith("::add-mask::")
        ]
        calls.append(
            {
                "url": url,
                "method": method,
                "headers": headers,
                "masks": masks,
                "request": json.loads(body) if body is not None else None,
            }
        )
        if url.startswith("https://oidc.example/"):
            calls.pop()
            return token_endpoint(url, headers)
        route = urllib.parse.urlsplit(url).path
        if route.endswith("/availability"):
            return m.Reply(
                200,
                {"content-type": "application/json"},
                json.dumps(availability(apps=True)).encode(),
            )
        if route.endswith("/rent"):
            nonce = json.loads(body)["runner_nonce"]
        if "/stop/" in route:
            return m.Reply(200, {"content-type": "application/json"}, b"{}")
        if "/logs/" in route:
            return m.Reply(
                200,
                {"content-type": "application/json"},
                json.dumps(
                    {
                        "cursor": 0,
                        "next_cursor": 1,
                        "chunks": [text],
                        "log_increment_sanitised": text,
                        "log_sha256": h,
                        "bytes_dropped": 0,
                    }
                ).encode(),
            )
        count += 1
        if (
            args.get("failure") == "rent"
            and count == 1
            or args.get("failure") == "status"
            and count > 1
        ):
            raise OSError(url + " " + prefix)
        state = args.get("state", "GRACE")
        verdict = args.get("verdict", "PASS")
        pending = args.get("pending") and count == 1
        if pending:
            state = "TESTING"
            verdict = "UNAVAILABLE"
        code = args.get("code", 200)
        data = {"state": state, "verdict": verdict, "log_tail_sanitised": text, "bytes_dropped": 0}
        if code == 200:
            data.update(
                run="123-1",
                id="900",
                sha=sha,
                cleanup_pending=True,
                result_at=None if pending else 2000,
                ended_at=None,
                started_at=1000,
                deadline_at=302000,
                gpu_model="RTX 4070",
                cost_estimate_usd=0.01,
                log_cursor=0 if pending else 1,
                tail_bytes_dropped=0,
                runner_nonce=nonce,
                log_sha256="0" * 64 if pending else h,
                **dict(manifest, protocol_version=status_protocol),
            )
        if code == 200:
            data.update(extra_status(verdict, pending))
        return m.Reply(code, {"content-type": "application/json"}, json.dumps(data).encode())

    m.https_request = request
    m.time.sleep = fake_sleep
    m.time.monotonic = lambda: now[0]
    sys.argv = ["runner.py", str(d / "manifest")]
    with contextlib.redirect_stdout(stdout):
        status = m.main()
    outputs = (d / "output").read_text()
    summary = (d / "summary").read_text() if (d / "summary").exists() else ""
print(
    json.dumps(
        {
            "status": status,
            "stdout": stdout.getvalue(),
            "summary": summary,
            "outputs": outputs,
            "routes": [urllib.parse.urlsplit(c["url"]).path.split("/")[2] for c in calls],
            "bearers": sorted({c["headers"]["Authorization"] for c in calls}),
            "oidc": oidc,
            "sleeps": sleeps,
            "token": TOKEN,
            "request_token": REQUEST_TOKEN,
            "sentinel": SENTINEL,
        }
    )
)
