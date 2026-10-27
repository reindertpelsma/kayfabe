#!/usr/bin/env python3
"""BASE-only release eligibility. GitHub metadata, never a Worker response.
No build inputs or artifacts are accepted from the Vast test path.
"""
import importlib.util
import json, os, re, sys, urllib.request
from pathlib import Path

REPO = "reindertpelsma/kayfabe"


def verify(sha, status, get_run, *, get=None):
    try:
        if not re.fullmatch("[0-9a-f]{40}", sha) or status["sha"] != sha:
            return False
        latest = next(s for s in status["statuses"] if s["context"] == "gpu-vast")
        # A GPU result, not the path policy's NOT_NEEDED: the report job writes the state.
        if (
            latest["state"] != "success"
            or latest["creator"]["login"] != "github-actions[bot]"
            or latest["creator"]["id"] != 41898282
        ):
            return False
        if latest["description"] not in {"GPU broker GRACE", "GPU broker NONE"}:
            reused = re.fullmatch(
                r"reused: same tree verified on ([0-9a-f]{7})", latest["description"]
            )
            if not reused or get is None:
                return False
            # Isolated Python deliberately ignores sys.path. Load only the sibling module
            # from the same trusted master checkout, never from environment/package paths.
            spec = importlib.util.spec_from_file_location(
                "gate_reuse", Path(__file__).with_name("gate_reuse.py")
            )
            policy = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(policy)
            commit, tree = policy.commit_tree(sha, get)
            for candidate in policy.candidates(sha, commit, get):
                if candidate.startswith(reused[1]):
                    proof = policy.metadata(candidate, tree, get)
                    if proof and proof["url"] == latest["target_url"]:
                        return True
            return False
        match = re.fullmatch(
            "https://github.com/" + REPO + r"/actions/runs/([1-9][0-9]{0,19})", latest["target_url"]
        )
        if not match:
            return False
        run = get_run(int(match[1]))
        if (
            run["id"] != int(match[1])
            or run["path"] != ".github/workflows/gpu-vast.yml"
            or run["status"] != "completed"
            or run["conclusion"] != "success"
        ):
            return False
        if run["repository"]["full_name"] != REPO or run["head_repository"]["full_name"] != REPO:
            return False
        # Releases build master. gpu-vast.yml runs on push to master (review W4: it has no
        # dispatch trigger), so require that push run to have tested this exact master SHA,
        # rather than guessing PR/merge correspondence.
        return run["event"] == "push" and run["head_branch"] == "master" and run["head_sha"] == sha
    except (KeyError, IndexError, AttributeError, TypeError, ValueError, StopIteration, OSError):
        return False


def api(path):
    request = urllib.request.Request(
        "https://api.github.com/repos/" + REPO + path,
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
            raise ValueError()
        return json.loads(raw)


def main():
    try:
        sha = os.environ["GITHUB_SHA"]
        if (
            os.environ["GITHUB_EVENT_NAME"] != "workflow_dispatch"
            or os.environ["GITHUB_REF"] != "refs/heads/master"
            or os.environ["GITHUB_REPOSITORY"] != REPO
            or not re.fullmatch("[0-9a-f]{40}", sha)
        ):
            raise ValueError()
        if not verify(
            sha,
            {"sha": sha, "statuses": api("/commits/" + sha + "/statuses?per_page=100")},
            lambda i: api("/actions/runs/" + str(i)),
            get=api,
        ):
            raise ValueError()
        print("GitHub same-SHA gpu-vast gate accepted " + sha)
        return 0
    except Exception:
        print("GitHub same-SHA gpu-vast gate refused")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
