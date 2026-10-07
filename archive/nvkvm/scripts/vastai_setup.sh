#!/usr/bin/env bash
# vastai_setup.sh — configure a fresh vast.ai node for nvkvm development
# Run as root on the vast.ai host after SSH'ing in.

set -euo pipefail

echo "=== nvkvm vast.ai node setup ==="

# ── Install host dependencies ──────────────────────────────────────────────
apt-get update -q
apt-get install -y \
    build-essential \
    linux-headers-$(uname -r) \
    qemu-kvm \
    qemu-system-x86 \
    libvirt-daemon-system \
    ovmf \
    wget \
    curl \
    git \
    python3-pip \
    kmod \
    pciutils \
    nvme-cli

# ── Verify NVIDIA driver is present ───────────────────────────────────────
echo "Host NVIDIA driver:"
nvidia-smi -L || { echo "ERROR: nvidia-smi not found"; exit 1; }

# ── Verify KVM support ─────────────────────────────────────────────────────
if [ ! -e /dev/kvm ]; then
    echo "ERROR: /dev/kvm not present. Ensure KVM is enabled in vast.ai instance settings."
    exit 1
fi
echo "KVM: /dev/kvm present"

# Check for nested virtualization
NESTED=$(cat /sys/module/kvm_intel/parameters/nested 2>/dev/null || \
         cat /sys/module/kvm_amd/parameters/nested 2>/dev/null || echo "N")
echo "Nested virt: $NESTED"

# ── Download Ubuntu cloud image for guest VM ───────────────────────────────
UBUNTU_IMG=/var/lib/libvirt/images/ubuntu-22.04-nvkvm.qcow2
if [ ! -f "$UBUNTU_IMG" ]; then
    echo "Downloading Ubuntu 22.04 cloud image..."
    wget -q -O /tmp/ubuntu-22.04.img \
        https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img
    qemu-img convert -f qcow2 -O qcow2 /tmp/ubuntu-22.04.img "$UBUNTU_IMG"
    qemu-img resize "$UBUNTU_IMG" +30G
fi

# ── Build the guest kernel module ─────────────────────────────────────────
echo "Building nvkvm-guest kernel module..."
cd /workspace/nvidia-gpu-passthrough/src/guest
make KDIR=/lib/modules/$(uname -r)/build
echo "Module built: $(ls -la nvkvm-guest.ko)"

# ── Set up cloud-init config for guest VM ─────────────────────────────────
CLOUD_INIT_DIR=/tmp/nvkvm-cloud-init
mkdir -p "$CLOUD_INIT_DIR"

cat > "$CLOUD_INIT_DIR/user-data" <<'CLOUDINIT'
#cloud-config
users:
  - name: nvkvm
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    lock_passwd: false
    passwd: "$6$rounds=4096$nvkvm$nvkvm"  # password: nvkvm

packages:
  - linux-headers-virtual
  - python3-pip
  - nvidia-cuda-toolkit

runcmd:
  - pip3 install cupy-cuda12x torch torchvision --extra-index-url https://download.pytorch.org/whl/cu121
  - pip3 install pynvml
  - echo "nvkvm-guest" >> /etc/modules
CLOUDINIT

cat > "$CLOUD_INIT_DIR/meta-data" <<METADATA
instance-id: nvkvm-test-01
local-hostname: nvkvm-guest
METADATA

genisoimage -output "$CLOUD_INIT_DIR/seed.iso" \
    -volid cidata -joliet -rock \
    "$CLOUD_INIT_DIR/user-data" \
    "$CLOUD_INIT_DIR/meta-data" \
    2>/dev/null

echo ""
echo "=== Setup complete ==="
echo ""
echo "To start a test VM:"
echo "  ./scripts/run_test_vm.sh"
echo ""
echo "Host GPU info:"
nvidia-smi
