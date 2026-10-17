#!/bin/bash
# l3_broker.sh <worktree> <tag>   -- the Linux display-broker lane (scripts/bench/display/interactive.sh run) with the Cinnamon guest:
# boot, KMS, Cinnamon, glxgears, ACPI shutdown. Private overlay of the shared desktop disk; shared GPU lock held by an outer flock -o.
set -uo pipefail
W=$1; TAG=$2
EV=/data/claude-candhw/ev/l3; mkdir -p "$EV"
export BENCH_DIR=/data/claude-candhw/bench KF_LOCK=/data/claude-candhw/fast.lock KF_INTERACTIVE_DIR=/data/claude-candhw/interactive
export KF_IMG=/data/claude-candhw/lc_desktop.qcow2
cd "$W" || exit 3
export KF3_REV=$(git rev-parse --short=8 HEAD)
IMGBASE=/var/lib/kf-windows-20261005/broker-interactive/desktop.qcow2
rm -f "$KF_IMG"; qemu-img create -q -f qcow2 -b "$IMGBASE" -F qcow2 "$KF_IMG" || exit 4
mkdir -p "$KF_INTERACTIVE_DIR"
bash /data/claude-candhw/wt/scripts/bench/candidate_hw/host_state.sh "pre_$TAG" "$EV" > /dev/null
rm -f "$EV/stop_$TAG"
bash /data/claude-candhw/wt/scripts/bench/candidate_hw/qsampler.sh "$EV/qemus_$TAG.txt" "$EV/stop_$TAG" &
SP=$!
T0=$(date '+%Y-%m-%d %H:%M:%S')
echo "L3_START tag=$TAG kf3=$KF3_REV $(date -Is)" | tee "$EV/l3_$TAG.txt"
flock -o /tmp/kayfabe-fastguest.lock bash scripts/bench/display/interactive.sh run > "$EV/launcher_$TAG.out" 2>&1 &
LP=$!
G=(ssh -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes ubuntu@192.168.77.2)
t0=$(date +%s)
up=0; for _ in $(seq 120); do "${G[@]}" true 2>/dev/null && { up=1; break; }; kill -0 $LP 2>/dev/null || break; sleep 3; done
echo "L3_SSH_UP=$up after $(( $(date +%s) - t0 )) s" | tee -a "$EV/l3_$TAG.txt"
cin=0; if [ $up = 1 ]; then for _ in $(seq 60); do "${G[@]}" 'pgrep -u ubuntu -x cinnamon >/dev/null' 2>/dev/null && { cin=1; break; }; sleep 3; done; fi
echo "L3_CINNAMON_UP=$cin after $(( $(date +%s) - t0 )) s" | tee -a "$EV/l3_$TAG.txt"
if [ $cin = 1 ]; then sleep 20; fi   # let the session settle before the checks
if [ $up = 1 ]; then
  timeout 120 "${G[@]}" 'bash -s' > "$EV/guest_checks_$TAG.txt" 2>&1 <<'GUEST'
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
echo "== kernel/driver"; uname -r; cat /proc/driver/nvidia/version | head -2
nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv,noheader 2>&1
echo "== KMS"; echo "modeset=$(sudo cat /sys/module/nvidia_drm/parameters/modeset 2>&1) fbdev=$(sudo cat /sys/module/nvidia_drm/parameters/fbdev 2>&1)"
ls /sys/class/drm; for c in /sys/class/drm/card*-*; do [ -e "$c/status" ] && echo "$c status=$(cat $c/status) mode0=$(head -1 $c/modes 2>/dev/null)"; done
sudo dmesg | grep -ai 'nvidia-drm\|\[drm\]' | head -12
echo "== desktop"; pgrep -u ubuntu -a 'cinnamon$|cinnamon-session|nemo|muffin' | cut -c1-120; pgrep -a lightdm | head -2; pgrep -a Xorg | cut -c1-120
xdpyinfo 2>&1 | grep -E 'dimensions|vendor string|NV-GLX|GLX' | head -5; xrandr 2>&1 | head -4
echo "== window manager"; xprop -root _NET_SUPPORTING_WM_CHECK _NET_ACTIVE_WINDOW 2>&1 | head -3
echo "== glxgears"; command -v glxgears glxinfo
timeout 14 glxgears -info 2>&1 | grep -aE 'GL_RENDERER|GL_VERSION|GL_VENDOR|frames in' | head -8
echo "== guest dmesg issues"; sudo dmesg | grep -aiE 'xid|nvrm.*(fail|error)|call trace|BUG:|oops' | head -10
echo "== screenshot"; command -v import xwd scrot 2>&1; (xwd -root -silent 2>/dev/null | wc -c) 2>&1
echo "== guest done"
sudo dmesg > /tmp/guest_dmesg.txt
GUEST
  echo "L3_GUEST_CHECKS_RC=$?" | tee -a "$EV/l3_$TAG.txt"
fi
"${G[@]}" 'cat /tmp/guest_dmesg.txt' > "$EV/guest_dmesg_$TAG.txt" 2>/dev/null
QP=$(cat "$KF_INTERACTIVE_DIR/qemu.pid" 2>/dev/null)
echo "L3_STOP qemu_pid=$QP $(date -Is)" | tee -a "$EV/l3_$TAG.txt"
# shutdown, guest-first (SHUTDOWN=guest): the guest's own `systemctl poweroff` over ssh (how interactive.sh prep ends); 60 s.
# then (or with SHUTDOWN=acpi) the ACPI power button through the monitor; 25 s. Serial growth and QEMU thread CPU sampled meanwhile.
mon=$(ls -t "$KF_INTERACTIVE_DIR"/run-*/qemu.mon | head -1); SERL=$(ls -t "$KF_INTERACTIVE_DIR"/run-*/serial.log | head -1)
sl0=$(wc -c < "$SERL"); T_STOP=$(date +%s)
if [ "${SHUTDOWN:-guest}" = guest ]; then
  timeout 20 "${G[@]}" 'sudo systemctl poweroff; echo poweroff_rc=$?' > "$EV/poweroff_$TAG.txt" 2>&1
  echo "L3_GUEST_POWEROFF ssh_rc=$? : $(tr '\n' ' ' < "$EV/poweroff_$TAG.txt" | cut -c1-200)" | tee -a "$EV/l3_$TAG.txt"
  for i in $(seq ${POWEROFF_WAIT:-120}); do kill -0 "$QP" 2>/dev/null || break; [ $i = 20 ] && { top -H -b -n1 -p "$QP" 2>/dev/null | head -14 > "$EV/top_$TAG.txt"; echo "serial_growth_20s=$(( $(wc -c < "$SERL") - sl0 ))" >> "$EV/top_$TAG.txt"; printf 'info status\ninfo cpus\n' | timeout 5 socat - "UNIX-CONNECT:$mon" 2>/dev/null | tr '\r' '\n' | grep -a "VM status\|CPU #\|pc=" | head -8 >> "$EV/top_$TAG.txt"; }; sleep 1; done
  if kill -0 "$QP" 2>/dev/null; then echo "L3_GUEST_POWEROFF_NOT_DONE_IN_WINDOW" | tee -a "$EV/l3_$TAG.txt"; else echo "L3_GUEST_POWEROFF_CLEAN (qemu exited at +$(( $(date +%s) - T_STOP )) s)" | tee -a "$EV/l3_$TAG.txt"; fi
fi
if kill -0 "$QP" 2>/dev/null; then
  printf 'system_powerdown\n' | timeout 5 socat - "UNIX-CONNECT:$mon" > /dev/null 2>&1
  for _ in $(seq 25); do kill -0 "$QP" 2>/dev/null || break; sleep 1; done
  kill -0 "$QP" 2>/dev/null && echo "L3_ACPI_POWERDOWN_IGNORED_25S" | tee -a "$EV/l3_$TAG.txt" || echo "L3_ACPI_POWERDOWN_CLEAN" | tee -a "$EV/l3_$TAG.txt"
fi
if kill -0 "$QP" 2>/dev/null; then echo "L3_NOT_STOPPED_BY_GUEST: killing own qemu $QP" | tee -a "$EV/l3_$TAG.txt"; bash scripts/bench/display/interactive.sh stop > "$EV/stop_$TAG.out" 2>&1; fi
for _ in $(seq 20); do kill -0 $LP 2>/dev/null || break; sleep 1; done
kill -0 $LP 2>/dev/null && echo "L3_LAUNCHER_STILL_RUNNING" | tee -a "$EV/l3_$TAG.txt"
wait $LP 2>/dev/null
echo "L3_EXIT launcher_done=1 $(date -Is)" | tee -a "$EV/l3_$TAG.txt"
touch "$EV/stop_$TAG"; wait $SP
bash /data/claude-candhw/wt/scripts/bench/candidate_hw/host_state.sh "post_$TAG" "$EV" | tee -a "$EV/l3_$TAG.txt"
diff <(sed 's/^\[[ 0-9.]*\] //' "$EV/xid_pre_$TAG.txt") <(sed 's/^\[[ 0-9.]*\] //' "$EV/xid_post_$TAG.txt") > "$EV/xid_new_$TAG.txt"
echo "XID_NEW_LINES(dmesg diff)=$(grep -c '^>' "$EV/xid_new_$TAG.txt")" | tee -a "$EV/l3_$TAG.txt"
bash /data/claude-candhw/wt/scripts/bench/candidate_hw/xid_map.sh "$EV" "$TAG" "$T0" "$(date '+%Y-%m-%d %H:%M:%S')" | tee -a "$EV/l3_$TAG.txt"
d=$(ls -dt "$KF_INTERACTIVE_DIR"/run-* | head -1); echo "RUN_DIR=$d" | tee -a "$EV/l3_$TAG.txt"
