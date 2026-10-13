#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""compare_with_observer.py against a synthetic observer export built from the module's own output."""
import json, os, subprocess, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), "validate"))
import compare_with_observer as c

cap = sys.argv[1]
recs = c.read_capture(cap)
mod, _ = c.module_messages(recs)
assert len(mod) == 40, len(mod)
checks = 0
def check(cond, msg):
    global checks
    checks += 1
    if not cond:
        print("FAIL", msg); sys.exit(1)

def write_obs(msgs, path):
    with open(path, "w") as f:
        f.write(json.dumps({"kind": "header"}) + "\n")
        for d, s, b in msgs:
            f.write(json.dumps({"kind": "record", "direction": d, "queue_sequence": s, "payload_hex": b.hex()}) + "\n")

with tempfile.TemporaryDirectory() as t:
    p = os.path.join(t, "obs.jsonl")
    # the observer saw everything the module saw, plus 3 earlier messages the module attached too late for
    obs = [(0, 1000 + i, b"\0" * 80) for i in range(3)] + mod
    write_obs(obs, p)
    r = c.compare(mod, c.observer_messages(p))
    check(r["matched"] == 40 and r["module_only"] == 0 and r["byte_conflicts"] == 0 and r["observer_only_keys"] == 3, "clean")
    # one flipped byte in the observer's copy: a conflict, and the verdict must be FAIL
    bad = [(d, s, (b[:60] + bytes([b[60] ^ 1]) + b[61:]) if i == 7 else b) for i, (d, s, b) in enumerate(obs)]
    write_obs(bad, p)
    r = c.compare(mod, c.observer_messages(p))
    check(r["byte_conflicts"] == 1, "byte conflict detected")
    # the module claims a message the observer never saw
    write_obs(obs[:-1], p)
    r = c.compare(mod, c.observer_messages(p))
    check(r["module_only"] == 1, "module-only message detected")
    rc = subprocess.run([sys.executable, "-I", os.path.join(os.path.dirname(HERE), "validate", "compare_with_observer.py"), cap, p]).returncode
    check(rc == 1, "exit status 1 on module_only")
    write_obs(obs, p)
    rc = subprocess.run([sys.executable, "-I", os.path.join(os.path.dirname(HERE), "validate", "compare_with_observer.py"), cap, p], capture_output=True).returncode
    check(rc == 0, "exit status 0 when clean")
print("test_compare: %d checks passed" % checks)
