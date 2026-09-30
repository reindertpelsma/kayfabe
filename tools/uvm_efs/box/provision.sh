#!/usr/bin/env bash
# tools/uvm_efs/box/provision.sh — prepare a rented vast KVM-template box for the b3 host-only
# EFS experiment (docs/design/V3_UVM_B3_IMPLEMENTATION.md). Run ON THE BOX as root:
#   KAYFABE_BRANCH=v3-uvm-b3 nohup bash provision.sh >/dev/null 2>&1 &
# Writes /root/efs/prov.log: one *_RC= line per step, then READY or NOT_READY, then an EXIT line.
# ⊘ Wait for the EXIT line. A killed job and a running one look the same (CLAUDE.md traps).
# Steps: apt hygiene -> kayfabe clone (this branch) -> host driver swap to OPEN 580.159.04
#        -> CUDA 12.6 toolkit -> sparse ogkm 580.159.04 nvidia-uvm (provenance diff only)
#        -> a STOCK rebuild of nvidia + nvidia-uvm from the installed DKMS tree (build-path check).
# Nothing here loads a patched module; tools/uvm_efs/box/build_efs.sh does that, on purpose, later.
mkdir -p /root/efs
exec >>/root/efs/prov.log 2>&1
set -uo pipefail
echo "PROV_START $(date -Is) kernel=$(uname -r) branch=${KAYFABE_BRANCH:-v3-uvm-b3}"
rm -f /root/efs/READY
export DEBIAN_FRONTEND=noninteractive

# ---- 1. apt hygiene: an unattended upgrade can pull a new kernel under a DKMS module ----------
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service >/dev/null 2>&1
w=0
while fuser /var/lib/dpkg/lock-frontend /var/lib/apt/lists/lock >/dev/null 2>&1; do
  [ $w -ge 1200 ] && { echo "DPKG_LOCK_TIMEOUT"; echo "NOT_READY"; echo "EXIT rc=90 $(date -Is)"; exit 90; }
  [ $((w % 60)) -eq 0 ] && echo "waiting for dpkg lock ${w}s"
  sleep 10; w=$((w + 10))
done
echo "dpkg lock free after ${w}s"
grep -a -E "linux-image|linux-headers|linux-generic" /var/log/unattended-upgrades/*.log 2>/dev/null | tail -3 | sed 's/^/unattended: /'
apt-get update -y 2>&1 | tail -1
apt-get install -y -q build-essential dkms "linux-headers-$(uname -r)" git curl wget pkg-config python3 jq 2>&1 | tail -1
APT_RC=$?; echo "APT_RC=$APT_RC gcc=$(gcc --version | head -1)"
ls -d "/lib/modules/$(uname -r)/build" >/dev/null 2>&1 && echo "KHEADERS=ok" || echo "KHEADERS=MISSING"
NEWEST=$(ls /boot/vmlinuz-* 2>/dev/null | sed 's|/boot/vmlinuz-||' | sort -V | tail -1)
[ "$NEWEST" != "$(uname -r)" ] && echo "REBOOT_NEEDED newest_installed=$NEWEST running=$(uname -r)"

# ---- 2. kayfabe (this branch) -------------------------------------------------------------------
BR=${KAYFABE_BRANCH:-v3-uvm-b3}
if [ ! -d /root/kayfabe/.git ]; then
  git clone -q -b "$BR" https://github.com/reindertpelsma/kayfabe.git /root/kayfabe
else
  git -C /root/kayfabe fetch -q origin "$BR" && git -C /root/kayfabe checkout -q -B "$BR" "origin/$BR"
fi
echo "CLONE_RC=$? head=$(git -C /root/kayfabe rev-parse --short HEAD 2>&1)"

# ---- 3. host driver: OPEN 580.159.04 (verified on content by the script itself) -----------------
if grep -q "Open Kernel Module.* 580.159.04 " /proc/driver/nvidia/version 2>/dev/null; then
  echo "DRIVER_RC=0 (already open 580.159.04)"
else
  bash /root/kayfabe/scripts/bench/provision_host_driver.sh > /root/efs/driver.log 2>&1
  echo "DRIVER_RC=$?"; grep -a -E "OPEN_MODULE|VERSION_MATCH|DRIVER_SWAP_DONE|open /dev" /root/efs/driver.log
fi
head -1 /proc/driver/nvidia/version

# ---- 4. CUDA 12.6 toolkit (no driver) -------------------------------------------------------------
if [ ! -x /usr/local/cuda-12.6/bin/nvcc ]; then
  wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb -O /root/cuda-keyring.deb
  dpkg -i /root/cuda-keyring.deb 2>&1 | tail -1
  apt-get update -y 2>&1 | tail -1
  apt-get install -y -q cuda-nvcc-12-6 cuda-cudart-dev-12-6 cuda-driver-dev-12-6 cuda-cuobjdump-12-6 2>&1 | tail -1
fi
/usr/local/cuda-12.6/bin/nvcc --version >/dev/null 2>&1; echo "CUDA_RC=$? $(/usr/local/cuda-12.6/bin/nvcc --version 2>&1 | tail -1)"

# ---- 5. ogkm 580.159.04, nvidia-uvm only (sparse) — to prove the DKMS tree IS the pinned source -----
if [ ! -d /root/ogkm/.git ]; then
  git clone -q --depth 1 --filter=blob:none --sparse -b 580.159.04 \
      https://github.com/NVIDIA/open-gpu-kernel-modules.git /root/ogkm && \
  git -C /root/ogkm sparse-checkout set kernel-open/nvidia-uvm kernel-open/common/inc src/common/sdk/nvidia/inc
fi
git -C /root/ogkm sparse-checkout set kernel-open/nvidia-uvm kernel-open/common/inc src/common/sdk/nvidia/inc 2>/dev/null
echo "OGKM_RC=$? ogkm_head=$(git -C /root/ogkm rev-parse HEAD 2>&1)"
DKMS_SRC=/usr/src/nvidia-580.159.04
if [ -d "$DKMS_SRC/nvidia-uvm" ]; then
  if diff -rq /root/ogkm/kernel-open/nvidia-uvm "$DKMS_SRC/nvidia-uvm" > /root/efs/ogkm_vs_dkms.diff 2>&1; then
    echo "SOURCE_MATCH=yes (dkms nvidia-uvm == ogkm 580.159.04 kernel-open/nvidia-uvm)"
  else
    echo "SOURCE_MATCH=no ($(wc -l < /root/efs/ogkm_vs_dkms.diff) differing entries, see ogkm_vs_dkms.diff)"
  fi
else
  echo "SOURCE_MATCH=unknown (no $DKMS_SRC/nvidia-uvm)"
fi

# ---- 6. stock rebuild of nvidia + nvidia-uvm from the DKMS tree (the path build_efs.sh will patch) ---
rm -rf /root/efs/stock && cp -a "$DKMS_SRC" /root/efs/stock
( cd /root/efs/stock && make -j"$(nproc)" modules SYSSRC="/lib/modules/$(uname -r)/build" \
      NV_KERNEL_MODULES="nvidia nvidia-uvm" > /root/efs/stock_build.log 2>&1 )
STOCK_RC=$?; echo "STOCK_BUILD_RC=$STOCK_RC"
ls -l /root/efs/stock/nvidia-uvm.ko 2>&1 | sed 's/^/stock: /'
modinfo /root/efs/stock/nvidia-uvm.ko 2>/dev/null | grep -E "^(version|vermagic)" | sed 's/^/stock: /'

nvidia-smi --query-gpu=name,driver_version,pci.device_id,uuid --format=csv,noheader
df -h / | tail -1
if grep -q "Open Kernel Module.* 580.159.04 " /proc/driver/nvidia/version && [ "$STOCK_RC" = 0 ] \
   && [ -x /usr/local/cuda-12.6/bin/nvcc ]; then
  touch /root/efs/READY; echo "READY"
else
  echo "NOT_READY"
fi
echo "EXIT $(date -Is)"
