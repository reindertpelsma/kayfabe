"""Conservative Site paths and API-only commit statuses; no source/artifact execution."""

import json
import os
import re
import sys
import urllib.request

REPO = "reindertpelsma/kayfabe"
SHA = r"[0-9a-f]{40}"
BOT = {"login": "github-actions[bot]", "id": 41898282}


def api(path, body=None):
    if not path.startswith("/") or ".." in path.replace("...", ""):
        raise ValueError("API path")
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


def paths_needed(files):
    seen, needed = set(), False
    for file in files:
        if file["status"] not in {
            "added",
            "removed",
            "modified",
            "renamed",
            "copied",
            "changed",
            "unchanged",
        }:
            raise ValueError("unknown file status")
        filename = file["filename"]
        if filename in seen:
            raise ValueError("duplicate file")
        seen.add(filename)
        paths = [filename]
        if file["status"] == "renamed":
            paths.append(file["previous_filename"])
        for path in paths:
            if (
                not isinstance(path, str)
                or not re.fullmatch(r"[A-Za-z0-9_./-]+", path)
                or path.startswith("/")
                or "//" in path
                or ".." in path.split("/")
            ):
                raise ValueError("uncertain path")
            if path.startswith(("site/", "docs/")) or path == ".github/workflows/site.yml":
                needed = True
    return needed


def pr_files(number, sha, get):
    if type(number) is not int or number < 1:
        raise ValueError("PR id")
    pr = get("/pulls/" + str(number))
    count = pr["changed_files"]
    if (
        pr["head"]["sha"] != sha
        or pr["base"]["repo"]["full_name"] != REPO
        or type(count) is not int
        or not 0 <= count <= 3000
    ):
        raise ValueError("PR binding/count")
    files = []
    for page in range(1, (count + 99) // 100 + 1):
        rows = get("/pulls/" + str(number) + "/files?per_page=100&page=" + str(page))
        if not isinstance(rows, list) or len(rows) != min(100, count - len(files)):
            raise ValueError("incomplete PR files")
        files.extend(rows)
    if len(files) != count:
        raise ValueError("PR file count")
    # The files endpoint follows a mutable PR. A synchronize or base update during
    # pagination must not authorize skipping the event's older immutable head.
    current = get("/pulls/" + str(number))
    if (
        current["head"]["sha"] != sha
        or current["base"].get("sha") != pr["base"].get("sha")
        or current["base"]["repo"]["full_name"] != REPO
        or current["changed_files"] != count
    ):
        raise ValueError("PR changed during pagination")
    return files


def decide(event, name, ref, sha, get=api):
    unknown = {"run": True, "deploy": False, "known": False}
    try:
        if not re.fullmatch(SHA, sha):
            return unknown
        if name in {"pull_request", "pull_request_target"}:
            if event["pull_request"]["head"]["sha"] != sha:
                return unknown
            needed = paths_needed(pr_files(event["pull_request"]["number"], sha, get))
            return {"run": needed, "deploy": False, "known": True}
        if name in {"schedule", "release", "workflow_dispatch"} or (
            name == "push" and ref.startswith("refs/tags/v")
        ):
            return {"run": True, "deploy": True, "known": True}
        if name == "push" and ref == "refs/heads/master":
            before = event["before"]
            if not re.fullmatch(SHA, before) or before == "0" * 40 or event["after"] != sha:
                return unknown
            diff = get("/compare/" + before + "..." + sha + "?per_page=100")
            if diff["base_commit"]["sha"] != before or diff["status"] not in {"ahead", "identical"}:
                return unknown
            files = diff["files"]
            if not isinstance(files, list) or len(files) >= 300:
                return unknown
            needed = paths_needed(files)
            return {"run": needed, "deploy": needed, "known": True}
    except Exception:
        pass
    return unknown


def find_reuse(sha, name, ref, get=api):
    if name != "push" or ref != "refs/heads/master" or not re.fullmatch(SHA, sha):
        return None
    try:
        commit = get("/commits/" + sha)
        tree = commit["commit"]["tree"]["sha"]
        if commit["sha"] != sha or not re.fullmatch(SHA, tree) or len(commit["parents"]) > 8:
            return None
        prs = get("/commits/" + sha + "/pulls?per_page=100")
        if len(prs) >= 100:
            return None
        parents = [p["sha"] for p in commit["parents"]] + [p["head"]["sha"] for p in prs]
        for parent in dict.fromkeys(parents):
            try:
                if not re.fullmatch(SHA, parent):
                    continue
                candidate = get("/commits/" + parent)
                if candidate["sha"] != parent or candidate["commit"]["tree"]["sha"] != tree:
                    continue
                rows = get("/commits/" + parent + "/statuses?per_page=100")
                if len(rows) >= 100:
                    continue
                status = next(s for s in rows if s["context"] == "site")
                if (
                    status["state"] != "success"
                    or any(status["creator"].get(k) != v for k, v in BOT.items())
                    or status["description"]
                    not in {"site build passed", "not needed: site and docs untouched"}
                ):
                    continue
                match = re.fullmatch(
                    "https://github.com/" + REPO + r"/actions/runs/([1-9][0-9]{0,19})",
                    status["target_url"],
                )
                if not match:
                    continue
                run = get("/actions/runs/" + match[1])
                if (
                    run["id"] != int(match[1])
                    or run["head_sha"] != parent
                    or run["repository"]["full_name"] != REPO
                    or run["status"] != "completed"
                    or run["conclusion"] != "success"
                    or run["path"]
                    not in {".github/workflows/site.yml", ".github/workflows/site-status.yml"}
                    or not (
                        run["event"] in {"pull_request", "pull_request_target"}
                        or (run["event"] == "push" and run["head_branch"] == "master")
                    )
                ):
                    continue
                checks = get("/commits/" + parent + "/check-runs?per_page=100")
                if (
                    checks["total_count"] != len(checks["check_runs"])
                    or checks["total_count"] > 100
                ):
                    continue
                ci = next(c for c in checks["check_runs"] if c["name"] == "CI")
                if (
                    ci["head_sha"] == parent
                    and ci["status"] == "completed"
                    and ci["conclusion"] == "success"
                    and ci["app"]["slug"] == "github-actions"
                    and ci["app"]["id"] == 15368
                ):
                    return {"parent": parent, "tree": tree, "url": status["target_url"]}
            except Exception:
                continue
    except Exception:
        pass
    return None


def report_status(event, name, own_id, get=api, post=api):
    trusted = None

    def write(sha, state, description, url):
        post(
            "/statuses/" + sha,
            {"context": "site", "state": state, "description": description, "target_url": url},
        )
        print("site " + state + ": " + description + " on " + sha + " (" + url + ")")

    try:
        if not re.fullmatch(r"[1-9][0-9]{0,19}", own_id):
            return
        if name == "pull_request_target":
            sha = event["pull_request"]["head"]["sha"]
            if not re.fullmatch(SHA, sha):
                return
            url = "https://github.com/" + REPO + "/actions/runs/" + own_id
            # A delayed PR-start callback must not overwrite an already-final same-SHA result.
            try:
                existing = next(
                    row
                    for row in get("/commits/" + sha + "/statuses?per_page=100")
                    if row["context"] == "site"
                )
                if existing["state"] in {"success", "error", "failure"} and all(
                    existing["creator"].get(k) == v for k, v in BOT.items()
                ):
                    print("Site already has a final same-SHA status; preserving it.")
                    return
            except (KeyError, TypeError, AttributeError, StopIteration, OSError):
                pass
            write(sha, "pending", "site build pending", url)
            policy = decide(event, name, "", sha, get)
            if policy["known"] and not policy["run"]:
                write(sha, "success", "not needed: site and docs untouched", url)
            return
        if name != "workflow_run" or type(event["workflow_run"]["id"]) is not int:
            return
        run = get("/actions/runs/" + str(event["workflow_run"]["id"]))
        sha = run["head_sha"]
        if (
            run["id"] != event["workflow_run"]["id"]
            or not re.fullmatch(SHA, sha)
            or run["path"] != ".github/workflows/site.yml"
            or run["repository"]["full_name"] != REPO
            or not (
                run["event"] == "pull_request"
                or (run["event"] == "push" and run["head_branch"] == "master")
            )
        ):
            return
        trusted = (sha, "https://github.com/" + REPO + "/actions/runs/" + str(run["id"]))
        policy = None
        if run["event"] == "pull_request":
            prs = get("/commits/" + sha + "/pulls?per_page=100")
            if len(prs) >= 100:
                return
            pr = next(
                p
                for p in prs
                if p["head"]["sha"] == sha
                and p["base"]["repo"]["full_name"] == REPO
                and p["head"]["repo"]["full_name"] == run["head_repository"]["full_name"]
            )
            policy = decide({"pull_request": pr}, "pull_request", "", sha, get)
        url = "https://github.com/" + REPO + "/actions/runs/" + str(run["id"])
        if event["action"] != "completed" and run["status"] != "completed":
            proof = find_reuse(sha, run["event"], "refs/heads/master", get)
            if proof:
                write(
                    sha,
                    "success",
                    "reused: same tree verified on " + proof["parent"][:7],
                    proof["url"],
                )
                print("Site reused tree " + proof["tree"] + " from " + proof["parent"])
            elif policy and policy["known"] and not policy["run"]:
                write(sha, "success", "not needed: site and docs untouched", url)
            else:
                write(sha, "pending", "site build pending", url)
            return
        if run["status"] != "completed" or run["conclusion"] != "success":
            write(sha, "error", "site build did not pass", url)
            return
        jobs = get("/actions/runs/" + str(run["id"]) + "/jobs?per_page=100")
        if jobs["total_count"] != len(jobs["jobs"]) or jobs["total_count"] > 100:
            raise ValueError("job list incomplete")
        build = next(j for j in jobs["jobs"] if j["name"] == "build")
        if run["event"] == "push" and build["conclusion"] == "skipped":
            # The completed event has no push.before. Read the original range from the
            # trusted policy job's typed GitHub annotation, never an artifact or PR output.
            policy_job = next(j for j in jobs["jobs"] if j["name"] == "policy")
            check_url = re.fullmatch(
                "https://api.github.com/repos/" + REPO + r"/check-runs/([1-9][0-9]*)",
                policy_job["check_run_url"],
            )
            if not check_url:
                raise ValueError("policy check URL")
            check = get("/check-runs/" + check_url[1])
            if (
                check["head_sha"] != sha
                or check["conclusion"] != "success"
                or check["app"]["id"] != 15368
                or check["app"]["slug"] != "github-actions"
            ):
                raise ValueError("policy check binding")
            rows = get("/check-runs/" + check_url[1] + "/annotations?per_page=100")
            if len(rows) >= 100:
                raise ValueError("policy annotation count")
            messages = [row["message"] for row in rows if row["title"] == "Site push range"]
            if len(messages) != 1:
                raise ValueError("policy annotation missing")
            binding = re.fullmatch("before=(" + SHA + ") after=" + sha, messages[0])
            if not binding:
                raise ValueError("push range binding")
            policy = decide(
                {"before": binding[1], "after": sha}, "push", "refs/heads/master", sha, get
            )
        if build["conclusion"] == "success":
            write(sha, "success", "site build passed", url)
        elif build["conclusion"] == "skipped" and policy and policy["known"] and not policy["run"]:
            write(sha, "success", "not needed: site and docs untouched", url)
        else:
            write(sha, "error", "site build evidence missing", url)
    except (KeyError, TypeError, ValueError, StopIteration, AttributeError, OSError):
        if trusted:
            write(trusted[0], "error", "site build evidence unavailable", trusted[1])
        else:
            print("Site status metadata unavailable; no success inferred.")


def main():
    if os.environ["GITHUB_REPOSITORY"] != REPO:
        raise ValueError("repository")
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf8") as stream:
        event = json.load(stream)
    name, ref = os.environ["GITHUB_EVENT_NAME"], os.environ["GITHUB_REF"]
    if sys.argv[1] == "policy":
        sha = (
            event["pull_request"]["head"]["sha"]
            if name == "pull_request"
            else os.environ["GITHUB_SHA"]
        )
        result = decide(event, name, ref, sha)
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf8") as output:
            for key, value in result.items():
                output.write(key + "=" + str(int(value)) + "\n")
        print(
            "Site paths: build="
            + str(int(result["run"]))
            + " deploy="
            + str(int(result["deploy"]))
            + " complete="
            + str(int(result["known"]))
        )
        if (
            name == "push"
            and ref == "refs/heads/master"
            and re.fullmatch(SHA, event.get("before", ""))
        ):
            print(
                "::notice file=.github/workflows/site.yml,line=1,title=Site push range::"
                "before=" + event["before"] + " after=" + sha
            )
        if name == "push" and ref == "refs/heads/master" and not result["known"]:
            raise ValueError("cannot authorize deployment from uncertain push paths")
    elif sys.argv[1] == "status":
        report_status(event, name, os.environ["GITHUB_RUN_ID"])
    else:
        raise ValueError("mode")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("Site policy/status operation failed; no credentials or raw API data logged.")
        raise SystemExit(1)
