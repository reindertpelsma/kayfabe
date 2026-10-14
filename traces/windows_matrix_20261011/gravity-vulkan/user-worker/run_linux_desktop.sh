#!/usr/bin/env bash
# Caller holds the global GPU lock with flock -o. Disposable Linux child only.
set -euo pipefail
REV=${1:?full source revision required}
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
W=/var/lib/kf-windows-20261005
TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=$W/codex-linux-desktop-${REV:0:9}
test "$(git -C "$TREE" rev-parse HEAD)" = "$REV"
test -z "$(git -C "$TREE" status --porcelain --untracked-files=no)"
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT"
mkdir -p "$ROOT"
START=$(date -Is)
echo "$START" > "$ROOT/START"
echo "HYPOTHESIS reviewed exact source preserves the provisioned Linux Cinnamon/Xorg desktop and OpenGL gears smoke."
echo "FALSIFIER boot/SSH fails, Cinnamon/Xorg absent, renderer is software, gears reports no frames, or guest/host has an unexpected fault. This is not render-hash parity or a relay-fix test."
launcher=
finish() {
    rc=$?; trap - EXIT
    if [ -n "$launcher" ] && kill -0 "$launcher" 2>/dev/null; then
        kill -TERM "$launcher" 2>/dev/null || true
        wait "$launcher" || true
    fi
    rm -f /tmp/kf-stop-winprod
    journalctl -k --since "$START" --until "$(date -Is)" --no-pager \
        | sed -n '/NVRM: Xid/p' > "$ROOT/host-xid.log" || true
    pgrep -a qemu-system > "$ROOT/remaining-qemu.log" || true
    cat /sys/kernel/iommu_groups/11/type > "$ROOT/iommu-after.txt"
    echo "$rc" > "$ROOT/EXIT"
    echo "LINUX_DESKTOP_EXIT=$rc at=$(date -Is) source=$REV"
}
trap finish EXIT
qemu-img create -q -f qcow2 -F qcow2 \
    -b "$W/broker-interactive/desktop.qcow2" "$ROOT/desktop.qcow2"
test ! -e /workspace/bench/kf3-bins/${REV:0:8}
ln -s "$W/readtrace/kf3-bins/${REV:0:8}" /workspace/bench/kf3-bins/${REV:0:8}
env KF_INTERACTIVE_DIR="$ROOT" KF_RUN_DIR="$ROOT/run" KF_IMG="$ROOT/desktop.qcow2" \
    KF3_REV="${REV:0:8}" KF_LOCK="$ROOT/private.lock" \
    bash "$TREE/scripts/bench/display/interactive.sh" > "$ROOT/launcher.log" 2>&1 &
launcher=$!
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
ready=0
for _ in $(seq 1 100); do
    kill -0 "$launcher" 2>/dev/null || { tail -10 "$ROOT/launcher.log"; exit 2; }
    if timeout 8 "${G[@]}" true 2>/dev/null; then ready=1; break; fi
    sleep 3
done
test "$ready" = 1
timeout 60 "${G[@]}" 'bash -s' > "$ROOT/guest-smoke.log" 2>&1 <<'GUEST'
set -euo pipefail
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 1 20); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
pgrep -a -x cinnamon
pgrep -a -x Xorg
glxinfo -B
set +e
timeout 20 stdbuf -oL glxgears > /tmp/kf-desktop-gears.log 2>&1
rc=$?
set -e
cat /tmp/kf-desktop-gears.log
test "$rc" = 124
grep -q 'frames in' /tmp/kf-desktop-gears.log
sudo dmesg | tail -150
echo LINUX_DESKTOP_GUEST_SMOKE=PASS
GUEST
grep -q '^LINUX_DESKTOP_GUEST_SMOKE=PASS$' "$ROOT/guest-smoke.log"
! grep -qiE 'llvmpipe|softpipe|NVRM: Xid' "$ROOT/guest-smoke.log"
if grep -qiE 'nvAssert|Flip event timeout|\*ERROR\*' "$ROOT/guest-smoke.log"; then
    echo LINUX_DESKTOP_SMOKE=PASS_WITH_WARNINGS
else
    echo LINUX_DESKTOP_SMOKE=PASS
fi
