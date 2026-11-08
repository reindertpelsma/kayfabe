#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free mutants: every changed estimator rule needs >=2 named test killers.

Runs only local Python in temporary directories. No installs, hardware or network.
The unmodified test suite must pass first; syntax/import failures never count as kills.
"""

import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCE = (HERE / "image_size_estimate.py").read_text()


def changed(old, new):
    if SOURCE.count(old) != 1:
        raise ValueError(f"mutation anchor is not unique: {old!r}")
    source = SOURCE.replace(old, new)
    compile(source, "image_size_estimate.py", "exec")
    return source


CASES = [
    ("missing-csv-schema", 'set(reader.fieldnames) != set(FIELDS)', 'False'),
    ("duplicate-id-allowed", 'row["id"] in seen', 'False'),
    ("nonfinite-negative-numbers", 'if not math.isfinite(value) or value < 0:', 'if False:'),
    ("partial-endpoints-allowed", 'if any(v is None for v in values) and not all(v is None for v in values):', 'if False:'),
    ("unordered-endpoints-allowed", 'if values[0] is not None and values != sorted(values):', 'if False:'),
    ("unknown-source-has-numbers", 'if source[0] is not None and (row["size_kind"] == "UNKNOWN" or row["confidence"] == "UNKNOWN"):', 'if False:'),
    ("windows-base-missing", 'BASE_USED_GB = 35.0', 'BASE_USED_GB = 0.0'),
    ("base-compression-extrapolated", 'BASE_COMPRESSED_GB = 19.09', 'BASE_COMPRESSED_GB = 35.0 * .55'),
    ("windows-base-added-on-linux", 'BASE_USED_GB if platform == "windows" else 0.0', 'BASE_USED_GB'),
    ("windows-compression-on-linux", 'BASE_COMPRESSED_GB if platform == "windows" else 0.0', 'BASE_COMPRESSED_GB'),
    ("platform-filter-removed", 'r["platform"] in (platform, "shared")', 'True'),
    ("free-filter-removed", '(set_name != "free-only" or r["free"] == "1")', 'True'),
    ("minimal-filter-removed", '(set_name != "minimal" or r["minimal"] == "1")', 'True'),
    ("cuda-double-counted", 'if row["in_base"] == "1" and platform == "windows":', 'if False:'),
    ("required-space-silently-installed", 'values[0] is None and basis == "required-space-proxy"', 'values[0] is None'),
    ("download-silently-expanded", 'row["size_kind"] == "REQUIRED SPACE":', 'row["size_kind"] in ("REQUIRED SPACE", "DOWNLOAD SIZE"):'),
    ("proxy-midpoint-uses-low", 'values = [low, low / 2 + high / 2, high]', 'values = [low, low, high]'),
    ("assets-use-app-ratio", 'ratio = app_ratio if row["compression_class"] == "app" else asset_ratio', 'ratio = app_ratio'),
    ("unknown-total-promoted", 'values if not unknown else None', 'values'),
    ("single-rate-factor-ten", 'v * 1000 / 55.6 / 60', 'v * 1000 / 5.56 / 60'),
    ("eight-rate-multiplied-again", 'v * 1000 / 223.0 / 60', 'v * 1000 / (223.0 * 8) / 60'),
    ("cheap-inbound-factor-ten", 'v * .004 for v in compressed', 'v * .04 for v in compressed'),
    ("expensive-inbound-factor-ten", 'v * .02 for v in compressed', 'v * .2 for v in compressed'),
    ("disk-archive-omitted", '(i + c + workspace_gb) * (1 + headroom)', '(i + workspace_gb) * (1 + headroom)'),
    ("disk-workspace-omitted", '(i + c + workspace_gb) * (1 + headroom)', '(i + c) * (1 + headroom)'),
    ("disk-headroom-omitted", '(i + c + workspace_gb) * (1 + headroom)', '(i + c + workspace_gb)'),
    ("disk-rounded-down", 'math.ceil(v / 25) * 25', 'math.floor(v / 25) * 25'),
    ("compression-validation-removed", 'if any(not math.isfinite(v) or not 0 < v <= 1 for v in (app_ratio, asset_ratio)):', 'if False:'),
    ("workspace-validation-removed", 'if any(not math.isfinite(v) or v < 0 for v in (workspace_gb, headroom)):', 'if False:'),
    ("empty-selection-allowed", 'if not selected:', 'if False:'),
]


def run(source):
    with tempfile.TemporaryDirectory(prefix="SZ1-image-mutant-") as directory:
        root = pathlib.Path(directory)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        (root / "docs").symlink_to(ROOT / "docs", target_is_directory=True)
        (ci / "image_size_estimate.py").write_text(source)
        (ci / "test_image_size_estimate.py").write_text((HERE / "test_image_size_estimate.py").read_text())
        result = subprocess.run([sys.executable, "-I", str(ci / "test_image_size_estimate.py")],
                                capture_output=True, text=True, timeout=60)
        output = result.stdout + result.stderr
        if "_FailedTest" in output or "SyntaxError" in output or "ImportError" in output:
            raise RuntimeError("invalid mutation run (import/syntax failure)")
        # Named failures only. A crash/error is not a successful mutant kill.
        killers = sorted(set(re.findall(r"^FAIL: (test_\S+)", output, re.M)))
        return result.returncode, killers, output


def main():
    code, _, output = run(SOURCE)
    if code:
        print("BASELINE FAIL\n" + output)
        return 2
    weak = 0
    for name, old, new in CASES:
        _, killers, _ = run(changed(old, new))
        good = len(killers) >= 2
        weak += not good
        print(f"{'KILLED' if good else 'WEAK'} {name}: {len(killers)} | {', '.join(killers)}", flush=True)
    print(f"{len(CASES) - weak}/{len(CASES)} mutants killed by >=2 named failing tests")
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
