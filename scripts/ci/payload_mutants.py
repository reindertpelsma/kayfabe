#!/usr/bin/env python3
"""Isolated, GPU-free mutation checks for the thin payload adapter."""

import ast
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / "vast_payload.py").read_text()
HOOK = (HERE / "vast_payload_hooks.sh").read_text()


def function(name, body):
    nodes = [n for n in ast.parse(SOURCE).body if isinstance(n, ast.FunctionDef) and n.name == name]
    assert len(nodes) == 1, name
    node = nodes[0]
    lines = SOURCE.splitlines(keepends=True)
    header = "".join(lines[node.lineno - 1 : node.body[0].lineno - 1])
    return (
        "".join(lines[: node.lineno - 1]) + header + body + "\n" + "".join(lines[node.end_lineno :])
    )


def replace(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


def scoped(name, old, new):
    node = next(
        n for n in ast.parse(SOURCE).body if isinstance(n, ast.FunctionDef) and n.name == name
    )
    lines = SOURCE.splitlines(keepends=True)
    value = "".join(lines[node.lineno - 1 : node.end_lineno])
    assert old in value
    value = value.replace(old, new)
    return "".join(lines[: node.lineno - 1]) + value + "".join(lines[node.end_lineno :])


CASES = [
    (
        "failed-provision-hardware-stale",
        replace('            result["hardware"] = hardware()', "            pass"),
    ),
    (
        "hardware-refreshed-only-after-success",
        replace(
            '            result["hardware"] = hardware()\n'
            '            rows = provision_rows(bounded_text("/root/prov/prov.log"), rc)',
            '            rows = provision_rows(bounded_text("/root/prov/prov.log"), rc)\n'
            '            result["hardware"] = hardware()',
        ),
    ),
    (
        "existing-client-target-redirected",
        replace(
            '            CARGO_TARGET_DIR=str(root / "target"),',
            '            CARGO_TARGET_DIR="/workspace/bench/kf-verify-target",',
        ),
    ),
    (
        "existing-client-target-nested",
        replace(
            '            CARGO_TARGET_DIR=str(root / "target"),',
            '            CARGO_TARGET_DIR=str(root / "target" / "misplaced"),',
        ),
    ),
    ("provision-exit-only", function("provision_rows", '    return [row("provision", 1)]')),
    ("merge-evidence-bypass", function("merge_rows", '    return [row("merge_bar", 1)]')),
    ("ladder-exit-only", function("ladder_rows", '    return [row("cuda_host", 4)]')),
    ("red-record-ignored", function("red_line", "    return False")),
    ("deadline-status-forged", scoped("run_bounded", "return 124", "return 0")),
    (
        "finished-prefix-lost",
        scoped(
            "execute_stages", '            result["suites"].extend(error.rows)', "            pass"
        ),
    ),
    (
        "first-red-continues",
        scoped("execute_stages", "            return 1", "            continue"),
    ),
    ("result-never-published", replace("    temporary.replace(destination)", "    pass")),
    ("approved-source-unpinned", function("prepare_checkout", '    return "kf-ci-approved"')),
    ("xid-unconditionally-admitted", function("xid_ok", "    return True")),
    (
        "hardware-unvalidated",
        function(
            "hardware",
            '    return {"gpu_name":"RTX 3060","compute_cap":860,"driver_version":"580.159.04"}',
        ),
    ),
    ("failed-stage-attribution-lost", replace("    if rc not in (0, 6):", "    if rc == 124:")),
    (
        "unattributed-stage-xid-admitted",
        replace(
            "        if collections.Counter(delta) != collections.Counter(allowed):",
            "        if False:",
        ),
    ),
    (
        "duplicate-cuda-rung-admitted",
        replace("        if not match or match[1] in seen:", "        if not match:"),
    ),
    (
        "merge-exit-marker-ignored",
        replace(
            '    if rc or red_line(text) or len(re.findall(r"^EXIT rc=0 .+$", text, re.M)) != 1:',
            "    if rc or red_line(text):",
        ),
    ),
    ("wrong-control-va", SOURCE.replace("0xA11000000", "0xA12000000")),
    ("unreadable-watch-admitted", function("watch_delta", "    return []")),
    (
        "other-page-type-admitted",
        replace(r're.search(r"\bFAULT_PDE\b", line)', r're.search(r"\bFAULT_(?:PDE|PTE)\b", line)'),
    ),
    ("non-ce-client-admitted", replace("            and client\n", "")),
    ("wrong-arm-admitted", replace('arm != "--missing-page-fault" or ', "")),
    ("xid-refusal-status-forged", replace("    return 0 if ok else 6", "    return 0")),
    (
        "arm-watch-not-installed",
        replace(
            '            BASH_ENV=str(root / "scripts/ci/vast_payload_hooks.sh"),',
            '            BASH_ENV="/nonexistent-v7-hook",',
        ),
    ),
]

COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_payload*.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print('KF_MUTATION_RESULT='+json.dumps({'tests':r.testsRun,
'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(root):
    p = subprocess.run(
        [sys.executable, "-c", COLLECT], cwd=root, text=True, capture_output=True, timeout=45
    )
    assert p.returncode == 0, p.stderr[-500:]
    line = next(s for s in p.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT="))
    return json.loads(line.split("=", 1)[1])


def main():
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-v7-mutants-") as directory:
        root = Path(directory)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        for name in [
            "vast_payload.py",
            "vast_payload.sh",
            "vast_payload_hooks.sh",
            "test_vast_payload.py",
            "test_vast_payload_v7.py",
            "vast_report.py",
        ]:
            shutil.copyfile(HERE / name, ci / name)
        bench = root / "scripts/bench"
        bench.mkdir()
        shutil.copyfile(HERE.parent / "bench/host_xid_watch.sh", bench / "host_xid_watch.sh")
        baseline = run(root)
        assert not baseline["killers"], baseline
        for name, source in CASES:
            assert source != SOURCE, name
            ast.parse(source)
            (ci / "vast_payload.py").write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print(name + ": " + str(len(killers)) + " killers", file=sys.stderr, flush=True)
        (ci / "vast_payload.py").write_text(SOURCE)
        (ci / "vast_payload_hooks.sh").write_text(
            '#!/bin/bash\ntimeout() { command timeout "$@"; }\n'
        )
        result = run(root)
        killers = sorted(set(result["killers"]))
        assert len(killers) >= 2, ("arm-hook-bypassed", killers)
        rows.append({"mutant": "arm-hook-bypassed", "killers": killers, "count": len(killers)})
    print(json.dumps({"run": "v7-payload-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
