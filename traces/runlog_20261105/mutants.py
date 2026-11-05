#!/usr/bin/env python3
"""Mutants for the gpu-vast runner's run-log work (KEY LINES, census, honest markers, full-log file,
upload step guard). Private copies of the sources; no network, no provider, no Worker.

Each mutant changes one fact of the new behaviour; every one must make at least one named test fail.
Run: nice -n 19 python3 traces/runlog_20261105/mutants.py   (writes mutations.json beside it)
"""

import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
CASES = []
RUNNER_TESTS = [
    "runner-runlog.test.js",
    "runner-head-tail.test.js",
    "round3-drop-count.test.js",
    "round3-runner.test.js",
]
GUARD_TESTS = ["runlog-workflow.test.js", "round3-bootstrap.test.js", "round3-isolation.test.js"]


def add(name, file, old, new, tests):
    path = ROOT / (file if file.startswith(("scripts/", ".github/")) else BROKER + file)
    source = path.read_text()
    assert source.count(old) == 1, (name, old, source.count(old))
    CASES.append((name, path, source, source.replace(old, new), tests))


R = "runner.py"
# --- KEY LINES
add("key-lines-unframed", R, "        self.show(block)", "        self.emit(block)", RUNNER_TESTS)
add(
    "key-lines-no-cell-pattern",
    R,
    '    ("cell", _BOX + r"(?:FAST|BARE)_CELL_ARM\\b"),\n',
    "",
    RUNNER_TESTS,
)
add(
    "key-lines-no-gate-fail",
    R,
    '    ("gate_fail", _BOX + r"WINDOWS_GATE_FAIL\\b"),\n',
    "",
    RUNNER_TESTS,
)
add(
    "key-lines-no-xid",
    R,
    '    ("xid", r"^(?:\\[box\\] )?\\[xid-watch\\] xid_(?:before|after|delta)\\b|NEW HOST Xid"),\n',
    "",
    RUNNER_TESTS,
)
add(
    "key-lines-byte-budget-ignored",
    R,
    "if len(chosen) >= max_lines or used + cost > max_bytes:",
    "if len(chosen) >= max_lines:",
    RUNNER_TESTS,
)
add(
    "key-lines-line-budget-ignored",
    R,
    "if len(chosen) >= max_lines or used + cost > max_bytes:",
    "if used + cost > max_bytes:",
    RUNNER_TESTS,
)
add(
    "key-lines-no-red-priority",
    R,
    "is_red = KEY_RED.search(line) is not None",
    "is_red = False",
    RUNNER_TESTS,
)
add(
    "key-lines-no-first-last",
    R,
    "    order += [i for name, _ in KEY_PATTERNS for i in seen.get(name, ())]\n",
    "",
    RUNNER_TESTS,
)
add(
    "key-lines-raw-not-sanitised",
    R,
    "key_lines(self.sanitised()[0])",
    'key_lines("".join(self.raw))',
    RUNNER_TESTS,
)
add("key-lines-line-uncut", R, "[:KEY_LINE_BYTES].decode", "[:1 << 30].decode", RUNNER_TESTS)
add(
    "key-lines-shown-when-small",
    R,
    "        if self.head_full:\n            self.show_keys()",
    "        if True:\n            self.show_keys()",
    RUNNER_TESTS,
)
add(
    "key-lines-before-group",
    R,
    '        self.emit("::group::Final bounded log: UNTRUSTED Worker/box output\\n")\n        if self.head_full:\n            self.show_keys()\n',
    '        if self.head_full:\n            self.show_keys()\n        self.emit("::group::Final bounded log: UNTRUSTED Worker/box output\\n")\n',
    RUNNER_TESTS,
)
add(
    "display-lines-raised-far",
    R,
    "DISPLAY_BYTES, DISPLAY_LINES = 65000, 270",
    "DISPLAY_BYTES, DISPLAY_LINES = 65000, 600",
    RUNNER_TESTS,
)
add(
    "display-bytes-raised",
    R,
    "DISPLAY_BYTES, DISPLAY_LINES = 65000, 270",
    "DISPLAY_BYTES, DISPLAY_LINES = 650000, 270",
    RUNNER_TESTS,
)
# --- census
add(
    "census-prefix-uncapped",
    R,
    'return " ".join(out)[:CENSUS_PREFIX] or "(blank)"',
    'return " ".join(out) or "(blank)"',
    RUNNER_TESTS,
)
# census-line-uncut (`line[:400]` before normalising) is an equivalent mutant: the key is cut at 60
# characters afterwards, so the output is the same; the slice only bounds the work per line.
add(
    "census-all-groups-named",
    R,
    "for key, g in ranked[:CENSUS_LINES]]",
    "for key, g in ranked]",
    RUNNER_TESTS,
)
add("census-keys-unbounded", R, "if len(groups) >= CENSUS_KEYS:", "if False:", RUNNER_TESTS)
add("census-unframed", R, "        self.show(rows)", "        self.emit(rows)", RUNNER_TESTS)
add(
    "census-not-sanitised",
    R,
    '        text, _ = sanitise(middle, self.ctx.get("redactions", ()))',
    "        text = middle",
    RUNNER_TESTS,
)
add(
    "census-others-uncounted",
    R,
    "    others = [overflow[0] + sum(g[0] for _, g in ranked[CENSUS_LINES:]), 0]",
    "    others = [overflow[0], 0]",
    RUNNER_TESTS,
)
add("census-not-before-tail", R, "            self.show_census(skipped)\n", "", RUNNER_TESTS)
add("census-digits-in-names-collapsed", R, '(?<![A-Za-z_])\\d+", "#"', '\\d+", "#"', RUNNER_TESTS)
# --- honest markers
add(
    "marker-sanitisation-wording",
    R,
    'str(removed) + " bytes removed by sanitisation]\\n"',
    'str(removed) + " bytes dropped by runner]\\n"',
    RUNNER_TESTS,
)
add(
    "marker-cap-uncounted",
    R,
    "capped = max(0, min(dropped, total - kept))",
    "capped = 0",
    RUNNER_TESTS,
)
# --- the artifact
add(
    "runlog-follows-symlinks",
    R,
    "os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,",
    "os.O_WRONLY | os.O_CREAT | os.O_TRUNC,",
    RUNNER_TESTS,
)
add(
    "runlog-cap-removed",
    R,
    'render(text, original, MAX_LOG - 512 - 8192, 1 << 30, marker="[untrusted] ")',
    'render(text, original, MAX_LOG * 8, 1 << 30, marker="[untrusted] ")',
    RUNNER_TESTS,
)
add(
    "runlog-marker-unprefixed",
    R,
    'render(text, original, MAX_LOG - 512 - 8192, 1 << 30, marker="[untrusted] ")',
    "render(text, original, MAX_LOG - 512 - 8192, 1 << 30)",
    RUNNER_TESTS,
)
add(
    "runlog-unsanitised",
    R,
    "cleaned=(text, original)",
    'cleaned=("".join(self.raw), original)',
    RUNNER_TESTS,
)
add(
    "runlog-line-prefix-missing",
    R,
    'rendered = "[untrusted] " + safe + "\\n"',
    'rendered = safe + "\\n"',
    RUNNER_TESTS,
)
add(
    "runlog-failure-escapes",
    R,
    '            except Exception:\n                self.emit("Full sanitised log: not written\\n")',
    '            except ZeroDivisionError:\n                self.emit("Full sanitised log: not written\\n")',
    RUNNER_TESTS,
)
add("runlog-dir-traversal", R, ' or "/../" in path', "", RUNNER_TESTS)
add(
    "runlog-dir-symlink",
    R,
    'return path if os.path.isdir(path) and not os.path.islink(path) else ""',
    "return path",
    RUNNER_TESTS,
)
add(
    "runlog-dir-from-anywhere",
    R,
    r'r"/[A-Za-z0-9_./-]{1,200}/gpu-vast-runlog"',
    r'r"[^\0]{1,200}"',
    RUNNER_TESTS,
)
# --- failure-path DIAGNOSTICS
add(
    "diag-always",
    R,
    "diag = self.diagnostics() if vanished else None",
    "diag = self.diagnostics()",
    RUNNER_TESTS,
)
add("diag-never", R, "diag = self.diagnostics() if vanished else None", "diag = None", RUNNER_TESTS)
add(
    "diag-not-in-artifact",
    R,
    "cleaned=(text, original), diagnostics=diag",
    "cleaned=(text, original), diagnostics=None",
    RUNNER_TESTS,
)
add(
    "diag-missing-fields-unlisted",
    R,
    "  Not returned by the Worker protocol (Worker-side follow-up)",
    "  (nothing else)",
    RUNNER_TESTS,
)
add(
    "diag-stage-name-unbounded",
    R,
    'name=([A-Za-z0-9_.-]{1,40})", before',
    'name=([^\\n]{1,4000})", before',
    RUNNER_TESTS,
)
add(
    "diag-box-line-unframed",
    R,
    "                self.show(diag[1])",
    '                self.emit(diag[1] + "\n")',
    RUNNER_TESTS,
)
add(
    "diag-no-transitions",
    R,
    'if not self.transitions or self.transitions[-1][1] != data["state"]:',
    "if False:",
    RUNNER_TESTS,
)
add(
    "diag-artifact-unsanitised",
    R,
    'block, _ = sanitise("\\n".join(lines) + "\\n")',
    'block = "\\n".join(lines) + "\\n"',
    RUNNER_TESTS,
)
# --- the protocol
add(
    "protocol-rent-body-changed",
    R,
    '                "tested_sha": self.ctx["sha"],',
    '                "tested_sha": self.ctx["sha"],\n                "extra": 1,',
    RUNNER_TESTS,
)
add("protocol-version", R, "STATUS_PROTOCOL = 6", "STATUS_PROTOCOL = 7", RUNNER_TESTS)
# --- the workflow guard, bootstrap, lint
add(
    "guard-upload-step-any",
    "workflow-guard.js",
    "allSteps[allSteps.length - 1].trim() === runlogUploadStep.trim()",
    "true",
    GUARD_TESTS,
)
add(
    "guard-upload-text-anywhere",
    "workflow-guard.js",
    "  const gpuText = workflows[gpuName].replace(runlogUploadStep, '');",
    "  const gpuText = '';",
    GUARD_TESTS,
)
# guard-upload-in-sweep (dropping `upload-artifact` from the sweep's own forbidden list) is not killed:
# the broker-step rule rejects any extra step in the sweep's `gpu` job as well. Two checks, one effect.
add(
    "bootstrap-no-log-dir",
    "workflow-bootstrap.js",
    "    '  KF_RUNLOG_DIR=\"$RUNNER_TEMP/gpu-vast-runlog\" \\\\',\n",
    "",
    GUARD_TESTS,
)
add(
    "bootstrap-sweep-gets-dir",
    "workflow-bootstrap.js",
    "  .replace('  KF_RUNLOG_DIR=\"$RUNNER_TEMP/gpu-vast-runlog\" \\\\\\n', '')\n",
    "",
    GUARD_TESTS,
)
add(
    "upload-retention",
    "workflow-bootstrap.js",
    "  '          retention-days: 30',",
    "  '          retention-days: 90',",
    GUARD_TESTS,
)
add(
    "upload-pin",
    "workflow-bootstrap.js",
    "upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2",
    "upload-artifact@v4",
    GUARD_TESTS,
)
LINT = "python3-unittest"
add(
    "lint-exception-any-action",
    "scripts/ci/workflow_lint.py",
    "if value in SECRET_JOB_USES.get((path.name, job.name), ()):",
    'if value.startswith("actions/"):',
    LINT,
)
add(
    "lint-exception-any-job",
    "scripts/ci/workflow_lint.py",
    "if value in SECRET_JOB_USES.get((path.name, job.name), ()):",
    "if any(value in v for v in SECRET_JOB_USES.values()):",
    LINT,
)


def run(work, tests):
    if tests == LINT:
        return subprocess.run(
            ["python3", "-m", "unittest", "test_workflow_lint"],
            cwd=work / "scripts/ci",
            env={"PATH": "/usr/bin:/bin:/usr/local/bin", "PYTHONPATH": str(work / "scripts/ci")},
            capture_output=True,
            text=True,
            timeout=300,
        )
    return subprocess.run(
        ["node", "--test", *[BROKER + "test/" + t for t in tests]],
        cwd=work,
        capture_output=True,
        text=True,
        timeout=600,
    )


def kills(tests, result):
    if tests == LINT:
        return re.findall(r"^(?:FAIL|ERROR): (test_\w+)", result.stderr, re.M)
    return re.findall(r"^\s*not ok [0-9]+ - (.+)$", result.stdout, re.M)


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="runlog-mutants-") as directory:
        work = Path(directory)
        for part in ("ci/vast-broker", ".github/workflows", "scripts/ci"):
            shutil.copytree(
                ROOT / part,
                work / part,
                ignore=shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__"),
            )
        (work / "node_modules").symlink_to(ROOT / "ci/vast-broker/node_modules")
        for tests in (RUNNER_TESTS, GUARD_TESTS, LINT):
            baseline = run(work, tests)
            assert baseline.returncode == 0, (
                tests,
                baseline.stdout[-3000:],
                baseline.stderr[-2000:],
            )
        for name, path, original, mutant, tests in CASES:
            target = work / path.relative_to(ROOT)
            try:
                target.write_text(mutant)
                result = run(work, tests)
                named = kills(tests, result)
                assert result.returncode and named, (
                    name,
                    result.stdout[-3000:],
                    result.stderr[-2000:],
                )
                results.append({"mutant": name, "kills": len(named), "tests": named})
                print(name, len(named), flush=True)
            finally:
                target.write_text(original)
    (Path(__file__).parent / "mutations.json").write_text(
        json.dumps({"baseline": "PASS", "mutants": results}, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
