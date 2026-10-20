#!/bin/bash
set -euo pipefail
. /root/bwr/common.sh
exec >/root/prov/bwr_candidate_build.log 2>&1
trap 'rc=$?; echo "CANDIDATE_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
export CARGO_BUILD_JOBS=48
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = $BASE ]
git fetch -q origin codex/sweep-nvdec-abi-20261020
git cat-file -e $CAND
git diff --name-only HEAD $CAND -- crates qemu cuda broker Cargo.toml Cargo.lock | tee /root/prov/bwr_candidate_scope.txt
[ "$(git diff --name-only HEAD $CAND -- crates qemu cuda broker Cargo.toml Cargo.lock | tr '\n' ' ')" = "crates/kf-abi/src/capability.rs crates/kf-abi/tests/capability_policy_550.rs crates/kf-chip/src/classes.rs " ]
[ -e /root/kayfabe-cand ] || git worktree add --detach /root/kayfabe-cand $CAND
export CARGO_TARGET_DIR=/root/bwr-cand-target
bash /root/kayfabe-cand/scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3-cand
sha256sum /workspace/bench/kf3-bins/7f22d18d/qemu-system-x86_64 /workspace/bench/kf3-bins/aa8fdea9/qemu-system-x86_64 | tee /root/prov/bwr_binary_hashes.txt
echo "CANDIDATE_SEPARATE_BINARY_READY $(date -u -Is)"
