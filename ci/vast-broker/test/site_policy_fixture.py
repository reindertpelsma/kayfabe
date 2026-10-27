import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location(
    "site_policy", Path(__file__).resolve().parents[1] / "site_policy.py"
)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
case, variant = sys.argv[1:]
sha, before, tree = (c * 40 for c in "abd")
repo = "reindertpelsma/kayfabe"
event = {"pull_request": {"number": 7, "head": {"sha": sha}}, "before": before, "after": sha}
pr = {
    "number": 7,
    "changed_files": 1,
    "head": {"sha": sha, "repo": {"full_name": repo}},
    "base": {"repo": {"full_name": repo}},
}
files = [{"filename": "crates/core.rs", "status": "modified"}]
data = {
    "/pulls/7": pr,
    "/pulls/7/files?per_page=100&page=1": files,
    "/compare/"
    + before
    + "..."
    + sha
    + "?per_page=100": {"base_commit": {"sha": before}, "status": "ahead", "files": files},
}
name, ref = (
    ("pull_request", "refs/pull/7/merge") if variant == "pr" else ("push", "refs/heads/master")
)
expected, deploy, known = False, False, True
if case in {"site", "docs", "workflow", "removed", "renamed-out", "renamed-in"}:
    files[0]["filename"] = {
        "site": "site/build.py",
        "docs": "docs/a.md",
        "workflow": ".github/workflows/site.yml",
        "removed": "docs/deleted.md",
        "renamed-in": "site/new.py",
        "renamed-out": "src/old.py",
    }[case]
    if case in {"renamed-out", "renamed-in"}:
        files[0].update(status="renamed", previous_filename="site/old.py")
    if case == "removed":
        files[0]["status"] = "removed"
    expected, deploy = True, variant != "pr"
if case in {"odd", "unknown-status", "count", "missing", "sha", "truncated"}:
    expected, known = True, False
if case == "odd":
    files[0]["filename"] = "src/odd\\name"
if case == "unknown-status":
    files[0]["status"] = "mystery"
if case == "count":
    pr["changed_files"] = 2
    data["/compare/" + before + "..." + sha + "?per_page=100"]["base_commit"]["sha"] = "f" * 40
if case == "missing":
    data.clear()
if case == "sha":
    pr["head"]["sha"] = before
    event["after"] = before
if case == "truncated":
    pr["changed_files"] = 3001
    files[:] = [{"filename": "src/f" + str(i), "status": "modified"} for i in range(300)]
if case == "prefix-only":
    files[0]["filename"] = "website/source.md"
if case == "tag":
    name, ref = "push", "refs/tags/v1"
    expected, deploy = True, True
if case in {"schedule", "release", "workflow_dispatch"}:
    name, ref = case, "refs/heads/master"
    expected, deploy = True, True


def get(path):
    return json.loads(json.dumps(data[path]))


if not case.startswith("status-") and not case.startswith("reuse-"):
    result = m.decide(event, name, ref, sha, get)
    assert (result["run"], result["deploy"], result["known"]) == (expected, deploy, known), result
elif case.startswith("reuse-"):
    data.update(
        {
            "/commits/"
            + sha: {"sha": sha, "commit": {"tree": {"sha": tree}}, "parents": [{"sha": before}]},
            "/commits/" + before: {"sha": before, "commit": {"tree": {"sha": tree}}},
            "/commits/" + sha + "/pulls?per_page=100": [],
            "/commits/"
            + before
            + "/statuses?per_page=100": [
                {
                    "context": "site",
                    "state": "success",
                    "creator": {"login": "github-actions[bot]", "id": 41898282},
                    "description": "site build passed",
                    "target_url": "https://github.com/" + repo + "/actions/runs/123",
                }
            ],
            "/commits/"
            + before
            + "/check-runs?per_page=100": {
                "total_count": 1,
                "check_runs": [
                    {
                        "name": "CI",
                        "head_sha": before,
                        "status": "completed",
                        "conclusion": "success",
                        "app": {"slug": "github-actions", "id": 15368},
                    }
                ],
            },
            "/actions/runs/123": {
                "id": 123,
                "head_sha": before,
                "path": ".github/workflows/site.yml",
                "event": "pull_request",
                "head_branch": "next",
                "status": "completed",
                "conclusion": "success",
                "repository": {"full_name": repo},
            },
        }
    )
    if case == "reuse-tree":
        data["/commits/" + before]["commit"]["tree"]["sha"] = "f" * 40
    status = data["/commits/" + before + "/statuses?per_page=100"][0]
    if case == "reuse-creator":
        status["creator"]["id"] = 1
    if case == "reuse-pending":
        status["state"] = "pending"
    if case == "reuse-ci":
        data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"][0][
            "conclusion"
        ] = "failure"
    if case == "reuse-run":
        data["/actions/runs/123"]["conclusion"] = "failure"
    event_name, reference = "push", "refs/heads/master"
    if case == "reuse-pr":
        event_name = "pull_request"
    if case == "reuse-tag":
        reference = "refs/tags/v1"
    proof = m.find_reuse(sha, event_name, reference, get)
    assert bool(proof) == (case == "reuse-valid"), proof
else:
    run = {
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
    jobs = {
        "total_count": 3,
        "jobs": [
            {"name": "policy", "conclusion": "success"},
            {"name": "build", "conclusion": "success"},
            {"name": "deploy", "conclusion": "skipped"},
        ],
    }
    if variant == "push" and case != "status-fork":
        run["event"] = "push"
        run["head_branch"] = "master"
    data.update(
        {
            "/actions/runs/123": run,
            "/actions/runs/123/jobs?per_page=100": jobs,
            "/commits/" + sha + "/pulls?per_page=100": [pr],
        }
    )
    files[0]["filename"] = "docs/a.md"
    if case == "status-no-build":
        jobs["jobs"][1]["conclusion"] = "skipped"
    if case == "status-untouched":
        files[0]["filename"] = "src/a.rs"
        jobs["jobs"][1]["conclusion"] = "skipped"
    if case == "status-failure":
        run["conclusion"] = "failure"
    if case == "status-foreign":
        run["repository"]["full_name"] = "other/repo"
    if case == "status-wrong-path":
        run["path"] = ".github/workflows/other.yml"
    if case == "status-fork":
        run["head_repository"]["full_name"] = "fork/repo"
        pr["head"]["repo"]["full_name"] = "fork/repo"
    if case == "status-stale":
        pr["head"]["sha"] = before
        if variant == "push":
            run["event"] = "workflow_dispatch"
    if case == "status-pr-start":
        source, payload = "pull_request_target", event
    else:
        source, payload = "workflow_run", {"action": "completed", "workflow_run": {"id": 123}}
    calls = []
    m.report_status(payload, source, "456", get, lambda path, body: calls.append((path, body)))
    if case in {"status-foreign", "status-wrong-path", "status-stale"}:
        assert not any(body["state"] == "success" for _, body in calls), calls
    else:
        final = calls[-1][1]
        expected_state = (
            "pending"
            if case == "status-pr-start"
            else ("error" if case in {"status-no-build", "status-failure"} else "success")
        )
        assert final["state"] == expected_state, calls
        assert all(path == "/statuses/" + sha for path, _ in calls)
        assert final["context"] == "site"
        if case == "status-untouched":
            assert final["description"] == "not needed: site and docs untouched"
