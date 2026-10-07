#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# build_cup_win.sh OUT_DIR — cross-compile the CUDA ladder (cup2, cup3, cup8) for a Windows guest,
# with no CUDA toolkit: an import library for the driver's own nvcuda.dll (nvcuda.def) and the
# ladder's minimal cuda.h (scripts/bench/cuda_min, LLP64-safe: CUdeviceptr is unsigned long long).
# The .exe files stay on the GPU host and go to the guest with `win_vm.sh scp-to`; they are build
# outputs, never committed. Graded like the Linux ladder: `CE rv=0xabcd1234`, `43`, `bad=0 maxerr=0`.
set -euo pipefail
OUT=${1:?usage: build_cup_win.sh OUT_DIR}
HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
REPO=$(cd "$HERE/../../../.." && pwd)
for c in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-dlltool; do
  command -v "$c" >/dev/null || { echo "REFUSED: missing $c (apt install gcc-mingw-w64-x86-64)"; exit 2; }
done
mkdir -p "$OUT"
x86_64-w64-mingw32-dlltool -d "$HERE/nvcuda.def" -l "$OUT/libnvcuda.a" -D nvcuda.dll
declare -A SRC=(
  [cup2]="$REPO/archive/nvkvm/tests/mode2/cup2.c"
  [cup3]="$REPO/scripts/bench/cup3.c"
  [cup8]="$REPO/scripts/bench/cup8.c"
)
for c in cup2 cup3 cup8; do
  echo "START build $c"
  x86_64-w64-mingw32-gcc -O2 -I "$REPO/scripts/bench/cuda_min" "${SRC[$c]}" -L "$OUT" -lnvcuda -o "$OUT/$c.exe"
  echo "EXIT build $c rc=$? $(stat -c %s "$OUT/$c.exe") bytes"
done
