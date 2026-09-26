#!/usr/bin/env bash
# E6' provisioning: open driver 580.159.04 + CUDA toolkit 12.6 + dkms symvers + ogkm headers.
# Writes clear phase markers; final line is the terminator with rc.
exec >/root/provision.log 2>&1
set -uo pipefail
echo "PROVISION_START $(date -Is)  kernel=$(uname -r)"

echo "=== PHASE 0: build deps + apt hygiene ==="
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer >/dev/null 2>&1
# wait for any unattended-upgrade to release the dpkg lock
w=0; while fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do [ $w -ge 900 ] && { echo "DPKG_LOCK_TIMEOUT"; echo "PROVISION_DONE rc=90"; exit 90; }; [ $((w%60)) -eq 0 ] && echo "waiting dpkg lock ${w}s"; sleep 10; w=$((w+10)); done
echo "dpkg lock free after ${w}s"
export DEBIAN_FRONTEND=noninteractive
apt-get update -y 2>&1 | tail -2
apt-get install -y build-essential dkms "linux-headers-$(uname -r)" git curl wget pkg-config 2>&1 | tail -3
echo "PHASE0_rc=$? gcc=$(gcc --version|head -1) headers=$(ls -d /lib/modules/$(uname -r)/build 2>&1)"

echo "=== PHASE 1: clone kayfabe (v3-uvm-n4) ==="
if [ ! -d /root/kayfabe/.git ]; then
  git clone -b v3-uvm-n4 https://github.com/reindertpelsma/kayfabe.git /root/kayfabe 2>&1 | tail -3
fi
echo "PHASE1_rc=$? head=$(git -C /root/kayfabe rev-parse --short HEAD 2>&1) branch=$(git -C /root/kayfabe rev-parse --abbrev-ref HEAD 2>&1)"

echo "=== PHASE 2: driver swap to OPEN 580.159.04 ==="
chmod +x /root/kayfabe/scripts/bench/provision_host_driver.sh
bash /root/kayfabe/scripts/bench/provision_host_driver.sh 2>&1 | sed 's/^/[drv] /'
DRVRC=${PIPESTATUS[0]}
echo "PHASE2_provision_host_driver_rc=$DRVRC"
VER=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1)
echo "driver now: $VER"
echo "$VER" | grep -q "Open Kernel Module" && echo "OPEN=yes" || { echo "OPEN=no"; echo "PROVISION_DONE rc=2"; exit 2; }
echo "$VER" | grep -q "580.159.04" && echo "V580=yes" || echo "V580=NO"

echo "=== PHASE 3: dkms build (regenerate Module.symvers with nvUvm* CRCs) ==="
dkms status 2>&1 | tail -5
dkms build nvidia/580.159.04 --force 2>&1 | tail -8
echo "PHASE3_dkms_rc=$?"
SYMV=$(find /var/lib/dkms/nvidia/580.159.04 -name Module.symvers 2>/dev/null | xargs -r grep -l nvUvmInterfaceRegisterGpu 2>/dev/null | head -1)
echo "SYMVERS=$SYMV"
[ -n "$SYMV" ] && echo "nvUvm_CRC_count=$(grep -c nvUvmInterface $SYMV)" || echo "NO_SYMVERS_WITH_NVUVM"

echo "=== PHASE 4: CUDA toolkit 12.6 (toolkit only, NO driver) ==="
cd /root
if ! command -v nvcc >/dev/null 2>&1 && [ ! -x /usr/local/cuda-12.6/bin/nvcc ]; then
  wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb -O /root/cuda-keyring.deb
  dpkg -i /root/cuda-keyring.deb 2>&1 | tail -1
  apt-get update -y 2>&1 | tail -1
  apt-get install -y cuda-nvcc-12-6 cuda-cudart-dev-12-6 cuda-cuobjdump-12-6 cuda-nvdisasm-12-6 cuda-crt-12-6 2>&1 | tail -4
fi
echo "PHASE4_rc=$?"
NVCC=$(command -v nvcc || echo /usr/local/cuda-12.6/bin/nvcc)
echo "nvcc=$NVCC ver=$($NVCC --version 2>&1 | tail -1)"

echo "=== PHASE 5: ogkm headers (open-gpu-kernel-modules 580.159.04) ==="
if [ ! -d /root/ogkm/.git ]; then
  git clone --depth 1 -b 580.159.04 https://github.com/NVIDIA/open-gpu-kernel-modules.git /root/ogkm 2>&1 | tail -3
fi
echo "PHASE5_rc=$? ogkm_head=$(git -C /root/ogkm rev-parse --short HEAD 2>&1)"
echo "clc7c0.h: $(find /root/ogkm -name clc7c0.h | head -1)"
echo "clc56f.h: $(find /root/ogkm -name clc56f.h | head -1)"

echo "=== PHASE 6: final state ==="
echo "modules: $(lsmod | grep -iE '^nvidia' | awk '{print $1}' | tr '\n' ' ')"
nvidia-smi --query-gpu=name,driver_version,uuid --format=csv,noheader 2>&1 | head -1
df -h / | tail -1
echo "PROVISION_DONE rc=0 $(date -Is)"
