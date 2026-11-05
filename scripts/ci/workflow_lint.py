#!/usr/bin/env python3
"""Supply-chain lint for .github/workflows and .github/actions (stdlib only, read-only).

Rules (each finding is printed as `file:line: rule: message`; exit 1 if there is any):

  pin         every `uses:` of a non-local action is a full 40-hex commit SHA
              (`docker://` images need `@sha256:<64 hex>`). A tag or branch moves; a SHA does not.
  permissions every workflow has a top-level `permissions:` key (and not `write-all`), so the
              token is least-privilege whatever the repository default is.
  allowlist   the jobs of gpu-vast.yml and gpu-vast-sweep.yml are exactly the ones in ALLOWLIST,
              each with exactly the permissions listed there. The Worker-facing `gpu` job may hold
              `id-token: write` and nothing else; a later edit cannot widen any of them silently.
  secrets     a job that touches the Worker secrets (`VAST_BROKER_*`, `secrets: inherit`, or the
              `gpu-vast` environment) has no `uses:` step at all: no checkout, cache, artifact or
              third-party action runs in the same job as the Worker credentials.

The parser is deliberately small: it understands the indentation shape these files use (top-level
keys at column 0, job names one level in, `permissions:` as a block or an inline `{}`/`{a: b}`),
skips `run: |` block scalars when looking for `uses:`, and FAILS on a `permissions:` shape it cannot
read rather than guessing. Run: `python3 scripts/ci/workflow_lint.py [--root DIR]`.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# file name -> job -> exact permissions. Read from the files as they are on master; narrowing is a
# one-line change here, widening has to be argued in review. `pending` and `report` write commit
# statuses on the PR head SHA; only `gpu` talks to the Worker.
ALLOWLIST: dict[str, dict[str, dict[str, str]]] = {
    "gpu-vast.yml": {
        "pending": {
            "statuses": "write",
            "contents": "read",
            "pull-requests": "read",
            "actions": "read",
            "checks": "read",
        },
        "policy": {"contents": "read", "pull-requests": "read"},
        "policy-check": {"contents": "read", "pull-requests": "read"},
        "gpu": {"id-token": "write"},
        "report": {"statuses": "write"},
    },
    "gpu-vast-sweep.yml": {"gpu": {"id-token": "write"}},
}

SHA = re.compile(r"[0-9a-f]{40}")
ACTION = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_./-]+)?@[0-9a-f]{40}")
DOCKER = re.compile(r"docker://\S+@sha256:[0-9a-f]{64}")
USES = re.compile(r"^(\s*)(?:-\s+)?uses:\s*(.*?)\s*$")
BLOCK_SCALAR = re.compile(r"^(\s*)(?:-\s+)?[\w.-]+:\s*[|>][-+0-9]*\s*$")
KEY = re.compile(r"^(\s*)([\w-]+):\s*(.*?)\s*$")


@dataclass
class Job:
    name: str
    start: int  # 1-based line of the `name:` header
    lines: list[tuple[int, str]] = field(default_factory=list)  # (lineno, text)


@dataclass
class Workflow:
    path: Path
    top_keys: dict[str, str] = field(default_factory=dict)  # key -> inline value
    header: list[tuple[int, str]] = field(default_factory=list)  # lines outside `jobs:`
    jobs: list[Job] = field(default_factory=list)


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def is_noise(line: str) -> bool:
    s = line.strip()
    return not s or s.startswith("#")


def strip_comment(value: str) -> str:
    # YAML: a comment starts at ` #`. Quoted values here never contain one.
    return re.sub(r"\s+#.*$", "", value).strip()


def parse(path: Path, text: str) -> Workflow:
    wf = Workflow(path)
    lines = text.splitlines()
    in_jobs = False
    job: Job | None = None
    job_indent: int | None = None
    for n, line in enumerate(lines, 1):
        if not in_jobs:
            wf.header.append((n, line))
            m = re.match(r"^([\w-]+):\s*(.*?)\s*$", line)
            if m and not line.startswith(" "):
                wf.top_keys[m.group(1)] = strip_comment(m.group(2))
                if m.group(1) == "jobs":
                    in_jobs = True
            continue
        if is_noise(line):
            if job is not None:
                job.lines.append((n, line))
            continue
        if not line.startswith(" "):  # a new top-level key after `jobs:`
            in_jobs = False
            job = None
            wf.header.append((n, line))
            m = re.match(r"^([\w-]+):\s*(.*?)\s*$", line)
            if m:
                wf.top_keys[m.group(1)] = strip_comment(m.group(2))
            continue
        if job_indent is None:
            job_indent = indent_of(line)
        m = re.match(r"^" + " " * job_indent + r"([\w-]+):\s*(#.*)?$", line)
        if indent_of(line) == job_indent and m:
            job = Job(m.group(1), n)
            wf.jobs.append(job)
            continue
        if job is not None:
            job.lines.append((n, line))
    return wf


def uses_in(lines: list[tuple[int, str]]) -> list[tuple[int, str]]:
    """(lineno, value) for each `uses:` outside run-style block scalars and comments."""
    found = []
    skip_until: int | None = None  # leave the block when indent <= this
    for n, line in lines:
        if skip_until is not None:
            if is_noise(line) or indent_of(line) > skip_until:
                continue
            skip_until = None
        if is_noise(line):
            continue
        b = BLOCK_SCALAR.match(line)
        if b:
            skip_until = len(b.group(1))
            continue
        m = USES.match(line)
        if m:
            found.append((n, strip_comment(m.group(2)).strip("'\"")))
    return found


def read_permissions(job: Job) -> tuple[dict[str, str] | None, str | None, int]:
    """(permissions, error, lineno). permissions is None when the job has no key."""
    base = None
    for i, (n, line) in enumerate(job.lines):
        if is_noise(line):
            continue
        if base is None:
            base = indent_of(line)
        if indent_of(line) != base:
            continue
        m = KEY.match(line)
        if not (m and m.group(2) == "permissions"):
            continue
        value = strip_comment(m.group(3))
        if value == "{}":
            return {}, None, n
        if value in ("read-all", "write-all"):
            return {"*": value}, None, n
        if value.startswith("{") and value.endswith("}"):
            out = {}
            for part in value[1:-1].split(","):
                k, sep, v = part.partition(":")
                if not sep:
                    return None, f"cannot read inline permissions {value!r}", n
                out[k.strip()] = v.strip()
            return out, None, n
        if value:
            return None, f"cannot read permissions value {value!r}", n
        out = {}
        for n2, line2 in job.lines[i + 1 :]:
            if is_noise(line2):
                continue
            if indent_of(line2) <= base:
                break
            m2 = KEY.match(line2)
            if not m2 or not strip_comment(m2.group(3)):
                return None, f"cannot read permissions entry {line2.strip()!r}", n2
            out[m2.group(2)] = strip_comment(m2.group(3))
        return out, None, n
    return None, None, job.start


def check_uses(path: Path, lines: list[tuple[int, str]], rel: str) -> list[str]:
    out = []
    for n, value in uses_in(lines):
        if value.startswith("./"):
            continue
        if value.startswith("docker://"):
            ok = bool(DOCKER.fullmatch(value))
        else:
            ok = bool(ACTION.fullmatch(value))
        if not ok:
            out.append(f"{rel}:{n}: pin: `uses: {value}` is not a full 40-hex commit SHA")
    return out


def touches_secrets(text_lines: list[tuple[int, str]]) -> bool:
    for _, line in text_lines:
        if is_noise(line):
            continue
        code = strip_comment(line)
        if "VAST_BROKER_" in code or re.search(r"secrets:\s*inherit", code):
            return True
        if re.match(r"^\s*environment:\s*gpu-vast\s*$", code):
            return True
    return False


def lint_workflow(path: Path, rel: str) -> list[str]:
    wf = parse(path, path.read_text())
    out = []
    # pin
    all_lines = list(wf.header) + [x for j in wf.jobs for x in j.lines]
    out += check_uses(path, all_lines, rel)
    # permissions (top level)
    if "permissions" not in wf.top_keys:
        out.append(f"{rel}:1: permissions: no top-level `permissions:` key")
    elif wf.top_keys["permissions"] == "write-all":
        out.append(f"{rel}:1: permissions: top-level `permissions: write-all`")
    # allowlist
    allowed = ALLOWLIST.get(path.name)
    if allowed is not None:
        names = [j.name for j in wf.jobs]
        for extra in sorted(set(names) - set(allowed)):
            out.append(f"{rel}: allowlist: job `{extra}` is not in ALLOWLIST (add it deliberately)")
        for gone in sorted(set(allowed) - set(names)):
            out.append(f"{rel}: allowlist: ALLOWLIST job `{gone}` is missing from the workflow")
    for job in wf.jobs:
        perms, err, n = read_permissions(job)
        if allowed is not None and job.name in allowed:
            want = allowed[job.name]
            if err:
                out.append(f"{rel}:{n}: allowlist: job `{job.name}`: {err}")
            elif perms != want:
                out.append(
                    f"{rel}:{n}: allowlist: job `{job.name}` has permissions {perms}, "
                    f"must be exactly {want}"
                )
        elif err:
            out.append(f"{rel}:{n}: permissions: job `{job.name}`: {err}")
        elif perms is not None and "*" in perms and perms["*"] == "write-all":
            out.append(f"{rel}:{n}: permissions: job `{job.name}` has `write-all`")
    # secrets
    header_touches = touches_secrets(wf.header)
    for job in wf.jobs:
        if header_touches or touches_secrets(job.lines):
            for n, value in uses_in(job.lines):
                out.append(
                    f"{rel}:{n}: secrets: job `{job.name}` touches the Worker secrets "
                    f"and also runs `uses: {value}`"
                )
    return out


def lint_action(path: Path, rel: str) -> list[str]:
    lines = list(enumerate(path.read_text().splitlines(), 1))
    return check_uses(path, lines, rel)


def lint(root: Path) -> list[str]:
    gh = root / ".github"
    out: list[str] = []
    workflows = sorted([*gh.glob("workflows/*.yml"), *gh.glob("workflows/*.yaml")])
    for path in workflows:
        out += lint_workflow(path, str(path.relative_to(root)))
    for path in sorted([*gh.glob("actions/**/action.yml"), *gh.glob("actions/**/action.yaml")]):
        out += lint_action(path, str(path.relative_to(root)))
    if not workflows:
        out.append(f"{gh}/workflows: no workflows found (wrong --root?)")
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    args = ap.parse_args(argv)
    findings = lint(args.root)
    for f in findings:
        print(f)
    if findings:
        print(f"workflow_lint: {len(findings)} finding(s)", file=sys.stderr)
        return 1
    print("workflow_lint: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
