#!/bin/sh
# kfml dev helper (measured on Ubuntu 26.04, gcc 15, glibc 2.42, CUDA 12.8 redist): make nvcc usable
# without touching the system. Run after `kfml setup --venv DIR --with-nvcc`.  Usage: hostcc_gcc14.sh DIR
# 1. apt-get download (no install, no sudo state change) of gcc-14/g++-14 .debs, dpkg -x into DIR/gcc14.
#    The runner then sets NVCC_PREPEND_FLAGS="-ccbin DIR/gcc14/usr/bin/g++-14".
# 2. glibc >= 2.41 declares cospi/sinpi/rsqrt (+f) noexcept; CUDA 12.8 math_functions.h does not, so nvcc
#    fails with "exception specification is incompatible". Patch the six first declarations in
#    DIR/cuda_home/include/crt/math_functions.h (original kept as .orig). Not needed on Ubuntu 22.04/24.04.
set -eu
V="$1"; D="$V/kfml-cache/debs"; mkdir -p "$D" "$V/gcc14"; cd "$D"
apt-get download gcc-14 g++-14 cpp-14 gcc-14-base gcc-14-x86-64-linux-gnu g++-14-x86-64-linux-gnu \
  cpp-14-x86-64-linux-gnu libgcc-14-dev libstdc++-14-dev >/dev/null
sha256sum ./*.deb > "$V/gcc14-debs.sha256"
for d in ./*.deb; do dpkg -x "$d" "$V/gcc14"; done
H="$V/cuda_home/include/crt/math_functions.h"
[ -f "$H.orig" ] || cp "$H" "$H.orig"
python3 - "$H" <<'PY'
import re, sys
p = sys.argv[1]; s = open(p).read()
s, n = re.subn(r"(__device_builtin__ (?:double|float)\s+(?:cospi|cospif|sinpi|sinpif|rsqrt|rsqrtf)\((?:double|float) x\));",
               r"\1 noexcept(true);", s)
open(p, "w").write(s); print("patched", n)
PY
