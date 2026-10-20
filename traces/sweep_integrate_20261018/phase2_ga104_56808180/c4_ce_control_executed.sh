#!/usr/bin/env bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH CARGO_BUILD_JOBS=8
exec >/root/prov/sweep_c4_ce_guest_ram.log 2>&1
trap 'rc=$?; echo "C4_CE_GUEST_RAM_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "C4_CE_GUEST_RAM_START $(date -u -Is)"
# Falsifier before running: if candidate4 also times out on this same arm, the timeout
# is not unique to the imported stack; its causal performance effect stays unresolved. Keep guest, kernel, driver, grader,
# budget180, integrated launcher, CPU flags, and stdout route unchanged.
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8 ]
grep -q '^LINUX_ARM_EXIT kind=raw version=580 rc=1 ' /root/prov/sweep_linux_raw580.log
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
base=0452613b5c71cb175271230b3e14c8e3c3eef2db
if ! git cat-file -e "$base^{commit}"; then git fetch --depth 1 origin "$base"; fi
git worktree add --detach /root/sweep-candidate4 "$base"
[ "$(git -C /root/sweep-candidate4 rev-parse HEAD)" = "$base" ]
mkdir -p /workspace/bench/sweep-c4-qemu-10.2.4
tar -C /workspace/bench/sweep-c4-qemu-10.2.4 --strip-components=1 -xf /workspace/bench/qemu-10.2.4.tar.xz
export CARGO_TARGET_DIR=/workspace/bench/sweep-c4-target
bash /root/sweep-candidate4/scripts/bench/build_kf3.sh /workspace/bench/sweep-c4-qemu-10.2.4 /workspace/bench/sweep-c4-qemu-build >/root/prov/sweep_c4_build.log 2>&1
sha256sum /workspace/bench/kf3-bins/0452613b/qemu-system-x86_64 >/root/prov/sweep_c4_qemu_sha256
unset CARGO_TARGET_DIR
export KF_DEVICE=kf3 KF_FASTGUEST_DIR=/workspace/bench/fastguest580
export QEMU_BIN=/workspace/bench/kf3-bins/0452613b/qemu-system-x86_64
sha256sum /workspace/bench/fastguest580/initrd.cpio.gz /workspace/bench/fastguest580/vmlinuz >/root/prov/sweep_c4_guest_hashes
# A baseline failure remains data; never change the grader to make this green.
set +e
KF_ARMS=--ce-client-guest-ram bash scripts/fastguest/run_fast_guest.sh sweep_c4_ga104_guest580_ce_guest_ram 180 >/root/prov/sweep_c4_ce_guest_ram_run.log 2>&1
rc=$?
set -e
echo "C4_CE_GUEST_RAM_RUN_RC=$rc"
grep '^== kf3 binary:\|FAST_VERDICT=' /root/prov/sweep_c4_ce_guest_ram_run.log || true
echo "C4_CE_GUEST_RAM_DONE $(date -u -Is)"
