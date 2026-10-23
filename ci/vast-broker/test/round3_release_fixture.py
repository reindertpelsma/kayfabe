import importlib.util, sys
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "release_gate", Path(__file__).resolve().parents[1] / "release_gate.py"
)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
case = sys.argv[1]
sha = "a" * 40
repo = "reindertpelsma/kayfabe"
status = {
    "sha": sha,
    "statuses": [
        {
            "context": "gpu-vast",
            "state": "success",
            "creator": {"login": "github-actions[bot]", "id": 41898282},
            "target_url": "https://github.com/" + repo + "/actions/runs/123",
            "description": "GPU broker GRACE",
        }
    ],
}
run = {
    "id": 123,
    "path": ".github/workflows/gpu-vast.yml",
    # Review W4: gpu-vast.yml runs on push to master (and pull_request_target), never dispatch.
    "event": "push",
    "head_sha": sha,
    "head_branch": "master",
    "status": "completed",
    "conclusion": "success",
    "repository": {"full_name": repo},
    "head_repository": {"full_name": repo},
}
if case == "wrong-sha":
    status["sha"] = "b" * 40
if case == "wrong-status":
    status["statuses"][0]["state"] = "pending"
if case == "wrong-creator":
    status["statuses"][0]["creator"]["id"] = 1
if case == "wrong-url":
    status["statuses"][0]["target_url"] = "https://evil.example/actions/runs/123"
if case == "wrong-workflow":
    run["path"] = ".github/workflows/evil.yml"
if case == "wrong-event":
    run["event"] = "pull_request"
if case == "wrong-run-sha":
    run["head_sha"] = "b" * 40
if case == "wrong-conclusion":
    run["conclusion"] = "failure"
if case == "wrong-repo":
    run["head_repository"]["full_name"] = "fork/kayfabe"
if case == "valid-none":
    status["statuses"][0]["description"] = "GPU broker NONE"
if case == "dispatch-run":
    run["event"] = "workflow_dispatch"
if case == "pr-target-run":
    run["event"] = "pull_request_target"
if case == "wrong-branch":
    run["head_branch"] = "feature"
if case == "not-needed":
    status["statuses"][0]["description"] = "GPU broker NOT_NEEDED"
if case == "no-description":
    del status["statuses"][0]["description"]
assert m.verify(sha, status, lambda run_id: run) == (case in {"valid", "valid-none"}), case
