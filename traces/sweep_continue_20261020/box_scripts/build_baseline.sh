#!/bin/bash
set -euo pipefail
. /root/sweep2/common.sh
exec >/root/prov/build_sibling.log 2>&1
trap 'rc=$?;echo "SIBLING_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
echo 'H_OLUT controlled sibling baseline: source90e7f2a54 production048c, no native-fixture change relevant to product. No GPU workload while building.'
git fetch --depth 1 origin 90e7f2a54dcf4a885d5d1bb7cd442a8b96309686
git worktree add --detach /root/sweep-sibling-wt 90e7f2a54dcf4a885d5d1bb7cd442a8b96309686
export CARGO_TARGET_DIR=/root/sweep-sibling-target CARGO_BUILD_JOBS=8
bash /root/sweep-sibling-wt/scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3
sha256sum /workspace/bench/kf3-bins/90e7f2a5/qemu-system-x86_64 >/root/prov/sibling_qemu_sha256
