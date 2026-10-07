#!/usr/bin/env bash
# setup_side.sh — install the app-matrix runtime ON THIS MACHINE (bench host or guest), as root.
# Identical on both sides so the host run is a valid baseline for the guest run.
# Expects the bundle already unpacked at /opt/apps/bundle. Prints SETUP_<x>=ok|FAIL per step.
set -uo pipefail
A=/opt/apps; D=$A/data; mkdir -p "$D"
ok(){ if [ "$2" -eq 0 ]; then echo "SETUP_$1=ok"; else echo "SETUP_$1=FAIL rc=$2"; fi; }
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq --no-install-recommends ffmpeg vulkan-tools libvulkan1 clinfo ocl-icd-libopencl1 hashcat \
  libegl1 libgles2 libgl1 python3-venv python3-pip curl ca-certificates unzip xz-utils libxi6 libxxf86vm1 libxfixes3 \
  libxrender1 libxkbcommon0 libsm6 libice6 >/tmp/setup_apt.log 2>&1; ok apt $?
# ⊘ the NVIDIA OpenCL ICD file is installed by the driver .run; assert it rather than assume it
[ -f /etc/OpenCL/vendors/nvidia.icd ] || echo "libnvidia-opencl.so.1" > /etc/OpenCL/vendors/nvidia.icd

if [ ! -x $A/venv/bin/python ]; then python3 -m venv $A/venv; fi
$A/venv/bin/pip -q install --upgrade pip >/dev/null 2>&1
# pinned: both sides must run the same runtime (w720: "latest" drifts per site)
$A/venv/bin/pip -q install torch==2.6.0 torchvision==0.21.0 --index-url https://download.pytorch.org/whl/cu124 >/tmp/setup_pip.log 2>&1; ok torch $?
$A/venv/bin/pip -q install transformers==4.46.3 accelerate==1.1.1 cupy-cuda12x==13.3.0 numpy==1.26.4 >>/tmp/setup_pip.log 2>&1; ok pylibs $?
$A/venv/bin/python -c 'import torch,torchvision,transformers,cupy;print("IMPORT_OK",torch.__version__,torchvision.__version__,transformers.__version__,cupy.__version__)' 2>&1 | tail -1

HF_HOME=$D/hf $A/venv/bin/python -c "
from transformers import AutoModelForCausalLM, AutoTokenizer
m='Qwen/Qwen2-0.5B-Instruct'; AutoTokenizer.from_pretrained(m); AutoModelForCausalLM.from_pretrained(m); print('HF_MODEL_CACHED')" 2>&1 | tail -1

G=$D/qwen2.5-1.5b-instruct-q4_k_m.gguf
[ -s "$G" ] || curl -fsSL -o "$G.tmp" https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf && mv -f "$G.tmp" "$G" 2>/dev/null
[ -s "$G" ]; ok gguf $?

# Blender 4.5.0 LTS (Cycles). ⊘ The Open Data launcher's mirror (ftp.nluug.nl) TLS-timed-out on
# the first box, so the tarball comes straight from download.blender.org and the render is a
# scripted scene (src/blender_render.py), not the launcher's.
BL=$D/blender
if [ ! -x "$BL/blender" ]; then
  mkdir -p "$BL" && curl -fsSL --retry 3 https://download.blender.org/release/Blender4.5/blender-4.5.0-linux-x64.tar.xz | tar xJ -C "$BL" --strip-components=1
fi
"$BL/blender" --version 2>/dev/null | head -1; [ -x "$BL/blender" ]; ok blender $?

# Geekbench 6 (GPU OpenCL); runs in its free mode and uploads the result
GB=$D/geekbench; mkdir -p "$GB"
[ -x "$GB/geekbench6" ] || curl -fsSL https://cdn.geekbench.com/Geekbench-6.4.0-Linux.tar.gz | tar xz -C "$GB" --strip-components=1
[ -x "$GB/geekbench6" ]; ok geekbench $?
df -h / | tail -1
echo SETUP_SIDE_DONE
