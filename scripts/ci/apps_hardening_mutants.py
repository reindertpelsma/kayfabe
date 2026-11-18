#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""STATUS: LIVE, 2026-11-12. Mutants of the apps hardening (renderer assertion, byte-exact CUDA probe, TIMEOUT verdict).

Each mutant changes one anchor that occurs EXACTLY ONCE in a disposable copy and must be killed by at least two
distinct test methods of test_apps_hardening.py. Falsifier: an anchor count != 1, a red baseline, or a mutant
with fewer than two killing methods. Never imports a harness module; reads and rewrites text only. No network.
Usage: apps_hardening_mutants.py [--only NAME]   (prints one JSON document; exit 1 on any weak mutant)
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
RC = "scripts/apps/src/renderer_check.py"
EGL = "scripts/apps/src/egl_offscreen.c"
CDP = "scripts/apps/src/cuda_drv_probe.c"
RUN = "scripts/apps/run_apps.sh"
GATE = "scripts/kfci/apps_gate.py"
REP = "scripts/ci/vast_run_report.py"
MUTANTS = [
    ("py-nvidia-not-required", RC, 'return bool(name) and "nvidia" in name.lower() and not SOFTWARE.search(name)',
     'return bool(name) and not SOFTWARE.search(name)'),
    ("py-software-marker-ignored", RC, 'return bool(name) and "nvidia" in name.lower() and not SOFTWARE.search(name)',
     'return bool(name) and "nvidia" in name.lower()'),
    ("py-empty-log-accepted", RC, '    if not text.strip():\n        return False, "empty log"\n', '    if not text.strip():\n        return True, "empty log"\n'),
    ("py-vulkan-gpu0-block-ignored", RC, 'names = [_first(r"deviceName\\s*=\\s*(.*)", block)]\n',
     'names = ["NVIDIA"]\n'),
    ("py-clinfo-first-platform-ignored", RC, 'names = [_first(r"^Platform #0: (.*)$", text), _first(r"^ `-- Device #0: (.*)$", text)]',
     'names = ["NVIDIA"]'),
    ("run-apps-renderer-check-skipped", RUN, 'case $name in egl_offscreen|vulkaninfo|vkpeak|clinfo|clpeak)', 'case $name in NO_ROW_x)'),
    ("run-apps-renderer-fail-ignored", RUN, 'v=FAIL; note="renderer: ${rout##*FAIL }";', 'note="renderer: ${rout##*FAIL }";'),
    ("c-egl-nvidia-not-required", EGL, 'int bad=!strcasestr(rend,"NVIDIA");', 'int bad=0;'),
    ("c-egl-software-marker-ignored", EGL, 'for(int i=0;soft[i];i++) if(strcasestr(rend,soft[i])) bad=1;', ''),
    ("c-egl-outside-pixel-unchecked", EGL, '&&!memcmp(out,eout,4)', ''),
    ("c-egl-inside-pixel-unchecked", EGL, '!memcmp(in,ein,4)&&', ''),
    ("c-egl-pixel-fail-continues", EGL, '        if(!okp) return 0;\n', ''),
    ("c-cuda-kernel-compare-removed", CDP, 'if (y[i] != ((x[i] * 3u + 7u) ^ i))', 'if (0)'),
    ("c-cuda-roundtrip-compare-removed", CDP, 'check("cuda_roundtrip", !memcmp(a, b, NBYTES), "8 MiB");', 'check("cuda_roundtrip", 1, "8 MiB");'),
    ("c-cuda-memset-compare-short", CDP, 'check("cuda_memset", !memcmp(a, b, 1 << 20), "");', 'check("cuda_memset", !memcmp(a, b, 1 << 18), "");'),
    ("c-cuda-221-unnamed", CDP, '    if (r == 221) strncat(buf, " JIT compiler missing", sizeof buf - strlen(buf) - 1);\n', ''),
    ("c-cuda-ctx-destroy-skipped", CDP, '    if (ctx) t4 = cuCtxDestroy_v2(ctx);\n', ''),
    ("c-cuda-teardown-result-ignored", CDP, 'check("cuda_teardown", !bad, d);', 'check("cuda_teardown", 1, d);'),
    ("c-cuda-exit-status-always-zero", CDP, '    return nok == nchecks && nchecks >= 7 ? 0 : 1;\n}', '    return 0;\n}'),
    ("c-cuda-hard-exit-hides-teardown-crash", CDP, '    fflush(stdout);\n    return nok == nchecks && nchecks >= 7 ? 0 : 1;',
     '    fflush(stdout);\n    extern void _exit(int); _exit(nok == nchecks && nchecks >= 7 ? 0 : 1);'),
    ("gate-timeout-line-never-emitted", GATE, 'if field(r".* verdict=([A-Z_]*) .*", row) != "TIMEOUT":', 'if True:'),
    ("gate-timeout-line-for-every-row", GATE, 'if field(r".* verdict=([A-Z_]*) .*", row) != "TIMEOUT":', 'if not row:'),
    ("report-timeout-cause-dropped", REP, '("[TIMEOUT quiet=%ss] " % m[2] + note).strip()', 'note'),
    ("report-timeout-line-ignored", REP, '        elif head == "APP_VERDICT":', '        elif head == "APP_VERDICT_x":'),
]
TREES = ["scripts/ci", "scripts/kfci", "scripts/apps", "scripts/bench/box", "scripts/bench/windows/appmatrix"]
COLLECT = '''import io,json,unittest
suite=unittest.defaultTestLoader.discover('.',pattern='test_apps_hardening.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))
'''


def run(root):
    env = {"PATH": os.environ["PATH"], "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([sys.executable, "-B", "-c", COLLECT], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=300, env=env)
    if p.returncode:
        raise RuntimeError("harness failure: " + p.stderr[-500:])
    return json.loads(p.stdout.splitlines()[-1])


def main(argv):
    only = argv[argv.index("--only") + 1] if "--only" in argv else None
    with tempfile.TemporaryDirectory(prefix="apphard-mut-") as tmp:
        root = Path(tmp)
        for tree in TREES:
            shutil.copytree(ROOT / tree, root / tree, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        for f in (ROOT / "scripts/bench").iterdir():
            if f.is_file():
                shutil.copyfile(f, root / "scripts/bench" / f.name)
        shutil.copytree(ROOT / "traces/apps_linux_20261107/r1", root / "traces/apps_linux_20261107/r1")
        base = run(root)
        if base["killers"]:
            print("BASELINE RED: " + str(base["killers"]))
            return 1
        out, weak = [], False
        for name, file, old, new in MUTANTS:
            if only and name != only:
                continue
            path = root / file
            src = path.read_text()
            if src.count(old) != 1:
                raise RuntimeError("%s: anchor count %d in %s" % (name, src.count(old), file))
            path.write_text(src.replace(old, new))
            try:
                res = run(root)
            finally:
                path.write_text(src)
            methods = sorted({k.split(" (", 1)[0] for k in res["killers"]})
            weak |= len(methods) < 2
            out.append(dict(mutant=name, killers=len(methods), methods=methods))
            print("%s: %d killer methods" % (name, len(methods)), file=sys.stderr, flush=True)
        print(json.dumps(dict(tests=base["tests"], cases=out), indent=1))
        return int(weak)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
