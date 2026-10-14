#!/bin/bash
cd /root/nvkvm-kata
bash scripts/nvkvm-kata-install.sh --only kernel \
  --nvkvm-src /root/nvkvm-pv-kata --kata-src /root/kata-containers \
  --qemu /opt/qemu-nvkvm/bin/qemu-system-x86_64 --jobs 8 --install-deps --yes
echo "KERNEL_STAGE_EXIT=$?"
