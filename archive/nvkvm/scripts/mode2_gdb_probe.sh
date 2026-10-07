#!/bin/bash
# mode2_gdb_probe.sh — runs cup2, lets it hang at cuCtxCreate, attaches gdb to
# the poll threads to capture the libcuda backtrace + the memory the GPU-event
# worker spins on (the unmet completion condition).
set -u
GUESTLIB=/usr/local/nvidia-guest/lib
LIBCUDA="$GUESTLIB/libcuda.so.580.159.04"

dmesg -C 2>/dev/null || true
LD_PRELOAD="$LIBCUDA" LD_LIBRARY_PATH="$GUESTLIB" /tmp/cup2 >/tmp/cup2.out 2>&1 &
CPID=$!
for i in $(seq 1 20); do grep -q totalMem /tmp/cup2.out 2>/dev/null && break
    kill -0 $CPID 2>/dev/null || { echo "cup2 exited"; cat /tmp/cup2.out; exit 1; }; sleep 1; done
sleep 6
echo "cup2 pid=$CPID"
for t in /proc/$CPID/task/*; do
    tid=$(basename "$t"); w=$(cat "$t/wchan" 2>/dev/null)
    case "$w" in *poll*) ;; *) continue;; esac
    comm=$(cat "$t/comm" 2>/dev/null)
    echo "===== gdb backtrace tid=$tid comm=$comm ====="
    gdb -p "$tid" -batch \
        -ex 'set pagination off' \
        -ex 'bt' \
        -ex 'info registers rdi rsi rdx rax rbx rcx' \
        2>/dev/null | grep -vE "^\[|warning:|Reading|^$" | head -40
done
kill -9 $CPID 2>/dev/null
