#!/usr/bin/env bash
# provision_tools.sh: install the NVIDIA profiling and debugging tools the gap probes drive, on THIS machine, from NVIDIA's official CUDA apt repository
# (docs/design/V3_GPUPV_GAP_MATRIX.md section 3). Run on the HOST box and inside the guest, as root, with the network. Nothing is stored in the repo, an image or R2.
# Prints GAPTOOLS_<name>=ok|FAIL lines; the exit status is 0 when the packages installed. Versions are pinned below (listed in the repository index on 2026-11-18).
set -euo pipefail
. "$(dirname "$0")/../open_rows/pins.env"
CUDA=12-9; SAN=12.9.79-1; CUPTI=12.9.79-1; NCU=12.9.1-1; NSYS=12.9.1-1
rel=$(. /etc/os-release && echo "${ID}${VERSION_ID//./}"); base=$OPENROWS_NVIDIA_REPO_BASE/$rel/x86_64
keyring=/usr/share/keyrings/kayfabe-nvidia-cuda.gpg
if ! grep -rqs "developer.download.nvidia.com/compute/cuda/repos/$rel" /etc/apt/sources.list /etc/apt/sources.list.d; then
  curl -fsSL --connect-timeout 20 -o /tmp/nvidia.key "$base/$OPENROWS_NVIDIA_KEY_FILE"
  gpg --dearmor --yes -o "$keyring" /tmp/nvidia.key
  echo "deb [signed-by=$keyring] $base/ /" > /etc/apt/sources.list.d/kayfabe-nvidia-cuda.list
fi
apt-get -o DPkg::Lock::Timeout=600 update -qq
DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 install -y -qq --no-install-recommends \
  "cuda-sanitizer-$CUDA=$SAN" "cuda-cupti-$CUDA=$CUPTI" "cuda-nsight-compute-$CUDA=$NCU" "cuda-nsight-systems-$CUDA=$NSYS"
for t in compute-sanitizer ncu nsys; do
  p=$(command -v "$t" || ls /usr/local/cuda-*/bin/"$t" 2>/dev/null | tail -n 1 || true)
  if [ -n "$p" ]; then echo "GAPTOOLS_$t=ok path=$p"; else echo "GAPTOOLS_$t=FAIL reason=not_found_after_install"; fi
done
ls /usr/local/cuda-*/extras/CUPTI/lib64/libcupti.so.* >/dev/null 2>&1 && echo "GAPTOOLS_cupti=ok" || echo "GAPTOOLS_cupti=FAIL reason=libcupti_not_found"
