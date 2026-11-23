"""Check three Blender production lane mutations in isolated copies."""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
PROD = HERE.parent / "apps" / "blender_prod"
TEST = HERE / "test_blender_prod.py"
COLLECT = """
import io, json, unittest
suite = unittest.defaultTestLoader.loadTestsFromName("test_blender_prod")
result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps(dict(count=result.testsRun,
                      failures=[test.id() for test, _ in result.failures],
                      errors=[test.id() for test, _ in result.errors])))
"""
MUTANTS = [
    ("zero-margin-means-unlimited", "image_compare.py", "limit = min(1, noise + margin)",
     "limit = 1 if margin == 0 else min(1, noise + margin)",
     {"ImageTests.test_zero_margin_rejects_one_channel_lsb",
      "ImageTests.test_zero_margin_keeps_measured_noise_as_bound"}),
    ("ignore-oom-class-mismatch", "runner.py", "if actual != reference:",
     "if actual[0] != reference[0]:",
     {"OutcomeTests.test_cuda_and_optix_oom_disagree",
      "OutcomeTests.test_generic_oom_does_not_substitute_for_driver_oom"}),
    ("silently-drop-new-features", "blender_case.py", "for value in sorted(set(values))]",
     'for value in sorted(set(values)) if area not in {"modifier", "bake"} or value in {"CLOTH", "COMBINED"}]',
     {"Inventory.test_keeps_all_new_feature_identifiers", "Inventory.test_one_identifier_is_a_case"}),
]


def run(root, mutation=None):
    ci = root / "scripts" / "ci"
    ci.mkdir(parents=True)
    shutil.copyfile(TEST, ci / TEST.name)
    prod = root / "scripts" / "apps" / "blender_prod"
    shutil.copytree(PROD, prod, ignore=shutil.ignore_patterns("__pycache__"))
    if mutation:
        _, filename, before, after, _ = mutation
        path = prod / filename
        original = path.read_text()
        if original.count(before) != 1:
            raise SystemExit(f"expected one mutation anchor in {filename}")
        path.write_text(original.replace(before, after))
    proc = subprocess.run([sys.executable, "-B", "-c", COLLECT], cwd=ci, capture_output=True,
                          check=True, text=True, timeout=30)
    return json.loads(proc.stdout)


def main():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        baseline = run(root / "baseline")
        if baseline["count"] == 0 or baseline["failures"] or baseline["errors"]:
            raise SystemExit(f"baseline failed: {baseline}")
        for mutant in MUTANTS:
            name, _, _, _, expected = mutant
            result = run(root / name, mutant)
            killers = {test.removeprefix("test_blender_prod.") for test in result["failures"]}
            if result["count"] != baseline["count"] or result["errors"] or not expected <= killers:
                raise SystemExit(f"{name} missing named killers: {result}")
            print(f"{name}: {', '.join(sorted(expected))}")
        print(f"{baseline['count']} baseline tests, {len(MUTANTS)} mutants killed")


if __name__ == "__main__":
    main()
