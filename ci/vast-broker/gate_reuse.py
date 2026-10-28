"""Same-tree GPU evidence; failures select the existing gate, never a fabricated PASS."""

import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.request

REPO = "reindertpelsma/kayfabe"
SHA = r"[0-9a-f]{40}"
BOT = {"login": "github-actions[bot]", "id": 41898282}


def api(path, body=None):
    if not path.startswith("/") or ".." in path:
        raise ValueError("invalid API path")
    request = urllib.request.Request(
        "https://api.github.com/repos/" + REPO + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={
            "Authorization": "Bearer " + os.environ["GH_TOKEN"],
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *_):
            return None

    with urllib.request.build_opener(NoRedirect).open(request, timeout=20) as response:
        raw = response.read(1048577)
        if len(raw) > 1048576:
            raise ValueError("API response too large")
        return json.loads(raw)


def original_logs(run_id):
    # gh follows GitHub's authenticated log-download flow; credentials/redirect URLs never print.
    process = subprocess.run(
        ["gh", "run", "view", str(run_id), "--repo", REPO, "--log"],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if process.returncode or len(process.stdout) > 8 * 1024 * 1024:
        raise ValueError("original log unavailable")
    return process.stdout


def commit_tree(sha, get):
    if not re.fullmatch(SHA, sha):
        raise ValueError("invalid SHA")
    commit = get("/commits/" + sha)
    tree = commit["commit"]["tree"]["sha"]
    if commit["sha"] != sha or not re.fullmatch(SHA, tree):
        raise ValueError("commit binding")
    return commit, tree


def candidates(sha, commit, get):
    parents = commit["parents"]
    if not isinstance(parents, list) or len(parents) > 8:
        raise ValueError("parent list")
    found = [p["sha"] for p in parents]
    # Squashes/fast-forwards may not retain the tested PR head as a direct parent.
    prs = get("/commits/" + sha + "/pulls?per_page=100")
    if not isinstance(prs, list) or len(prs) >= 100:
        raise ValueError("incomplete PR list")
    found += [p["head"]["sha"] for p in prs if p["head"]["repo"]["full_name"] == REPO]
    return list(dict.fromkeys(found))


def ci_passed(sha, get):
    checks = get("/commits/" + sha + "/check-runs?per_page=100")
    if checks["total_count"] != len(checks["check_runs"]) or checks["total_count"] > 100:
        return False
    matches = [check for check in checks["check_runs"] if check["name"] == "CI"]
    if not matches:
        return False
    # Push and PR CI may both exist. Every qualifying record must be complete,
    # successful, and bound through its job/check suite to the intended workflow.
    for check in matches:
        if (
            check["head_sha"] != sha
            or check["status"] != "completed"
            or check["conclusion"] != "success"
            or check["app"]["slug"] != "github-actions"
            or check["app"]["id"] != 15368
        ):
            return False
        match = re.fullmatch(
            "https://github.com/" + REPO + r"/actions/runs/([1-9][0-9]*)/job/([1-9][0-9]*)",
            check["details_url"],
        )
        if not match or check["id"] != int(match[2]):
            return False
        run = get("/actions/runs/" + match[1])
        if (
            run["id"] != int(match[1])
            or run["head_sha"] != sha
            or run["path"] != ".github/workflows/ci.yml"
            or run["repository"]["full_name"] != REPO
            or run["status"] != "completed"
            or run["conclusion"] != "success"
            or run["check_suite_id"] != check["check_suite"]["id"]
        ):
            return False
    return True


def metadata(parent, tree, get):
    _, candidate_tree = commit_tree(parent, get)
    if candidate_tree != tree:
        return None
    rows = get("/commits/" + parent + "/statuses?per_page=100")
    if not isinstance(rows, list) or len(rows) >= 100:
        return None
    gpu_rows = [row for row in rows if row["context"] == "gpu-vast"]
    status = gpu_rows.pop(0)
    # A fast-forward can reuse the same SHA. Preserve its immediately preceding original
    # evidence through our own copied statuses; never look past a pending/failed/foreign row.
    depth = 0
    while status["description"] == "reused: same tree verified on " + parent[:7]:
        if (
            depth >= 8
            or status["state"] != "success"
            or any(status["creator"].get(key) != value for key, value in BOT.items())
        ):
            return None
        previous = gpu_rows.pop(0)
        if previous["target_url"] != status["target_url"]:
            return None
        status = previous
        depth += 1
    if status["state"] != "success":
        return None
    if any(status["creator"].get(key) != value for key, value in BOT.items()):
        return None
    if status["description"] not in {"GPU broker GRACE", "GPU broker NONE"}:
        return None
    match = re.fullmatch(
        "https://github.com/" + REPO + r"/actions/runs/([1-9][0-9]{0,19})",
        status["target_url"],
    )
    if not match:
        return None
    run = get("/actions/runs/" + match[1])
    if (
        run["id"] != int(match[1])
        or run["head_sha"] != parent
        or run["path"] != ".github/workflows/gpu-vast.yml"
        or run["status"] != "completed"
        or run["conclusion"] != "success"
        or run["repository"]["full_name"] != REPO
        or run["head_repository"]["full_name"] != REPO
        or not (
            run["event"] == "pull_request_target"
            or (run["event"] == "push" and run["head_branch"] == "master")
        )
    ):
        return None
    if not ci_passed(parent, get):
        return None
    return {
        "parent": parent,
        "tree": tree,
        "url": status["target_url"],
        "description": status["description"],
        "run_id": int(match[1]),
    }


def summary_lines(text, parent):
    # Copy bounded, literal public result lines. Never replay arbitrary workflow commands,
    # capabilities, URLs, source snippets, control characters or provider-supplied Markdown.
    patterns = [
        r"RENTAL machine=[0-9]+ host=[0-9]+ instance=[0-9]+ gpu=[A-Za-z0-9_]+ price=[0-9.]+",
        r"\[untrusted\] \[Worker\] selection profile=check die=[A-Z0-9_]+ "
        r"reason=[A-Z_]+ ignored=[0-9]+",
        r"\[untrusted\] \[box\] CUDA_LADDER_STARTED=[0-9T:+-]+ mode=(host|guest) "
        r"reps=[0-9]+ rungs=[a-z0-9,]+ rev=" + parent[:8],
        r"\[untrusted\] \[box\] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4",
        r"\[untrusted\] \[box\] stage name=(provision|cuda_host|merge_bar|cuda_guest) "
        r"rc=[0-9]+ duration_ms=[0-9]+ result=PASS",
    ]
    groups = [[] for _ in patterns]
    final_blocks = 0
    for raw in text.splitlines():
        line = raw.split("Z ", 1)[-1]
        if line.startswith("[untrusted] [box] KF_FINAL_SUMMARY_BEGIN "):
            final_blocks += 1
        for index, pattern in enumerate(patterns):
            if re.fullmatch(pattern, line) and line not in groups[index]:
                groups[index].append(line)
    expected = ["provision", "cuda_host", "merge_bar", "cuda_guest"]
    stages = [line.split("stage name=", 1)[1].split(" ", 1)[0] for line in groups[-1]]
    modes = [line.split(" mode=", 1)[1].split(" ", 1)[0] for line in groups[2]]
    if any(not group for group in groups) or sorted(stages) != sorted(expected):
        raise ValueError("missing original result lines")
    if sorted(modes) != ["guest", "host"] or final_blocks != 1:
        raise ValueError("incomplete or conflicting original verdict")
    if any(len(groups[index]) != 1 for index in [0, 1, 3]):
        raise ValueError("ambiguous original rental or verdict")
    return [line for group in groups for line in group]


def find_reuse(sha, event, ref, get=api, logs=original_logs):
    if event != "push" or ref != "refs/heads/master":
        return None
    try:
        commit, tree = commit_tree(sha, get)
        for parent in candidates(sha, commit, get):
            try:
                proof = metadata(parent, tree, get)
                if proof:
                    proof["lines"] = summary_lines(logs(proof["run_id"]), parent)
                    return proof
            except (Exception,):
                continue
    except Exception:
        pass
    return None


def render(proof):
    # Recheck the output schema before it reaches either log or summary channels.
    if (
        not re.fullmatch(SHA, proof["parent"])
        or not re.fullmatch(SHA, proof["tree"])
        or proof["url"] != "https://github.com/" + REPO + "/actions/runs/" + str(proof["run_id"])
        or proof["description"] not in {"GPU broker GRACE", "GPU broker NONE"}
    ):
        raise ValueError("invalid replay proof")
    lines = summary_lines("\n".join(proof["lines"]), proof["parent"])
    return (
        "\n".join(
            [
                "Reused same tree verified on " + proof["parent"],
                "Tree SHA: " + proof["tree"],
                "Verified run: " + proof["url"],
                "Original status: " + proof["description"],
                "Checks: tree equality; creator github-actions[bot]/41898282; "
                "CI completed success; "
                "trusted GPU workflow completed success; original verdict PASS.",
                "Original run result lines (untrusted provider evidence remains untrusted):",
                *lines,
            ]
        )
        + "\n"
    )


def main():
    if os.environ.get("GITHUB_REPOSITORY") != REPO:
        raise ValueError("repository")
    mode = sys.argv[1]
    if mode == "prepare":
        sha = os.environ["TARGET_SHA"]
        if not re.fullmatch(SHA, sha):
            raise ValueError("target")
        run_url = "https://github.com/" + REPO + "/actions/runs/" + os.environ["GITHUB_RUN_ID"]
        proof = find_reuse(sha, os.environ["GITHUB_EVENT_NAME"], os.environ["GITHUB_REF"])
        if proof:
            print(render(proof), end="")
            api(
                "/statuses/" + sha,
                {
                    "state": "success",
                    "context": "gpu-vast",
                    "description": "reused: same tree verified on " + proof["parent"][:7],
                    "target_url": proof["url"],
                },
            )
            # A separate context preserves the parent's original evidence while linking forward.
            api(
                "/statuses/" + proof["parent"],
                {
                    "state": "success",
                    "context": "gpu-vast-reused-by/" + sha[:7],
                    "description": "same tree reused by " + sha[:7],
                    "target_url": run_url,
                },
            )
        else:
            print("No complete reusable evidence; running the existing gate policy.")
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf8") as output:
            output.write("reused=" + ("true" if proof else "false") + "\n")
            output.write("proof=" + json.dumps(proof, separators=(",", ":")) + "\n")
    elif mode == "replay":
        proof = json.loads(os.environ["REUSE_PROOF"])
        text = render(proof)
        print(text, end="")
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf8") as summary:
            summary.write("<pre>" + html.escape(text) + "</pre>\n")
    else:
        raise ValueError("mode")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("GPU evidence operation failed (no credentials or raw API reply logged).")
        raise SystemExit(1)
