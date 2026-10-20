#!/usr/bin/env bash
set -euo pipefail
. /root/bwr/common.sh
exec >/root/prov/bwr_install.log 2>&1
trap 'rc=$?; echo "WIN_BASE_INSTALL_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WIN_BASE_INSTALL_START std-VGA generic LTSC $(date -u -Is)"
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = $BASE ]
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=300 install -y -qq ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full >/root/prov/bwr_prereq_install.log 2>&1
export WINVM_QEMU=/workspace/bench/kf3-bins/7f22d18d/qemu-system-x86_64
[ -x "$WINVM_QEMU" ]
for i in $(seq 1 360); do
  grep -q '^WIN_MEDIA_EXIT rc=0 ' /root/prov/bwr_media.log && break
  kill -0 "$(cat /root/prov/bwr_media.pid)" || { echo MEDIA_FETCH_FAILED; exit 3; }
  sleep 5
done
grep -q '^WIN_MEDIA_EXIT rc=0 ' /root/prov/bwr_media.log
bash scripts/bench/windows/win_vm.sh install
