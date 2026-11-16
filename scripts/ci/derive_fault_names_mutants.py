#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/bench/derive_native_fault_names.py (STATUS: LIVE, 2026-11-16).

Each mutant is a one-line change of the script; each must be killed by at least two independently named
failing tests of scripts/ci/test_derive_native_fault_names.py (the fixture and committed-JSON layers; the
oracle layer runs too when third_party/ogkm-580 is fetched, and may add killers, never the only ones).
Run: python3 scripts/ci/derive_fault_names_mutants.py   (exit 0 only if every mutant has >= 2 killers)
"""

import io
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "bench", "derive_native_fault_names.py")
SOURCE = open(SCRIPT).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "turing-engines-dropped",
        changed("return engines | turing_engines, clients | turing_clients", "return engines, clients | turing_clients"),
    ),
    (
        "turing-client-dropped",
        changed("return engines | turing_engines, clients | turing_clients", "return engines | turing_engines, clients"),
    ),
    (
        "hand-added-engine",
        changed(
            "return engines | turing_engines, clients | turing_clients",
            'return engines | turing_engines | {"HOST3"}, clients | turing_clients',
        ),
    ),
    (
        "hand-added-client",
        changed(
            "return engines | turing_engines, clients | turing_clients",
            'return engines | turing_engines, clients | turing_clients | {"HUBCLIENT_PMU"}',
        ),
    ),
    (
        "pbdma-names-unchecked",
        changed('assert names == ["HOST%d" % i for i in range(int(count.group(1)))], "Turing PBDMA names changed"', "pass"),
    ),
    (
        "host-cpu-printer-and-header-unchecked",
        changed('assert len(printed) == len(defined) == 1, "Turing HOST_CPU printer and header disagree"', "pass"),
    ),
    (
        "esc-printer-and-header-unchecked",
        changed('assert clients and clients == header_clients, "ESC definitions and printer disagree"', "pass"),
    ),
    (
        "hash-of-sources-dropped",
        changed('"sha256": {p: hashlib.sha256(sources[p]).hexdigest() for p in FILES}', '"sha256": {}'),
    ),
]


def run(name, source, jobs_dir):
    # The script finds third_party/ogkm-580 from its own location: mirror that layout under jobs_dir.
    bench = os.path.join(jobs_dir, name, "scripts", "bench")
    os.makedirs(bench)
    os.symlink(os.path.join(HERE, "..", "..", "third_party"), os.path.join(jobs_dir, name, "third_party"))
    path = os.path.join(bench, "derive_native_fault_names.py")
    with open(path, "w") as f:
        f.write(source)
    env = dict(os.environ, KF_DERIVE_FAULT_NAMES_SCRIPT=path)
    code = (
        "import io,json,sys,unittest\n"
        "sys.path.insert(0,%r)\n"
        "import test_derive_native_fault_names as t\n"
        "s=unittest.defaultTestLoader.loadTestsFromModule(t)\n"
        "r=unittest.TextTestRunner(stream=io.StringIO()).run(s)\n"
        "print('KF_MUTATION_RESULT='+json.dumps({'tests':r.testsRun,"
        "'killers':[x.id() for x,_ in r.failures+r.errors]}))\n"
    ) % HERE
    p = subprocess.run([sys.executable, "-c", code], env=env, text=True, capture_output=True, timeout=300)
    line = next((s for s in p.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    assert line, (name, p.stdout[-300:], p.stderr[-500:])
    return json.loads(line.split("=", 1)[1])


def main():
    rows, weak = [], []
    with tempfile.TemporaryDirectory(prefix="kf-derive-mutants-") as directory:
        base = run("baseline", SOURCE, directory)
        assert not base["killers"], base
        print("baseline: %d tests, no killers" % base["tests"], file=sys.stderr)
        for name, source in CASES:
            if len(sys.argv) > 1 and sys.argv[1] not in name:
                continue
            assert source != SOURCE, name
            killers = sorted(set(run(name, source, directory)["killers"]))
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print("%s: %d killers" % (name, len(killers)), file=sys.stderr)
            if len(killers) < 2:
                weak.append(name)
    print(json.dumps({"run": "derive-fault-names-mutants-20261116", "cases": rows}, indent=2))
    if weak:
        raise SystemExit("weak mutants (fewer than two killers): " + ", ".join(weak))


if __name__ == "__main__":
    main()
