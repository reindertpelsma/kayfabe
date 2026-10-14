#!/bin/bash
# Host-side: sync the pti worktree to the pushed branch and run the GPU-free checks.
set -uo pipefail
export PATH=/root/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/pti-20261008
git fetch -q origin claude/passthrough-interrupt-20261008
git checkout -q -f --detach FETCH_HEAD
R=$(git rev-parse --short=8 HEAD)
echo "REV=$R"
export CARGO_BUILD_JOBS=12 CARGO_TARGET_DIR=$W/pti-target
echo "== fmt"; cargo fmt --all --check 2>&1 | head -30; echo "fmt-rc=${PIPESTATUS[0]}"
echo "== tests"
for c in kf-chan kf-rm kf-abi; do
  cargo test -q -p $c 2>&1 | grep -E "^test result|FAILED|panicked|error(\[|:)" | sort | uniq -c | head -20
done
echo "== kf-qemu build"
cargo build --release -p kf-qemu 2>&1 | grep -E "^(error|warning)" -A6 | head -60
echo "build-rc=${PIPESTATUS[0]}"
echo "CHECK_DONE $R"
