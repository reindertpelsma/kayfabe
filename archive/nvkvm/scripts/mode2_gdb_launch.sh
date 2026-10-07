#!/bin/bash
# mode2_gdb_launch.sh — launch cup2 UNDER gdb (so we catch the early event-wake
# burst) and break at libcuda's per-fd completion callback dispatch. On hit,
# dump the callback object (holds the completion sema ptr + target the worker
# re-checks) and the callback disasm. This is the decisive completion-bit probe.
set -u
GUESTLIB=/usr/local/nvidia-guest/lib
LIBCUDA="$GUESTLIB/libcuda.so.580.159.04"
timeout 70 gdb -batch \
    -ex "set environment LD_PRELOAD=$LIBCUDA" \
    -ex "set environment LD_LIBRARY_PATH=$GUESTLIB" \
    -ex 'set pagination off' \
    -ex 'set breakpoint pending on' \
    -ex 'file /tmp/cup2' \
    -ex 'break cuInit' \
    -ex 'run' \
    -ex 'printf "=== libcuda loaded (at cuInit); setting callback bp ===\n"' \
    -ex 'break *(cuVDPAUCtxCreate+0x61362)' \
    -ex 'delete 1' \
    -ex 'continue' \
    -ex 'printf "=== HIT #1 ===\n"' \
    -ex 'printf "rax=%p rcx(obj)=%p rdi=%p r13=%p r14=%p\n", $rax, $rcx, $rdi, $r13, $r14' \
    -ex 'printf "cb object @rcx:\n"' -ex 'x/32gx $rcx' \
    -ex 'printf "@rax:\n"' -ex 'x/8gx $rax' \
    -ex 'printf "callback disasm:\n"' -ex 'x/50i *(unsigned long*)$rax' \
    2>&1 | grep -vE "^\[|warning:|Reading|Thread|New |Inferior|process |^$|Continuing" | head -110
