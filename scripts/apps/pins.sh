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

# --- Hugging Face model for hf_generate (guest, setup_side.sh): Qwen/Qwen2-0.5B-Instruct at one revision -------------
# Laid out as the offline cache layout (hub/models--Qwen--Qwen2-0.5B-Instruct/snapshots/<rev>/<file>, refs/main = <rev>)
# exactly as the open image recipe does; each file is "name sha256 size". Revision and digests agree with recipe.json
# and model.safetensors was read from the Hugging Face API on 2026-11-10.
PIN_QWEN2_REV=c540970f9e29518b1d8f06ab8b24cba66ad77b6d
PIN_QWEN2_URL_BASE=https://huggingface.co/Qwen/Qwen2-0.5B-Instruct/resolve/c540970f9e29518b1d8f06ab8b24cba66ad77b6d
PIN_QWEN2_FILE_1="LICENSE c156170b718ec29139d3653d40ed1986fd92fb7e0959b5c71f3c48f62e6636f4 11344"
PIN_QWEN2_FILE_2="config.json 168aa1bd401abc3bc262ba15ba4e499627a8b4e006e9d050b47c22de20660185 659"
PIN_QWEN2_FILE_3="generation_config.json e558847a8b4402616f1273797b015104dc266fe4b520056fca88823ba8f8ebe6 242"
PIN_QWEN2_FILE_4="merges.txt 599bab54075088774b1733fde865d5bd747cbcc7a547c5bc12610e874e26f5e3 1671839"
PIN_QWEN2_FILE_5="tokenizer.json f7c9b2dba4a296b1aa76c16a34b8225c0c118978400d4bb66bff0902d702f5b8 7028015"
PIN_QWEN2_FILE_6="tokenizer_config.json 482bd979881423375ca5414e4e0d94cd7c5349dbb17fffd46b4d36d71e62a1bc 1287"
PIN_QWEN2_FILE_7="vocab.json ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910 2776833"
PIN_QWEN2_FILE_8="model.safetensors 130282af0dfa9fe5840737cc49a0d339d06075f83c5a315c3372c9a0740d0b96 988097824"

# --- bounded retries of every provisioning fetch (pin_lib.sh: pin_retry, pin_fetch_file) ----------------------------
# 3 attempts with a short backoff between them (seconds, space separated). Inside an attempt a connection that does
# not open in 20 s, or a transfer below 50 kB/s for 60 s, ends it. The whole provisioning is capped by the stage
# (scripts/kfci/apps_gate.py, PROVISION_CAP_S), so a persistent failure still ends as a MEDIA retry.
PIN_RETRY_ATTEMPTS=3
# the backoff may be set from the environment (the tests use "0 0"); the number of attempts may not
: "${PIN_RETRY_BACKOFF:=3 10}"
