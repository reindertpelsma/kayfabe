#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
probe_output=${KFGT_PROBE_OUTPUT:?Set KFGT_PROBE_OUTPUT to a trusted output directory outside the repository}
mkdir -p "$probe_output"
x86_64-w64-mingw32-gcc -std=c11 -O2 -Wall -Wextra -Werror -D_WIN32_WINNT=0x0a00 \
    -Wl,--dynamicbase,--nxcompat -o "$probe_output/d3d11_probe.exe" \
    tests/d3d11_probe.c -ld3d11 -ldxgi
python3 - "$probe_output" <<'PY'
import hashlib, json, pathlib, subprocess, sys
out=pathlib.Path(sys.argv[1])
metadata=dict(source_revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
              source_sha256=hashlib.sha256(pathlib.Path('tests/d3d11_probe.c').read_bytes()).hexdigest(),
              compiler=subprocess.check_output(['x86_64-w64-mingw32-gcc','--version'],text=True).splitlines()[0],
              exe_sha256=hashlib.sha256((out/'d3d11_probe.exe').read_bytes()).hexdigest(),
              runtime_tested=False)
(out/'build-info.json').write_text(json.dumps(metadata,indent=2)+'\n')
print(json.dumps(metadata,indent=2))
PY
