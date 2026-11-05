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


def add(name, file, old, new, tests=None):
    path = ROOT / (file if file.startswith(("scripts/", ".github/")) else BROKER + file)
    source = path.read_text()
    assert source.count(old) == 1, (name, old, source.count(old))
    CASES.append((name, path, source, source.replace(old, new), tests or RUNNER_TESTS))


R = "runner.py"
SAN2 = '    for secret in secrets_by_length:\n        text = text.replace(secret, "[redacted]")\n    text = re.sub(r"\\bBearer'
M = [
    # --- KEY LINES
    ("key-lines-unframed", "        self.show(block)", "        self.emit(block)"),
    ("key-lines-no-cell-pattern", '    ("cell", _BOX + r"(?:FAST|BARE)_CELL_ARM\\b"),\n', ""),
    ("key-lines-no-gate-fail", '    ("gate_fail", _BOX + r"WINDOWS_GATE_FAIL\\b"),\n', ""),
    (
        "key-lines-byte-budget-ignored",
        "if len(chosen) >= max_lines or used + cost > max_bytes:",
        "if len(chosen) >= max_lines:",
    ),
    (
        "key-lines-line-budget-ignored",
        "if len(chosen) >= max_lines or used + cost > max_bytes:",
        "if used + cost > max_bytes:",
    ),
    ("key-lines-no-red-priority", "is_red = KEY_RED.search(line) is not None", "is_red = False"),
    (
        "key-lines-no-first-last",
        "    order += [i for name, _ in KEY_PATTERNS for i in seen.get(name, ())]\n",
        "",
    ),
    (
        "key-lines-raw-not-sanitised",
        "key_lines(self.sanitised()[0])",
        'key_lines("".join(self.raw))',
    ),
    ("key-lines-line-uncut", "[:KEY_LINE_BYTES].decode", "[:1 << 30].decode"),
    (
        "key-lines-scan-uncut",
        "lines = [line[:KEY_SCAN] for line in text.splitlines()]",
        "lines = text.splitlines()",
    ),
    (
        "key-lines-shown-when-small",
        "        if self.head_full:\n            self.show_keys()",
        "        if True:\n            self.show_keys()",
    ),
    (
        "display-lines-raised-far",
        "DISPLAY_BYTES, DISPLAY_LINES = 65000, 270",
        "DISPLAY_BYTES, DISPLAY_LINES = 65000, 600",
    ),
    (
        "display-bytes-raised",
        "DISPLAY_BYTES, DISPLAY_LINES = 65000, 270",
        "DISPLAY_BYTES, DISPLAY_LINES = 650000, 270",
    ),
    # --- census (census-line-uncut, `line[:400]`, is an equivalent mutant: the key is cut at 60 characters
    # afterwards; the slice only bounds the work per line)
    (
        "census-prefix-uncapped",
        'return " ".join(out)[:CENSUS_PREFIX] or "(blank)"',
        'return " ".join(out) or "(blank)"',
    ),
    ("census-all-groups-named", "for key, g in ranked[:CENSUS_LINES]]", "for key, g in ranked]"),
    ("census-keys-unbounded", "if len(groups) >= CENSUS_KEYS:", "if False:"),
    ("census-unframed", "        self.show(rows)", "        self.emit(rows)"),
    (
        "census-not-sanitised",
        '        text, _ = sanitise(middle, self.ctx.get("redactions", ()))',
        "        text = middle",
    ),
    (
        "census-others-uncounted",
        "    others = [overflow[0] + sum(g[0] for _, g in ranked[CENSUS_LINES:]), 0]",
        "    others = [overflow[0], 0]",
    ),
    ("census-not-before-tail", "            self.show_census(skipped)\n", ""),
    ("census-digits-in-names-collapsed", '(?<![A-Za-z_])\\d+", "#"', '\\d+", "#"'),
    # --- honest markers
    (
        "marker-sanitisation-wording",
        'str(removed) + " bytes removed by sanitisation]\\n"',
        'str(removed) + " bytes dropped by runner]\\n"',
    ),
    ("marker-cap-uncounted", "capped = max(0, min(dropped, total - kept))", "capped = 0"),
    # --- redaction order (review F5)
    ("redact-before-strip-only", SAN2, '    text = re.sub(r"\\bBearer'),
    (
        "redact-after-strip-only",
        '    for secret in secrets_by_length:\n        text = text.replace(secret, "[redacted]")\n    text = re.sub(r"(?:\\x1b\\]',
        '    text = re.sub(r"(?:\\x1b\\]',
    ),
    # --- failure-path DIAGNOSTICS
    ("diag-always", "diag = self.diagnostics() if vanished else None", "diag = self.diagnostics()"),
    ("diag-never", "diag = self.diagnostics() if vanished else None", "diag = None"),
    (
        "diag-missing-fields-unlisted",
        "  Not returned by the Worker protocol (Worker-side follow-up)",
        "  (nothing else)",
    ),
    (
        "diag-stage-name-unbounded",
        'name=([A-Za-z0-9_.-]{1,40})", before',
        'name=([^\\n]{1,4000})", before',
    ),
    (
        "diag-box-line-unframed",
        "                self.show(diag[1])",
        '                self.emit(diag[1] + "\\n")',
    ),
    (
        "diag-no-transitions",
        'if not self.transitions or self.transitions[-1][1] != data["state"]:',
        "if False:",
    ),
    (
        "diag-claims-log-below-always",
        '                if self.raw\n                else "; the Worker returned no log text"',
        '                if True\n                else "; the Worker returned no log text"',
    ),
    # --- the protocol
    (
        "protocol-rent-body-changed",
        '                "tested_sha": self.ctx["sha"],',
        '                "tested_sha": self.ctx["sha"],\n                "extra": 1,',
    ),
    ("protocol-version", "STATUS_PROTOCOL = 6", "STATUS_PROTOCOL = 7"),
    # --- no file output (owner ruling 2026-11-05)
    (
        "file-output-comes-back",
        "\n\ndef integer(v, lo=0",
        "\n\ndef write_runlog():\n    pass\n\n\ndef integer(v, lo=0",
    ),
]
for name, old, new in M:
    add(name, R, old, new)


def run(work, tests):
    return subprocess.run(
        ["node", "--test", *[BROKER + "test/" + t for t in tests]],
        cwd=work,
        capture_output=True,
        text=True,
        timeout=600,
    )


def kills(tests, result):
    return re.findall(r"^\s*not ok [0-9]+ - (.+)$", result.stdout, re.M)


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="runlog-mutants-") as directory:
        work = Path(directory)
        for part in ("ci/vast-broker", ".github/workflows"):
            shutil.copytree(
                ROOT / part,
                work / part,
                ignore=shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__"),
            )
        (work / "node_modules").symlink_to(ROOT / "ci/vast-broker/node_modules")
        baseline = run(work, RUNNER_TESTS)
        assert baseline.returncode == 0, (baseline.stdout[-3000:], baseline.stderr[-2000:])
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
