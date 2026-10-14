#!/bin/bash
# Host-side: the final-revision cycle (GPU-free parts): checkout, tests, fmt check, gates, clippy, stamped build, initrd.
set -uo pipefail
export PATH=/root/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-ceint
git fetch -q origin claude/rawclient-ce-interrupt-20261008
git checkout -q -f --detach FETCH_HEAD
R=$(git rev-parse --short=8 HEAD)
echo "REV=$R"
export CARGO_BUILD_JOBS=12 CARGO_TARGET_DIR=$W/ceint-target
echo "== fmt"; cargo fmt -p kayfabe-isolate-host -p kayfabe-rm-ladder --check && echo fmt-clean
echo "== tests"; cargo test -p kayfabe-isolate-host --lib -- osevent hostabi 2>&1 | grep "test result\|FAILED\|panicked"
echo "== ci_gates"; bash scripts/ci_gates.sh > $W/ceint-gates-$R.log 2>&1; grep -n "claims:\|AT LEAST\|ALL GATES" $W/ceint-gates-$R.log
echo "== clippy"; bash scripts/ci/clippy.sh 2>&1 | tail -2 | cut -c1-200
echo "== build"; KAYFABE_BUILD_REV=$R cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder 2>&1 | tail -1
mkdir -p $W/ceint-bins/$R; cp $W/ceint-target/release/kayfabe-rm-ladder $W/ceint-bins/$R/
echo "== initrd"
PATH=/tmp/ceint-shim:$PATH KF_FROM_HOST=1 CLIENT=$W/ceint-bins/$R/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 $W/ceint-fastguest-$R 2>&1 | tail -2
echo "== kf3 binary for the guest: $W/kf3-bins/6d9e6a76 (same kf-* sources as $R; only kayfabe-* crates differ)"
echo "FINAL_DONE $R"
