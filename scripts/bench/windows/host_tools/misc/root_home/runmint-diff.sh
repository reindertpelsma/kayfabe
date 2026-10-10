#!/bin/bash
set -u
Q=/root/newqemu/qemu-nvkvm/bin/qemu-system-x86_64
export NVKVM_STUB_PATH=/root/newqemu/lib/nvkvm_stub
export NVKVM_PRESENT_TIMING=1
exec "$Q" -name nvkvm-mint-difftest \
  -enable-kvm -m 12G -smp 8 -cpu host -machine q35 \
  -drive file=/opt/nvkvm-guest/mint-22.3.qcow2,format=qcow2,if=virtio \
  -netdev user,id=net0,hostfwd=tcp::12223-:22 -device virtio-net-pci,netdev=net0 \
  -vga none \
  -device virtio-nvgpu-pci-non-transitional,id=nvkvm0 -device nvkvm-gpu,addr=7 \
  -device virtio-keyboard-pci -device virtio-tablet-pci \
  -serial unix:/root/mint-serial.sock,server=on,wait=off -display none
