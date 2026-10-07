#!/bin/bash
# mode2_strace.sh — strace libcuda's GPU-event worker to see what it does after
# poll() returns: read()/ioctl() on the os-event fds reveals the completion-check
# mechanism (event-data vs ioctl-status vs pure mapped-memory poll).
set -u
GUESTLIB=/usr/local/nvidia-guest/lib
LIBCUDA="$GUESTLIB/libcuda.so.580.159.04"
LD_PRELOAD="$LIBCUDA" LD_LIBRARY_PATH="$GUESTLIB" /tmp/cup2 >/tmp/cup2.out 2>&1 &
CPID=$!
for i in $(seq 1 20); do grep -q totalMem /tmp/cup2.out 2>/dev/null && break
    kill -0 $CPID 2>/dev/null || { echo "cup2 exited"; exit 1; }; sleep 1; done
sleep 5
WTID=""
for t in /proc/$CPID/task/*; do
    tid=$(basename "$t"); w=$(cat "$t/wchan" 2>/dev/null)
    case "$w" in *poll*) ;; *) continue;; esac
    to=$(awk '{print $4}' "$t/syscall" 2>/dev/null)
    [ "$to" != "0xffffffffffffffff" ] && [ "$to" != "0x0" ] && WTID=$tid
done
echo "worker tid=$WTID"
[ -n "$WTID" ] || { echo "no worker"; kill -9 $CPID; exit 1; }
echo "=== strace worker (3s) ==="
timeout 3 strace -tt -f -e trace=poll,ppoll,read,pread64,ioctl,write,futex -p "$WTID" 2>&1 | head -60
kill -9 $CPID 2>/dev/null
