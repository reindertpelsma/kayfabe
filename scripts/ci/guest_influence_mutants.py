#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the guest-influence caps (STATUS: LIVE, 2026-11-20).

`docs/design/V3_GUEST_INFLUENCE_AUDIT.md` G-1 (GT200_DEBUGGER sessions, `crates/kf-qemu/src/chan.rs`
`admit_debugger`/`PendingDebugger`), G-2 (engine objects per twin, `admit_engine_object`) and G-3
(the host handle counter, `crates/kf-host/src/lib.rs` `next_handle`/`admit_want`). Each mutant is a
one-line change; each must be killed by at least two independently named failing tests. Every file
touched is overwritten in place and ALWAYS restored (also on an interrupt).
Run: python3 scripts/ci/guest_influence_mutants.py [mutant-name ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CHAN = "crates/kf-qemu/src/chan.rs"
HOST = "crates/kf-host/src/lib.rs"
FILES = {}
for rel in (CHAN, HOST):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()


def changed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old)
    return rel, FILES[rel].replace(old, new)


CASES = [
    ("debugger-busy-ignored", changed(CHAN, "    if live.contains_key(&key) || pending.contains_key(&key) {\n        return Err(\"DEBUGGER_HANDLE_BUSY\");", "    if false {\n        return Err(\"DEBUGGER_HANDLE_BUSY\");")),
    ("debugger-busy-ignores-pending", changed(CHAN, "    if live.contains_key(&key) || pending.contains_key(&key) {", "    if live.contains_key(&key) {")),
    ("debugger-gr-cap-removed", changed(CHAN, "    if on_obj >= DEBUGGERS_PER_GR_OBJECT {", "    if false {")),
    ("debugger-gr-cap-off-by-one", changed(CHAN, "    if on_obj >= DEBUGGERS_PER_GR_OBJECT {", "    if on_obj > DEBUGGERS_PER_GR_OBJECT {")),
    ("debugger-gr-cap-ignores-pending", changed(CHAN, "        .saturating_add(pending.values().filter(|g| **g == host_obj).count());", "        .saturating_add(0);")),
    ("debugger-vm-cap-removed", changed(CHAN, "    if live.len().saturating_add(pending.len()) >= per_vm {", "    if false {")),
    ("debugger-vm-cap-ignores-pending", changed(CHAN, "    if live.len().saturating_add(pending.len()) >= per_vm {", "    if live.len() >= per_vm {")),
    ("debugger-seat-never-released", changed(CHAN, "        if let Ok(mut m) = self.pending.lock() {\n            m.remove(&self.key);\n        }", "        let _ = &self.key;")),
    ("engine-twin-cap-removed", changed(CHAN, "    if v.objects.len().saturating_add(v.pending_objects.len()) >= ENGINE_OBJECTS_PER_TWIN {", "    if false {")),
    ("engine-twin-cap-off-by-one", changed(CHAN, "    if v.objects.len().saturating_add(v.pending_objects.len()) >= ENGINE_OBJECTS_PER_TWIN {", "    if v.objects.len().saturating_add(v.pending_objects.len()) > ENGINE_OBJECTS_PER_TWIN {")),
    ("engine-twin-cap-ignores-pending", changed(CHAN, "    if v.objects.len().saturating_add(v.pending_objects.len()) >= ENGINE_OBJECTS_PER_TWIN {", "    if v.objects.len() >= ENGINE_OBJECTS_PER_TWIN {")),
    ("mint-wraps-again", changed(HOST, "    if h != 0 {\n        *next = h.checked_add(1).unwrap_or(0);", "    if true {\n        *next = h.wrapping_add(1);")),
    ("null-want-admitted", changed(HOST, "    if want == 0 {\n        kf_arch::rate_eprintln!(\n            \"kf-host: HOST HANDLES EXHAUSTED", "    if false {\n        kf_arch::rate_eprintln!(\n            \"kf-host: HOST HANDLES EXHAUSTED")),
]

TESTS = [
    ["-p", "kf-qemu", "--lib", "--", "debugger_tests", "engine_object"],
    ["-p", "kf-host", "--lib", "--", "mint_tests"],
]


def run(overrides):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    killed, rc = set(), 0
    for args in TESTS:
        p = subprocess.run(["cargo", "test"] + args, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        if "could not compile" in out:
            raise SystemExit("the mutant does not compile:\n" + out[-800:])
        killed |= set(re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out))
        rc |= p.returncode
    return sorted(killed), rc


def main():
    try:
        base, rc = run({})
        assert not base and rc == 0, "baseline is not green: %s rc=%s" % (base, rc)
        print("baseline: green")
        weak = []
        wanted = sys.argv[1:]
        for name, (rel, text) in CASES:
            if wanted and name not in wanted:
                continue
            killers, _rc = run({rel: text})
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(FILES[rel])
            print("%-34s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
        return 1 if weak else 0
    finally:
        for rel, text in FILES.items():
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(text)


if __name__ == "__main__":
    sys.exit(main())
