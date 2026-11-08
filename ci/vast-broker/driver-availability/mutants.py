#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Each mutant must fail at least two distinct, named offline fixture tests."""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
JS = (HERE / "validate.js").read_text()
PY = (HERE / "generate.py").read_text()
INPUT_HASHES = {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                for name in ("validate.js", "generate.py", "selector.test.js", "test_generate.py", "sources.json")}


def changed(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"mutant anchor not unique: {old}")
    return source.replace(old, new)


CASES = [
    ("js", "unpinned-admitted", changed(JS, "d.status === 'PINNED' && d.probe.status === 200", "true && d.probe.status === 200")),
    ("js", "known-bad-admitted", changed(JS, "return t.known_bad.some(", "return false && t.known_bad.some(")),
    ("js", "duplicates-admitted", changed(JS, "&& !share(slot, owner, d, t)", "&& false")),
    ("js", "ownership-reset-between-branches", changed(JS, "for (const branch of branches) {\n    const slots = [];", "for (const branch of branches) {\n    assigned.clear();\n    const slots = [];")),
    ("js", "core-crosses-family", changed(JS, "a.box.family === b.box.family && a.os === b.os", "true && a.os === b.os")),
    ("js", "ordinary-role-core-sharing", changed(JS, "a.box.role === 'core-reference' && b.box.role === 'core-reference'", "true")),
    ("js", "undesignated-core-sharing", changed(JS, "t.core_reference_versions.some(r => r.family === a.box.family && r.os === d.os\n      && exactVersionKey(r.version) === exactVersionKey(d.version))", "true")),
    ("js", "same-box-core-duplicates", changed(JS, "a.box.id !== b.box.id &&", "true &&")),
    ("js", "newest-first", changed(JS, "compare(a.last_tested ?? '', b.last_tested ?? '')", "compare(b.last_tested ?? '', a.last_tested ?? '')")),
    ("js", "seed-ignored", changed(JS, "seed = String(seed);", "seed = 'fixed';")),
    ("js", "blackwell-floor-ignored", changed(JS, "branch.id < t.families[b.family].sweep_floor", "false")),
    ("js", "branch-pruned", changed(JS, "[...t.branches].sort((a, b) => a.id - b.id)", "t.branches.filter(b => b.id !== 590).sort((a, b) => a.id - b.id)")),
    ("js", "version-not-normalized", changed(JS, "return [parts[0], parts[1], parts[2] ?? 0].join('.');", "return version;")),
    ("js", "input-mutated", changed(JS, "[...boxes].sort", "boxes.sort")),
    ("js", "package-lines-mixed", changed(JS, "(os !== 'windows' || (d.product_line ?? 'geforce') === (b.product_line ?? 'geforce'))", "true")),
    ("js", "malformed-sha-admitted", changed(JS, "d.sha256 === null || /^[0-9a-f]{64}$/.test(d.sha256)", "true")),
    ("js", "pin-size-not-required", changed(JS, "d.size !== null\n        &&", "true\n        &&")),
    ("js", "installer-origin-not-checked", changed(JS, "insist(official(d.url, true),", "insist(true,")),
    ("js", "redirect-origin-not-checked", changed(JS, "d.probe.final_url === null || official(d.probe.final_url, true)", "true")),
    ("js", "head-method-not-checked", changed(JS, "d.probe.method === 'HEAD'", "true")),
    ("js", "pin-can-have-http-error", changed(JS, "d.probe.status === 200 && d.probe.final_url !== null", "true && d.probe.final_url !== null")),
    ("js", "known-good-proof-not-required", changed(JS, "insist(b.test_records.some(r => r.result === 'pass'),", "insist(true,")),
    ("js", "abi-can-prune-sweep", changed(JS, "t.abi_boundary_hypothesis.used_for_selection === false", "true")),
    ("js", "future-test-date-admitted", changed(JS, "d.last_tested <= t.generated_at.slice(0, 10)", "true")),
    ("js", "windows-branch-evidence-ignored", changed(JS, "d.branch_evidence.branch === d.branch", "true")),
    ("js", "notebook-installer-admitted", changed(JS, "&& path.includes('-desktop-')", "")),
    ("py", "checksum-filename-ignored", changed(PY, 'if m and m[2] in (filename, "./" + filename):', 'if m:')),
    ("py", "checksum-conflicts-ignored", changed(PY, 'if len(found) > 1:', 'if False:')),
    ("py", "errors-treated-as-absent", changed(PY, '"UNAVAILABLE" if head["status"] == 404 else "UNKNOWN:probe-error"', '"UNAVAILABLE"')),
    ("py", "installer-get-instead-of-head", changed(PY, 'head = client.request(url, "HEAD")', 'head = client.request(url, "GET")')),
    ("py", "missing-checksum-promoted", changed(PY, 'else "UNPINNED:no-sha256"', 'else "PINNED"')),
    ("py", "download-size-mismatch-admitted", changed(changed(PY, 'if size > expected_size:', 'if False:'), 'if size != expected_size:', 'if False:')),
    ("py", "download-disk-guard-removed", changed(PY, 'if shutil.disk_usage("/")[2] - expected_size < 6 * GIB:', 'if False:')),
    ("py", "published-tesla-host-rewritten", changed(PY, 'urls.append(u)', 'urls.append(u.replace("us.download.nvidia.com", "download.nvidia.com"))').replace('urls.append(historical[version])', 'urls.append(historical[version].replace("us.download.nvidia.com", "download.nvidia.com"))')),
    ("py", "search-query-flags-inverted", changed(PY, '"isWHQL": 0 if studio else 1', '"isWHQL": 1 if studio else 0')),
    ("py", "api-whql-flag-ignored", changed(PY, 'or d.get("IsWHQL") != "1"', 'or False')),
    ("py", "url-userinfo-admitted", changed(PY, 'not u.username and not u.password', 'True')),
    ("py", "download-size-guard-removed", changed(PY, 'if not 0 < expected_size <= 15 * GIB:', 'if False:')),
    ("py", "redirect-head-becomes-get", changed(PY, 'return super().redirect_request(req, fp, code, msg, headers, newurl)', 'redirected = super().redirect_request(req, fp, code, msg, headers, newurl)\n        redirected.method = "GET"\n        return redirected')),
]


def run(kind, name, source):
    with tempfile.TemporaryDirectory(prefix="DRV1-mutant-") as tmp:
        root = Path(tmp)
        module = root / "ci/vast-broker/driver-availability"
        module.mkdir(parents=True)
        (root / "ci/vast-broker/package.json").write_text('{"type":"module"}\n')
        for top in ("docs", "traces", "tools"):
            (root / top).symlink_to(ROOT / top, target_is_directory=True)
        target = "validate.js" if kind == "js" else "generate.py"
        for filename in ("selector.test.js", "validate.js", "generate.py", "test_generate.py", "sources.json"):
            if filename == target:
                (module / filename).write_text(source)
            else:
                shutil.copyfile(HERE / filename, module / filename)
        env = {**os.environ, "DRV1_FIXTURES_ONLY": "1", "PYTHONDONTWRITEBYTECODE": "1"}
        cmd = ["node", "--test", str(module / "selector.test.js")] if kind == "js" else [sys.executable, str(module / "test_generate.py")]
        result = subprocess.run(cmd, cwd=root, capture_output=True, text=True, env=env, timeout=180)
        output = result.stdout + result.stderr
        pattern = r"^not ok \d+ - (.+)$" if kind == "js" else r"^(?:FAIL|ERROR): (.+)$"
        killers = sorted(set(re.findall(pattern, output, flags=re.M)))
        return {"name": name, "language": kind, "exit_code": result.returncode, "killers": killers, "killer_count": len(killers)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    for kind, source in (("js", JS), ("py", PY)):
        baseline = run(kind, "baseline", source)
        if baseline["exit_code"] != 0:
            print(json.dumps(baseline))
            return 2
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for row in pool.map(lambda case: run(*case), CASES):
            rows.append(row)
            print(f"{'ok' if row['killer_count'] >= 2 else 'WEAK'} {row['name']}: {row['killer_count']} killers", flush=True)
    if args.output:
        args.output.write_text(json.dumps({"rule": "every mutant >=2 distinct named killers", "input_sha256": INPUT_HASHES, "mutants": rows}, indent=2) + "\n")
    if any(hashlib.sha256((HERE / name).read_bytes()).hexdigest() != digest for name, digest in INPUT_HASHES.items()):
        print("inputs changed during mutation run; discard these results")
        return 2
    weak = [row for row in rows if row["killer_count"] < 2]
    print(f"{len(rows) - len(weak)}/{len(rows)} mutants killed by >=2 tests")
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
