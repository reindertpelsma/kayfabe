#!/bin/bash
# windows_lane.sh -- take a fresh vast KVM box to "a sealed Windows base image and a kf3 QEMU",
# the shortest way, for a lease whose subject is the Windows guest (no Linux suites).
#   KAYFABE_BRANCH=<branch> bash windows_lane.sh          (run ON the box, detached; copy
#   provision_box.sh to /root first, as scripts/bench/box/README.md says)
# Log: /root/prov/winlane.log; every stage prints "STAGE <name> START|EXIT rc=N <utc>", the last
# line is "WINLANE_EXIT rc=N" and a "WINLANE_TIMING" table. A log without WINLANE_EXIT belongs to
# a dead run.
#
# What it saves against provision_full.sh plus a separate `win_vm.sh install` (2026-10-27, TUR1,
# lease 57882297: 25 min Linux provisioning, then a 32 min install that ran beside other work):
#   - the Windows media (5 GB, pinned by sha256) download while the host driver and the QEMU
#     build run;
#   - no thin-guest image, no raw client, no Linux suites;
#   - the install gets every core (WINVM_SMP, at most 16) and runs its disk with cache=unsafe
#     (WINVM_INSTALL_CACHE; install-time boots only, the base image is sealed after a clean
#     shutdown); WINLANE_INSTALL_CACHE= (set, empty) turns that off.
# What it does NOT do: restore a prebuilt Windows image. win_vm.sh never restores a VM from another
# VM's state (the swtpm state, the harness ssh key and the Windows password are per VM), and the
# private image store is write-restricted and its licence review is open (README of the store).
set -u
mkdir -p /root/prov
exec > /root/prov/winlane.log 2>&1
T0=$(date +%s)
TIMING=()
BRANCH=${KAYFABE_BRANCH:?KAYFABE_BRANCH}
export KAYFABE_BRANCH=$BRANCH
HOST_DRIVER=${HOST_DRIVER:-595.91.07}
export HOST_DRIVER KF_HOST_DRIVER=$HOST_DRIVER
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-$(nproc)}
export PATH=$HOME/.cargo/bin:$PATH
export WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm}
# a per-VM SMBIOS identity from the first boot on (an installed VM must not change
# identity later), and a bound on the install wait: a lease is shorter than win_vm.sh's
# 4 h default (docs/design/V3_WINDOWS_ON_VAST.md 3.1)
export WINVM_VM_IDENTITY=${WINVM_VM_IDENTITY:-1}
export WINVM_INSTALL_WAIT_S=${WINVM_INSTALL_WAIT_S:-4200}
GUEST_RUN=/root/NVIDIA-Linux-x86_64-580.159.04.run
rc_all=0

stage() { # stage NAME CMD...: run, time and record one stage; a failing stage stops the lane
  local name=$1 t rc
  shift
  t=$(date +%s)
  echo "STAGE $name START $(date -u -Is)"
  "$@"
  rc=$?
  echo "STAGE $name EXIT rc=$rc $(date -u -Is)"
  TIMING+=("$name=$(($(date +%s) - t))s")
  [ "$rc" = 0 ] || { rc_all=$rc; finish; }
}

finish() {
  echo "WINLANE_TIMING total=$(($(date +%s) - T0))s ${TIMING[*]}"
  echo "WINLANE_EXIT rc=$rc_all $(date -u -Is)"
  exit "$rc_all"
}

box() { bash /root/provision_box.sh > /root/prov/box.log 2>&1; }
driver() {
  (cd /root/kayfabe && bash scripts/bench/provision_host_driver.sh > /root/prov/driver.log 2>&1)
}
guest_run() { # the bench tree wants the guest's pinned package at a fixed path
  [ -s "$GUEST_RUN" ] && return 0
  local base=https://us.download.nvidia.com/XFree86/Linux-x86_64/580.159.04
  curl --fail --location --retry 3 --connect-timeout 20 -o "$GUEST_RUN.part" \
    "$base/$(basename "$GUEST_RUN")" && mv "$GUEST_RUN.part" "$GUEST_RUN"
}
tree() {
  (cd /root/kayfabe && bash scripts/bench/provision_bench_tree.sh > /root/prov/tree.log 2>&1)
}
kf3() {
  (cd /root/kayfabe && bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 \
    /workspace/bench/qemu-build-kf3 > /root/prov/kf3.log 2>&1)
}
prereqs() {
  DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=300 install -y -qq \
    ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full \
    > /root/prov/win_prereq.log 2>&1
}
install_windows() {
  local smp rev
  smp=$(nproc)
  [ "$smp" -le 16 ] || smp=16
  rev=$(git -C /root/kayfabe rev-parse --short=8 HEAD)
  (cd /root/kayfabe && WINVM_SMP=$smp WINVM_INSTALL_CACHE=${WINLANE_INSTALL_CACHE-unsafe} \
    WINVM_SSH_CONNECT_TIMEOUT=100 WINVM_QEMU=/workspace/bench/kf3-bins/$rev/qemu-system-x86_64 \
    bash scripts/bench/windows/win_vm.sh install > /root/prov/win_install.log 2>&1)
}

stage box box
# the media fetch needs only the checkout; it runs beside everything that follows
(cd /root/kayfabe && bash scripts/bench/windows/win_vm.sh fetch > /root/prov/win_fetch.log 2>&1
  echo "FETCH_EXIT rc=$? $(date -u -Is)" >> /root/prov/win_fetch.log) &
FETCH_PID=$!
stage guest_run guest_run
stage driver driver
stage tree tree
stage kf3 kf3
stage prereqs prereqs
wait "$FETCH_PID"
grep -q "FETCH_EXIT rc=0" /root/prov/win_fetch.log \
  || { rc_all=3; echo "media fetch failed"; finish; }
stage install_windows install_windows
finish
