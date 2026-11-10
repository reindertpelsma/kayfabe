#!/usr/bin/env bash
# Build the V3 app-matrix bundle ON THE HOST (bench box, root): one set of binaries that runs
# unchanged on the host (baseline) and inside the kf3 fat guest, so a guest-only failure indicts
# kayfabe, not a toolchain difference.  Output: /workspace/apps/bundle/{bin,lib,samples,...}
# + /workspace/apps/bundle.tgz.  Idempotent; every step prints BUILD_<name>=ok|FAIL.
#
# Exit status: 0 only when EVERY step printed ok. Any BUILD_*=FAIL makes this script print BUILD_BUNDLE_FAIL, remove
# bundle.tgz and exit 1 (it used to write the tarball regardless, so a failed toolchain surfaced as failing apps).
# The stage (scripts/kfci/apps_gate.py) additionally scans the log for BUILD_*=FAIL and turns it into a MEDIA retry.
#
# Inputs: every fetch that is ours to pin is pinned in pins.sh and fetched through pin_lib.sh, which refuses an empty,
# malformed or mismatching pin. NVIDIA's CUDA toolkit is installed on the box from NVIDIA's own repository
# (pin_install_cuda_toolkit) and is never stored by us.
#
# The app set is the one nvkvm-pv (the Mode-1 sibling) validated — see
# docs/design/V3_APP_MATRIX.md §1 for the inventory and where each came from.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.sh
. "$HERE/pins.sh"
# shellcheck source=pin_lib.sh
. "$HERE/pin_lib.sh"
W=${KF_APPS_ROOT:-/workspace/apps}; B=$W/bundle; S=$W/srcs   # the stage passes KF_APPS_ROOT through
mkdir -p "$B"/{bin,lib,samples,gpu-burn,llama,share} "$S"
say(){ echo "[build_bundle $(date +%T)] $*"; }
BUILD_FAILS=0
ok(){ if [ "$2" -eq 0 ]; then echo "BUILD_$1=ok"; else echo "BUILD_$1=FAIL rc=$2"; BUILD_FAILS=$((BUILD_FAILS+1)); fi; }
export DEBIAN_FRONTEND=noninteractive

# ---- CUDA toolkit 12.6 (toolkit only — never the driver meta-packages) -----------------------
# From NVIDIA's own repository, pinned (pins.sh); the only place the cache-only apt shim is bypassed (pin_lib.sh).
if [ ! -x /usr/local/cuda-12.6/bin/nvcc ]; then
  say "installing $PIN_CUDA_TOOLKIT_PACKAGE=$PIN_CUDA_TOOLKIT_VERSION from NVIDIA's repository"
  pin_install_cuda_toolkit >/tmp/cuda_apt.log 2>&1
  rc=$?
  [ "$rc" -eq 0 ] || tail -n 8 /tmp/cuda_apt.log   # the reason (PIN_REFUSED, apt error) must reach the provisioning log
  ok cuda_toolkit $rc
fi
export PATH=/usr/local/cuda-12.6/bin:$PATH CUDA_HOME=/usr/local/cuda-12.6
nvcc --version | tail -1

# ---- open Ubuntu dev packages (policy: provision_box.sh --packages apps; the setup cache holds them) -----------
# Through the normal `apt-get`: inside the Vast payload that is the cache-only shim, so a package that is missing
# from the cache fails HERE, loudly, instead of failing later as a cmake or compiler error in some app build.
apt-get install -y -qq cmake git build-essential libegl-dev libgles-dev pkg-config unzip ocl-icd-opencl-dev opencl-headers \
  >/tmp/open_dev_apt.log 2>&1
rc=$?
for t in cmake git gcc make pkg-config unzip; do command -v "$t" >/dev/null 2>&1 || rc=1; done
for h in /usr/include/EGL/egl.h /usr/include/GLES2/gl2.h /usr/include/CL/cl.h; do [ -f "$h" ] || rc=1; done
[ "$rc" -eq 0 ] || tail -n 8 /tmp/open_dev_apt.log
ok open_dev_packages $rc

# ---- nvkvm-pv realapp kernels (static cudart; cuBLAS/cuFFT from the bundled libs) ----------
for f in "$HERE"/src/*.cu; do
  n=$(basename "$f" .cu); ex=""
  case $n in sgemm_cublas) ex="-lcublas";; fft_cufft) ex="-lcufft";; esac
  nvcc -O3 -arch=sm_86 -cudart static -o "$B/bin/$n" "$f" $ex 2>"/tmp/build_$n.err"; ok "$n" $?
done
gcc -O2 "$HERE/src/egl_offscreen.c" -o "$B/bin/egl_offscreen" -lEGL -lGLESv2 2>/tmp/build_egl.err; ok egl_offscreen $?

# ---- CUDA libraries the dynamic apps need (guest has the driver but no toolkit) -------------
for l in libcudart.so.12 libcublas.so.12 libcublasLt.so.12 libcufft.so.11 libcurand.so.10 libnvrtc.so.12 libnvrtc-builtins.so.12.6 libnvJitLink.so.12 libcusparse.so.12 libcusolver.so.11; do
  p=$(readlink -f /usr/local/cuda-12.6/lib64/$l 2>/dev/null) && [ -f "$p" ] && cp -u "$p" "$B/lib/$l"
done
ls "$B/lib" | tr '\n' ' '; echo

# CUDA headers for runtime compilation (CuPy's NVRTC path #includes vector_types.h etc.; the
# guest has no toolkit) — both sides point CUDA_PATH here, so they compile against the same headers
mkdir -p "$B/cuda" && cp -a /usr/local/cuda-12.6/targets/x86_64-linux/include "$B/cuda/"
# ---- cuda-samples v12.5 (Makefile tree; the self-verifying subset) ---------------------------
pin_git_checkout "$S/cuda-samples" "$PIN_CUDA_SAMPLES_URL" "$PIN_CUDA_SAMPLES_TAG" "$PIN_CUDA_SAMPLES_COMMIT"; ok src_cuda_samples $?
SAMPLES="1_Utilities/deviceQuery 1_Utilities/bandwidthTest 0_Introduction/vectorAdd 0_Introduction/matrixMul \
0_Introduction/simpleStreams 0_Introduction/asyncAPI 0_Introduction/simpleAtomicIntrinsics 0_Introduction/simpleIPC \
0_Introduction/UnifiedMemoryStreams 0_Introduction/simpleCallback 0_Introduction/simpleOccupancy 0_Introduction/clock_nvrtc \
0_Introduction/vectorAddDrv 0_Introduction/matrixMulDrv 0_Introduction/simpleZeroCopy 0_Introduction/simpleCooperativeGroups \
3_CUDA_Features/cudaTensorCoreGemm 3_CUDA_Features/cdpSimpleQuicksort 3_CUDA_Features/graphMemoryNodes 3_CUDA_Features/simpleCudaGraphs \
3_CUDA_Features/globalToShmemAsyncCopy 3_CUDA_Features/bf16TensorCoreGemm 4_CUDA_Libraries/simpleCUBLAS 4_CUDA_Libraries/simpleCUFFT \
4_CUDA_Libraries/conjugateGradient 4_CUDA_Libraries/MersenneTwisterGP11213 2_Concepts_and_Techniques/reduction 2_Concepts_and_Techniques/sortingNetworks \
2_Concepts_and_Techniques/scan 2_Concepts_and_Techniques/histogram 5_Domain_Specific/BlackScholes 5_Domain_Specific/fastWalshTransform \
6_Performance/transpose 0_Introduction/concurrentKernels 4_CUDA_Libraries/conjugateGradientUM 6_Performance/UnifiedMemoryPerf"
for d in $SAMPLES; do
  n=$(basename "$d")
  ( cd "$S/cuda-samples/Samples/$d" && make -s SMS=86 CUDA_PATH=/usr/local/cuda-12.6 >/tmp/build_s_$n.log 2>&1 ) \
    && cp -u "$S/cuda-samples/Samples/$d/$n" "$B/samples/" 2>/dev/null
  # driver-API samples load a .fatbin/.ptx from their own dir at runtime
  cp -u "$S/cuda-samples/Samples/$d/"*.fatbin "$S/cuda-samples/Samples/$d/"*.ptx "$B/samples/" 2>/dev/null
  [ -x "$B/samples/$n" ]; ok "sample_$n" $?
done

# ---- gpu-burn --------------------------------------------------------------------------------
pin_git_checkout "$S/gpu-burn" "$PIN_GPU_BURN_URL" "" "$PIN_GPU_BURN_COMMIT"; ok src_gpu_burn $?
( cd "$S/gpu-burn" && make -s CUDAPATH=/usr/local/cuda-12.6 COMPUTE=86 >/tmp/build_gpuburn.log 2>&1 ) \
  && cp -u "$S/gpu-burn/gpu_burn" "$S"/gpu-burn/compare.* "$B/gpu-burn/"
[ -x "$B/gpu-burn/gpu_burn" ]; ok gpu_burn $?

# ---- llama.cpp (CUDA backend, sm_86) --------------------------------------------------------
# pinned to the revision of the first matrix (V3_APP_MATRIX.md §1) so reruns are comparable; no environment override
pin_git_checkout "$S/llama.cpp" "$PIN_LLAMA_URL" "" "$PIN_LLAMA_COMMIT"; ok src_llama_cpp $?
( cd "$S/llama.cpp" && cmake -B build -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86 -DLLAMA_CURL=OFF \
    -DCMAKE_BUILD_TYPE=Release >/tmp/build_llama.log 2>&1 && cmake --build build -j"$(nproc)" --target llama-cli llama-bench llama-simple >>/tmp/build_llama.log 2>&1 )
rm -f "$B"/llama/lib*.so*
cp -u "$S/llama.cpp/build/bin/"llama-* "$B/llama/" 2>/dev/null
find "$S/llama.cpp/build" -name '*.so*' -exec cp -P -u {} "$B/llama/" \; 2>/dev/null
[ -x "$B/llama/llama-cli" ]; ok llama_cpp $?
git -C "$S/llama.cpp" log --oneline -1 | sed 's/^/LLAMA_REV=/'

# ---- clpeak (OpenCL peak) -----------------------------------------------------------------------
# 1.1.2 = the OpenCL-only release; HEAD grew CUDA fp8 backends whose ptxas needs sm_89 (pins.sh)
pin_git_checkout "$S/clpeak" "$PIN_CLPEAK_URL" "$PIN_CLPEAK_TAG" "$PIN_CLPEAK_COMMIT"; ok src_clpeak $?
( cd "$S/clpeak" && rm -rf build && cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++ >/tmp/build_clpeak.log 2>&1 && cmake --build build -j"$(nproc)" >>/tmp/build_clpeak.log 2>&1 ) \
  && cp -u "$S/clpeak/build/clpeak" "$B/bin/"
[ -x "$B/bin/clpeak" ]; ok clpeak $?

# ---- vkpeak (Vulkan compute peak) ------------------------------------------------------------
if [ ! -x "$B/bin/vkpeak" ]; then
  VKDIR=$(mktemp -d /tmp/vkpeak.XXXXXX)
  pin_fetch_file "$PIN_VKPEAK_URL" "$VKDIR/vkpeak.zip" "$PIN_VKPEAK_SHA256" "$PIN_VKPEAK_SIZE" \
    && unzip -o -q "$VKDIR/vkpeak.zip" -d "$VKDIR/x" && cp "$(find "$VKDIR/x" -name vkpeak -type f | head -1)" "$B/bin/vkpeak" && chmod +x "$B/bin/vkpeak"
  rm -rf "$VKDIR"
fi
[ -x "$B/bin/vkpeak" ]; ok vkpeak $?

cp -u "$HERE/src/ai_bench.py" "$HERE"/src/*.py "$B/share/" 2>/dev/null
cp -u "$HERE/run_apps.sh" "$B/"
# A bundle with any failed step is never written: the stage must see "not built", not 66 apps of which some fail.
bundle_fail() { rm -f "$W/bundle.tgz" "$W/bundle.tgz.tmp"; echo "BUILD_BUNDLE_FAIL count=$BUILD_FAILS"; exit 1; }
[ "$BUILD_FAILS" -eq 0 ] || bundle_fail
( cd "$W" && tar czf bundle.tgz.tmp bundle && mv -f bundle.tgz.tmp bundle.tgz ); ok bundle_tar $?
[ "$BUILD_FAILS" -eq 0 ] || bundle_fail
echo "BUNDLE $(du -sh "$B" | cut -f1) $(ls -la "$W/bundle.tgz" | awk '{print $5}')B  sha=$(sha256sum "$W/bundle.tgz" | cut -c1-16)"
echo BUILD_BUNDLE_DONE
