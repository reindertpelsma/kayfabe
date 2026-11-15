#!/bin/bash
# pcnight3 E2: pass-through wrapper for the kf3 qemu: adds display-max-fps=30 to the kf3-gpu device (no source change)
args=()
for a in "$@"; do case "$a" in kf3-gpu,*) a="$a,display-max-fps=30";; esac; args+=("$a"); done
exec /workspace/bench/kf3-bins/b20fd84b/qemu-system-x86_64 "${args[@]}"
