#!/usr/bin/env bash
# Build the V3 app-matrix bundle ON THE HOST (bench box, root): one set of binaries that runs
# unchanged on the host (baseline) and inside the kf3 fat guest, so a guest-only failure indicts
# kayfabe, not a toolchain difference.  Output: /workspace/apps/bundle/{bin,lib,samples,...}
# + /workspace/apps/bundle.tgz.  Idempotent; every step prints BUILD_<name>=ok|FAIL.
#
# The app set is the one nvkvm-pv (the Mode-1 sibling) validated — see
# docs/design/V3_APP_MATRIX.md §1 for the inventory and where each came from.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
W=/workspace/apps; B=$W/bundle; S=$W/srcs
mkdir -p "$B"/{bin,lib,samples,gpu-burn,llama,share} "$S"
say(){ echo "[build_bundle $(date +%T)] $*"; }
ok(){ if [ "$2" -eq 0 ]; then echo "BUILD_$1=ok"; else echo "BUILD_$1=FAIL rc=$2"; fi; }
export DEBIAN_FRONTEND=noninteractive

# ---- CUDA toolkit 12.6 (toolkit only — never the driver meta-packages) -----------------------
if [ ! -x /usr/local/cuda-12.6/bin/nvcc ]; then
  say "installing cuda-toolkit-12-6"
  . /etc/os-release; REL=ubuntu${VERSION_ID//./}
  curl -fsSL -o /tmp/cuda-keyring.deb "https://developer.download.nvidia.com/compute/cuda/repos/$REL/x86_64/cuda-keyring_1.1-1_all.deb" \
    && dpkg -i /tmp/cuda-keyring.deb >/dev/null && apt-get update -qq \
    && apt-get install -y -qq --no-install-recommends cuda-toolkit-12-6 >/tmp/cuda_apt.log 2>&1
  ok cuda_toolkit $?
fi
export PATH=/usr/local/cuda-12.6/bin:$PATH CUDA_HOME=/usr/local/cuda-12.6
nvcc --version | tail -1
apt-get install -y -qq cmake git build-essential libegl-dev libgles-dev pkg-config unzip >/dev/null 2>&1

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

# ---- cuda-samples v12.5 (Makefile tree; the self-verifying subset) ---------------------------
[ -d "$S/cuda-samples" ] || git clone -q --depth 1 --branch v12.5 https://github.com/NVIDIA/cuda-samples.git "$S/cuda-samples"
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
[ -d "$S/gpu-burn" ] || git clone -q --depth 1 https://github.com/wilicc/gpu-burn.git "$S/gpu-burn"
( cd "$S/gpu-burn" && make -s CUDAPATH=/usr/local/cuda-12.6 COMPUTE=86 >/tmp/build_gpuburn.log 2>&1 ) \
  && cp -u "$S/gpu-burn/gpu_burn" "$S"/gpu-burn/compare.* "$B/gpu-burn/"
[ -x "$B/gpu-burn/gpu_burn" ]; ok gpu_burn $?

# ---- llama.cpp (CUDA backend, sm_86) --------------------------------------------------------
[ -d "$S/llama.cpp" ] || git clone -q --depth 1 https://github.com/ggml-org/llama.cpp.git "$S/llama.cpp"
( cd "$S/llama.cpp" && cmake -B build -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86 -DLLAMA_CURL=OFF \
    -DCMAKE_BUILD_TYPE=Release >/tmp/build_llama.log 2>&1 && cmake --build build -j"$(nproc)" --target llama-cli llama-bench llama-simple >>/tmp/build_llama.log 2>&1 )
rm -f "$B"/llama/lib*.so*
cp -u "$S/llama.cpp/build/bin/"llama-* "$B/llama/" 2>/dev/null
find "$S/llama.cpp/build" -name '*.so*' -exec cp -P -u {} "$B/llama/" \; 2>/dev/null
[ -x "$B/llama/llama-cli" ]; ok llama_cpp $?
git -C "$S/llama.cpp" log --oneline -1 | sed 's/^/LLAMA_REV=/'

# ---- clpeak (OpenCL peak) -----------------------------------------------------------------------
apt-get install -y -qq ocl-icd-opencl-dev opencl-headers >/dev/null 2>&1
[ -d "$S/clpeak" ] || git clone -q --depth 1 --recurse-submodules https://github.com/krrishnarraj/clpeak.git "$S/clpeak"
( cd "$S/clpeak" && rm -rf build && cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++ >/tmp/build_clpeak.log 2>&1 && cmake --build build -j"$(nproc)" >>/tmp/build_clpeak.log 2>&1 ) \
  && cp -u "$S/clpeak/build/clpeak" "$B/bin/"
[ -x "$B/bin/clpeak" ]; ok clpeak $?

# ---- vkpeak (Vulkan compute peak) ------------------------------------------------------------
if [ ! -x "$B/bin/vkpeak" ]; then
  curl -fsSL -o /tmp/vkpeak.zip https://github.com/nihui/vkpeak/releases/download/20250531/vkpeak-20250531-ubuntu.zip \
    && unzip -o -q /tmp/vkpeak.zip -d /tmp/vkpeak && cp "$(find /tmp/vkpeak -name vkpeak -type f | head -1)" "$B/bin/vkpeak" && chmod +x "$B/bin/vkpeak"
fi
[ -x "$B/bin/vkpeak" ]; ok vkpeak $?

cp -u "$HERE/src/ai_bench.py" "$HERE"/src/*.py "$B/share/" 2>/dev/null
cp -u "$HERE/run_apps.sh" "$B/"
( cd "$W" && tar czf bundle.tgz.tmp bundle && mv -f bundle.tgz.tmp bundle.tgz )
echo "BUNDLE $(du -sh "$B" | cut -f1) $(ls -la "$W/bundle.tgz" | awk '{print $5}')B  sha=$(sha256sum "$W/bundle.tgz" | cut -c1-16)"
echo BUILD_BUNDLE_DONE
