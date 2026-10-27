"""Mutate a temporary copy; each guard mutation must fail two distinct tests."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = {
    "verification_removed": (
        'if offer.get("verification") != "verified":',
        "if False:",
    ),
    "verification_truthiness": (
        'if offer.get("verification") != "verified":',
        'if not offer.get("verification"):',
    ),
    "blacklist_removed": (
        'if offer.get("machine_id") in blacklisted:',
        "if False:",
    ),
    "vm_status_removed": (
        'if offer.get("is_vm_deverified") is not False:',
        "if False:",
    ),
    "missing_vm_status_accepted": (
        'if offer.get("is_vm_deverified") is not False:',
        'if offer.get("is_vm_deverified", False) is not False:',
    ),
}
DRIVER = """
import json
from pathlib import Path
import unittest
suite = unittest.defaultTestLoader.discover('.', pattern='test_offer_selector.py')
result = unittest.TextTestRunner(verbosity=0).run(suite)
Path('result.json').write_text(json.dumps({
    'run': result.testsRun,
    'killers': sorted({test.id() for test, _ in result.failures + result.errors})
}))
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    source = (HERE / "offer_selector.py").read_text()
    test = (HERE / "test_offer_selector.py").read_text()
    rows = []
    for name, (before, after) in MUTANTS.items():
        if source.count(before) != 1:
            raise ValueError("mutation site is not unique: " + name)
        with tempfile.TemporaryDirectory(prefix="sw3-mutant-") as directory:
            root = Path(directory)
            (root / "offer_selector.py").write_text(source.replace(before, after))
            (root / "test_offer_selector.py").write_text(test)
            subprocess.run(
                [sys.executable, "-c", DRIVER],
                cwd=root,
                capture_output=True,
                text=True,
                check=True,
                timeout=30,
            )
            result = json.loads((root / "result.json").read_text())
        if result["run"] != 14 or len(result["killers"]) < 2:
            raise ValueError("insufficient distinct killing tests: " + name)
        rows.append({"mutant": name, **result})
    args.output.write_text(json.dumps(rows, indent=2) + "\n")
    print(json.dumps({row["mutant"]: len(row["killers"]) for row in rows}))


if __name__ == "__main__":
    main()
