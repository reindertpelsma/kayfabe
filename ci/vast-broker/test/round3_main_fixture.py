"""Execute the real isolated controller entry point with no network."""

import contextlib, hashlib, importlib.util, io, json, os, sys, tempfile, urllib.parse
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "runner_main", Path(__file__).resolve().parents[1] / "runner.py"
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
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
manifest["protocol_version"] = args.get("manifest_protocol", manifest["protocol_version"])
status_protocol = args.get("status_protocol", manifest["protocol_version"])
sha = "a" * 40
base = "d" * 40
calls = []
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
            "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "mock-request-token",
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
        text += origin + " " + prefix + " mock.jwt.value\n"
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
            return m.Reply(200, {"content-type": "application/json"}, b'{"value":"mock.jwt.value"}')
        route = urllib.parse.urlsplit(url).path
        if route.endswith("/availability"):
            return m.Reply(
                200, {"content-type": "application/json"}, json.dumps(availability()).encode()
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
    m.time.sleep = lambda _: None
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
            "calls": calls,
            "nonce": nonce,
            "workflow_sha": os.environ["GITHUB_SHA"],
        }
    )
)
