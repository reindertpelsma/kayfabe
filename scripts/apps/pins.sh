# Linux apps matrix: every input that the HAND PATH (build_bundle.sh, setup_side.sh) fetches and that is ours to pin.
# Data only: sourced by build_bundle.sh, copied into the guest beside setup_side.sh, and parsed by
# scripts/ci/test_apps_pins.py, which also checks every value against scripts/apps/image/recipe.json (the open
# image recipe pins the same inputs) so the two paths cannot drift apart silently.
#
# Rules (docs/design/V3_APPS_IMAGE_RECIPE.md, "Linux apps on a Worker box"):
#   * a git input is pinned by the full 40-hex commit; the tag or branch is only how it is fetched, and the
#     checked-out HEAD must equal the pin (pin_lib.sh: pin_git_checkout);
#   * a file input is pinned by sha256 (and size where the host publishes one) and is refused, never "accepted
#     with a warning", when the digest differs or the pin is empty (pin_lib.sh: pin_fetch_file);
#   * a pin that is unknown is the empty string and the fetch refuses (TODO-FAILS-CLOSED); there is no default.
# Provenance of the values: commits and digests were read from the upstream hosts on 2026-11-10 and agree with the
# recipe's 2026-11-07 entries; the commits of cuda-samples, llama.cpp and clpeak also match the revisions the box
# agent's 2026-11-10 bundle printed (traces/apps_linux_media_20261110/bundle_build.log). The gpu-burn pin is the
# upstream HEAD of 2026-10-02, which was HEAD when the box agent built (INFERRED: that build printed no revision).

# --- git sources -------------------------------------------------------------------------------------------
PIN_CUDA_SAMPLES_URL=https://github.com/NVIDIA/cuda-samples.git
PIN_CUDA_SAMPLES_TAG=v12.5
PIN_CUDA_SAMPLES_COMMIT=9c688d7ff78455ed42e345124d1495aad6bf66de
PIN_GPU_BURN_URL=https://github.com/wilicc/gpu-burn.git
PIN_GPU_BURN_COMMIT=0d19c94dcb2e9b6858b7586a4c327674cd67113b
PIN_LLAMA_URL=https://github.com/ggml-org/llama.cpp.git
PIN_LLAMA_COMMIT=4b1a27fa0eb875bbca4f6cfe936e3d65adc685c0
# 1.1.2 = the OpenCL-only release; HEAD grew CUDA fp8 backends whose ptxas needs sm_89. Submodules follow the commit.
PIN_CLPEAK_URL=https://github.com/krrishnarraj/clpeak.git
PIN_CLPEAK_TAG=1.1.2
PIN_CLPEAK_COMMIT=a4d617d7d054450aa83965d0fd9d62ee2009957f

# --- file sources ------------------------------------------------------------------------------------------
PIN_VKPEAK_URL=https://github.com/nihui/vkpeak/releases/download/20250531/vkpeak-20250531-ubuntu.zip
PIN_VKPEAK_SHA256=5d233c230d4dc14c43e76da3feb4a7374f9e79d0a0bbea00b1b8f33f6ac07eaf
PIN_VKPEAK_SIZE=1239961
# Guest side (setup_side.sh): the GGUF at an immutable Hugging Face revision, and the Blender 4.5.0 LTS tarball.
PIN_GGUF_REV=91cad51170dc346986eccefdc2dd33a9da36ead9
PIN_GGUF_URL=https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/qwen2.5-1.5b-instruct-q4_k_m.gguf
PIN_GGUF_SHA256=6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e
PIN_GGUF_SIZE=1117320736
PIN_BLENDER_URL=https://download.blender.org/release/Blender4.5/blender-4.5.0-linux-x64.tar.xz
PIN_BLENDER_SHA256=1188b95cc12321c770b631939f7c25a096910b6f884a990bf9c0f62d52b38aec
PIN_BLENDER_SIZE=376170648

# --- NVIDIA CUDA toolkit: installed ON THE BOX, at run time, from NVIDIA's own repository -----------------------
# Nothing from NVIDIA is stored in our images, R2 or release assets (owner ruling 2026-11-07). The repository's
# signing key arrives inside cuda-keyring_1.1-1_all.deb; that deb is pinned by sha256 per Ubuntu release, and any
# release that has no pin is refused. The toolkit metapackage is pinned by exact version (nvcc 12.6.85 = CUDA 12.6.3,
# the 12.6 line's last release); the versions of its NVIDIA dependencies are whatever NVIDIA's signed index resolves.
PIN_CUDA_KEYRING_URL_BASE=https://developer.download.nvidia.com/compute/cuda/repos
PIN_CUDA_KEYRING_DEB=cuda-keyring_1.1-1_all.deb
PIN_CUDA_KEYRING_SHA256_ubuntu2204=d93190d50b98ad4699ff40f4f7af50f16a76dac3bb8da1eaaf366d47898ff8df
PIN_CUDA_KEYRING_SHA256_ubuntu2404=d2a6b11c096396d868758b86dab1823b25e14d70333f1dfa74da5ddaf6a06dba
PIN_CUDA_TOOLKIT_PACKAGE=cuda-toolkit-12-6
PIN_CUDA_TOOLKIT_VERSION=12.6.3-1
