#!/usr/bin/env bash
# setup_side.sh — install the app-matrix runtime ON THIS MACHINE (bench host or guest), as root.
# Identical on both sides so the host run is a valid baseline for the guest run.
# Expects the bundle already unpacked at /opt/apps/bundle. Prints SETUP_<x>=ok|FAIL per step.
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# Pinned inputs (pins.sh) and the fail-closed fetch helpers (pin_lib.sh) travel beside this script. Without them there
# is nothing to verify a download against, so nothing is fetched: refusing is the only safe default.
if [ ! -f "$HERE/pins.sh" ] || [ ! -f "$HERE/pin_lib.sh" ]; then echo "SETUP_pins=FAIL rc=1"; echo SETUP_SIDE_DONE; exit 1; fi
. "$HERE/pins.sh"
. "$HERE/pin_lib.sh"
A=${APPS_ROOT:-/opt/apps}; D=$A/data; mkdir -p "$D"   # APPS_ROOT: the same override run_apps.sh honours
SETUP_FAILS=0
ok(){ if [ "$2" -eq 0 ]; then echo "SETUP_$1=ok"; else echo "SETUP_$1=FAIL rc=$2"; SETUP_FAILS=$((SETUP_FAILS+1)); fi; }
export DEBIAN_FRONTEND=noninteractive
apt-get -o Acquire::Retries=3 update -qq >/dev/null 2>&1
apt-get -o Acquire::Retries=3 install -y -qq --no-install-recommends ffmpeg vulkan-tools libvulkan1 clinfo ocl-icd-libopencl1 hashcat \
  libegl1 libgles2 libgl1 python3-venv python3-pip curl ca-certificates unzip xz-utils libxi6 libxxf86vm1 libxfixes3 \
  libxrender1 libxkbcommon0 libsm6 libice6 >/tmp/setup_apt.log 2>&1; ok apt $?
# ⊘ the NVIDIA OpenCL ICD file is installed by the driver .run; assert it rather than assume it
ICD_DIR=${OPENCL_VENDORS_DIR:-/etc/OpenCL/vendors}
[ -f "$ICD_DIR/nvidia.icd" ] || echo "libnvidia-opencl.so.1" > "$ICD_DIR/nvidia.icd"

if [ ! -x $A/venv/bin/python ]; then python3 -m venv $A/venv; fi
pin_retry $A/venv/bin/pip -q install --upgrade pip >/dev/null 2>&1
# pinned: both sides must run the same runtime (w720: "latest" drifts per site)
pin_retry $A/venv/bin/pip -q install --retries 5 --timeout 30 torch==2.6.0 torchvision==0.21.0 --index-url https://download.pytorch.org/whl/cu124 >/tmp/setup_pip.log 2>&1; ok torch $?
pin_retry $A/venv/bin/pip -q install --retries 5 --timeout 30 transformers==4.46.3 accelerate==1.1.1 cupy-cuda12x==13.3.0 numpy==1.26.4 >>/tmp/setup_pip.log 2>&1; ok pylibs $?
# the imports row: no IMPORT_OK line means the runtime the apps need is not there (a provisioning failure, not an app result)
IMPORTS=$($A/venv/bin/python -c 'import torch,torchvision,transformers,cupy;print("IMPORT_OK",torch.__version__,torchvision.__version__,transformers.__version__,cupy.__version__)' 2>&1 | tail -1)
echo "$IMPORTS"
case "$IMPORTS" in IMPORT_OK\ *) rc=0;; *) rc=1;; esac; ok imports $rc

# Qwen2-0.5B-Instruct at its pinned revision (pins.sh), laid out as the offline cache, then loaded OFFLINE once here:
# the apps run offline (run_apps.sh sets HF_HUB_OFFLINE=1), so a layout that cannot be loaded must fail now as a
# provisioning step, not later as a graded hf_generate FAIL.
pin_hf_snapshot "$D/hf"; rc=$?
if [ "$rc" -eq 0 ]; then
  HF_HOME=$D/hf HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 $A/venv/bin/python -c "
from transformers import AutoModelForCausalLM, AutoTokenizer
m='Qwen/Qwen2-0.5B-Instruct'; AutoTokenizer.from_pretrained(m); AutoModelForCausalLM.from_pretrained(m); print('HF_MODEL_CACHED')" 2>&1 | tail -1 | tee /tmp/setup_hf.out
  grep -qx HF_MODEL_CACHED /tmp/setup_hf.out; rc=$?
fi
ok hf_model $rc

# Immutable revision + sha256 (pins.sh): an existing file is accepted only if it is those bytes, a download only likewise.
G=$D/qwen2.5-1.5b-instruct-q4_k_m.gguf
pin_verify_file "$G" "$PIN_GGUF_SHA256" "$PIN_GGUF_SIZE" 2>/dev/null \
  || pin_fetch_file "$PIN_GGUF_URL" "$G" "$PIN_GGUF_SHA256" "$PIN_GGUF_SIZE"
ok gguf $?

# Blender 4.5.0 LTS (Cycles). ⊘ The Open Data launcher's mirror (ftp.nluug.nl) TLS-timed-out on
# the first box, so the tarball comes straight from download.blender.org and the render is a
# scripted scene (src/blender_render.py), not the launcher's.
BL=$D/blender
if [ ! -x "$BL/blender" ]; then
  BLT=$(mktemp -d /tmp/blender.XXXXXX)
  mkdir -p "$BL" && pin_fetch_file "$PIN_BLENDER_URL" "$BLT/blender.tar.xz" "$PIN_BLENDER_SHA256" "$PIN_BLENDER_SIZE" \
    && tar xJf "$BLT/blender.tar.xz" -C "$BL" --strip-components=1
  rm -rf "$BLT"
fi
"$BL/blender" --version 2>/dev/null | head -1; [ -x "$BL/blender" ]; ok blender $?

# Geekbench 6 (GPU OpenCL); runs in its free mode and uploads the result. ⊘ [2026-11-07] docs/DOWNLOADS.md
# lists Geekbench as "not allowed (scripts need Pro), manual" and the free mode uploads the result to a
# third party, so it is NOT fetched by default and `geekbench_gpu` is a policy skip in matrix_table.py.
# KF_APPS_GEEKBENCH=1 fetches it for a manual, owner-approved run.
GB=$D/geekbench; mkdir -p "$GB"
if [ "${KF_APPS_GEEKBENCH:-0}" = 1 ]; then
  [ -x "$GB/geekbench6" ] || curl -fsSL https://cdn.geekbench.com/Geekbench-6.4.0-Linux.tar.gz | tar xz -C "$GB" --strip-components=1
  [ -x "$GB/geekbench6" ]; ok geekbench $?
else
  echo "SETUP_geekbench=SKIPPED (policy: docs/DOWNLOADS.md)"
fi
df -h / | tail -1
echo SETUP_SIDE_DONE
[ "$SETUP_FAILS" -eq 0 ]
