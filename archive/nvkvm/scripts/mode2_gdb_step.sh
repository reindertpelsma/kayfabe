#!/bin/bash
# mode2_gdb_step.sh — capture libcuda's GPU-event worker post-poll completion
# check via STATIC disassembly (robust; no slow single-stepping). Shows the
# instructions after poll() returns -> the memory load + compare that decides
# "not done -> re-poll", plus registers + libcuda base to compute the address.
set -u
GUESTLIB=/usr/local/nvidia-guest/lib
LIBCUDA="$GUESTLIB/libcuda.so.580.159.04"
LD_PRELOAD="$LIBCUDA" LD_LIBRARY_PATH="$GUESTLIB" /tmp/cup2 >/tmp/cup2.out 2>&1 &
CPID=$!
for i in $(seq 1 20); do grep -q totalMem /tmp/cup2.out 2>/dev/null && break
    kill -0 $CPID 2>/dev/null || { echo "cup2 exited"; cat /tmp/cup2.out; exit 1; }; sleep 1; done
sleep 5
WTID=""
for t in /proc/$CPID/task/*; do
    tid=$(basename "$t"); w=$(cat "$t/wchan" 2>/dev/null)
    case "$w" in *poll*) ;; *) continue;; esac
    sc=$(cat "$t/syscall" 2>/dev/null); to=$(echo "$sc" | awk '{print $4}')
    [ "$to" != "0xffffffffffffffff" ] && [ "$to" != "0x0" ] && WTID=$tid
done
echo "worker tid=$WTID (cup2=$CPID)"
[ -n "$WTID" ] || { echo "no finite-timeout poll thread"; kill -9 $CPID; exit 1; }
timeout 40 gdb -p "$WTID" -batch \
    -ex 'set pagination off' \
    -ex 'info proc mappings' \
    -ex 'frame 1' \
    -ex 'printf "=== caller pc (post-poll return site) ===\n"' \
    -ex 'p/x $pc' \
    -ex 'x/50i $pc-0x30' \
    -ex 'info registers rax rbx rcx rdx rsi rdi rbp rsp r8 r9 r12 r13 r14 r15' \
    2>&1 | grep -iE "libcuda|=== |pc |0x[0-9a-f]+ <|mov|cmp|test|jne|je |jmp|call|lea|rax|rbx|rcx|rdx|rsi|rdi|rbp|r1[2-5]|r8|r9" | head -90
kill -9 $CPID 2>/dev/null
