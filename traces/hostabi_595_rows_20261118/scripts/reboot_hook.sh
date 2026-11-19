#!/usr/bin/env bash
# POST_CAPTURE_HOOK (evidence-run local; not in the repo): cold CUDA smoke (cup3: cuInit + cuCtxCreate + module + a tiny kernel,
# graded on KERNEL rv=43), then a guest reboot INSIDE THE SAME RUNNING QEMU (device reset), wait for the guest, and the same smoke again.
# Every guest call is bounded; a hang is recorded, not retried.
set -uo pipefail
TAG=${1:-none}
REPO=${KF_REPO:?KF_REPO}
G=$REPO/scripts/bench/gssh_nv
SRC=$REPO/scripts/bench/cup3.c
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
echo "RB_START $(date -Is) tag=$TAG repo_rev=$(git -C "$REPO" rev-parse --short=8 HEAD) qemu_pid=${KF_QEMU_PID:-?}"
$G 'cat > /home/ubuntu/rb_cup3.c' < "$SRC" || { echo "RB_FATAL push failed"; exit 2; }
$G 'cd /home/ubuntu && rm -f rb_cup3 && gcc -O0 -o rb_cup3 rb_cup3.c -lcuda 2>&1; echo GCC_RC=$?' | tr -d '\r'
smoke() {  # <phase>
    local ph=$1 t0 out rc
    t0=$(date +%s)
    out=$(timeout 170 $G "cd /home/ubuntu && (timeout 120 ./rb_cup3 > rb_cup3_$ph.out 2>&1; echo CUP3_RC=\$?); grep -E '^(CTX OK|MODULE OK|LAUNCH OK|SYNC OK|KERNEL rv=|DONE|FAIL)' rb_cup3_$ph.out | tr '\n' '|'" 2>&1 | tr -d '\r' | tr '\n' ' ')
    rc=${PIPESTATUS[0]}
    local v=FAIL; case "$out" in *"KERNEL rv=43"*) v=PASS ;; esac
    [ -z "$out" ] && v=HANG_OR_NO_ANSWER
    echo "RB_SMOKE phase=$ph verdict=$v ssh_rc=$rc secs=$(( $(date +%s) - t0 )) out=[$out]"
}
echo "RB_DRIVER_LOADED $($G 'timeout 60 nvidia-smi --query-gpu=driver_version --format=csv,noheader' 2>&1 | tr -d '\r' | head -1)"
smoke cold
BID=$($G 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null | tr -d '\r')
echo "RB_REBOOT_ISSUED $(date -Is)"
timeout 20 $G 'sudo systemctl reboot' >/dev/null 2>&1
up=0; t0=$(date +%s)
for i in $(seq 1 100); do
    kill -0 "${KF_QEMU_PID:-0}" 2>/dev/null || { echo "RB_QEMU_GONE during reboot wait"; break; }
    nb=$(timeout 10 $G 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null | tr -d '\r')
    if [ -n "$nb" ] && [ "$nb" != "$BID" ]; then up=1; break; fi
    sleep 4
done
echo "RB_GUEST_BACK up=$up secs=$(( $(date +%s) - t0 )) qemu_alive=$(kill -0 "${KF_QEMU_PID:-0}" 2>/dev/null && echo yes || echo no)"
if [ "$up" = 1 ]; then
    sleep 5
    echo "RB_AFTER_NVSMI $(timeout 120 $G 'timeout 90 nvidia-smi -L 2>&1 | sed "s/(UUID.*//"' 2>&1 | tr -d '\r' | head -2 | tr '\n' ' ')"
    $G 'ls /home/ubuntu/rb_cup3 >/dev/null 2>&1 && echo CUP3_BIN_PRESENT' | tr -d '\r'
    smoke after_reboot
    echo "--- guest dmesg NVRM/Xid (filtered) ---"
    timeout 60 $G 'sudo dmesg | grep -E "NVRM|Xid" | tail -25' 2>&1 | tr -d '\r' | sed 's/GPU-[0-9a-f-]\{36\}/GPU-<uuid>/g' | cut -c1-220
fi
echo "--- qemu.log lines around device reset (filtered) ---"
grep -anE "device reset|act device reset|GSP .*->|channel plane|HOST-ABI REFUSED|NOT complete" "$QLOG" 2>/dev/null | cut -c1-260 | head -60
echo "RB_END $(date -Is)"
