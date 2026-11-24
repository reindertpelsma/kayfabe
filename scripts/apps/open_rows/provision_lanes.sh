#!/usr/bin/env bash
# provision_lanes.sh [ROOT] ["steps"]: build what the nvbandwidth and llama.cpp Vulkan lanes need, on THIS machine, from the repo only (needs root, network).
#   nvbandwidth    built with the CUDA toolkit at the pinned upstream commit (pins.env)                         -> ROOT/bin/nvbandwidth
#   llama_vulkan   llama.cpp at the pinned commit of the CUDA rows (../pins.sh) built with -DGGML_VULKAN=ON    -> ROOT/llama_vk/llama-bench
#                  (glslc and Vulkan headers from the distribution, else the LunarG SDK checked against LunarG's own sha256)
#   llama_model    (not a default step) the GGUF of the CUDA llama rows, sha256 pinned in ../pins.sh          -> ROOT/data/
# (Every command of a step ends `|| return 1`: a step runs inside `if`, where `set -e` is off.)
# Prints one OPENROWS_<step>=ok|FAIL reason=... line per step (the prefix of provision.sh); a failed step never stops the others and never makes a gate bundle
# fail (build_bundle.sh ignores the exit status): the row reports NOT_RUN with the reason. Trust anchors: docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md section 4.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
. "$HERE/pins.env"
# shellcheck source=../pin_lib.sh
. "$HERE/../pin_lib.sh"
# the llama.cpp, model and CUDA toolkit pins are the ones of the CUDA rows: one pin, never a copy
# shellcheck source=../pins.sh
. "$HERE/../pins.sh"
ROOT=${1:-/opt/open_rows}; FAILS=0
mkdir -p "$ROOT/bin" "$ROOT/src"
step() { local n=$1; shift; if "$@" >"$ROOT/$n.log" 2>&1; then echo "OPENROWS_$n=ok"; else echo "OPENROWS_$n=FAIL reason=$(tail -n 1 "$ROOT/$n.log" | tr ' =' '__' | cut -c1-120)"; FAILS=$((FAILS+1)); fi; }
# Distribution packages through the box's apt-get: under the Vast payload that is the cache-only shim (a package the cache lacks fails here, loudly, and the row is
# NOT_RUN); on a standalone machine it is the real one, which waits for the dpkg lock a fresh machine's unattended-upgrades holds.
apt_need() {
  if [ "$(type -t apt-get)" = function ]; then apt-get install -y -qq "$@"; return; fi
  apt-get -o DPkg::Lock::Timeout=900 -o Acquire::Retries=3 update -qq && apt-get -o DPkg::Lock::Timeout=900 install -y -qq --no-install-recommends "$@"
}
find_nvcc() { command -v nvcc 2>/dev/null || ls /usr/local/cuda*/bin/nvcc 2>/dev/null | sort -V | tail -n 1; }
nvbandwidth() {
  local nvcc src=$ROOT/src/nvbandwidth
  { command -v cmake && command -v g++ && command -v git; } >/dev/null || apt_need cmake build-essential git || return 1
  [ -f /usr/include/boost/program_options.hpp ] || apt_need libboost-program-options-dev || return 1
  # the toolkit the box already has (build_bundle.sh installs the pinned one first), else the pinned install of pin_lib.sh (NVIDIA's repository, keyring pinned): not a new bypass
  [ -n "$(find_nvcc || true)" ] || pin_install_cuda_toolkit || return 1
  nvcc=$(find_nvcc) || { echo "nvbandwidth: no_nvcc (no CUDA toolkit here and the pinned install did not provide one)"; return 1; }
  pin_git_checkout "$src" "$OPENROWS_NVB_URL" "$OPENROWS_NVB_TAG" "$OPENROWS_NVB_COMMIT" || return 1
  # architectures are explicit (the last with PTX, so a newer die JIT-compiles): the bundle binary must run on every die, not only the one that built it
  cmake -S "$src" -B "$src/build" -DCMAKE_BUILD_TYPE=Release -DCMAKE_CUDA_COMPILER="$nvcc" -DCMAKE_CUDA_ARCHITECTURES="$OPENROWS_NVB_ARCHS" >"$ROOT/nvbandwidth_cmake.log" 2>&1 \
    || { tail -n 5 "$ROOT/nvbandwidth_cmake.log"; return 1; }
  cmake --build "$src/build" -j"$(nproc)" >"$ROOT/nvbandwidth_build.log" 2>&1 || { tail -n 5 "$ROOT/nvbandwidth_build.log"; return 1; }
  install -m 0755 "$src/build/nvbandwidth" "$ROOT/bin/nvbandwidth" || return 1
  "$ROOT/bin/nvbandwidth" --help | head -n 1
}
# glslc and current Vulkan headers: the distribution's (Ubuntu 24.04 has `glslc`; 22.04, the Vast KVM image, has none: MEASURED 2026-11-23, traces/open_lanes_20261123/), else the LunarG
# Vulkan SDK of the pinned version, whose sha256 is read from LunarG's own checksum endpoint for that version (an official-origin checksum, the MinGit class of anchor; none in git)
lunarg_sdk() {
  local d=$ROOT/vulkan-sdk v=$OPENROWS_LUNARG_SDK_VERSION
  [ -x "$d/$v/x86_64/bin/glslc" ] && return 0
  mkdir -p "$d" || return 1
  curl -fsSL --retry 3 --connect-timeout 20 -o "$d/sdk.tar.xz" "$OPENROWS_LUNARG_SDK_BASE/$v/linux/vulkan_sdk.tar.xz" || return 1
  curl -fsSL --retry 3 --connect-timeout 20 -o "$d/sdk.sha" "$OPENROWS_LUNARG_CHECKSUM_BASE/$v/linux/vulkan_sdk.tar.xz.txt" || return 1
  [ "$(sha256sum "$d/sdk.tar.xz" | cut -d' ' -f1)" = "$(cut -d' ' -f1 "$d/sdk.sha")" ] || { echo "lunarg_sdk: SDK_HASH_MISMATCH the download is not what LunarG lists for $v"; return 1; }
  tar -C "$d" -xf "$d/sdk.tar.xz" && rm -f "$d/sdk.tar.xz" || return 1
}
llama_vulkan() {
  local src=$ROOT/src/llama.cpp sdk=""
  { command -v cmake && command -v g++ && command -v git; } >/dev/null || apt_need cmake build-essential git || return 1
  if ! { command -v glslc && [ -f /usr/include/vulkan/vulkan.h ]; } >/dev/null; then
    apt_need libvulkan-dev glslc 2>/dev/null || { lunarg_sdk || return 1; sdk=$ROOT/vulkan-sdk/$OPENROWS_LUNARG_SDK_VERSION/x86_64; }
  fi
  pin_git_checkout "$src" "$PIN_LLAMA_URL" "" "$PIN_LLAMA_COMMIT" || return 1
  # the CUDA rows' commit, so a throughput difference between the CUDA and the Vulkan row is the backend's
  ( cd "$src" && VULKAN_SDK=$sdk cmake -B build-vk -DGGML_VULKAN=ON -DGGML_CUDA=OFF -DLLAMA_CURL=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DCMAKE_BUILD_TYPE=Release >"$ROOT/llama_vulkan_cmake.log" 2>&1 ) \
    || { tail -n 8 "$ROOT/llama_vulkan_cmake.log"; return 1; }
  cmake --build "$src/build-vk" -j"$(nproc)" --target llama-bench >"$ROOT/llama_vulkan_build.log" 2>&1 || { tail -n 8 "$ROOT/llama_vulkan_build.log"; return 1; }
  rm -rf "$ROOT/llama_vk" && mkdir -p "$ROOT/llama_vk" && cp "$src/build-vk/bin/llama-bench" "$ROOT/llama_vk/" || return 1
  find "$src/build-vk" -name '*.so*' -exec cp -P {} "$ROOT/llama_vk/" \; || return 1
  # the loader the binary was linked against travels with it (a machine without libvulkan.so.1 still runs it)
  { [ -z "$sdk" ] || cp -P "$sdk"/lib/libvulkan.so* "$ROOT/llama_vk/"; } || return 1
  git -C "$src" log --oneline -1
}
llama_model() {
  mkdir -p "$ROOT/data" || return 1
  pin_fetch_file "$PIN_GGUF_URL" "$ROOT/data/qwen2.5-1.5b-instruct-q4_k_m.gguf" "$PIN_GGUF_SHA256" "$PIN_GGUF_SIZE"
}
STEPS=${2:-nvbandwidth llama_vulkan}
want() { case " $STEPS " in *" $1 "*) return 0;; esac; return 1; }
want nvbandwidth && step nvbandwidth nvbandwidth
want llama_vulkan && step llama_vulkan llama_vulkan
want llama_model && step llama_model llama_model
[ "$FAILS" -eq 0 ]
