#!/bin/bash
# pcnight4 E2: pass-through wrapper for the kf3 qemu: adds display-max-fps=$FPS to the kf3-gpu device (no source change)
args=()
FPS=${KF_FPS:-30}
for a in "$@"; do case "$a" in kf3-gpu,*) a="$a,display-max-fps=$FPS";; esac; args+=("$a"); done
exec /workspace/bench/kf3-bins/b20fd84b/qemu-system-x86_64 "${args[@]}"
