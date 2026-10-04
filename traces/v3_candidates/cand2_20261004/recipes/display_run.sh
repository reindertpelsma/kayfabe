#!/usr/bin/env bash
# One serial run on the owned vdisp2 box. Inspect verdicts, not just EXIT.
set -euo pipefail
kind=${1:?refresh|fps30|x1130|async60|broker|refresh_hint|fifo_timing}
tag=${2:?unique tag}
[[ "$tag" =~ ^[a-zA-Z0-9_-]+$ ]]
test ! -e "/root/prov/$tag.log"
exec > "/root/prov/$tag.log" 2>&1
echo "START kind=$kind tag=$tag $(date -Is)"
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
if pgrep -x qemu-system-x86 || pgrep -x cargo || pgrep -x ninja; then
    echo "another build or guest is active"; exit 2
fi
cd /root/kayfabe
test "$(git rev-parse HEAD)" = 9d82f2598d267732c87476da27f501aaabe2c4af
git diff --quiet
git diff --cached --quiet
export KF_DEVICE=kf3 KF_FIRMWARE=ovmf NVKVM_RAM_MB=8192 KF_SMP=6
export QEMU_BIN=/workspace/bench/kf3-bins/9d82f259/qemu-system-x86_64
export DISPLAY_HOLD_S=10 DISPLAY_FLIPS=120
export PRE_POWEROFF_GUEST_CMD='sudo systemctl stop lightdm 2>/dev/null; sudo sync; echo s | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1; echo u | sudo tee /proc/sysrq-trigger >/dev/null; sleep 1'
case "$kind" in
    refresh)
        export KF3_DEV_EXTRA=display=on,gop=on
        export POST_CAPTURE_HOOK=$PWD/scripts/bench/display/refresh_hook.sh
        timeout --kill-after=10 360 bash scripts/bench/boot_capture.sh "$tag" -- \
            -vga none -qmp "unix:/workspace/bench/run_$tag.qmp,server=on,wait=off"
        grep -q '^REFRESH_VERDICT PASS:' "/workspace/bench/run_${tag}_probe.log"
        ;;
    fps30)
        export KF3_DEV_EXTRA=display=on,gop=on,display-max-fps=30 FPS_BOUND=30
        export POST_CAPTURE_HOOK=$PWD/scripts/bench/display/max_fps_hook.sh
        timeout --kill-after=10 900 bash scripts/bench/boot_capture.sh "$tag" -- \
            -vga none -vnc 127.0.0.1:0 -qmp "unix:/workspace/bench/run_$tag.qmp,server=on,wait=off"
        ;;
    fifo_timing)
        export KF3_DEV_EXTRA=display=on,gop=on,x11-dispsw=on,display-max-fps=30
        export POST_CAPTURE_HOOK=/root/fifo_timing_hook.sh
        timeout --kill-after=10 480 bash scripts/bench/boot_capture.sh "$tag" -- -vga none
        grep -q '^FIFO_TIMING_VERDICT PASS ' "/workspace/bench/run_${tag}_probe.log"
        ;;
    x1130)
        export DISPLAY_KF3_EXTRA=gop=on,x11-dispsw=on,display-max-fps=30 FPS_BOUND=30
        export DISPLAY_HOOK=max_fps_x11_hook DISPLAY_DESKTOP=1 D4_R1B=1
        timeout --kill-after=10 1200 bash scripts/bench/display/lane.sh "$tag"
        ;;
    async60)
        export DISPLAY_KF3_EXTRA=gop=on,display-max-fps=60 FPS_BOUND=60 DISPLAY_ASYNC=1
        timeout --kill-after=10 600 bash scripts/bench/display/lane.sh "$tag"
        ;;
    broker)
        export BRK_KF3_EXTRA=gop=on,x11-dispsw=on BRK_CURSOR=1 BRK_VNC=1 BRK_DRI3=1 BRK_RESILIENCE=1
        export BRK_HOLD=40
        bash /root/d2_watch.sh "$tag" &
        watcher=$!
        timeout --kill-after=10 1200 bash scripts/bench/display/broker_lane.sh run "$tag"
        wait "$watcher"
        ;;
    refresh_hint)
        (
        export R6_FIFO=/run/${tag}_broker.fifo
        socket=/run/${tag}_broker.sock
        test ! -e "$R6_FIFO"
        test ! -e "$socket"
        mkfifo "$R6_FIFO"
        exec 8<>"$R6_FIFO"
        /opt/nvkvm-broker/nvkvm-display-broker --socket "$socket" --backend test \
            --present-mode=shm --persist --verbose <&8 > "/root/prov/${tag}_broker.log" 2>&1 &
        broker_pid=$!
        trap 'kill "$broker_pid" 2>/dev/null || true' EXIT
        for _ in $(seq 50); do [ -S "$socket" ] && break; sleep 0.1; done
        test -S "$socket"
        export KF3_DEV_EXTRA="display=on,gop=on,display-broker=$socket,display-broker-uid=0,display-broker-vram=off"
        export POST_CAPTURE_HOOK=/root/refresh_hint_hook.py
        timeout --kill-after=10 480 bash scripts/bench/boot_capture.sh "$tag" -- -vga none
        grep -q '^R6_VERDICT PASS ' "/workspace/bench/run_${tag}_probe.log"
        )
        ;;
    *) exit 2 ;;
esac
