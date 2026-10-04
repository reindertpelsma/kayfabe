#!/bin/bash
# hang_catch.sh <tag> — beside a lane run: after the hook's screendump step, is the guest reachable and does
# the monitor answer? If not, every QEMU thread's stack (gdb, batch) before boot_capture kills it.
TAG=$1; P=/workspace/bench/run_${TAG}_probe.log; MON=/workspace/bench/run_${TAG}.mon
OUTD=/root/mf/ev/$TAG; mkdir -p "${OUTD:?}"
exec > /root/mf/hang_$TAG.log 2>&1
echo "HANG_START $TAG $(date -Is)"
for i in $(seq 900); do grep -aq 'DISPLAY_PATTERN_MATCH' $P 2>/dev/null && break; sleep 1; done
echo "HANG_PATTERN_LINE after ${i}s: $(grep -a 'DISPLAY_PATTERN_MATCH' $P | tail -1) $(date +%T)"
sleep 3
g=$(timeout 8 /root/wt-mf/scripts/bench/gssh_nv 'echo ALIVE' 2>&1 | tr -d '\r' | tail -1)
m=$(timeout 8 python3 - "$MON" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(6)
time.sleep(0.2)
try:
    s.recv(65536); s.sendall(b"info status\n"); time.sleep(1); print(s.recv(65536).decode(errors="replace").replace("\r","").replace("\n"," | ")[-160:])
except Exception as e:
    print("monitor:", e)
PY
)
echo "HANG_PROBE guest=[$g] monitor=[$m] $(date +%T)"
pid=$(pgrep -x qemu-system-x86 | head -1)
if [ "$g" != ALIVE ] || ! printf '%s' "$m" | grep -q running; then
    echo "HANG_DETECTED pid=$pid; dumping stacks"
    timeout 120 gdb -p "$pid" -batch -ex 'set pagination off' -ex 'info threads' -ex 'thread apply all bt 40' > "$OUTD/qemu_stacks.txt" 2>&1
    echo "HANG_STACKS rc=$? lines=$(wc -l < "$OUTD/qemu_stacks.txt")"
    sleep 2
    timeout 120 gdb -p "$pid" -batch -ex 'set pagination off' -ex 'thread apply all bt 40' > "$OUTD/qemu_stacks2.txt" 2>&1
    echo "HANG_STACKS2 rc=$? lines=$(wc -l < "$OUTD/qemu_stacks2.txt")"
fi
echo "HANG_EXIT $(date -Is)"
