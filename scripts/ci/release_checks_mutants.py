#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutate the real release-check rules in isolated scratch trees; >=2 tests per mutant."""

from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent


def changed(file, old, new):
    source = (HERE / file).read_text()
    assert source.count(old) == 1, (file, old, source.count(old))
    return file, source.replace(old, new)


def cases():
    return [
        (
            "patch-cli-failure-hidden",
            changed("no_patch_check.py", "return int(bool(errors))", "return 0"),
        ),
        (
            "clone-disk-safety-bypassed",
            changed(
                "clean_clone_check.sh", '[ "$free_kib" -ge 6291456 ]', "[ 1 -eq 1 ]"
            ),
        ),
        (
            "advisory-validator-failure-hidden",
            changed(
                "../../.github/workflows/release-claims.yml",
                "run: python3 scripts/ci/release_claims.py validate",
                "run: python3 scripts/ci/release_claims.py validate || true",
            ),
        ),
        (
            "advisory-validates-summary-only",
            changed(
                "../../.github/workflows/release-claims.yml",
                "run: python3 scripts/ci/release_claims.py validate",
                "run: python3 scripts/ci/release_claims.py summary",
            ),
        ),
        (
            "clone-build-doc-drift-ignored",
            changed(
                "clean_clone_inputs.py",
                'if command != "cargo build --workspace --release":',
                "if False:",
            ),
        ),
        (
            "clone-test-doc-drift-ignored",
            changed(
                "clean_clone_inputs.py", 'if "cargo test" not in text:', "if False:"
            ),
        ),
        (
            "clone-required-inputs-ignored",
            changed(
                "clean_clone_inputs.py", "if not (root / rel).is_file():", "if False:"
            ),
        ),
        (
            "clone-literal-includes-ignored",
            changed(
                "clean_clone_inputs.py",
                "if not path.is_file():\n                    errors.append",
                "if False:\n                    errors.append",
            ),
        ),
        (
            "clone-archived-readers-allowed",
            changed(
                "clean_clone_inputs.py",
                "if rel in moved and not (root / rel).is_file():",
                "if False:",
            ),
        ),
        (
            "clone-preflight-not-enforced",
            changed(
                "clean_clone_check.sh",
                '|| fail "documented steps or committed inputs are missing (no evidence is fetched)"',
                "|| true",
            ),
        ),
        (
            "clone-depth-removed",
            changed("clean_clone_check.sh", "--depth=1", "--depth=2"),
        ),
        (
            "clone-build-failure-ignored",
            changed(
                "clean_clone_check.sh",
                '|| fail "documented release build failed; inspect the named prerequisite/input above"',
                "|| true",
            ),
        ),
        (
            "clone-test-failure-ignored",
            changed(
                "clean_clone_check.sh",
                '|| fail "GPU-free tests failed; a missing runtime input must be committed, not restored from an evidence archive"',
                "|| true",
            ),
        ),
        (
            "patch-new-artifacts-allowed",
            changed(
                "no_patch_check.py",
                "for rel in sorted(actual - patches.keys()):",
                "for rel in []:",
            ),
        ),
        (
            "patch-opt-in-pointer-unchecked",
            changed(
                "no_patch_check.py",
                "if not pointer or not (root / pointer).is_file():",
                "if False:",
            ),
        ),
        (
            "patch-new-applicators-allowed",
            changed(
                "no_patch_check.py",
                "if APPLICATION.search(text) and rel not in applicators:",
                "if False:",
            ),
        ),
        (
            "patch-review-hash-ignored",
            changed(
                "no_patch_check.py",
                "hashlib.sha256(path.read_bytes()).hexdigest() != digest",
                "False",
            ),
        ),
        (
            "patch-stock-reachability-ignored",
            changed("no_patch_check.py", "if rel in applicators:", "if False:"),
        ),
        (
            "patch-default-qualification-ignored",
            changed("no_patch_check.py", 'if "by default" not in text:', "if False:"),
        ),
        (
            "patch-exceptions-omitted",
            changed("no_patch_check.py", 'if "opt-in" not in text:', "if False:"),
        ),
        (
            "patch-stock-entrypoint-missing-allowed",
            changed("no_patch_check.py", "if rel not in sources:", "if False:"),
        ),
        (
            "patch-classification-unchecked",
            changed(
                "no_patch_check.py",
                'if item["kind"] not in {"opt-in", "recorded-only", "archive-only"}:',
                "if False:",
            ),
        ),
    ]


def run(file=None, source=None):
    if file and file.endswith(".yml"):
        test = "test_release_checks_workflows.py"
    else:
        test = (
            "test_no_patch_check.py"
            if file == "no_patch_check.py"
            else "test_clean_clone_check.py"
        )
    with tempfile.TemporaryDirectory(prefix="kf-release-mutant-") as temp:
        root = Path(temp)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        subprocess.run(
            ["git", "init", "-q", str(root)], check=True, capture_output=True
        )
        for name in (
            "clean_clone_check.sh",
            "clean_clone_inputs.py",
            "no_patch_check.py",
            "test_clean_clone_check.py",
            "test_no_patch_check.py",
            "test_release_checks_workflows.py",
            "release_claims.py",
        ):
            shutil.copyfile(HERE / name, ci / name)
        shutil.copytree(
            HERE / "fixtures/release_checks", ci / "fixtures/release_checks"
        )
        workflows = root / ".github/workflows"
        workflows.mkdir(parents=True)
        for name in ("ci.yml", "release-claims.yml"):
            shutil.copyfile(
                HERE.parents[1] / ".github/workflows" / name, workflows / name
            )
        if file:
            (ci / file).write_text(source)
        result = subprocess.run(
            (
                [
                    sys.executable,
                    "-I",
                    "-m",
                    "unittest",
                    "discover",
                    "-s",
                    str(ci),
                    "-p",
                    test,
                ]
                if file
                else [
                    sys.executable,
                    "-I",
                    "-m",
                    "unittest",
                    "discover",
                    "-s",
                    str(ci),
                    "-p",
                    "test_*.py",
                ]
            ),
            cwd=root,
            capture_output=True,
            text=True,
            timeout=120,
        )
        output = result.stdout + result.stderr
        killers = sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", output, re.M)))
        return result.returncode, killers, output


def main():
    code, _, output = run()
    if code:
        print("Unmutated baseline failed:\n" + output)
        return 2
    weak = 0
    for name, (file, source) in cases():
        code, killers, output = run(file, source)
        ok = code != 0 and len(killers) >= 2
        weak += not ok
        print(
            f'{"ok" if ok else "WEAK"} {name}: {len(killers)} killers: {", ".join(killers)}',
            flush=True,
        )
        if not ok:
            print(output[-2000:])
    print(
        f"{len(cases()) - weak}/{len(cases())} mutants killed by >=2 independently named tests"
    )
    return int(bool(weak))


if __name__ == "__main__":
    sys.exit(main())
