"""Restore the historical operator fixture closure and run every inherited suite.

Source/metadata copies only. This command never creates a lease or contacts a GPU host.
Historical authority fixtures are used by fake-provider tests, never as live authority.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

BW6 = "4e990f69d4c0491e90b1c6b4b73ffb2edb8df51a"
PROVIDER = "d53cfd9f44d7bdbc87887c2e19fb3431545d84b5"
TRACE = "traces/blackwell_ab_20261023/ROUND_B1r_bw6"
NATIVE = "traces/sweep_windows_20261021/lease_57064037/native"
NATIVE_XIDS = "traces/blackwell_resume_cont_20261022/lease_metadata/final_kernel_xids.json"
SUITES = (
    "test_saved_xid32_watch_loop",
    "test_phase_watch_loop",
    "test_windows_observation",
    "test_saved_xid32_observation",
    "test_saved_observation_hold",
    "test_phase_xid_guard",
    "test_precreate_watchdog",
    "test_fuzz_findings",
    "test_fuzz_fixture",
    "test_native_findings_wrapper",
    "test_media_wait",
    "test_console_pin",
    "test_console_pin_types",
    "test_round_budget",
    "test_init_observer",
    "test_readiness",
    "test_provider_retry",
    "test_box_fetch",
    "test_b1r_budget",
    "test_box_fetch_retry",
    "test_console_pin_settle",
    "test_b1r_pin",
    "test_b1r_ops",
    "test_die_pair_table",
    "test_b1r_kit",
    "test_b1r_snippets",
    "test_b1r_legacy_ops",
)


def assert_green(rows):
    if len(rows) != len(SUITES) or {r["suite"] for r in rows} != set(SUITES):
        return False
    return all(r["ok"] is True and type(r["ran"]) is int and r["ran"] > 0 for r in rows)


def complete_receipt(body):
    operator = body.get("operator", {})
    budget = operator.get("budget_watch", {})
    precreate = operator.get("precreate_watchdog", {})
    return (
        assert_green(body.get("suites", []))
        and body.get("static") == []
        and budget.get("absence_receipt") is True
        and budget.get("destroy_called") is True
        and precreate.get("state") == "ARMED"
        and precreate.get("result") == "no contract before deadline"
    )


def isolated_environment(env):
    return {key: value for key, value in env.items() if not key.startswith("GIT_")}


def git(repo, *args):
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], env=isolated_environment(os.environ)
    )


def fixture_git(repo, mirror):
    # Own Git directory: temporary repositories created by tests must remain independent.
    subprocess.run(
        ["git", "init", "--quiet", "--initial-branch=fixture", str(mirror)],
        env=isolated_environment(os.environ),
        check=True,
    )
    objects = Path(git(repo, "rev-parse", "--git-path", "objects").decode().strip())
    if not objects.is_absolute():
        objects = repo / objects
    alternate = mirror / ".git/objects/info/alternates"
    alternate.parent.mkdir(parents=True, exist_ok=True)
    alternate.write_text(str(objects.resolve()) + "\n")
    git(mirror, "update-ref", "refs/heads/fixture", BW6)


def restore(repo, revision, prefixes, output, records):
    listing = git(repo, "ls-tree", "-r", revision, *prefixes).decode().splitlines()
    suffixes = {".py", ".ps1", ".sh", ".json", ".flags"}
    for line in listing:
        info, name = line.split("\t", 1)
        mode, kind, digest = info.split()
        if kind != "blob":
            continue
        if not name.startswith(("scripts/bench/", NATIVE + "/")) and (
            Path(name).suffix not in suffixes
        ):
            continue
        data = git(repo, "cat-file", "blob", digest)
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        target.chmod(0o755 if mode == "100755" else 0o644)
        records.append(
            {
                "revision": revision,
                "path": name,
                "git_blob": digest,
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )


def build(repo, output):
    output.mkdir(exist_ok=False)
    records = []
    mirror = output / "repo"
    prefixes = ["scripts/bench", "traces/blackwell_ab_20261023", NATIVE, NATIVE_XIDS]
    restore(repo, BW6, prefixes, mirror, records)
    fixture_git(repo, mirror)
    provider = output / "provider"
    restore(repo, PROVIDER, ["scripts/bench"], provider, records)
    kit = mirror / TRACE / "attempt1/run_kit"
    workspace = output / "workspace"
    shutil.copytree(kit / "workspace_seed", workspace)
    # A fresh isolated rehearsal ledger, never the historical hardware ledger.
    ledger = kit.parent / "owned_instances.json"
    historical = ledger.read_bytes() if ledger.exists() else None
    if historical is not None:
        (output / "historical_ledger.json").write_bytes(historical)
    ledger.write_text("[]\n")
    (workspace / "grant_pin.json").write_text('{"machine": null, "variant": "default"}\n')
    (workspace / "stretch.json").write_text('{"attempts_used": 0, "provider_retries_used": 0}\n')
    (output / "source_lock.json").write_text(json.dumps(records, indent=2) + "\n")
    return mirror, provider, kit, workspace


def run(repo, output):
    mirror, provider = output / "repo", output / "provider"
    kit, workspace = mirror / TRACE / "attempt1/run_kit", output / "workspace"
    spec = importlib.util.spec_from_file_location("bw8_inherited_dry", kit / "operator_dry_loop.py")
    legacy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(legacy)
    mapping = {
        str(legacy.BW.parents[2]): mirror,
        str(legacy.PR.parents[2]): provider,
    }
    legacy.BW = mirror / "scripts/bench/blackwell"
    legacy.PR = provider / "scripts/bench/blackwell"
    legacy.PR_NATIVE = legacy.PR.parent
    env = dict(
        isolated_environment(os.environ),
        PYTHONDONTWRITEBYTECODE="1",
        KF_BW3_D=str(workspace),
        KF_BW3_K=str(kit),
        KF_BW4_BW=str(legacy.BW),
    )
    env["PYTHONPATH"] = os.pathsep.join(
        map(
            str,
            [
                legacy.BW,
                provider / "scripts/bench/blackwell",
                provider / "scripts/bench",
                kit,
                workspace,
            ],
        )
    )
    rows = []
    logs = output / "logs"
    logs.mkdir(exist_ok=True)
    for directory, suite in legacy.SUITES:
        for old, new in mapping.items():
            if str(directory).startswith(old + "/"):
                directory = new / str(directory).removeprefix(old + "/")
                break
        process = subprocess.run(
            [sys.executable, "-B", "-m", "unittest", suite],
            cwd=directory,
            env=env,
            capture_output=True,
            text=True,
            timeout=600,
        )
        (logs / (suite + ".txt")).write_text(process.stdout + process.stderr)
        count = re.search(r"^Ran (\d+) tests?", process.stderr, re.M)
        row = {
            "suite": suite,
            "ran": int(count[1]) if count else 0,
            "ok": process.returncode == 0 and bool(count),
            "rc": process.returncode,
        }
        rows.append(row)
        print(suite, row["ran"], "GREEN" if row["ok"] else "FAIL", flush=True)
    roots = [legacy.BW, legacy.PR, legacy.PR_NATIVE, kit, workspace]
    for name in ("ROUND_B1n_bw4", "ROUND_B1r_bw6"):
        roots.append(mirror / "traces/blackwell_ab_20261023" / name / "rehearsal")
    # The unchanged inherited timer subprocesses use a fake provider in their own PATH.
    sys.path.insert(0, str(workspace))
    body = {
        "scope": "historical replay with fake providers; not live authority",
        "suites": rows,
        "operator": legacy.dry_operator(workspace),
        "static": legacy.static_problems(roots),
        "tests": sum(r["ran"] for r in rows),
    }
    body["passed"] = complete_receipt(body)
    body["sources"] = legacy.sources(roots)
    body["utc"] = legacy.dt.datetime.now(legacy.dt.timezone.utc).isoformat()
    (output / "rehearsal.json").write_text(json.dumps(body, indent=2) + "\n")
    return body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["restore", "run"])
    parser.add_argument("repo", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.mode == "restore":
        build(args.repo, args.output)
    else:
        result = run(args.repo, args.output)
        return 0 if result["passed"] else 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
