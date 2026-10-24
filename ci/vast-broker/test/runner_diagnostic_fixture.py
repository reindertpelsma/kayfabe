"""No network: the real runner main() against a fake HTTPS layer, to check the one-line
diagnostic printed outside the untrusted block. argv[1] is a JSON object:
  env   - overrides on the baseline environment (null removes a variable)
  reply - what the Worker answers: html403, text404, redirect302, timeout, reset
Prints one JSON object with status, stdout, stderr, summary and outputs."""

import contextlib, http.client, importlib.util, io, json, os, sys, tempfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "runner_diag", Path(__file__).resolve().parents[1] / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner_diag"] = m
spec.loader.exec_module(m)
args = json.loads(sys.argv[1])
SENTINEL = args["sentinel"]
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}


class Response:
    def __init__(self, status, headers, body):
        self.status, self.headers, self.body = status, headers, body

    def getheader(self, name, default=None):
        for key, value in self.headers.items():
            if key.lower() == name.lower():
                return value
        return default

    def getheaders(self):
        return list(self.headers.items())

    def read(self, limit=None):
        return self.body


class Connection:
    def __init__(self, host, port=443, timeout=None):
        self.host = host

    def request(self, method, path, body=None, headers=None):
        if self.host == "oidc.example":
            return
        if args["reply"] == "timeout":
            raise TimeoutError(SENTINEL)
        if args["reply"] == "reset":
            raise ConnectionResetError(SENTINEL)

    def getresponse(self):
        if self.host == "oidc.example":
            return Response(200, {"Content-Type": "application/json"}, b'{"value":"a.b.c"}')
        page = ("<html>" + SENTINEL + "</html>").encode()
        reply = args["reply"]
        if reply == "html403":
            return Response(403, {"Content-Type": "text/html; charset=utf-8"}, page)
        if reply == "text404":
            return Response(404, {"Content-Type": "text/plain"}, ("Not Found " + SENTINEL).encode())
        if reply == "weird":
            return Response(502, {"Content-Type": "x y<z>" + "q" * 80}, page)
        if reply == "redirect302":
            return Response(
                302,
                {"Content-Type": "text/html", "Location": "https://" + SENTINEL + ".example/"},
                b"",
            )
        raise AssertionError(reply)

    def close(self):
        pass


http.client.HTTPSConnection = Connection
m.time.sleep = lambda _: None
stdout, stderr = io.StringIO(), io.StringIO()
with tempfile.TemporaryDirectory(prefix="kf-diag-") as directory:
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
    env = {
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
    env.update(args.get("env", {}))
    os.environ.clear()
    os.environ.update({k: v for k, v in env.items() if v is not None})
    sys.argv = ["runner.py", str(d / "manifest")]
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        status = m.main()
    summary = (d / "summary").read_text() if (d / "summary").exists() else ""
    outputs = (d / "output").read_text()
print(
    json.dumps(
        {
            "status": status,
            "stdout": stdout.getvalue(),
            "stderr": stderr.getvalue(),
            "summary": summary,
            "outputs": outputs,
        }
    )
)
