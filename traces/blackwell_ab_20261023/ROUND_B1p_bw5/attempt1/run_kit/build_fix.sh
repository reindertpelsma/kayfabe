#!/bin/bash
# BW5 B1p: build the pinned fix revision (fix_pin.json) on the box. The worktree is checked to be
# the exact clean pinned chain, the GPU-free tests must pass BEFORE the kf3 build, and the receipt
# records the binary's sha256 only when it carries the route log line. Revision-private target.
set -euo pipefail
exec >/root/prov/observed_fix_build.log 2>&1
trap 'rc=$?; echo "OBSERVED_FIX_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH
pin=/root/sweep2/fix_pin.json
fix=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["source"])' "$pin")
[[ $fix =~ ^[0-9a-f]{40}$ ]]
work=/workspace/bw-fix
git -C /root/kayfabe fetch --depth 8 origin "$fix"
git -C /root/kayfabe worktree add --detach "$work" "$fix"
python3 /root/sweep2/fix_build.py source "$pin" "$work"
cd "$work"
export CARGO_TARGET_DIR=/workspace/fix-target CARGO_BUILD_JOBS=8
df -h /
crates="-p kf-chip -p kf-gsp -p kf-chan -p kf-qemu -p kf-rm"
# shellcheck disable=SC2086
KAYFABE_NO_KVM=1 nice -n 19 cargo test -j8 --release $crates >/root/prov/observed_fix_tests.log 2>&1
python3 /root/sweep2/fix_build.py tests "$pin" /root/prov/observed_fix_tests.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 \
 /workspace/bench/qemu-build-fix >/root/prov/observed_fix_qemu_build.log 2>&1
python3 /root/sweep2/fix_build.py receipt "$pin" "$work" /workspace/bench \
 /root/prov/observed_fix_tests.log /root/prov/observed_fix_qemu_build.log \
 /root/prov/observed_fix_binary.json
