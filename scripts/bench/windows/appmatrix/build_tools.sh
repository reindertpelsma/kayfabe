#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# build_tools.sh OUT_DIR — cross-compile the matrix's own Windows test programs with mingw-w64 (no
# Windows, no CUDA toolkit, no MSVC):
#   kf_dxprobe.exe   Direct3D 11 / 12 compute+draw probe (tools/src/kf_dxprobe.cpp)
#   kf_glgears.exe   OpenGL (WGL) gears + readback        (tools/src/kf_glgears.c)
#   cup2/cup3/cup8   the CUDA driver-API ladder           (../cuda/build_cup_win.sh)
# Outputs are build products: they are staged on the GPU host by build_appdisk.sh and NEVER committed.
# Writes OUT_DIR/tools.sha256.
set -euo pipefail
OUT=${1:?usage: build_tools.sh OUT_DIR}
HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
for c in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-g++ x86_64-w64-mingw32-dlltool; do
  command -v "$c" >/dev/null || { echo "REFUSED: missing $c (apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64)"; exit 2; }
done
mkdir -p "$OUT"
echo "START build kf_dxprobe"
x86_64-w64-mingw32-g++ -O2 -std=gnu++17 -static -static-libgcc -static-libstdc++ "$HERE/tools/src/kf_dxprobe.cpp" \
  -o "$OUT/kf_dxprobe.exe" -ld3d11 -ld3d12 -ldxgi -ldxguid
echo "EXIT build kf_dxprobe rc=$? $(stat -c %s "$OUT/kf_dxprobe.exe") bytes"
echo "START build kf_glgears"
x86_64-w64-mingw32-gcc -O2 -static "$HERE/tools/src/kf_glgears.c" -o "$OUT/kf_glgears.exe" -lopengl32 -lgdi32 -lm
echo "EXIT build kf_glgears rc=$? $(stat -c %s "$OUT/kf_glgears.exe") bytes"
bash "$HERE/../cuda/build_cup_win.sh" "$OUT"
rm -f "$OUT/libnvcuda.a"
( cd "$OUT" && sha256sum kf_dxprobe.exe kf_glgears.exe cup2.exe cup3.exe cup8.exe > tools.sha256 && cat tools.sha256 )
