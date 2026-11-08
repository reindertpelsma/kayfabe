#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline WB1 mutation checks: each rule needs two distinct named failing tests.

Only small fixtures and mocked compilers/transports run. No downloads, Zig build,
guest, GPU, mount or upload. Mutants live in disposable private copies.
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = [
    ("ci-guard", "package_bundle.py", 'for key in ("CI", "KF_RUN_TOKEN"):', 'for key in ("KF_RUN_TOKEN",):'),
    ("run-token-guard", "package_bundle.py", 'for key in ("CI", "KF_RUN_TOKEN"):', 'for key in ("CI",):'),
    ("empty-environment-guard", "package_bundle.py", 'if key in env and not (key == "CI" and allow_ci):', 'if env.get(key) and not (key == "CI" and allow_ci):'),
    ("profile-allowlist", "package_bundle.py", 'if set(allowed) - OPEN_PACKAGES:', 'if False:'),
    ("selection-boundary", "package_bundle.py", 'if pid not in allowed or pid not in byid:', 'if pid not in byid:'),
    ("input-sha-pin", "package_bundle.py", 'if not re.fullmatch(r"[0-9a-f]{64}", fl.get("sha256") or ""):', 'if False:'),
    ("input-size-pin", "package_bundle.py", 'if type(fl.get("size")) is not int or not 0 < fl["size"] <= (1 << 50 if image else bundle_iso.MAX_FILE):', 'if False:'),
    ("input-path", "package_bundle.py", 'if len(bundle_iso.valid_path(name)) != 1:', 'if False:'),
    ("input-url", "package_bundle.py", 'if url.scheme != "https" or not url.hostname or url.username or url.password or url.fragment:', 'if False:'),
    ("dependency-cycle", "package_bundle.py", 'if pid in active:', 'if False:'),
    ("cached-sha", "package_bundle.py", 'or sha256(path) != fl["sha256"]:', ':'),
    ("range-validation", "package_bundle.py", 'if response.headers.get("Content-Range") != expected:', 'if False:'),
    ("download-sha", "package_bundle.py", 'if sha256(part) != fl["sha256"]:', 'if False:'),
    ("image-sha", "package_bundle.py", 'or sha256(iso) != receipt["image"]["sha256"]:', ':'),
    ("image-identity", "package_bundle.py", 'if not expected or receipt["image"]["identity"] != identity(expected):', 'if False:'),
    ("image-file-list", "package_bundle.py", 'if "KFAPPS.ID" not in expected or set(r.files) != set(expected):', 'if False:'),
    ("image-file-bytes", "package_bundle.py", 'if got[name] != pin:', 'if False:'),
    ("identity-marker", "package_bundle.py", 'if b"".join(r.chunks("KFAPPS.ID")) != (identity(expected) + "\\n").encode("ascii"):', 'if False:'),
    ("legacy-tree-hint", "package_bundle.py", 'mode=p["stage"]["mode"]', 'mode="tree" if p.get("image_form") == "tree" else p["stage"]["mode"]'),
    ("sorted-image-files", "bundle_iso.py", 'paths = sorted(files)', 'paths = list(files)'),
    ("fixed-recording-date", "bundle_iso.py", 'DATE7 = bytes((100, 1, 1, 0, 0, 0, 0))', 'DATE7 = bytes((126, 11, 7, 12, 30, 0, 0))'),
    ("iso-endian-twins", "bundle_iso.py", 'if little != big:', 'if False:'),
    ("iso-volume-size", "bundle_iso.py", 'if self._number(d, 80, 4) * BLOCK != self.size or self._number(d, 128, 2) != BLOCK:', 'if False:'),
    ("iso-payload-bytes", "bundle_iso.py", 'h.update(chunk); f.write(chunk)', 'h.update(chunk); f.write(bytes(len(chunk)))'),
    ("compiler-archive-pin", "bundle_tools.py", 'verify_file("zig", pin, archive)', 'pass'),
    ("compiler-archive-path", "bundle_tools.py", 'if any(p in ("", ".", "..") or ":" in p or "\\\\" in p for p in parts):', 'if False:'),
    ("compiler-pe-timestamp", "bundle_tools.py", 'if info["timestamp"] != 0:', 'if False:'),
    ("compiler-runtime-dll", "bundle_tools.py", 'if any(x in info["imports"] for x in ("libgcc_s_seh-1.dll", "libstdc++-6.dll", "libc++.dll", "libwinpthread-1.dll")):', 'if False:'),
    ("compiler-build-path", "bundle_tools.py", 'if str(output).encode() in path.read_bytes() or str(output).encode("utf-16-le") in path.read_bytes():', 'if False:'),
    ("compiler-failure", "bundle_tools.py", 'if result.returncode:', 'if False:'),
    ("compiler-source-epoch", "bundle_tools.py", 'SOURCE_DATE_EPOCH="0"', 'SOURCE_DATE_EPOCH="1"'),
    ("iso-file-bounds", "bundle_iso.py", 'if start < 19 or (start + max(1, blocks(size))) * BLOCK > self.size:', 'if False:'),
    ("iso-volume-label", "bundle_iso.py", 'if not allow_any_label and (self.label not in ("KFAPPS", "KFAPPSP") or pvd[40:72].rstrip(b" ") != self.label.encode("ascii")):', 'if False:'),
    ("guest-bundle-staging-table", "guest/kf_stage.ps1", 'if (Test-Path $bundleTable) { $tab = Read-Json $bundleTable }', 'if ($false) { $tab = Read-Json $bundleTable }'),
    ("repository-notices", "package_bundle.py", 'for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):', 'for name in ():'),
    ("compiler-notices", "bundle_tools.py", 'return dict(tools, **notices)', 'return tools'),
    ("iso-large-file-unsigned", "bundle_iso.py", 'return struct.pack("<I", n) + struct.pack(">I", n)', 'return struct.pack("<i", n) + struct.pack(">i", n)'),
]

COLLECT = '''import io,json,unittest
s=unittest.defaultTestLoader.discover('tests',pattern='test_package_bundle.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
'''


def run(root):
    # PowerShell startup/JIT can be slow on a shared low-priority dev host. This
    # bounds infrastructure time only; every mutant still needs two named kills.
    scratch = root.parents[3] / ".fixture-tmp"
    scratch.mkdir(exist_ok=True)
    env = dict(os.environ, TMPDIR=str(scratch), TEMP=str(scratch), TMP=str(scratch))
    p = subprocess.run([sys.executable, "-c", COLLECT], cwd=root, env=env, capture_output=True, text=True, timeout=240)
    if p.returncode:
        raise RuntimeError("mutation test harness failed: " + p.stderr[-500:])
    return json.loads(p.stdout.splitlines()[-1])


def fixture(parent, name):
    root = Path(parent) / name / "scripts/bench/windows/appmatrix"
    shutil.copytree(HERE, root, ignore=shutil.ignore_patterns("__pycache__"))
    # Fake compile tests copy the real probe sources; no compiler runs.
    repo = HERE.parents[3]
    import bundle_tools
    for rel in bundle_tools.SOURCES + ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md", "scripts/ci/kf_fetch.py"):
        dst = root.parents[3] / rel
        if not dst.exists():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo / rel, dst)
    return root


def one(parent, case):
    name, relative, old, new = case
    root = fixture(parent, name)
    p = root / relative
    source = p.read_text(encoding="utf-8")
    if source.count(old) != 1:
        raise RuntimeError(f"{name}: mutation anchor count {source.count(old)}")
    p.write_text(source.replace(old, new), encoding="utf-8")
    result = run(root)
    killers = sorted(set(result["killers"]))
    print(f"{name}: {len(killers)} killers", file=sys.stderr, flush=True)
    return dict(mutant=name, count=len(killers), killers=killers)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=2)
    args = ap.parse_args()
    with tempfile.TemporaryDirectory(prefix="kf-bundle-mutants-") as parent:
        baseline = run(fixture(parent, "baseline"))
        if baseline["killers"]:
            raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(pool.map(lambda case: one(parent, case), MUTANTS))
    print(json.dumps(dict(tests=baseline["tests"], cases=rows), indent=2))
    return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    sys.exit(main())
