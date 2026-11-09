"""Replay actual mocked Worker status/log pages through the trusted BASE runner. No network."""

import io
import json
from pathlib import Path
import runpy
import sys

data = json.load(sys.stdin)
runner = runpy.run_path(
    str(Path(__file__).resolve().parents[1] / "runner.py"), run_name="middle_apps_replay"
)
manifest = {
    key: data["status"][key] for key in ("protocol_version", "worker_version", "bundle_sha256")
}


def client(route, method="GET", body=None):
    value = (
        data["availability"]
        if route.startswith("/availability")
        else data["status"] if route == "/rent" else data["pages"].get(route, {})
    )
    return runner["Reply"](200, {"content-type": "application/json"}, json.dumps(value).encode())


controller = runner["Runner"](
    {
        "run": "123-1",
        "sha": data["status"]["sha"],
        "nonce": data["status"]["runner_nonce"],
        "profile": "check",
        "apps_policy_required": True,
    },
    manifest,
    client,
    emit=lambda _: None,
    summary=io.StringIO(),
)
result = controller.execute()
print(json.dumps({"result": result, "retry": controller.retry}))
