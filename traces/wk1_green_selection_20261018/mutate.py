#!/usr/bin/env python3
"""Run alone from the repository root; restore source after every selection mutant."""
import json
import subprocess
from pathlib import Path

source = Path("ci/vast-broker/src/offers.js")
original = source.read_text()
mutants = [
    ("default die order", "if (profile === 'check' && preferences === undefined)", "if (false)"),
    (
        "expensive representative",
        "offer.pricing.effective < selected.pricing.effective",
        "offer.pricing.effective > selected.pricing.effective",
    ),
]
for name, old, new in mutants:
    assert original.count(old) == 1, name
    try:
        source.write_text(original.replace(old, new))
        run = subprocess.run(
            [
                "nice",
                "-n19",
                "node",
                "--test",
                "ci/vast-broker/test/green-selection.test.js",
                "ci/vast-broker/workerd/green-selection.test.js",
            ],
            text=True,
            capture_output=True,
        )
        kills = sum(line.startswith("not ok ") for line in run.stdout.splitlines())
        print(json.dumps({"mutant": name, "kills": kills, "exit": run.returncode}), flush=True)
        assert run.returncode and kills >= 2, name
    finally:
        source.write_text(original)
