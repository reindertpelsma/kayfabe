#!/usr/bin/env python3
"""Mutation checks of the payload instruction (S1/S3, 2026-11-15): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of payload_instruction.py, vast_payload.py, the
instruction schema or kfci/planner_assignment.py; test_payload_instruction.py runs against the copy and the failing
test names are the killers. The baseline (the unmutated copy) must be green. A mutant never runs anything on a path
outside its own temporary tree.

    python3 scripts/ci/instruction_mutants.py [--jobs N] [substring-of-mutant-name]
"""

import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "inst": "scripts/ci/payload_instruction.py",
    "payload": "scripts/ci/vast_payload.py",
    "schema": "scripts/kfci/schemas/instruction.schema.json",
    "checker": "scripts/kfci/planner_assignment.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_payload_instruction"
# the whole of scripts/ci and scripts/kfci (the payload tests import each other), the xid guard the payload imports,
# and the broker's copy of the schema (a test compares the two copies)
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def M(name, key, old, new):
    return (name, key, old, new)


MUTANTS = [
    M("schema-top-level-open", "schema", '"additionalProperties": false,\n  "$defs"', '"additionalProperties": true,\n  "$defs"'),
    M("schema-artifact-open", "schema", '"additionalProperties": false\n    }\n  }\n}', '"additionalProperties": true\n    }\n  }\n}'),
    M("schema-version-const-dropped", "schema", '"schema": {\n      "const": "kayfabe.payload-instruction/1"\n    }', '"schema": {\n      "type": "string"\n    }'),
    M("schema-artifact-count-unbounded", "schema", '"maxItems": 32', '"maxItems": 3200'),
    M("schema-tier-name-opened", "schema", '"unknown"\n      ]', '"unknown",\n        "huge"\n      ]'),
    M("schema-required-dropped", "schema", '"required": [\n    "schema",\n    "tier",', '"required": [\n    "schema",'),
    M("inst-stage-name-check-dropped", "inst", "    if len(set(i.stages)) != len(i.stages) or any(s not in RUNNABLE_EXTRA_STAGES for s in i.stages):", "    if False:"),
    M("inst-size-bound-removed", "inst", "if len(raw) > MAX_BYTES:", "if False:"),
    M("inst-duplicate-keys-allowed", "inst", "if key in out:", "if False:"),
    M("inst-schema-check-bypassed", "inst", "    if error:\n        raise InstructionError(error)\n", "    pass\n"),
    M("inst-path-check-removed", "inst", 'if a.path.startswith("/") or any(p in ("", ".", "..") for p in parts):', "if False:"),
    M("inst-duplicate-artifact-allowed", "inst", "if a.name in names:", "if False:"),
    M("inst-windows-runs-needs-windows-dropped", "inst", "if i.windows_runs == 2 and not i.windows:", "if False:"),
    M("inst-winapps-needs-windows-dropped", "inst", "if i.winapps is not None and not i.windows:", "if False:"),
    M("inst-rehearsal-needs-large-dropped", "inst", 'if i.rehearsal and i.tier != "large":', "if False:"),
    M("inst-stages-need-large-dropped", "inst", 'if i.stages and i.tier != "large":', "if False:"),
    M("inst-spec-stage-coupling-dropped", "inst", "if wanted != (i.spec is not None):", "if False:"),
    M("inst-spec-needs-windows-dropped", "inst", "if wanted and not i.windows:", "if False:"),
    M("inst-linux-apps-middle-tier-dropped", "inst", 'if i.linux_apps == "middle" and i.tier not in ("middle", "large"):', "if False:"),
    M("inst-linux-apps-large-tier-dropped", "inst", 'if i.linux_apps == "large" and i.tier != "large":', "if False:"),
    M("inst-hash-of-something-else", "inst", "return parse(raw), hashlib.sha256(raw).hexdigest()", 'return parse(raw), hashlib.sha256(b"").hexdigest()'),
    M("checker-defs-ignored", "checker", "return check(defs[reference[8:]], value, path, defs)", 'return check(SCHEMA["$defs"][reference[8:]], value, path, defs)'),
    M("payload-file-beside-env-allowed", "payload", "if any(name in environ for name in INSTRUCTION_ENV):", "if False:"),
    M("payload-unplanned-stage-selectable", "payload", "        if name not in plan:\n            raise EvidenceError(\"preflight\")", "        pass"),
    M("payload-empty-selection-allowed", "payload", "    if not chosen:\n        raise EvidenceError(\"preflight\")", "    pass"),
    M("payload-only-ignored", "payload", "(not only or n in only) and n not in skip", "n not in skip"),
    M("payload-skip-ignored", "payload", "(not only or n in only) and n not in skip", "(not only or n in only)"),
    M("payload-dry-run-runs", "payload", "if args.dry_run or args.emit_instruction:", "if False:"),
    M("payload-hash-not-recorded", "payload", 'result["instruction_sha256"] = digest', "pass"),
    M("payload-selection-not-recorded", "payload", 'result["selection"] = {"only": args.only, "skip": args.skip}', "pass"),
    M("payload-plan-omits-winapps", "payload", "        if i.winapps:\n            names.append", "        if False:\n            names.append"),
    M("payload-plan-omits-second-windows", "payload", "        if i.windows_runs == 2:\n            names.append", "        if False:\n            names.append"),
    M("payload-explicit-apps-profile-lost", "payload", '(("middle" if tier == "large" and explicit_apps else tier) if linuxapps else None)', "(tier if linuxapps else None)"),
    M("payload-env-adapter-skips-matrix-refusal", "payload", "    require_executable_release(environ)\n    environ.update(selected_drivers)", "    environ.update(selected_drivers)"),
    M("payload-dry-run-prints-no-hash", "payload", 'print("KF_PAYLOAD_INSTRUCTION sha256=" + digest, flush=True)', "pass"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root, names):
    done = subprocess.run(
        [sys.executable, "-B", "-c", COLLECT, *names],
        cwd=root / "scripts/ci",
        capture_output=True,
        text=True,
        timeout=900,
        env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"},
    )
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(old), old[:60])}
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(run_tests(root, [SUITE])["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-instruction-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}), [SUITE])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
