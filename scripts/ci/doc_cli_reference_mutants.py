#!/usr/bin/env python3
"""Mutation checks of the command-line reference generator (2026-11-20): every mutant needs two named killers.

STATUS: LIVE, 2026-11-20 (scripts/ci/doc_cli_reference.py, scripts/ci/test_doc_cli_reference.py).

Each mutant is one exact-once text replacement in a private copy of doc_cli_reference.py. The copy sits in a scratch tree
with the tests, the device source and the page, and test_doc_cli_reference runs there; the failing test names are the
killers. The baseline (the unmutated copy) must be green, and a mutant that fewer than two tests kill is a failure of this
script. Nothing outside the scratch tree is touched.

    python3 scripts/ci/doc_cli_reference_mutants.py [substring-of-mutant-name]
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TARGET = "scripts/ci/doc_cli_reference.py"
COPY = [
    TARGET,
    "scripts/ci/doc_cli_reference_text.json",
    "scripts/ci/test_doc_cli_reference.py",
    "qemu/hw/misc/kf3/kf3.c",
    "qemu/hw/misc/kf3/kf3.h",
    "docs/howto/cli-reference.md",
]
COLLECT = """import io, json, sys, unittest
sys.path.insert(0, '.')
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""

MUTANTS = [
    # missing property
    ("parser-loses-int64-properties", '"INT64": "int64", ', ""),
    ("parser-skips-the-last-property", "for m in MACRO.finditer(body):", "for m in list(MACRO.finditer(body))[:-1]:"),
    ("parser-skips-string-properties", 'if kind == "STRING":\n            row["default_text"] = "unset"', 'if kind == "STRING":\n            continue'),
    ("page-rows-ignore-the-new-marker", "(?: \\\\\\*)?", ""),
    ("missing-row-check-dropped", "for name in sorted(source.keys() - shown.keys()):", "for name in sorted(set()):"),
    ("extra-row-check-dropped", "for name in sorted(shown.keys() - source.keys()):", "for name in sorted(set()):"),
    ("text-file-gap-ignored", "for name in sorted(source.keys() - text.keys()):", "for name in sorted(set()):"),
    ("text-file-extra-ignored", "for name in sorted(text.keys() - source.keys()):", "for name in sorted(set()):"),
    ("duplicate-property-allowed", "if len({r[\"name\"] for r in rows}) != len(rows):", "if False:"),
    ("unreadable-declaration-skipped", 'raise RefError("unrecognised property declaration: "', 'continue; raise RefError("unrecognised property declaration: "'),
    # wrong default
    ("bool-default-inverted", '{"true": "on", "false": "off"}[args[3]]', '{"true": "off", "false": "on"}[args[3]]'),
    ("on-off-auto-default-swapped", 'ON_OFF_AUTO = {"ON_OFF_AUTO_AUTO": "auto", "ON_OFF_AUTO_ON": "on",', 'ON_OFF_AUTO = {"ON_OFF_AUTO_AUTO": "on", "ON_OFF_AUTO_ON": "auto",'),
    ("on-off-auto-unknown-default-accepted", "if args[3] not in ON_OFF_AUTO:", "if False:"),
    ("integer-default-off-by-one", 'row["default_text"] = str(value)', 'row["default_text"] = str(value + 1)'),
    ("mib-is-a-million", "MIB = 1024 * 1024", "MIB = 1000 * 1000"),
    ("define-not-resolved", "elif factor in names:", "elif False:"),
    ("default-comparison-dropped", "if shown[name] != source[name][\"default_text\"]:", "if False:"),
    ("default-comparison-inverted", "if shown[name] != source[name][\"default_text\"]:", "if shown[name] == source[name][\"default_text\"]:"),
    ("string-default-shown-as-zero", 'row["default_text"] = "unset"', 'row["default_text"] = "0"'),
    # stale table
    ("stale-comparison-dropped", "if block != generate(root):", "if False:"),
    ("stale-comparison-skipped-when-text-is-incomplete", "if not (source.keys() - text.keys()) and not (text.keys() - source.keys()):", "if False:"),
    ("generated-meaning-dropped", 'entry["status"], entry["meaning"])', 'entry["status"], "")'),
    ("generated-status-dropped", '(name, row["type"], default, entry["status"], entry["meaning"])', '(name, row["type"], default, "", entry["meaning"])'),
    ("write-keeps-the-old-block", "path.write_text(page.replace(block, generate(root), 1))", "path.write_text(page)"),
    ("comments-not-stripped", "    text = re.sub(r\"/\\*.*?\\*/\", \"\", text, flags=re.S)\n    return re.sub(r\"//[^\\n]*\", \"\", text)", "    return text"),
]


def run_suite(tree):
    result = subprocess.run(
        [sys.executable, "-B", "-I", "-c", COLLECT, "test_doc_cli_reference"],
        cwd=tree / "scripts/ci", capture_output=True, text=True, timeout=300,
    )
    for line in result.stdout.splitlines():
        if line.startswith("KF_MUTATION_RESULT="):
            return json.loads(line.split("=", 1)[1])
    raise SystemExit("no result line from the test run:\n" + result.stdout + result.stderr)


def scratch():
    tree = Path(tempfile.mkdtemp(prefix="kf-cli-mut-"))
    for rel in COPY:
        (tree / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(ROOT / rel, tree / rel)
    return tree


def main(argv):
    only = argv[1] if len(argv) > 1 else ""
    source = (ROOT / TARGET).read_text()
    tree = scratch()
    try:
        base = run_suite(tree)
        if base["killers"]:
            print("BASELINE RED:", base["killers"])
            return 1
        print("baseline green: %d tests" % base["tests"])
        bad = []
        for name, old, new in MUTANTS:
            if only not in name:
                continue
            if source.count(old) != 1:
                bad.append("%s: the replaced text occurs %d times, not once" % (name, source.count(old)))
                continue
            (tree / TARGET).write_text(source.replace(old, new))
            killers = sorted({k.rsplit(".", 1)[1] for k in run_suite(tree)["killers"]})
            verdict = "ok" if len(killers) >= 2 else "TOO FEW KILLERS"
            print("%-52s %2d killers  %s" % (name, len(killers), verdict))
            if len(killers) < 2:
                bad.append("%s: %s" % (name, killers))
        (tree / TARGET).write_text(source)
    finally:
        shutil.rmtree(tree, ignore_errors=True)
    if bad:
        print("MUTANTS_FAIL")
        print("\n".join(bad))
        return 1
    print("MUTANTS_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
