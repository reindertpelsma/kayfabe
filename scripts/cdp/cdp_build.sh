#!/usr/bin/env bash
# cdp_build.sh OUTDIR — build the CDP probe and the nvdiff ioctl recorder ON THE BOX into OUTDIR.
# Both are built from this tree's sources (never copied from anywhere else); the guest receives
# the same two files the bare-metal run uses. `docs/design/V3_CDP.md`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT=${1:?outdir}; mkdir -p "$OUT"
NVCC=${NVCC:-$(command -v nvcc || echo /usr/local/cuda/bin/nvcc)}
"$NVCC" -O2 -arch=${CDP_ARCH:-sm_86} -rdc=true -o "$OUT/cdp_probe" "$HERE/cdp_probe.cu" -lcudadevrt \
  || { echo "CDP_BUILD probe FAILED"; exit 3; }
cc -shared -fPIC -O2 -w -o "$OUT/nvdiff_shim.so" "$HERE/nvdiff/nvdiff_shim.c" -ldl -lpthread \
  || { echo "CDP_BUILD shim FAILED"; exit 4; }
echo "CDP_BUILD ok probe=$(sha256sum "$OUT/cdp_probe" | cut -c1-12) shim=$(sha256sum "$OUT/nvdiff_shim.so" | cut -c1-12) src=$(sha256sum "$HERE/cdp_probe.cu" | cut -c1-12)"
