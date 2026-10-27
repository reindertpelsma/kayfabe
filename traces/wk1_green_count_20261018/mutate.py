#!/usr/bin/env python3
"""Run alone from repository root; require two independent kills per count-ceiling mutant."""
import json
import subprocess
from pathlib import Path

source = Path("ci/vast-broker/src/policy.js")
original = source.read_text()
old = "dailyCount: number('DAILY_RENTALS', DEFAULTS.dailyCount, 40)"
assert original.count(old) == 1
for ceiling in [20, 400]:
    try:
        source.write_text(original.replace(old, old.replace(", 40)", ", " + str(ceiling) + ")")))
        run = subprocess.run(
            [
                "nice",
                "-n19",
                "node",
                "--test",
                "ci/vast-broker/test/green-count.test.js",
                "ci/vast-broker/workerd/green-count.test.js",
            ],
            text=True,
            capture_output=True,
        )
        kills = sum(line.startswith("not ok ") for line in run.stdout.splitlines())
        print(json.dumps({"ceiling": ceiling, "kills": kills, "exit": run.returncode}), flush=True)
        assert run.returncode and kills >= 2
    finally:
        source.write_text(original)
