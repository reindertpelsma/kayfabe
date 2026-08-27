#!/bin/bash
# mode2_gdb_cb.sh — break at libcuda's per-fd event callback dispatch and
# statically dump the callback fn + its object (the completion state it checks).
# Symbol-relative breakpoint (cuVDPAUCtxCreate is an exported symbol) so ASLR is
# handled automatically. NO single-stepping (that hangs over nested ssh).
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
# 0x4069c2 (call *(rax)) = cuVDPAUCtxCreate@@Base + 0x61362  (0x4069c2-0x3a5660)
timeout 50 gdb -p "$WTID" -batch \
    -ex 'set pagination off' \
    -ex 'break *(cuVDPAUCtxCreate+0x61362)' \
    -ex 'continue' \
    -ex 'printf "=== HIT callback dispatch ===\n"' \
    -ex 'printf "rax(disp obj)=%p  rcx(arg)=%p  rdi(outbuf)=%p\n", $rax, $rcx, $rdi' \
    -ex 'printf "callback fn ptr [rax]:\n"' \
    -ex 'x/2gx $rax' \
    -ex 'printf "callback object [rcx=arg]:\n"' \
    -ex 'x/24gx $rcx' \
    -ex 'printf "=== callback fn disasm (*(void**)rax) ===\n"' \
    -ex 'x/60i *(unsigned long*)$rax' \
    2>&1 | grep -vE "^\[|warning:|Reading|Run till|Continuing|Thread|New LWP|^$" | head -110
kill -9 $CPID 2>/dev/null
