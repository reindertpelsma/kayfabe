#!/bin/bash
# Serial: v3 gates, then the GR-tier native oracle (with the §U gate check), at the checked-out commit.
B=/var/lib/kf-windows-20261005
cd $B/kayfabe-deferred-translated
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=$B/target-deferred
OUT=$B/deferred-native-$(git rev-parse --short=8 HEAD)
mkdir -p $OUT
{
echo "DEFERRED_NATIVE_START $(date -Is) rev=$(git rev-parse HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
echo "QEMU_PROCS=$(pgrep -c qemu)"
nvidia-smi --query-gpu=name,driver_version,pstate,temperature.gpu,display_active --format=csv,noheader
echo "XID_DMESG_BEFORE=$(dmesg | grep -c 'NVRM: Xid')"
} > $OUT/host-before.txt 2>&1
bash scripts/bench/v3_gates.sh $OUT/v3_gates.log > /dev/null 2>&1
echo "V3_GATES_RC=$?" >> $OUT/host-before.txt
{
echo "NATIVE_GR_TIER_START $(date -Is) rev=$(git rev-parse HEAD)"
timeout 300 $CARGO_TARGET_DIR/release/kf-gr-tier
echo "NATIVE_GR_TIER_EXIT rc=$? $(date -Is)"
} > $OUT/gr-tier-native.log 2>&1
{
echo "QEMU_PROCS=$(pgrep -c qemu)"
nvidia-smi --query-gpu=name,driver_version,pstate,temperature.gpu,display_active --format=csv,noheader
echo "XID_DMESG_AFTER=$(dmesg | grep -c 'NVRM: Xid')"
dmesg | grep 'NVRM: Xid' | tail -3
echo "DEFERRED_NATIVE_EXIT $(date -Is)"
} > $OUT/host-after.txt 2>&1
