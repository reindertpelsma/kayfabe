# common.sh — sourced by every R4 stage script on the box (evidence/driver only; no kayfabe code).
set -u
export PATH=$HOME/.cargo/bin:$PATH
KF=/root/kayfabe
B=/workspace/bench
L=/root/r4/logs; mkdir -p "$L"
REV=$(git -C "$KF" rev-parse --short=8 HEAD)
QB=$B/kf3-bins/$REV/qemu-system-x86_64
APPIMG=$B/guest_apps.qcow2
GR=/workspace/gfxset/results
ON=doorbell-ioeventfd=on
# one step = START line, the command's own log, an RC line (a killed step has START and no RC)
step(){ local n=$1; shift; echo "STEP $n START $(date -Is)"; ( cd "$KF" && "$@" ) > "$L/$n.log" 2>&1; local rc=$?; echo "STEP $n RC=$rc $(date -Is)"; tail -n 4 "$L/$n.log" | cut -c1-300 | sed "s/^/  $n| /"; return 0; }
alive_qemu(){ pgrep -x qemu-system-x86 >/dev/null || ss -tln | grep -q ':2223 '; }
gpu_idle(){ local u; for _ in $(seq 1 120); do u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1 | tr -dc 0-9); [ -n "$u" ] && [ "$u" -le 512 ] && return 0; sleep 1; done; echo "GPU_NOT_IDLE used=$u"; }
