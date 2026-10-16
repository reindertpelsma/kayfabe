#!/bin/bash
# l2_suite.sh <worktree> <tag> [budget]  -- one fast suite (30 arms), checkout-bound binary
# (run_fast_guest.sh picks $BENCH/kf3-bins/<HEAD rev>), a private run lock because the outer `flock -o`
# holds /tmp/kayfabe-fastguest.lock (the rootless audit's F1 work-around).
W=$1; TAG=$2; BUDGET=${3:-180}
export BENCH_DIR=/data/claude-life/bench KF_LOCK=/data/claude-life/fast.lock
EV=/data/claude-life/ev/l2; mkdir -p "$EV"
cd "$W" || exit 3
rev=$(git rev-parse --short=8 HEAD)
bash /data/claude-life/wt-ev/scripts/bench/lifecycle_hw/host_state.sh "pre_$TAG" "$EV" > /dev/null
rm -f "$EV/stop_$TAG"
bash /data/claude-life/wt-ev/scripts/bench/lifecycle_hw/qsampler.sh "$EV/qemus_$TAG.txt" "$EV/stop_$TAG" &
SP=$!
echo "SUITE_START tag=$TAG rev=$rev $(date -Is)" | tee "$EV/run_$TAG.txt"
flock -o /tmp/kayfabe-fastguest.lock bash scripts/fastguest/fast_suite.sh "$TAG" "$BUDGET" > "$EV/suite_$TAG.stdout" 2>&1
rc=$?
echo "SUITE_EXIT tag=$TAG rc=$rc $(date -Is)" | tee -a "$EV/run_$TAG.txt"
touch "$EV/stop_$TAG"; wait $SP
bash /data/claude-life/wt-ev/scripts/bench/lifecycle_hw/host_state.sh "post_$TAG" "$EV" | tee -a "$EV/run_$TAG.txt"
diff <(sed 's/^\[[ 0-9.]*\] //' "$EV/xid_pre_$TAG.txt") <(sed 's/^\[[ 0-9.]*\] //' "$EV/xid_post_$TAG.txt") > "$EV/xid_new_$TAG.txt"
echo "XID_NEW_LINES=$(grep -c '^>' "$EV/xid_new_$TAG.txt")" | tee -a "$EV/run_$TAG.txt"
cp "$BENCH_DIR/${TAG}_suite.out" "$EV/" 2>/dev/null
