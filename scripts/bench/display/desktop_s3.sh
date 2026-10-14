#!/usr/bin/env bash
# desktop_s3.sh <kf3-rev> <tag> — Linux desktop guest (ogkm) on kf3: boot, prove GPU works, S3 suspend, QMP/monitor wake,
# prove GPU works again. Same harness as desktop_smoke.sh (throw-away overlay, private lock/run dir).
set -uo pipefail
REV=${1:?rev}; TAG=${2:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"
W=/var/lib/kf-windows-20261005
ROOT=$W/claude-s3-$TAG
[ -e "$ROOT" ] && { echo "S3_FAIL $ROOT exists"; exit 2; }
mkdir -p "$ROOT"; echo "S3_START tag=$TAG rev=$REV $(date -Is)" | tee "$ROOT/START"
[ -x "/workspace/bench/kf3-bins/$REV/qemu-system-x86_64" ] || ln -sfn "$W/kf3-bins/$REV" "/workspace/bench/kf3-bins/$REV"
qemu-img create -q -f qcow2 -F qcow2 -b "$W/broker-interactive/desktop.qcow2" "$ROOT/desktop.qcow2" || exit 2
WRAP=/data/claude-planes-bin
env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" KF_RUN_DIR="$ROOT/run" KF_IMG="$ROOT/desktop.qcow2" \
    KF3_REV="$REV" KF_LOCK="$ROOT/private.lock" KF_RAM_MB=${KF_RAM_MB:-4096} KF_FB_MB=${KF_FB_MB:-4096} \
    bash "$HERE/interactive.sh" > "$ROOT/launcher.log" 2>&1 < /dev/null &
launcher=$!
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
mon() { python3 - "$ROOT/run/qemu.mon" "$1" <<'PY'
import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(3)
time.sleep(0.3)
try: s.recv(4096)
except Exception: pass
s.sendall((sys.argv[2]+"\n").encode()); time.sleep(0.5)
try: print(s.recv(4096).decode(errors="replace").strip().splitlines()[-2:])
except Exception as e: print("mon:",e)
PY
}
finish() {
    rc=$?
    env PATH="$WRAP:$PATH" KF_INTERACTIVE_DIR="$ROOT" bash "$HERE/interactive.sh" stop > "$ROOT/stop.log" 2>&1 < /dev/null
    for _ in $(seq 30); do kill -0 "$launcher" 2>/dev/null || break; sleep 1; done
    kill -0 "$launcher" 2>/dev/null && kill "$launcher"
    echo "S3_EXIT rc=$rc $(date -Is)" | tee "$ROOT/EXIT"
}
trap finish EXIT
ready=0
for _ in $(seq 1 100); do
    kill -0 "$launcher" 2>/dev/null || { echo "S3_FAIL launcher exited"; tail -5 "$ROOT/launcher.log"; exit 3; }
    timeout 8 "${G[@]}" true 2>/dev/null && { ready=1; break; }; sleep 3
done
[ "$ready" = 1 ] || { echo "S3_FAIL ssh never came up"; exit 3; }
echo "S3_GUEST_UP $(date -Is)"
cat > "$ROOT/probe.sh" <<'GUEST'
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 1 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
echo "CINNAMON $(pgrep -c -x cinnamon) XORG $(pgrep -c -x Xorg)"
uname -r; cat /proc/driver/nvidia/version | head -1
timeout 30 nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; echo "SMI_RC $?"
timeout 10 stdbuf -oL glxgears > /tmp/g.log 2>&1; tail -1 /tmp/g.log | sed 's/^/GEARS /'
echo "PM: $(cat /sys/power/state) mem_sleep=$(cat /sys/power/mem_sleep)"
systemctl is-enabled nvidia-suspend nvidia-resume nvidia-hibernate 2>&1 | tr '\n' ' '; echo
grep -h "PreserveVideo\|NVreg" /etc/modprobe.d/* 2>/dev/null | head -3
cat /proc/driver/nvidia/params 2>/dev/null | grep -i "Preserve\|DynamicPower\|EnableGpuFirmware"
GUEST
echo "== before"; timeout 120 "${G[@]}" 'bash -s' < "$ROOT/probe.sh" 2>&1 | sed 's/^/BEFORE /'
timeout 30 "${G[@]}" 'sudo sh -c "echo 1 > /sys/power/pm_print_times; echo 1 > /sys/power/pm_debug_messages; echo N > /sys/module/printk/parameters/console_suspend; dmesg -n 8; echo 8 > /proc/sys/kernel/printk"' 2>&1 | head -3
# ★ 2026-10-14 (docs/design/V3_S3_RESUME.md): S3_CYCLES suspend/wake cycles in one boot (default 1).
# PASS per cycle: ssh back, nvidia-smi names the GPU, glxgears renders frames, no Xid in the guest.
Q="$ROOT/run/qemu.log"
pass_all=1
for cyc in $(seq 1 "${S3_CYCLES:-1}"); do
    echo "== suspend cycle $cyc"; echo "S3_SUSPEND_CMD cycle=$cyc $(date -Is)"
    timeout 20 "${G[@]}" 'sudo systemctl suspend' > "$ROOT/suspend-$cyc.out" 2>&1 &
    st=""
    for i in $(seq 1 60); do
        st=$(mon "info status" | tr -d '\n'); case "$st" in *suspended*) break;; esac; sleep 2
    done
    echo "S3_STATE_AFTER_CMD cycle=$cyc $st (t=$((i*2))s)"
    sleep 3; mon "system_wakeup" > /dev/null; echo "S3_WAKE_SENT cycle=$cyc $(date -Is)"
    back=0
    for i in $(seq 1 25); do timeout 8 "${G[@]}" true 2>/dev/null && { back=1; break; }; sleep 3; done
    echo "S3_SSH_BACK=$back cycle=$cyc after ~$((i*3))s $(date -Is)"
    grep -a "GSP phase\|S3 WAKE\|GSP-RESUME\|PM transition\|device reset #" "$Q" | tail -12 | cut -c1-200 | sed "s/^/S3_KF3 c$cyc /"
    echo "S3_STATUS cycle=$cyc $(mon 'info status' | tr -d '\n')"
    if [ "$back" = 1 ]; then
        timeout 120 "${G[@]}" 'bash -s' < "$ROOT/probe.sh" > "$ROOT/after-$cyc.txt" 2>&1
        sed "s/^/AFTER c$cyc /" "$ROOT/after-$cyc.txt"
        timeout 40 "${G[@]}" 'sudo dmesg | grep -i "nvrm\|nvidia\|Xid\|timeout" | tail -25' 2>&1 | cut -c1-220 | sed "s/^/DMESG c$cyc /"
        xid=$(timeout 20 "${G[@]}" 'sudo dmesg | grep -c "Xid" || true' 2>/dev/null); xid=${xid:-99}
        gears=$(grep -o 'GEARS [0-9]* frames' "$ROOT/after-$cyc.txt" | awk '{print $2}')
        smi=$(grep -c "NVIDIA GeForce" "$ROOT/after-$cyc.txt")
        ok=0; [ "${gears:-0}" -gt 0 ] && [ "$smi" -ge 1 ] && [ "$xid" = 0 ] && ok=1
        if [ "$ok" != 1 ]; then
            # what the guest is stuck in: blocked tasks, their kernel stacks, the newest kernel lines
            timeout 60 "${G[@]}" 'sudo sh -c "ps -eo pid,stat,wchan:32,etime,cmd | grep -E \" D|nvidia|Xorg|glxgears\" | grep -v grep | head -20; for p in \$(pgrep -f \"nvidia-smi|glxgears|nvidia-persist\"); do echo == \$p; cat /proc/\$p/stack; done; systemctl --no-pager status nvidia-suspend nvidia-resume 2>&1 | grep -E \"Active|exited|failed\"; dmesg | tail -40"' 2>&1 | cut -c1-200 | sed "s/^/DIAG c$cyc /"
        fi
        echo "S3_CYCLE_RESULT cycle=$cyc ssh=1 nvidia_smi=$smi gears_frames=${gears:-0} xid=$xid pass=$ok"
        [ "$ok" = 1 ] || { pass_all=0; break; }
    else
        grep -a "calling \|returned \|PM: \|NVRM\|nvidia" "$ROOT/run/serial.log" | tr -d "\000" | tail -40 | cut -c1-200 | sed 's/^/SERIAL /'
        top -H -b -n1 | head -14 | tail -8 | cut -c1-100 | sed 's/^/HOSTTOP /'
        echo "S3_CYCLE_RESULT cycle=$cyc ssh=0 pass=0"
        pass_all=0; break
    fi
done
echo "S3_RESULT pass=$pass_all cycles=${S3_CYCLES:-1} rev=$REV"
# S3_HOLD=<s>: keep the guest up that long before the stop, for a look inside (diagnostic runs only)
[ -n "${S3_HOLD:-}" ] && { echo "S3_HOLDING ${S3_HOLD}s $(date -Is)"; sleep "$S3_HOLD"; }
[ "$pass_all" = 1 ]
