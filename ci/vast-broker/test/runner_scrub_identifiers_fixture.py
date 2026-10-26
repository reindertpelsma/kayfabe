"""Drive runner.scrub() over JSON-listed inputs: stdin [{"text": str, "redactions": [str]}] -> stdout [str].

KF_RUNNER_PATH overrides the runner under test (scripts/ci/merge_bar_mutants.py uses it).
"""

import importlib.util, json, os, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "runner", os.environ.get("KF_RUNNER_PATH") or HERE.parent / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
cases = json.load(sys.stdin)
json.dump([m.scrub(c["text"], redactions=c.get("redactions", ()))[0] for c in cases], sys.stdout)
