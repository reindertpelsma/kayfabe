#!/usr/bin/env python3
"""Ratchet overlong in-use source lines, and reject new ones in base...HEAD.

No third-party Python packages or formatter are needed. Counts are per file;
removing debt in one file never buys another file an exception. The initial
inventory is reviewed once; --update can only lower it. See docs/CI_V3.md.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BASELINE = "scripts/ci/linelen-baseline.json"
LIMIT = 100
LANGUAGES = {
    ".rs": "Rust",
    ".c": "C",
    ".h": "Header",
    ".cc": "C++",
    ".cpp": "C++",
    ".cxx": "C++",
    ".hh": "Header",
    ".hpp": "Header",
    ".cu": "CUDA",
    ".cuh": "CUDA",
    ".sh": "Shell",
    ".bash": "Shell",
    ".zsh": "Shell",
    ".py": "Python",
    ".js": "JavaScript/TypeScript",
    ".jsx": "JavaScript/TypeScript",
    ".mjs": "JavaScript/TypeScript",
    ".cjs": "JavaScript/TypeScript",
    ".ts": "JavaScript/TypeScript",
    ".tsx": "JavaScript/TypeScript",
    ".toml": "TOML",
    ".yaml": "YAML",
    ".yml": "YAML",
}
IN_USE_TREES = (
    "qemu/hw/misc/kf3/",
    "cuda/",
    "broker/",
    "ci/",
    "tools/",
    "tests/",
    "scripts/ci/",
    "scripts/fastguest/",
    "scripts/drivermatrix/",
    "scripts/apps/",
    "scripts/bench/box/",
    "scripts/bench/cuda_min/",
    ".github/workflows/",
)
# Gate entry points and their checked-in build/boot/ladder helpers, not all of bench/.
BENCH_FILES = frozenset(
    "scripts/bench/" + name
    for name in (
        "v3_gates.sh",
        "build_kf3.sh",
        "cuda_ladder.sh",
        "boot_capture.sh",
        "boot_nvkvm.sh",
        "assert_boot_evidence.sh",
        "qemu_identity.sh",
        "qemu_identity_selftest.sh",
        "provision_box.sh",
        "provision_bench_tree.sh",
        "provision_host_driver.sh",
        "cup2_hook_deadline.sh",
        "cup3_hook.sh",
        "cup8_hook.sh",
        "cup8bench_hook.sh",
        "cup3.c",
        "cup8.c",
        "cup8bench.c",
        "uvmparams_shim.c",
        "gssh_nv",
    )
)
IN_USE_FILES = BENCH_FILES | {
    "Cargo.toml",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "pyproject.toml",
    ".clang-format",
    ".editorconfig",
    "scripts/ci_gates.sh",
}
GENERATED_HEADER = re.compile(
    r"^\s*(?://|#|/\*|\*)\s*(?:@generated\b|"
    r"(?:auto[- ]?generated|generated)\b[^\n]*(?:do not edit|don't edit))",
    re.IGNORECASE | re.MULTILINE,
)
REASON = re.compile(r"(?:#|//|/\*)\s*long-line-ok:\s*(.*?)\s*(?:\*/)?$")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")


def git(root: Path, *args: str) -> bytes:
    result = subprocess.run(["git", *args], cwd=root, capture_output=True, check=False)
    if result.returncode:
        raise ValueError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def tracked(root: Path) -> list[str]:
    return [p.decode("utf-8") for p in git(root, "ls-files", "-z").split(b"\0") if p]


def language(path: str) -> str | None:
    if path in {".clang-format", ".editorconfig"}:
        return "Config"
    if path == "scripts/bench/gssh_nv":
        return "Shell"
    return LANGUAGES.get(PurePosixPath(path).suffix.lower())


def path_exemption(path: str) -> str | None:
    parts = PurePosixPath(path).parts
    for part in parts[:-1]:
        if part in {"archive", "archives"} or part.startswith(("archive-", "archive_")):
            return "archive"
        if part in {"third_party", "vendor", "vendored", "node_modules"}:
            return "vendored"
        if part == "traces":
            return "evidence"
    if len(parts) > 1 and parts[0] == "crates" and parts[1].startswith("kayfabe-"):
        return "frozen pre-v3"
    if "generated" in parts[:-1] or PurePosixPath(path).stem.endswith("_generated"):
        return "generated"
    name = parts[-1]
    if name.endswith(".lock") or name in {"yarn.lock", "package-lock.json", "pnpm-lock.yaml"}:
        return "lockfile"
    product = len(parts) > 1 and parts[0] == "crates" and parts[1].startswith("kf-")
    if not (product or path.startswith(IN_USE_TREES) or path in IN_USE_FILES):
        return "outside in-use set (legacy/one-off)"
    return None


def source_lines(content: str) -> list[str]:
    """Use Git's physical LF lines, not Unicode's additional line separators."""
    lines = content.split("\n")
    if lines[-1] == "":
        lines.pop()
    return [line.removesuffix("\r") for line in lines]


def generated(content: str) -> bool:
    # Header comments only: a generator that emits a marker is still hand-written source.
    return bool(GENERATED_HEADER.search("\n".join(source_lines(content)[:20])))


def exempt_line(line: str) -> bool:
    text = line.strip()
    match = REASON.search(text)
    if match and match[1].strip(" */"):
        return True
    # A sole URL/hash may be in a comment; surrounding code is never exempt.
    for prefix in ("//", "#", "/*", "*"):
        if text.startswith(prefix):
            text = text[len(prefix) :].strip()
            break
    text = text.removesuffix("*/").strip()
    return bool(re.fullmatch(r"https?://\S+|[0-9a-fA-F]{32,}", text))


def overlong(content: str) -> list[tuple[int, int]]:
    return [
        (number, len(line.expandtabs(8)))
        for number, line in enumerate(source_lines(content), 1)
        if len(line.expandtabs(8)) > LIMIT and not exempt_line(line)
    ]


def scan(
    root: Path, include_exempt: bool = False
) -> tuple[dict[str, int], list[tuple[str, str, int, int, int]]]:
    counts = {}
    rows = []
    for path in tracked(root):
        lang = language(path)
        if lang is None:
            continue
        reason = path_exemption(path)
        if reason and not include_exempt:
            continue
        source = root / path
        if source.is_symlink() or not source.is_file():
            continue
        content = source.read_bytes().decode("utf-8-sig")
        if reason is None and generated(content):
            reason = "generated"
        if reason and not include_exempt:
            continue
        lines = source_lines(content)
        raw = sum(len(line.expandtabs(8)) > LIMIT for line in lines)
        debt = len(overlong(content)) if reason is None else 0
        if debt:
            counts[path] = debt
        rows.append((path, reason or "IN-USE", len(lines), raw, debt))
    return counts, rows


def parse_baseline(content: str) -> dict[str, int]:
    value = json.loads(content)
    if not isinstance(value, dict) or any(
        not isinstance(path, str) or type(count) is not int or count <= 0
        for path, count in value.items()
    ):
        raise ValueError("baseline must map source paths to positive integer counts")
    return value


def baseline_at(root: Path, revision: str) -> dict[str, int] | None:
    paths = git(root, "ls-tree", "--name-only", "-z", revision, "--", BASELINE)
    if not paths:
        return None  # The first, reviewed enrollment; added source lines still fail.
    return parse_baseline(git(root, "show", f"{revision}:{BASELINE}").decode("utf-8"))


def increases(actual: dict[str, int], baseline: dict[str, int]) -> list[str]:
    return sorted(path for path, count in actual.items() if count > baseline.get(path, 0))


def added_lines(root: Path, base: str, head: str) -> list[str]:
    failures = []
    revision_range = f"{base}...{head}"
    names = git(
        root,
        "diff",
        "--no-color",
        "--name-only",
        "-z",
        "--no-renames",
        "--diff-filter=AM",
        revision_range,
    )
    for name in filter(None, names.split(b"\0")):
        path = name.decode("utf-8")
        if language(path) is None or path_exemption(path):
            continue
        entry = git(root, "ls-tree", "-z", head, "--", path)
        if not entry.startswith((b"100644 ", b"100755 ")):
            continue
        content = git(root, "show", f"{head}:{path}").decode("utf-8-sig")
        if generated(content):
            continue
        bad = dict(overlong(content))
        if not bad:
            continue
        diff = git(
            root,
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--unified=0",
            revision_range,
            "--",
            path,
        ).decode("utf-8")
        number = None
        for line in diff.split("\n"):
            match = HUNK.match(line)
            if match:
                number = int(match[1])
            elif number is not None and line.startswith("+"):
                if number in bad:
                    failures.append(f"NEW LINE {path}:{number}: {bad[number]} > {LIMIT} columns")
                number += 1
            elif number is not None and line.startswith(" "):
                number += 1
    return failures


def report(rows: list[tuple[str, str, int, int, int]]) -> None:
    totals = defaultdict(lambda: [0, 0, 0, 0])
    for path, reason, lines, raw, debt in rows:
        group = "IN-USE" if reason == "IN-USE" else "EXEMPT"
        stats = totals[(group, language(path))]
        for index, value in enumerate((1, lines, raw, debt)):
            stats[index] += value
    print("scope language files lines raw-over-100 baseline-debt")
    for (group, lang), stats in sorted(totals.items()):
        print(f"{group} {lang}: " + " ".join(str(value) for value in stats))
    print("Top 30 IN-USE files (baseline debt):")
    for path, _, _, _, debt in sorted(rows, key=lambda row: (-row[4], row[0]))[:30]:
        if debt:
            print(f"{debt:5} {path}")
    print("EXEMPT by reason (not baselined):")
    reasons = defaultdict(lambda: [0, 0])
    for _, reason, _, raw, _ in rows:
        if reason != "IN-USE":
            reasons[reason][0] += 1
            reasons[reason][1] += raw
    for reason, (files, raw) in sorted(reasons.items()):
        print(f"{reason}: files={files} raw-over-100={raw}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--base", help="PR base ref; check added lines in base...head")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--update", action="store_true", help="lower the existing baseline only")
    parser.add_argument("--report", action="store_true", help="show in-use and exempt inventories")
    args = parser.parse_args()
    try:
        root = args.root.resolve()
        path = root / BASELINE
        baseline = parse_baseline(path.read_text())
        actual, rows = scan(root, include_exempt=args.report)
        if args.report:
            report(rows)
        failures = []
        for name in increases(actual, baseline):
            failures.append(f"COUNT INCREASE {name}: {baseline.get(name, 0)} -> {actual[name]}")
        history = None
        if args.base:
            merge_base = git(root, "merge-base", args.base, args.head).decode().strip()
            history = baseline_at(root, merge_base)
            failures.extend(added_lines(root, args.base, args.head))
        elif args.update:
            history = baseline_at(root, "HEAD")
        if history is not None:
            for name in increases(baseline, history):
                failures.append(
                    f"BASELINE INCREASE {name}: {history.get(name, 0)} -> {baseline[name]}"
                )
        for failure in failures:
            print(failure)
        print(
            f"linelen: files={sum(row[1] == 'IN-USE' for row in rows)} "
            f"debt={sum(actual.values())} new={len(failures)} "
            f"resolved={sum(baseline.values()) - sum(actual.values())}"
        )
        if failures:
            return 1
        if args.update:
            path.write_text(json.dumps(actual, sort_keys=True, indent=2) + "\n")
            print(f"LOWERED baseline: {len(actual)} files; no increases permitted")
        return 0
    except (OSError, UnicodeError, ValueError, subprocess.SubprocessError) as error:
        print(f"linelen: INFRASTRUCTURE ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
