#!/usr/bin/env bash
# On owned box 54137212, after provision_full.sh finishes. No timed GPU run
# overlaps this preparation. Both repositories are pulled from public GitHub.
set -euo pipefail
exec > /root/prov/cand2_display_prep.log 2>&1
echo "START $(date -Is)"
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
# Full provision built successfully but its old helpers looked only under
# repo/target. The box now symlinks that path to the explicit target directory;
# these are the successful tree/fast-guest retries, not the failed READY marker.
grep -q 'BENCH_TREE_DONE' /root/prov/cand2_tree_retry.log
grep -q '^== kernel release: 6.8.0-142-generic$' /root/prov/cand2_fg_retry.log
test -s /workspace/bench/fastguest/initrd.cpio.gz
if pgrep -x qemu-system-x86 || pgrep -x cargo || pgrep -x ninja; then
    echo "another build or guest is active"; exit 2
fi
cd /root/kayfabe
git diff --quiet
git diff --cached --quiet
git fetch origin 9d82f2598d267732c87476da27f501aaabe2c4af
git checkout --detach FETCH_HEAD
export PATH=/root/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/workspace/bench/kf-target-cand2-display CARGO_BUILD_JOBS=20
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 \
    > /root/prov/cand2_display_build.log 2>&1
echo "BUILD_RC=0 $(date -Is)"
bash scripts/bench/provision_guest_gfx.sh > /root/prov/cand2_guest_gfx.log 2>&1
echo "GUEST_GFX_RC=0 $(date -Is)"
bash scripts/bench/provision_guest_display.sh all > /root/prov/cand2_guest_display.log 2>&1
echo "GUEST_DISPLAY_RC=0 $(date -Is)"
bash scripts/bench/display/broker_lane.sh prep 9f2fd00 > /root/prov/cand2_broker_prep.log 2>&1
echo "BROKER_PREP_RC=0 $(date -Is)"
cat /root/prov/cand2_broker_prep.log
