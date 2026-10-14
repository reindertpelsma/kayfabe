#!/bin/bash
# Host-side, GPU-free: check out the branch tip in /root/kf-nsi-nogate-20261008, build the stamped raw
# client, the kf3 device for this revision, and the fast-guest initrd carrying that client.
set -uo pipefail
export PATH=/root/.cargo/bin:$PATH
W=/var/lib/kf-nsi-20261008
K=/var/lib/kf-windows-20261005
mkdir -p $W/bins $W/bench
cd /root/kf-nsi-nogate-20261008
git fetch -q origin claude/passthrough-nsi-nogate-20261008
git checkout -q -f --detach FETCH_HEAD
R=$(git rev-parse --short=8 HEAD)
echo "BUILD_START $(date -Is) rev=$R"
export CARGO_BUILD_JOBS=12
echo "== raw-client tests"; cargo test -p kayfabe-isolate-host --lib -- osevent hostabi 2>&1 | grep "test result\|FAILED\|panicked"
echo "== raw client"
KAYFABE_BUILD_REV=$R CARGO_TARGET_DIR=$W/target cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder 2>&1 | tail -1
mkdir -p $W/bins/$R && cp $W/target/release/kayfabe-rm-ladder $W/bins/$R/ && sha256sum $W/bins/$R/kayfabe-rm-ladder | cut -c1-16
echo "== kf3"
bash scripts/bench/build_kf3.sh $K/qemu-10.2.4 > $W/buildkf3-$R.log 2>&1; echo "build_kf3 rc=$?"; grep KF3_BUILT $W/buildkf3-$R.log
echo "== initrd"
PATH=/tmp/ceint-shim:$PATH KF_FROM_HOST=1 CLIENT=$W/bins/$R/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 $W/fastguest-$R > $W/initrd-$R.log 2>&1; echo "initrd rc=$?"; tail -2 $W/initrd-$R.log
ls -la $W/fastguest-$R
echo "BUILD_DONE $(date -Is) rev=$R"
