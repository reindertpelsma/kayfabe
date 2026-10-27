"""No network: real runner diagnostic admission and public-number boundary."""

import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location(
    "budget_runner", Path(__file__).parents[1] / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = m
spec.loader.exec_module(m)
args = json.loads(sys.argv[1])
ctx = {"run": "123-1", "sha": "a" * 40, "nonce": "f" * 64}
data = {
    "state": "UNAVAILABLE_BUDGET",
    "verdict": "UNAVAILABLE",
    "log_tail_sanitised": "",
    "bytes_dropped": 0,
    "reason": args["reason"],
    "budget": args["budget"],
}
try:
    m.validate_status(data, ctx, {}, 409)
    scope = {}
    exec(Path(__file__).with_name("v5_fixture_data.py").read_text(), scope)
    out = []

    def client(route, method="GET", body=None):
        if route.startswith("/availability"):
            return m.Reply(
                200,
                {"content-type": "application/json"},
                json.dumps(scope["availability"]()).encode(),
            )
        return m.Reply(409, {"content-type": "application/json"}, json.dumps(data).encode())

    result = m.Runner(ctx, {}, client, emit=out.append).execute()
    print(json.dumps({"valid": True, "text": "".join(out), "result": result}))
except m.Invalid:
    print(json.dumps({"valid": False}))
