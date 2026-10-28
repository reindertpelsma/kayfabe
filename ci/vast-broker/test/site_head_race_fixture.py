"""FALSIFIER: a changed PR binding cannot authorize skipping the original head."""

import copy
import importlib.util
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location(
    "site_policy", Path(__file__).resolve().parents[1] / "site_policy.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
mode, change = sys.argv[1:]
sha, base = "a" * 40, "b" * 40
repo = "reindertpelsma/kayfabe"
pr = {
    "number": 7,
    "changed_files": 1,
    "head": {"sha": sha, "repo": {"full_name": repo}},
    "base": {"sha": base, "repo": {"full_name": repo}},
}
updated = copy.deepcopy(pr)
if change == "head":
    updated["head"]["sha"] = "c" * 40
elif change == "base":
    updated["base"]["sha"] = "d" * 40
elif change == "count":
    updated["changed_files"] = 2
elif change != "stable":
    raise AssertionError(change)
reads = 0


def get(path):
    global reads
    if path == "/pulls/7":
        reads += 1
        return copy.deepcopy(pr if reads == 1 else updated)
    if path == "/pulls/7/files?per_page=100&page=1":
        # The newer PR is code-only; the original may have changed site/build.py.
        return [{"filename": "crates/core.rs", "status": "modified"}]
    if path == "/commits/" + sha + "/statuses?per_page=100":
        return []
    if path == "/commits/" + sha + "/pulls?per_page=100":
        return [copy.deepcopy(pr)]
    if path == "/actions/runs/123":
        return {
            "id": 123,
            "path": ".github/workflows/site.yml",
            "event": "pull_request",
            "head_sha": sha,
            "head_branch": "feature",
            "status": "completed",
            "conclusion": "success",
            "repository": {"full_name": repo},
            "head_repository": {"full_name": repo},
        }
    if path == "/actions/runs/123/jobs?per_page=100":
        return {"total_count": 1, "jobs": [{"name": "build", "conclusion": "skipped"}]}
    raise KeyError(path)


event = {"pull_request": copy.deepcopy(pr)}
stable = change == "stable"
if mode == "policy":
    result = module.decide(event, "pull_request", "", sha, get)
    assert result == {"run": not stable, "deploy": False, "known": stable}, result
else:
    writes = []
    if mode == "start":
        source, payload = "pull_request_target", event
    elif mode == "completion":
        source, payload = "workflow_run", {"action": "completed", "workflow_run": {"id": 123}}
    else:
        raise AssertionError(mode)
    module.report_status(
        payload, source, "456", get, lambda path, body: writes.append((path, body))
    )
    assert writes, "must leave a visible non-passing status on uncertainty"
    assert all(path == "/statuses/" + sha for path, _ in writes), writes
    assert any(body["state"] == "success" for _, body in writes) == stable, writes
assert reads == 2, ("must bracket pagination with PR reads", reads)
