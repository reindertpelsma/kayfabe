#!/bin/bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# One stage of the rootless proof (traces/rootless_proof_20261125/PREREGISTRATION.md). Runs AS THE DROPPED USER (started by
# run_proof.py through kfroot.py run); prints its own identity and capability sets first and an exit line last.
set -uo pipefail
echo "ROOTLESS_PROC stage=$1 id=$(id) memlock_kb=$(ulimit -l)"
grep -E '^(Uid|Gid|Groups|CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs|Seccomp):' /proc/self/status
export HOME=/home/kfrun BENCH_DIR=/workspace/bench BENCH=/workspace/bench KF_DEVICE=kf3
export PATH=/home/kfrun/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export WINVM_ROOT=/workspace/winvm WINVM_VM_IDENTITY=1 WINVM_INSTALL_WAIT_S=4200 KF_GUEST_USER=kf
cd /workspace/kf-src || exit 1
rc=0
case "$1" in
    toolchain)
        d=$(mktemp -d)
        python3 -I scripts/ci/kf_trust.py fetch rustup-init-1.29.3 --out "$d/rustup-init" || rc=$?
        [ "$rc" != 0 ] || { chmod 0700 "$d/rustup-init" && "$d/rustup-init" -y --default-toolchain none --profile minimal > "$d/log" 2>&1 || rc=$?; }
        pin=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
        [ "$rc" != 0 ] || rustup toolchain install "$pin" --profile minimal --target x86_64-unknown-linux-musl,x86_64-unknown-uefi > "$d/log2" 2>&1 || rc=$?
        tail -3 "$d/log2" 2> /dev/null; cargo --version || rc=$? ;;
    build) bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /workspace/rlprov/build_kf3.log 2>&1 || rc=$?
        tail -3 /workspace/rlprov/build_kf3.log ;;
    arm) KF_ARMS=--timer bash scripts/fastguest/run_fast_guest.sh "$2" 180 || rc=$? ;;
    suite) bash scripts/fastguest/fast_suite.sh "$2" 180 || rc=$?
        grep -aE '^FAST_SUITE_(PASS|FAIL|CRASH)' "/workspace/bench/${2}_suite.out" ;;
    rerun) shift; bash scripts/fastguest/fast_suite.sh "$@" || rc=$? ;;
    wininstall) WINVM_INSTALL_CACHE=unsafe bash scripts/bench/windows/win_vm.sh install || rc=$? ;;
    wingate) KF_ROOTLESS=require KF_WINGATE_IDLE_S=120 bash scripts/bench/box/windows_gate.sh "$2" /workspace/rlprov || rc=$? ;;
    *) echo "unknown stage $1"; rc=2 ;;
esac
echo "ROOTLESS_STAGE_EXIT stage=$1 rc=$rc"
exit "$rc"
