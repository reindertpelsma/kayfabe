import importlib.util
import json
import contextlib
import io
import os
import tempfile
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
if case in {"odd", "unknown-status", "count", "missing", "sha", "truncated", "too-many-files"}:
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
if case in {"truncated", "too-many-files"}:
    pr["changed_files"] = 3001 if case == "truncated" else 10000
    files[:] = [{"filename": "src/f" + str(i), "status": "modified"} for i in range(300)]
if case == "prefix-only":
    files[0]["filename"] = "website/source.md"
if case == "tag":
    name, ref = "push", "refs/tags/v1"
    expected, deploy = True, True
if case in {"schedule", "release", "workflow_dispatch"}:
    name, ref = case, "refs/heads/master"
    expected, deploy = True, True


calls_read = []


def get(path):
    calls_read.append(path)
    return json.loads(json.dumps(data[path]))


if not case.startswith("status-") and not case.startswith("reuse-"):
    result = m.decide(event, name, ref, sha, get)
    assert (result["run"], result["deploy"], result["known"]) == (expected, deploy, known), result
    if variant == "pr" and case in {"truncated", "too-many-files"}:
        assert not any("/files?" in path for path in calls_read), calls_read
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
    ci_check = data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"][0] if data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"] else None
    ci_run = {
        "id": 321, "head_sha": before, "path": ".github/workflows/ci.yml",
        "repository": {"full_name": repo}, "status": "completed", "conclusion": "success",
        "check_suite_id": 654,
    }
    data["/actions/runs/321"] = ci_run
    if ci_check:
        ci_check.update(id=987, check_suite={"id": 654},
                        details_url="https://github.com/" + repo + "/actions/runs/321/job/987")
    if case == "reuse-ci-other-workflow":
        ci_run["path"] = ".github/workflows/unrelated.yml"
    if case == "reuse-ci-run-pending":
        ci_run["status"] = "in_progress"
    if case == "reuse-ci-run-head":
        ci_run["head_sha"] = sha
    if case == "reuse-ci-suite":
        ci_run["check_suite_id"] = 1
    if case in {"reuse-ci-ambiguous-failed", "reuse-ci-ambiguous-pending", "reuse-ci-multiple-success"}:
        duplicate = json.loads(json.dumps(ci_check))
        if case == "reuse-ci-ambiguous-failed":
            duplicate["conclusion"] = "failure"
        if case == "reuse-ci-ambiguous-pending":
            duplicate["status"] = "in_progress"
        # Legitimate push+PR runs: distinct job, suite and run of the same intended workflow.
        duplicate.update(id=988, check_suite={"id": 655},
                         details_url="https://github.com/" + repo + "/actions/runs/322/job/988")
        data["/actions/runs/322"] = {**ci_run, "id": 322, "check_suite_id": 655}
        data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"].append(duplicate)
        data["/commits/" + before + "/check-runs?per_page=100"]["total_count"] = 2
    if variant == "pr":
        data["/commits/" + sha]["parents"] = []
        data["/commits/" + sha + "/pulls?per_page=100"] = [{"head": {"sha": before}}]
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
    if case == "reuse-ci-pending":
        data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"][0][
            "status"
        ] = "queued"
    if case == "reuse-ci-app":
        data["/commits/" + before + "/check-runs?per_page=100"]["check_runs"][0]["app"]["id"] = 1
    if case == "reuse-run-sha":
        data["/actions/runs/123"]["head_sha"] = sha
    if case == "reuse-run":
        data["/actions/runs/123"]["conclusion"] = "failure"
    event_name, reference = "push", "refs/heads/master"
    if case == "reuse-pr":
        event_name = "pull_request"
    if case == "reuse-tag":
        reference = "refs/tags/v1"
    proof = m.find_reuse(sha, event_name, reference, get)
    assert bool(proof) == (case in {"reuse-valid", "reuse-ci-multiple-success"}), proof
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
    jobs["jobs"][0]["check_run_url"] = "https://api.github.com/repos/" + repo + "/check-runs/10"
    data["/check-runs/10"] = {
        "head_sha": sha,
        "conclusion": "success",
        "app": {"id": 15368, "slug": "github-actions"},
    }
    data["/check-runs/10/annotations?per_page=100"] = [
        {"title": "Site push range", "message": "before=" + before + " after=" + sha}
    ]
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
    if case == "status-bad-annotation":
        files[0]["filename"] = "src/a.rs"
        run["event"], run["head_branch"] = "push", "master"
        jobs["jobs"][1]["conclusion"] = "skipped"
        if variant == "pr":
            data["/check-runs/10"]["app"]["id"] = 1
        else:
            data["/check-runs/10"]["head_sha"] = before
    if case == "status-late-start":
        data["/commits/" + sha + "/statuses?per_page=100"] = [
            {
                "context": "site",
                "state": "success" if variant == "pr" else "error",
                "creator": {"login": "github-actions[bot]", "id": 41898282},
            }
        ]
    if case in {"status-pr-start", "status-late-start"}:
        source, payload = "pull_request_target", event
    else:
        source, payload = "workflow_run", {"action": "completed", "workflow_run": {"id": 123}}
    if case == "status-late-run":
        payload["action"] = "in_progress"
    calls = []
    m.report_status(payload, source, "456", get, lambda path, body: calls.append((path, body)))
    if case == "status-late-start":
        assert calls == [], calls
    elif case in {"status-foreign", "status-wrong-path", "status-stale"}:
        assert not any(body["state"] == "success" for _, body in calls), calls
    else:
        final = calls[-1][1]
        expected_state = (
            "pending"
            if case == "status-pr-start"
            else (
                "error"
                if case in {"status-no-build", "status-failure", "status-bad-annotation"}
                else "success"
            )
        )
        assert final["state"] == expected_state, calls
        assert all(path == "/statuses/" + sha for path, _ in calls)
        assert final["context"] == "site"
        if case == "status-untouched":
            assert final["description"] == "not needed: site and docs untouched"

if case in {"untouched", "site", "missing"}:
    original_decide = m.decide
    m.decide = lambda event, name, ref, sha: original_decide(event, name, ref, sha, get)
    with tempfile.TemporaryDirectory(prefix="wk2-site-cli-") as directory:
        event_path, output_path = Path(directory) / "event.json", Path(directory) / "output"
        event_path.write_text(json.dumps(event))
        os.environ.update(
            {
                "GITHUB_REPOSITORY": repo,
                "GITHUB_EVENT_PATH": str(event_path),
                "GITHUB_EVENT_NAME": name,
                "GITHUB_REF": ref,
                "GITHUB_SHA": sha,
                "GITHUB_OUTPUT": str(output_path),
            }
        )
        sys.argv = ["site_policy.py", "policy"]
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            if variant == "push" and case == "missing":
                try:
                    m.main()
                    raise AssertionError("uncertain push authorized deployment")
                except ValueError:
                    pass
            else:
                m.main()
        outputs = dict(line.split("=", 1) for line in output_path.read_text().splitlines())
        assert outputs["run"] == str(int(expected))
        assert outputs["deploy"] == str(int(deploy))
        assert outputs["known"] == str(int(known))
        if variant == "push":
            assert "title=Site push range::before=" + before + " after=" + sha in stdout.getvalue()
        else:
            assert "::notice" not in stdout.getvalue()
