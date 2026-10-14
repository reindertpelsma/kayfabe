#!/bin/bash
# Host-side: check out the pushed branch head in the ceint worktree and build the stamped client.
set -uo pipefail
export PATH=/root/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-ceint
git fetch -q origin claude/rawclient-ce-interrupt-20261008
git checkout -q -f --detach FETCH_HEAD
R=$(git rev-parse --short=8 HEAD)
echo "REV=$R"
KAYFABE_BUILD_REV=$R CARGO_BUILD_JOBS=12 CARGO_TARGET_DIR=$W/ceint-target cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder 2>&1 | tail -1
mkdir -p $W/ceint-bins/$R
cp $W/ceint-target/release/kayfabe-rm-ladder $W/ceint-bins/$R/
sha256sum $W/ceint-bins/$R/kayfabe-rm-ladder | cut -c1-16
pgrep -a qemu-system | cut -c1-70
nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader
