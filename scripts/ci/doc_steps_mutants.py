#!/usr/bin/env python3
"""Mutation checks of the documented-commands runner and the install_flow stage (2026-11-15): every mutant needs two named killers.

Each mutant is one exact-once text replacement in a private copy of doc_steps.py, install_flow.py, vast_payload.py or
vast_report.py; test_doc_steps.py runs against the copy and the failing test names are the killers. The baseline (the
unmutated copy) must be green. A mutant never runs anything on a path outside its own temporary tree.

    python3 scripts/ci/doc_steps_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "steps": "scripts/ci/doc_steps.py",
    "flow": "scripts/ci/install_flow.py",
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
    "budget": "scripts/ci/profile_resources.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_doc_steps"
# the whole of scripts/ci and scripts/kfci (the payload tests import each other), the docs the tests read, and the
# files the payload imports or reads (the same set instruction_mutants.py copies)
COPY_DIRS = ["scripts/ci", "scripts/kfci", "docs/howto", "scripts/lib"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/provision_box.sh",
    "scripts/bench/qemu.pin",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def M(name, key, old, new):
    return (name, key, old, new)


MUTANTS = [
    # extractor
    M("marker-detection-off", "steps", 'if re.match(r"\\s*<!--\\s*kf-", line):', "if False:"),
    M("marker-kind-exec-accepted", "steps", "(kf-run|kf-args)", "(kf-run|kf-args|kf-exec)"),
    M("marker-must-be-whole-line-dropped", "steps", 'MARKER_RE = re.compile(r"^<!-- (kf-run|kf-args): ([a-z][a-z0-9_]{0,39})((?: [a-z]+=[A-Za-z0-9]+)*) -->$")', 'MARKER_RE = re.compile(r"<!-- (kf-run|kf-args): ([a-z][a-z0-9_]{0,39})((?: [a-z]+=[A-Za-z0-9]+)*) -->")'),
    M("unknown-key-accepted", "steps", "if key not in KEYS[kind] or key in values:", "if False:"),
    M("duplicate-name-accepted", "steps", "    if name in seen:", "    if False:"),
    M("user-not-checked", "steps", "    if user not in USERS:", "    if False:"),
    M("timeout-cap-dropped", "steps", "or int(timeout) > MAX_TIMEOUT:", ":"),
    M("empty-block-accepted", "steps", "    if not text.strip():", "    if False:"),
    M("sudo-allowed-unprivileged", "steps", 'if kind == "kf-run" and user == "unpriv" and re.search', 'if False and re.search'),
    M("marker-without-fence-skipped", "steps", 'if not opening or opening.group(1) or opening.group(2)[0] != "`" or "`" in opening.group(3):', "if False:"),
    M("indented-fence-accepted", "steps", 'if not opening or opening.group(1) or', 'if not opening or'),
    M("fence-char-ignored-on-close", "steps", "closing.group(2)[0] == fence[0] and ", ""),
    M("fence-length-ignored-on-close", "steps", "len(closing.group(2)) >= fence[1] and ", ""),
    M("closing-fence-info-string-ignored", "steps", " and not closing.group(3).strip():", ":"),
    M("unterminated-fence-accepted", "steps", "    if fence is not None:\n        _refuse(doc, body_start or len(lines), \"unterminated fence\")", "    if False:\n        pass"),
    M("dangling-marker-accepted", "steps", '    if marker is not None:\n        _refuse(doc, marker[0], "marker has no fenced block")', "    if False:\n        pass"),
    M("crlf-not-normalised", "steps", '    text = text.replace("\\r\\n", "\\n")', "    pass"),
    M("bare-cr-accepted", "steps", '    if "\\r" in text:', "    if False:"),
    M("nul-accepted", "steps", '    if "\\0" in text:\n        raise DocStepError(f"{doc}: NUL byte")', "    if False:\n        pass"),
    M("doc-size-unbounded", "steps", "    if len(raw) > MAX_DOC_BYTES:", "    if False:"),
    M("hash-of-text-not-bytes", "steps", "hashlib.sha256(raw).hexdigest(), tuple(steps)", "hashlib.sha256(text.encode()).hexdigest(), tuple(steps)"),
    # substitution
    M("substitution-not-exactly-once", "steps", "text.count(sub.literal) != 1", "text.count(sub.literal) < 1"),
    M("substitution-path-unchecked", "steps", "            if not inside(prefix, path):", "            if False:"),
    M("substitution-newline-accepted", "steps", '"\\n" in sub.replacement or ', ""),
    M("inside-prefix-by-string-prefix", "steps", 'real.startswith(base.rstrip("/") + "/")', "real.startswith(base)"),
    M("inside-without-symlinks", "steps", "    real = os.path.realpath(path)", "    real = os.path.abspath(path)"),
    # runner
    M("done-marker-not-required", "steps", "        if rc == 0 and not reached:", "        if False:"),
    M("cwd-escape-allowed", "steps", "        if not inside(self.prefix, cwd):", "        if False:"),
    M("unpriv-root-allowed", "steps", "if step.user == \"unpriv\" and self.require_unpriv and", "if False and"),
    M("caller-environment-inherited", "steps", '        env = {\n            "HOME"', '        env = {**os.environ,\n            "HOME"'),
    M("path-not-carried", "steps", '        (self.state / "path").write_text((self.state / "path.new").read_text())', "        pass"),
    M("cwd-not-carried", "steps", '        (self.state / "cwd").write_text(cwd)', "        pass"),
    M("root-sudo-function-dropped", "steps", "        prologue.append('sudo() { \"$@\"; }')", "        pass"),
    M("drop-applied-to-root-steps", "steps", 'if step.user == "unpriv" and self.drop:', "if self.drop:"),
    M("failed-step-does-not-stop", "steps", '        if record["rc"]:\n            raise DocStepError(f"{step.doc}: step {step.name} failed rc={record[\'rc\']}")', "        pass"),
    M("timeout-not-applied", "steps", "                    timeout=step.timeout, check=False,", "                    timeout=None, check=False,"),
    M("pipefail-dropped", "steps", '"set -euo pipefail",', '"set -eu",'),
    M("unsafe-prefix-accepted", "steps", '        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", str(Path(prefix).resolve())):', "        if False:"),
    # install_flow
    M("attested-path-accepted", "flow", "    if attested_path:\n        raise FlowError(\"attested_not_implemented\")", "    if False:\n        pass"),
    M("unplanned-marker-allowed", "flow", "    if marked - planned - set(NOT_RUN):", "    if False:"),
    M("missing-planned-step-allowed", "flow", "    if planned - marked:", "    if False:"),
    M("small-card-keeps-8192", "flow", "    if vram_mib < 10240:", "    if False:"),
    M("drop-keeps-capabilities", "flow", '"--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs",', '"--no-new-privs",'),
    M("drop-allows-root-identity", "flow", "        if not isinstance(n, int) or n <= 0:", "        if False:"),
    M("built-binary-rev-unchecked", "flow", "if rev != sha[:8] or not doc_steps.inside(prefix, path):", "if not doc_steps.inside(prefix, path):"),
    M("built-binary-path-unchecked", "flow", "if rev != sha[:8] or not doc_steps.inside(prefix, path):", "if rev != sha[:8]:"),
    M("cup8-grade-accepts-any-bad", "flow", "bad=0 maxerr=0", "bad=\\d+ maxerr=0"),
    M("cup8-grade-accepts-indent", "flow", 'CUP8_PASS = re.compile(r"^CUP8', 'CUP8_PASS = re.compile(r"^\\s*CUP8'),
    M("foreign-prefix-deleted", "flow", "        if not (prefix / MARK).exists() and any(prefix.iterdir()):", "        if False:"),
    M("root-steps-not-recorded", "flow", '"root_steps": runner.root_steps() + list(root_setup),', '"root_steps": [],'),
    M("doc-hashes-not-recorded", "flow", '"docs": doc_hashes(docs),', '"docs": {},'),
    M("clone-url-literal-changed", "flow", 'REPO_URL = "https://github.com/reindertpelsma/kayfabe.git"', 'REPO_URL = "https://github.com/reindertpelsma/other.git"'),
    # payload stage
    M("stage-in-default-large-plan", "payload", "INSTALL_FLOW_IN_LARGE = False", "INSTALL_FLOW_IN_LARGE = True"),
    M("stage-planned-in-every-tier", "payload", 'if i.tier == "large" and (INSTALL_FLOW_IN_LARGE', 'if (INSTALL_FLOW_IN_LARGE'),
    M("stage-not-selectable-by-only", "payload", 'or "install_flow" in only):', "):"),
    M("stage-after-windows", "payload", '    if i.tier == "large" and (INSTALL_FLOW_IN_LARGE or "install_flow" in only):\n        names.append("install_flow")\n    if i.windows:', '    if i.windows:'),
    M("rows-accept-red-verdict", "payload", 'or doc.get("verdict") != "PASS" or doc.get("sha") != sha:', "or doc.get('sha') != sha:"),
    M("rows-accept-other-commit", "payload", 'or doc.get("verdict") != "PASS" or doc.get("sha") != sha:', 'or doc.get("verdict") != "PASS":'),
    M("rows-accept-nonzero-rc", "payload", 'if rc or type(doc) is not dict or doc.get("verdict")', 'if type(doc) is not dict or doc.get("verdict")'),
    M("result-record-not-kept", "payload", '    result["install_flow"] = doc\n', ""),
    M("rows-accept-duplicate-lines", "payload", "    if len(found) != 1:\n        raise EvidenceError(\"install_flow\")\n    try:\n        doc = json.loads(found[0])", "    if not found:\n        raise EvidenceError(\"install_flow\")\n    try:\n        doc = json.loads(found[-1])"),
    M("stage-time-floor-dropped", "payload", 'stage.update(min_remaining_ms=INSTALL_FLOW_MIN_REMAINING_MS, short_row="install_flow_retry_time")', 'stage.update(short_row="install_flow_retry_time")'),
    # budget guard
    M("budget-manual-flag-unchecked", "budget", "            if flags[flag] is not False\n", "            if False\n"),
    M("budget-manual-stage-skips-nothing", "budget", "        missing = stage_names(source) - stages.keys() - manual.keys()", "        missing = stage_names(source) - stages.keys()"),
    M("budget-manual-row-allowed", "budget", '"manual-only stage has a budget row: " + name for name in sorted(manual) if name in stages', '"manual-only stage has a budget row: " + name for name in sorted(manual) if False'),
    M("budget-flag-truthy-ok", "budget", "            if flags[flag] is not False\n", "            if not flags[flag] is False and flags[flag]\n"),
    # reporter
    M("report-install-flow-key-refused", "report", '            "install_flow",\n            "kfml",\n            "net_bytes",\n        }', '            "kfml",\n            "net_bytes",\n        }'),
    M("report-install-flow-unvalidated", "report", '            validate_install_flow(doc["install_flow"])', "            pass"),
    M("report-doc-hash-unchecked", "report", 're.fullmatch("[0-9a-f]{64}", v) for k, v in value["docs"].items())', "True for k, v in value['docs'].items())"),
    M("report-size-unbounded", "report", "    ok = ok and len(json.dumps(value, separators=(\",\", \":\")).encode()) <= 8192\n    if not ok:\n        raise ValueError(\"install_flow\")", "    pass\n    if not ok:\n        raise ValueError(\"install_flow\")"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        if (ROOT / rel).exists():
            shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    # the files the stage-resource table cites (profile_resources.py checks they exist)
    table = json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())["payloadStageResources"]
    for row in table.values():
        target = root / row["source"]
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / row["source"], target)
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
    with tempfile.TemporaryDirectory(prefix="kf-docsteps-mutants-") as parent:
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
