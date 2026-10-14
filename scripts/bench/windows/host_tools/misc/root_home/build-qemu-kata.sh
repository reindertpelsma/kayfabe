#!/bin/bash
set -x
export NVKVM_QEMU_PREFIX=/opt/qemu-nvkvm-kata
export NVKVM_QEMU_SRC=/root/qemu-src-nvkvm-kata
bash /root/nvkvm-pv-kata/scripts/build_qemu.sh --install-deps
echo "BUILD_QEMU_EXIT=$?"
