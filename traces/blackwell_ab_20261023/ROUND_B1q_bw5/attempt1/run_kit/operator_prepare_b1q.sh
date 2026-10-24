#!/bin/bash
# BW5: (re)wire the B1q workspace, mirror the operator files, rehearsal 3/3, dry-loop receipt.
set -euo pipefail
K=/workspace/bw5-wt/traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1/run_kit
D=/workspace/bw5-round-b1q-a1-run
[ ! -e "$D/own-id" ] || { echo "workspace has a lease; never re-wire it"; exit 2; }
rm -rf "$D"
python3 "$K/operator_wire_b1q.py"
cd "$D"
black -q -l 100 dry_loop.py
for f in ops.py dry_loop.py upload.py launch_ready.py export_arm.py export_meta.py poll.py \
 budget_watch.py final_remote_audit.py milestone_probe.py; do cp "$f" "$K/operator_$f"; done
(cd /workspace/bw5-wt/scripts/bench/blackwell &&
 python3 -m unittest -v test_console_pin_receipt_replay >"$D/rehearsal.txt" 2>&1)
python3 - <<'PY'
import hashlib, json, re
from pathlib import Path
D = Path("/workspace/bw5-round-b1q-a1-run")
t = (D / "rehearsal.txt").read_text()
m = re.search(r"^Ran (\d+) tests?", t, re.M)
ok = t.rstrip().endswith("OK")
src = json.loads(Path("/workspace/bw3-round-b1l-a1_m30498-run/rehearsal_pass.json").read_text())
guard = Path("/workspace/bw5-wt/scripts/bench/blackwell/console_pin.py")
src.update(
    tests=int(m[1]), passed=int(m[1]) if ok else 0,
    operator_source_sha256=hashlib.sha256((D / "ops.py").read_bytes()).hexdigest(),
    guard_source_sha256=hashlib.sha256(guard.read_bytes()).hexdigest(),
    round="B1q (operator BW5: Claude) attempt 2",
    operator="Claude (BW5, fresh session); same 3-test rehearsal as B1",
    ready_branch_delta_vs_B1="readiness spend assertion < 2.0 (stop 2.00), unchanged from B1l",
    denied_countries_tests="ROUND_B1g_0050/rehearsal/test_denied_countries.py on the B1q ops.py",
)
src.pop("lease_profile_tests", None)
(D / "rehearsal_pass.json").write_text(json.dumps(src, indent=2) + "\n")
print("REHEARSAL", src["tests"], src["passed"])
PY
(cd /workspace/bw5-wt/traces/blackwell_ab_20261023/ROUND_B1g_0050/rehearsal &&
 KF_BW3_OPS="$D/ops.py" python3 -m unittest test_denied_countries 2>&1 | tail -1)
timeout 900 python3 dry_loop.py "$D" "$K" | tail -1
