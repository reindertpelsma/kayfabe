#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/sweep_windows_install.log 2>&1
trap 'rc=$?; echo "WIN_BASE_INSTALL_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WIN_BASE_INSTALL_START pinned-QEMU/std-VGA baseline preparation $(date -u -Is)"
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = 1700543393c34f93adc0611430e3d49ab38eca3a ]
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=300 install -y -qq ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full >/root/prov/sweep_windows_prereq_install.log 2>&1
# This prepares a sealed generic LTSC guest. The install/check boots use standard VGA;
# real kayfabe validation later uses the pinned per-revision product QEMU and19flags.
export WINVM_ROOT=/workspace/sweep-win-57193877 WINVM_QEMU=/workspace/bench/kf3-bins/17005433/qemu-system-x86_64
export WINVM_RAM_MB=8192 WINVM_SMP=8
[ -x "$WINVM_QEMU" ]
# Media fetch is independently pinned; wait only for its own bounded active job.
for i in $(seq 1 120); do
  grep -q '^WIN_MEDIA_EXIT rc=0 ' /root/prov/sweep_windows_media.log && break
  kill -0 "$(cat /root/prov/sweep_windows_media.pid)" || { echo MEDIA_FETCH_FAILED; exit 3; }
  sleep 5
done
grep -q '^WIN_MEDIA_EXIT rc=0 ' /root/prov/sweep_windows_media.log
bash scripts/bench/windows/win_vm.sh install
