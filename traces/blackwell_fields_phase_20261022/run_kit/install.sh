#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/sweep_windows_install.log 2>&1
trap 'rc=$?; echo "WIN_BASE_INSTALL_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WIN_BASE_INSTALL_START pinned-QEMU/std-VGA baseline preparation $(date -u -Is)"
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b ]
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=300 install -y -qq ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full >/root/prov/sweep_windows_prereq_install.log 2>&1
# This prepares a sealed generic LTSC guest. The install/check boots use standard VGA;
# real kayfabe validation later uses the pinned per-revision product QEMU and19flags.
export WINVM_ROOT=/workspace/bw-capc-validation WINVM_QEMU=/workspace/bench/kf3-bins/6e3fd3fd/qemu-system-x86_64
export WINVM_RAM_MB=8192 WINVM_SMP=8
[ -x "$WINVM_QEMU" ]
# Media fetch is independently pinned; wait only for its own bounded active job.
python3 -E -s /root/sweep2/guard/blackwell/media_wait.py /root/sweep2/lease.json /root/prov/sweep_windows_media.log /root/prov/sweep_windows_media.pid
bash scripts/bench/windows/win_vm.sh install
