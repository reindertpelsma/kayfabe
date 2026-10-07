#!/usr/bin/env bash
# Source-only helpers for attributing counters to the QEMU spawned by boot_capture.
# A PID alone can be recycled; capture and validate Linux /proc's starttime as well.
kf_qemu_starttime() {
    local pid=${1:-} stat comm
    local -a fields
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
    IFS= read -r comm 2>/dev/null < "/proc/$pid/comm" || return 1
    [[ "$comm" == qemu-system-* ]] || return 1
    IFS= read -r stat 2>/dev/null < "/proc/$pid/stat" || return 1
    # comm is parenthesized and may contain spaces/parentheses. Starttime is field
    # 22, i.e. index 19 after stripping pid and the final closing parenthesis.
    read -r -a fields <<< "${stat##*) }"
    [[ ${#fields[@]} -ge 20 && ${fields[0]} != Z && ${fields[19]} =~ ^[0-9]+$ ]] || return 1
    printf '%s\n' "${fields[19]}"
}

kf_qemu_identity_matches() {
    local pid=${1:-} expected=${2:-} actual
    [[ "$expected" =~ ^[0-9]+$ ]] || return 1
    actual=$(kf_qemu_starttime "$pid") || return 1
    [[ "$actual" == "$expected" ]]
}
