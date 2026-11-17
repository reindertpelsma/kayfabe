"""Record the first native identity; the second independent probe must bind to it."""

import csv
import json
from pathlib import Path
import re

root = Path("/root/sw4")
evidence = root / "evidence"
probe = (evidence / "die_probe.log").read_text()
match = re.search(r"^host_driver=([0-9.]+) arch=\(([0-9]+), ([0-9]+), ([0-9]+)\)$", probe, re.M)
if match is None:
    raise ValueError("initial native identity incomplete")
rows = list(csv.reader((evidence / "binding.csv").read_text().splitlines(), skipinitialspace=True))
if len(rows) != 1 or len(rows[0]) != 3:
    raise ValueError("one adapter required")
plan = json.loads((root / "sshproof.json").read_text())
plan.update(driver=match[1], pci=rows[0][2].strip(), arch_impl=[int(match[2]), int(match[3])])
(evidence / "binding_plan.json").write_text(json.dumps(plan, indent=2) + "\n")
