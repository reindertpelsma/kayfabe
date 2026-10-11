#!/usr/bin/env bash
# Exact89e8ddf8 Linux regression + allocator smokes + matched Gravity control/probe.
# Trusted172.22.1.20 only. Caller holds shared GPU lock; never use owner1380 disk.
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-late-fifo-host-89e8ddf8
REV=89e8ddf8
ROOT=$W/codex-late-fifo-20261011-$REV
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
POST=/tmp/kf-late-fifo-post-89e8ddf8.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
export PATH="$HOME/.cargo/bin:$PATH"
# Reuse our own incremental compile cache; installed binaries remain per revision.
export CARGO_TARGET_DIR=/data/kf-prod-defaults-target-3252a087
export BENCH_DIR=$ROOT/linux/bench KF_FASTGUEST_DIR=$ROOT/linux/fastguest
export KF_LOCK=$ROOT/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$ROOT/linux/bench" "$ROOT/linux/logs"
test "$(git -C "$TREE" rev-parse HEAD)" = 89e8ddf8b55d0b501d836c95ce40b3114eecab8e
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test -f "$QEMU_BIN"
grep -q 'V3_GATES_EXIT pass=9 fail=0 births_ok=1 gate10=PASS' "$ROOT/linux/logs/gates.log"
test ! -e "$ROOT/RESUME_START"
date -Is > "$ROOT/RESUME_START"
echo "LATE_FIFO_RESUME_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
cleanup() {
    rc=$?; trap - EXIT
    # Only this campaign's explicit run IDs; owner1380 is never named here.
    for n in {1490..1499} {1510..1514}; do
        test -e "$W/windows-broker-run$n.state" || continue
        bash "$B" stop "$n" || true
    done
    bash "$POST" || true
    echo "LATE_FIFO_CAMPAIGN_EXIT=$rc at=$(date -Is)"
    echo "$rc" > "$ROOT/EXIT"
}
trap cleanup EXIT
cd "$TREE"
echo 'HYPOTHESIS Linux fast paths remain correct on exact89e8ddf8.'
echo 'FALSIFIER any actually started fast arm fails; output-dir precondition failure was NOTRUN.'
mv "$ROOT/linux/logs/fast.log" "$ROOT/linux/logs/fast-notrun.log"
bash scripts/fastguest/fast_suite.sh codexlatefifo 180 > "$ROOT/linux/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN" > "$ROOT/linux/binary.sha256"
echo "LATE_FIFO_LINUX_EXIT=0 at=$(date -Is)"
test ! -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_DROP=KF3_TIMER_MAP WIN_DISABLE_MPO=1
export PROD_EXTRA_FLAGS= WIN_FLAGS=
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
for lane in smoke control probe; do
    case "$lane" in
        smoke)
            apps=glgears_wgl,vectorAdd_demo,cup8_ladder,torch_ai_bench; base=1490; per=4
            export WIN_FLAGS=
            echo 'HYPOTHESIS exact revision boots and actual-demand allocator permits original torch workload.'
            echo 'FALSIFIER failed original proof/readback, capacity refusal, recovery or host Xid.' ;;
        control)
            apps=gravitymark_vk; base=1495; per=1
            export WIN_FLAGS=
            echo 'HYPOTHESIS original GravityMark Vulkan failure remains on matched current production baseline.'
            echo 'FALSIFIER original600s workload clean-PASS with image/timing/GPU proof.' ;;
        probe)
            apps=gravitymark_vk; base=1510; per=1
            export WIN_FLAGS=KF3_RELAY_PB_PEEK=after-fifo-update
            echo 'HYPOTHESIS late submitted packet on faulting twin contains0x200 with registered FIFO-update API.'
            echo 'FALSIFIER positively identified faulting method differs; absent/bounded/racy sample is INCONCLUSIVE.'
            echo 'ONE_VARIABLE only KF3_RELAY_PB_PEEK differs from matched control; no hardware-execution inference.' ;;
    esac
    start=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
    echo "LATE_FIFO_LANE_START=$lane at=$(date -Is)"
    bash scripts/bench/windows/appmatrix/run_windows_apps.sh --run-dir "$ROOT/$lane" \
        --kf3-rev "$REV" --broker "$B" --run-base "$base" \
        --override-dir "$W/codex-matrix-focus-tools-20261011" --iso "$MEDIA" \
        --apps "$apps" --tier 3 --per-guest "$per" --no-isolate \
        > "$ROOT/$lane-console.log" 2>&1
    python3 /tmp/kf-capture-xids.py "$ROOT/$lane" "$start"
    echo "LATE_FIFO_LANE_EXIT=$lane rc=0 at=$(date -Is)"
done
