#!/bin/bash
# l2_fastguest.sh <worktree>: rebuild the raw client and the fast guest (host mode: the host's own nvidia modules) once, for both revisions.
set -uo pipefail
W=$1; O=/data/claude-candhw/ev/l2
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=/data/claude-candhw/target CARGO_BUILD_JOBS=12
cd "$W"
echo "FG_START rev=$(git rev-parse --short=8 HEAD) $(date -Is)"
nice -n 10 cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > $O/client_build.log 2>&1
echo "CLIENT rc=$? sha256=$(sha256sum $CARGO_TARGET_DIR/release/kayfabe-rm-ladder | cut -c1-16)"
KF_FROM_HOST=1 CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /data/claude-candhw/bench/fastguest > $O/fg_build.log 2>&1
echo "FG rc=$? $(tail -1 $O/fg_build.log | cut -c1-200)"
ls -la /data/claude-candhw/bench/fastguest
echo "FG_EXIT $(date -Is)"
