import importlib.util
import json
import contextlib
import html
import io
import os
import tempfile
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


case, variant = sys.argv[1:]
sha, parent, old, tree = (c * 40 for c in "abcd")
if variant == "fast-forward":
    parent = sha
repo = "reindertpelsma/kayfabe"
url = "https://github.com/" + repo + "/actions/runs/123"
status = {
    "context": "gpu-vast",
    "state": "success",
    "description": "GPU broker GRACE",
    "creator": {"login": "github-actions[bot]", "id": 41898282},
    "target_url": url,
}
run = {
    "id": 123,
    "path": ".github/workflows/gpu-vast.yml",
    "head_sha": parent,
    "head_branch": "next",
    "event": "pull_request_target",
    "status": "completed",
    "conclusion": "success",
    "repository": {"full_name": repo},
    "head_repository": {"full_name": repo},
}
commit = lambda value, t, parents: {
    "sha": value,
    "commit": {"tree": {"sha": t}},
    "parents": [{"sha": p} for p in parents],
}
checks = {
    "total_count": 1,
    "check_runs": [
        {
            "name": "CI",
            "head_sha": parent,
            "status": "completed",
            "conclusion": "success",
            "app": {"slug": "github-actions", "id": 15368},
        }
    ],
}
data = {
    "/commits/" + sha: commit(sha, tree, [old, parent] if variant == "merge" else [old]),
    "/commits/" + parent: commit(parent, tree, []),
    "/commits/" + old: commit(old, "e" * 40, []),
    "/commits/"
    + sha
    + "/pulls?per_page=100": (
        [] if variant == "merge" else [{"head": {"sha": parent, "repo": {"full_name": repo}}}]
    ),
    "/commits/" + parent + "/statuses?per_page=100": [status],
    "/commits/" + parent + "/check-runs?per_page=100": checks,
    "/actions/runs/123": run,
}
lines = [
    "RENTAL machine=42 host=43 instance=44 gpu=RTX_3060 price=0.200000",
    "[untrusted] [Worker] selection profile=check die=GA106 reason=DEFAULT_ORDER ignored=0",
    "[untrusted] [box] CUDA_LADDER_STARTED=2026-10-27T21:59:04+00:00 mode=host reps=1 "
    "rungs=cup2,cup3,cup8,cup8bench rev=" + parent[:8],
    "[untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4",
] + [
    "[untrusted] [box] stage name=" + name + " rc=0 duration_ms=1 result=PASS"
    for name in ["provision", "cuda_host", "merge_bar", "cuda_guest"]
]
event, ref = "push", "refs/heads/master"
if case == "tree":
    data["/commits/" + parent]["commit"]["tree"]["sha"] = "f" * 40
if case in ["status-failure", "status-pending"]:
    status["state"] = case.split("-")[1]
if case == "creator-id":
    status["creator"]["id"] = 1
if case == "creator-login":
    status["creator"]["login"] = "someone"
if case == "ci-pending":
    checks["check_runs"][0]["status"] = "in_progress"
if case == "ci-failed":
    checks["check_runs"][0]["conclusion"] = "failure"
if case == "ci-forged":
    checks["check_runs"][0]["app"]["id"] = 1
if case == "ci-missing":
    checks["check_runs"] = []
    checks["total_count"] = 0
if case == "ci-truncated":
    checks["total_count"] = 101
if case == "run-failed":
    run["conclusion"] = "failure"
if case == "run-sha":
    run["head_sha"] = old
if case == "run-path":
    run["path"] = ".github/workflows/evil.yml"
if case == "fork":
    run["head_repository"]["full_name"] = "elsewhere/repo"
if case == "tag":
    ref = "refs/tags/v1"
if case == "pr":
    event = "pull_request_target"
if case == "logs-missing":
    lines = []
if case == "logs-fail":
    lines[3] = lines[3].replace("PASS", "FAIL")
if case == "logs-revision":
    lines[2] = lines[2].replace(parent[:8], old[:8])
if case == "logs-duplicate-stage":
    lines[4:] = [
        "[untrusted] [box] stage name=provision rc=0 duration_ms=" + str(i) + " result=PASS"
        for i in range(1, 5)
    ]
if case == "logs-missing-guest-stage":
    lines[-1] = (
        lines[-1].replace("cuda_guest", "provision").replace("duration_ms=1", "duration_ms=2")
    )
# Valid evidence includes both CUDA modes for this exact original revision.
lines.insert(3, lines[2].replace("mode=host", "mode=guest"))
if case == "logs-no-guest-start":
    lines = [line for line in lines if "mode=guest" not in line]
if case == "logs-no-host-start":
    lines = [line for line in lines if "mode=host" not in line]
if case == "logs-conflicting-final":
    lines.append("[untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=FAIL stages=4")
if case == "logs-duplicate-final":
    lines.append("[untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4")
if case == "logs-schema-overflow":
    lines += [lines[0].replace("machine=42", "machine=" + str(i)) for i in range(100, 125)]
if case == "not-needed":
    status["description"] = "GPU broker NOT_NEEDED"
if case == "api-error":
    data.clear()


commit_reads = 0


def get(path):
    global commit_reads
    value = json.loads(json.dumps(data[path]))
    if path == "/commits/" + sha:
        commit_reads += 1
        # A SHA cannot have two trees; inject inconsistent API responses for FF negatives.
        if variant == "fast-forward" and case in {"tree", "release-tree"} and commit_reads == 1:
            value["commit"]["tree"]["sha"] = tree
    return value


if case.startswith("release"):
    module = load("release_gate")
    child = {
        "sha": sha,
        "statuses": [{**status, "description": "reused: same tree verified on " + parent[:7]}],
    }
    if case == "release-forged":
        child["statuses"][0]["creator"] = {"id": 1, "login": "other"}
    if case == "release-forged-id":
        child["statuses"][0]["creator"] = {"id": 1, "login": "github-actions[bot]"}
    if case == "release-forged-login":
        child["statuses"][0]["creator"] = {"id": 41898282, "login": "other"}
    if case == "release-lost-parent":
        status["state"] = "failure"
    if case == "release-tree":
        data["/commits/" + parent]["commit"]["tree"]["sha"] = old
    if variant == "fast-forward":
        data["/commits/" + parent + "/statuses?per_page=100"].insert(0, child["statuses"][0])
    expected = case == "release-valid"
    assert module.verify(sha, child, lambda _: run, get=get) == expected
else:
    module = load("gate_reuse")
    proof = module.find_reuse(sha, event, ref, get, lambda _: "\n".join(lines))
    assert bool(proof) == (case == "valid"), (case, proof)
    if proof:
        assert proof["parent"] == parent and proof["tree"] == tree
        assert proof["url"] == url and proof["description"] == status["description"]
        rendered = module.render(proof)
        for line in lines:
            assert line in rendered, line
        for value in [parent, tree, url, "creator", "CI", "same tree"]:
            assert value in rendered

if case == "valid":
    module.api = lambda *_: (_ for _ in ()).throw(AssertionError("replay contacted API"))
    module.original_logs = module.api
    with tempfile.NamedTemporaryFile(mode="w+") as summary:
        os.environ.update(
            {
                "GITHUB_REPOSITORY": repo,
                "REUSE_PROOF": json.dumps(proof),
                "GITHUB_STEP_SUMMARY": summary.name,
            }
        )
        sys.argv = ["gate_reuse.py", "replay"]
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            module.main()
        summary.seek(0)
        assert module.render(proof) in output.getvalue()
        assert html.escape(module.render(proof)) in summary.read()
