#!/usr/bin/env bash
# ★ Build the four managed-memory apps of the V3 app matrix for the guest fault-plane lane
# (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §8 E-A) into /workspace/bench/uvmapps — the same
# sources and flags as `scripts/apps/build_bundle.sh` (cuda-samples v12.5, SMS=86; attach_verify
# from scripts/apps/src, static cudart), plus the CUDA libraries the dynamic samples load. On the
# BOX (never copied back). Idempotent.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
O=/workspace/bench/uvmapps; S=/workspace/bench/src; mkdir -p "$O/lib" "$S"
CUDA=/usr/local/cuda-12.6; export PATH="$CUDA/bin:$PATH"
[ -d "$S/cuda-samples" ] || git clone -q --depth 1 --branch v12.5 https://github.com/NVIDIA/cuda-samples.git "$S/cuda-samples" || { echo "UVMAPPS_BUILD clone failed"; exit 2; }
for d in 0_Introduction/UnifiedMemoryStreams 6_Performance/UnifiedMemoryPerf 4_CUDA_Libraries/conjugateGradientUM; do
  n=$(basename "$d")
  ( cd "$S/cuda-samples/Samples/$d" && make -s SMS=86 CUDA_PATH=$CUDA >"/tmp/build_uvm_$n.log" 2>&1 ) \
    && cp -f "$S/cuda-samples/Samples/$d/$n" "$O/" && echo "UVMAPPS_BUILT $n" || { echo "UVMAPPS_BUILD_FAILED $n"; tail -5 "/tmp/build_uvm_$n.log"; }
done
nvcc -O3 -arch=sm_86 -cudart static -o "$O/attach_verify" "$REPO/scripts/apps/src/attach_verify.cu" && echo "UVMAPPS_BUILT attach_verify"
for l in libcudart.so.12 libcublas.so.12 libcublasLt.so.12 libcusparse.so.12 libnvJitLink.so.12; do
  p=$(readlink -f $CUDA/lib64/$l 2>/dev/null) && [ -f "$p" ] && cp -f "$p" "$O/lib/$l"
done
ls "$O" "$O/lib" | tr '\n' ' '; echo
echo "UVMAPPS_BUILD_DONE"
