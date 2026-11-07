#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free release definition mutations. Every mutant needs two distinct named failing tests.

Only synthetic files, mocked runners and Node VM checks are used; no GPU, browser, VM, or network.
Each mutant runs in a private copy. The ordinary test baseline must pass before any mutation runs.
"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ORACLE = "guest/py/release_oracles.py"
MUTANTS = [
    ("image-size-ignored", ORACLE, 'if width < 2 or height < 2 or len(actual) != count or len(reference) != count:', 'if False:'),
    ("image-finite-ignored", ORACLE, 'if not all(math.isfinite(x) and 0 <= x <= 1 for x in actual + reference):', 'if False:'),
    ("image-blank-accepted", ORACLE, 'if min(span_actual, span_ref) < .05:', 'if False:'),
    ("image-errors-ignored", ORACLE, 'if mae > IMAGE_MAE or worst > IMAGE_MAX_ERROR:', 'if False:'),
    ("image-mean-limit-ignored", ORACLE, 'if mae > IMAGE_MAE or worst > IMAGE_MAX_ERROR:', 'if worst > IMAGE_MAX_ERROR:'),
    ("image-max-limit-ignored", ORACLE, 'if mae > IMAGE_MAE or worst > IMAGE_MAX_ERROR:', 'if mae > IMAGE_MAE:'),
    ("text-parity-ignored", ORACLE, 'if min(len(actual), len(reference)) <= 20 or actual != reference:', 'if False:'),
    ("reference-identity-ignored", ORACLE, 'for key, value in expected.items():', 'for key, value in {}.items():'),
    ("reference-provenance-ignored", ORACLE,
     'if (not isinstance(doc, dict) or doc.get("schema") != 1 or doc.get("kind") != "bare-metal" or not raw\n'
     '            or not re.fullmatch(r"[0-9a-f]{40}", str(doc.get("revision", "")))\n'
     '            or not re.fullmatch(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}", str(doc.get("date", "")))\n'
     '            or not isinstance(doc.get("evidence"), str) or not doc["evidence"].strip()\n'
     '            or doc.get("sha256") != hashlib.sha256(raw).hexdigest()):', 'if False:'),
    ("cuda-offload-ignored", ORACLE,
     'if rc != 0 or not offload or offload[1] != offload[2] or not re.search(r"(?:Device 0:|CUDA0:).*NVIDIA", log):', 'if False:'),
    ("llama-runner-skips-comparison", "guest/py/llama_release.py",
     'digest = compare_text(p.stdout.decode("utf-8-sig"), raw.decode("utf-8-sig"))', 'digest = "0" * 64'),
    ("llama-runner-skips-reference", "guest/py/llama_release.py",
     'from release_oracles import check_llm_run, check_reference, compare_text',
     'from release_oracles import check_llm_run, compare_text\ncheck_reference = lambda *args: None'),
    ("blender-runner-skips-comparison", "guest/py/blender_release.py",
     'stats = compare_pixels(list(actual.pixels), list(reference.pixels), 960, 540)', 'stats = {"mae": 0, "max_error": 0}'),
    ("blender-runner-skips-reference", "guest/py/blender_release.py",
     'from release_oracles import check_reference, compare_pixels',
     'from release_oracles import compare_pixels\ncheck_reference = lambda *args: None'),
    ("bundle-source-pin-ignored", "release_bundle.py",
     'if (receipt.get("schema") != 1 or receipt.get("source_commit") != source["commit"]\n'
     '            or receipt.get("source_repository") != source["repository"]\n'
     '            or receipt.get("built_projects") != source["projects"]\n'
     '            or receipt.get("patches") != source["patches"]):', 'if False:'),
    ("bundle-required-files-ignored", "release_bundle.py",
     'if not isinstance(files, dict) or not files or not set(source["required_files"]) <= set(files):', 'if False:'),
    ("bundle-hashes-ignored", "release_bundle.py", 'if hashlib.file_digest(f, "sha256").hexdigest() != expected:', 'if False:'),
    ("bundle-file-list-ignored", "release_bundle.py", 'if seen != set(files) | {"receipt.json"}:', 'if False:'),
]
for api in ("webgl", "webgpu"):
    path = "guest/web/" + api + "_conformance.html"
    MUTANTS += [
        (api + "-all-results-pass", path, 'return checks.length === 4 && checks.every(x => x === true) && gpu === true && frames >= 8;', 'return true;'),
        (api + "-failed-check-ignored", path, 'checks.every(x => x === true)', 'true'),
        (api + "-frame-count-ignored", path, 'frames >= 8', 'true'),
    ]
MUTANTS += [
    ("webgpu-unknown-fallback-accepted", "guest/web/webgpu_conformance.html", 'fallback === false', 'fallback !== true'),
    ("webgpu-other-vendor-accepted", "guest/web/webgpu_conformance.html", "String(info.vendor).toLowerCase() === 'nvidia'", 'true'),
]
VRS_BODY = '''+    // kayfabe release gate: require and exercise coarse shading; no unsupported fallback.
+    if (m_shadingRateTier < D3D12_VARIABLE_SHADING_RATE_TIER_1)
+    {
+        ThrowIfFailed(E_NOTIMPL);
+    }
+    m_refractionShadingRate = D3D12_SHADING_RATE_2X2;
+    m_sceneShadingRate = D3D12_SHADING_RATE_2X2;
+    m_postprocessShadingRate = D3D12_SHADING_RATE_2X2;
+
'''
MUTANTS.append(("vrs-coarse-gate-disabled", "directx_vrs_gate.patch", VRS_BODY, "+    // Mutant: no feature guard or coarse shading.\n"))
MUTANTS += [
    ("qga-read-replayed-without-seek", "qga.py",
     'self.call("guest-file-read", {"handle": handle, "count": count}, retry=False)',
     'self.call("guest-file-read", {"handle": handle, "count": count}, retry=True)'),
    ("qga-reconnect-loses-read-offset", "qga.py",
     'self.call("guest-file-seek", {"handle": handle, "offset": offset, "whence": 0})',
     'if attempt == 0: self.call("guest-file-seek", {"handle": handle, "offset": offset, "whence": 0})'),
    ("qga-timeout-or-refusal-replayed", "qga.py",
     'raise  # preserve the existing timeout/refusal budget', 'pass'),
]

COLLECT = '''import io,json,unittest
s=unittest.defaultTestLoader.discover('tests',pattern='test_release*.py')
s.addTests(unittest.defaultTestLoader.discover('tests',pattern='test_conformance_pages.py'))
s.addTests(unittest.defaultTestLoader.discover('tests',pattern='test_qga_retry.py'))
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
'''


def run(root):
    p = subprocess.run([sys.executable, "-c", COLLECT], cwd=root, capture_output=True, text=True, timeout=120)
    if p.returncode:
        raise RuntimeError("test harness failed: " + p.stderr[-500:])
    return json.loads(p.stdout.splitlines()[-1])


def fixture(parent, name):
    root = Path(parent) / name / "scripts/bench/windows/appmatrix"
    shutil.copytree(HERE, root, ignore=shutil.ignore_patterns("__pycache__"))
    return root


def one(parent, case):
    name, relative, old, new = case
    root = fixture(parent, name)
    p = root / relative
    source = p.read_text()
    if source.count(old) != 1:
        raise RuntimeError(name + ": mutation anchor count " + str(source.count(old)))
    p.write_text(source.replace(old, new))
    if name == "vrs-coarse-gate-disabled":
        # Keep the patch well-formed and re-pin its bytes so the C++ behavior tests must kill it.
        p.write_text(p.read_text().replace("@@ -45,6 +45,15 @@", "@@ -45,6 +45,7 @@"))
        cfg_path = root / "release_software.json"
        cfg = json.loads(cfg_path.read_text())
        cfg["directx"]["patches"]["vrs_gate"] = hashlib.sha256(p.read_bytes()).hexdigest()
        cfg_path.write_text(json.dumps(cfg))
    result = run(root)
    killers = sorted(set(result["killers"]))
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=2)
    args = ap.parse_args()
    with tempfile.TemporaryDirectory(prefix="kf-win-release-mutants-") as parent:
        baseline = run(fixture(parent, "baseline"))
        if baseline["killers"]:
            raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(pool.map(lambda case: one(parent, case), MUTANTS))
    print(json.dumps({"tests": baseline["tests"], "cases": rows}, indent=2))
    return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    sys.exit(main())
