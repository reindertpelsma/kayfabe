#!/usr/bin/env bash
# mini_bundle.sh - a CUDA-samples-only cut of scripts/apps/build_bundle.sh (hwval_20261120): the same pinned
# toolkit, the same pinned cuda-samples checkout, the same make line, but only the samples named in $SAMPLES and
# none of torch / blender / llama.cpp / clpeak / vkpeak / open rows. Output: /workspace/apps/bundle/{samples,lib,bin,cuda}
# + /workspace/apps/bundle.tgz. Run ON the box from a checkout (argument 1 = checkout, default /root/kayfabe).
set -uo pipefail
REPO=${1:-/root/kayfabe}
HERE=$REPO/scripts/apps
. "$HERE/pins.sh"
. "$HERE/pin_lib.sh"
W=/workspace/apps; B=$W/bundle; S=$W/srcs
mkdir -p "$B"/{bin,lib,samples,share} "$S"
export DEBIAN_FRONTEND=noninteractive
say(){ echo "[mini_bundle $(date +%T)] $*"; }
fails=0
ok(){ if [ "$2" -eq 0 ]; then echo "BUILD_$1=ok"; else echo "BUILD_$1=FAIL rc=$2"; fails=$((fails+1)); fi; }
if [ ! -x /usr/local/cuda-12.6/bin/nvcc ]; then
  say "installing $PIN_CUDA_TOOLKIT_PACKAGE=$PIN_CUDA_TOOLKIT_VERSION"
  pin_install_cuda_toolkit >/tmp/cuda_apt.log 2>&1; rc=$?
  [ "$rc" -eq 0 ] || tail -n 8 /tmp/cuda_apt.log
  ok cuda_toolkit $rc
fi
export PATH=/usr/local/cuda-12.6/bin:$PATH CUDA_HOME=/usr/local/cuda-12.6
nvcc --version | tail -1
apt-get install -y -qq build-essential git pkg-config unzip >/tmp/open_dev_apt.log 2>&1; ok dev_packages $?
rc=0
for l in libcudart.so.12 libcublas.so.12 libcublasLt.so.12 libcufft.so.11 libcurand.so.10 libnvrtc.so.12 libnvrtc-builtins.so.12.6 libnvJitLink.so.12 libcusparse.so.12 libcusolver.so.11; do
  p=$(readlink -f /usr/local/cuda-12.6/lib64/$l 2>/dev/null)
  if [ -f "$p" ] && cp -u "$p" "$B/lib/$l"; then :; else echo "missing CUDA library $l"; rc=1; fi
done
ok cuda_libs $rc
mkdir -p "$B/cuda" && cp -a /usr/local/cuda-12.6/targets/x86_64-linux/include "$B/cuda/" && [ -f "$B/cuda/include/vector_types.h" ]; ok cuda_headers $?
pin_git_checkout "$S/cuda-samples" "$PIN_CUDA_SAMPLES_URL" "$PIN_CUDA_SAMPLES_TAG" "$PIN_CUDA_SAMPLES_COMMIT"; ok src_cuda_samples $?
SAMPLES=${SAMPLES:-"1_Utilities/deviceQuery 1_Utilities/bandwidthTest 0_Introduction/vectorAdd 0_Introduction/matrixMul \
0_Introduction/simpleStreams 0_Introduction/asyncAPI 0_Introduction/simpleCallback 0_Introduction/vectorAddDrv 0_Introduction/matrixMulDrv \
0_Introduction/simpleCooperativeGroups 3_CUDA_Features/cudaTensorCoreGemm 3_CUDA_Features/graphMemoryNodes 3_CUDA_Features/simpleCudaGraphs \
4_CUDA_Libraries/simpleCUBLAS 4_CUDA_Libraries/simpleCUFFT"}
for d in $SAMPLES; do
  n=$(basename "$d")
  ( cd "$S/cuda-samples/Samples/$d" && make -s SMS=86 CUDA_PATH=/usr/local/cuda-12.6 >/tmp/build_s_$n.log 2>&1 ) \
    && cp -u "$S/cuda-samples/Samples/$d/$n" "$B/samples/" 2>/dev/null
  cp -u "$S/cuda-samples/Samples/$d/"*.fatbin "$S/cuda-samples/Samples/$d/"*.ptx "$B/samples/" 2>/dev/null
  [ -x "$B/samples/$n" ]; ok "sample_$n" $?
done
cp -u "$HERE/run_apps.sh" "$B/"
[ "$fails" -eq 0 ] || { echo "MINI_BUNDLE_FAIL count=$fails"; exit 1; }
( cd "$W" && tar czf bundle.tgz.tmp bundle && mv -f bundle.tgz.tmp bundle.tgz ); ok bundle_tar $?
mkdir -p /opt/apps && rm -rf /opt/apps/bundle && cp -a "$B" /opt/apps/bundle
echo "BUNDLE $(du -sh "$B" | cut -f1) sha=$(sha256sum "$W/bundle.tgz" | cut -c1-16)"
echo MINI_BUNDLE_DONE
