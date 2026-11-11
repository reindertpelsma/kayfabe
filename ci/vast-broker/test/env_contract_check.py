"""The box side of the Worker environment contract, read from the one place that defines it.

stdin JSON {"env": {...}} and/or {"pairs": [[name, value], ...]}: for each name, report whether scripts/ci/vast_report.py SPEC has a row
for it and whether that row's validator accepts the value; with "full": true, also the ordinary
validate_env verdict (required names, deadline order). Prints JSON; never reads the network.
"""

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts/ci"))
import vast_report  # noqa: E402

data = json.load(sys.stdin)
rows = {name: check for name, _required, check in vast_report.SPEC}
env = data.get("env", {})
pairs = [*env.items(), *data.get("pairs", [])]
out = {
    "spec": sorted(rows),
    "unknown": sorted({name for name, _ in pairs if name not in rows}),
    "malformed": sorted({n for n, value in pairs if n in rows and not rows[n](value)}),
}
if data.get("full"):
    out["problems"] = vast_report.validate_env(env)
print(json.dumps(out))
