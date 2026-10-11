#!/usr/bin/env bash
# Caller holds /tmp/kayfabe-fastguest.lock. Exact source, disposable disks only.
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=b081f705
TREE=/data/kf-fifo-refusal-host-$REV
ROOT=$W/codex-fifo-refusal-20261011-$REV
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
POST=/tmp/kf-fifo-refusal-post-b081f705.sh
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/data/kf-fifo-refusal-target-$REV
export BENCH_DIR=$ROOT/linux/bench KF_FASTGUEST_DIR=$ROOT/linux/fastguest
export KF_LOCK=$ROOT/private.lock KF_LOCKED=1 KF_DEVICE=kf3 KF_FROM_HOST=1
export QEMU_BIN=$W/readtrace/kf3-bins/$REV/qemu-system-x86_64
mkdir -p "$ROOT/linux/bench" "$ROOT/linux/logs"
test "$(git -C "$TREE" rev-parse --short=8 HEAD)" = "$REV"
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$ROOT/START"
date -Is > "$ROOT/START"
echo "FIFO_REFUSAL_CAMPAIGN_START=$(date -Is) source=$(git -C "$TREE" rev-parse HEAD)"
cleanup() {
    rc=$?; trap - EXIT
    for n in {1540..1544}; do
        test -e "$W/windows-broker-run$n.state" || continue
        bash "$B" stop "$n" || true
    done
    bash "$POST" || true
    echo "FIFO_REFUSAL_CAMPAIGN_EXIT=$rc at=$(date -Is) (recording status, not app verdict)"
    echo "$rc" > "$ROOT/EXIT"
}
trap cleanup EXIT
python3 /tmp/kf-fifo-refusal-cleanup-b081f705.py
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
cd "$TREE"
echo 'HYPOTHESIS default-off refusal diagnostic preserves Linux behavior.'
echo 'FALSIFIER any gate/census/fast arm fails on exact source.'
bash scripts/bench/v3_gates.sh "$ROOT/linux/logs/gates.log" > "$ROOT/linux/logs/gates-console.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$ROOT/linux/logs/build.log" 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > "$ROOT/linux/logs/client.log" 2>&1
CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh \
    /workspace/bench/guest.qcow2 "$KF_FASTGUEST_DIR" > "$ROOT/linux/logs/fastguest.log" 2>&1
bash scripts/fastguest/fast_suite.sh codexfiforefuse 180 > "$ROOT/linux/logs/fast.log" 2>&1
sha256sum "$QEMU_BIN" > "$ROOT/linux/binary.sha256"
echo "FIFO_REFUSAL_LINUX_EXIT=0 at=$(date -Is)"
test ! -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_DROP=KF3_TIMER_MAP WIN_DISABLE_MPO=1
export PROD_EXTRA_FLAGS= WIN_FLAGS=KF3_WIN_REFUSE_PT_FIFO_UPDATE=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
echo 'HYPOTHESIS Windows uses a valid fallback after NOT_SUPPORTED for PT user FIFO migration registration.'
echo 'FALSIFIER original workload failure, device-lost, refused-handle trigger, hostXid or recovery.'
echo 'ONE_VARIABLE narrow registration-refusal flag; unchanged original600s oracle/media/production flags.'
echo 'COVERAGE no later plain RmControl is NOT no native execution; missing sampler records are NOT zero triggers.'
start=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
echo "FIFO_REFUSAL_WINDOWS_START=$(date -Is)"
bash scripts/bench/windows/appmatrix/run_windows_apps.sh --run-dir "$ROOT/refusal" \
    --kf3-rev "$REV" --broker "$B" --run-base 1540 \
    --override-dir "$W/codex-matrix-focus-tools-20261011" \
    --iso "$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img" \
    --apps gravitymark_vk --tier 3 --per-guest 1 --no-isolate \
    > "$ROOT/refusal-console.log" 2>&1
python3 /tmp/kf-capture-xids.py "$ROOT/refusal" "$start"
echo "FIFO_REFUSAL_WINDOWS_EXIT=0 at=$(date -Is) (read app verdict separately)"
