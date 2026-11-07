#!/usr/bin/env bash
# GPU-free to test with fake roots/QEMU; on hardware this initializes KVM only,
# with no CPU execution (-S -machine none). Success is capability, not a boot.
set -uo pipefail
emit() { printf 'NESTED_PROBE %s=%s\n' "$1" "$2"; }
param() {
    local value
    value=$(cat "${KF_NESTED_SYS_ROOT:-/sys}/module/$1/parameters/nested" 2>/dev/null) || value=unknown
    case "$value" in Y|y|1) echo yes ;; N|n|0) echo no ;; *) echo unknown ;; esac
}
cpuinfo=${KF_NESTED_CPUINFO:-/proc/cpuinfo}
qemu=${QEMU_BIN:-qemu-system-x86_64}
device=no; [ ! -e "${KF_NESTED_DEV_ROOT:-/dev}/kvm" ] || device=yes
emit kvm_device "$device"
emit kvm_intel_nested "$(param kvm_intel)"
emit kvm_amd_nested "$(param kvm_amd)"
for flag in vmx svm; do
    value=no
    if grep -Eq "^[[:space:]]*(flags|Features)[[:space:]]*:.*[[:space:]]$flag([[:space:]]|$)" "$cpuinfo" 2>/dev/null; then value=yes; fi
    emit "cpu_$flag" "$value"
done
start=no
if [ "$device" = yes ]; then
    # quit is sent after QEMU has initialized its selected accelerator; a timeout
    # or an early error is never capability evidence. No tiny VM is executed.
    if printf 'quit\n' | timeout --signal=TERM -k 1 5 "$qemu" -accel kvm -machine none -S \
        -nodefaults -display none -monitor stdio >/dev/null 2>&1; then start=yes; fi
fi
emit kvm_start "$start"
tcg=no
if accelerators=$(timeout --signal=TERM -k 1 5 "$qemu" -accel help 2>/dev/null); then
    if grep -Eq '(^|[[:space:]])tcg([[:space:]]|$)' <<<"$accelerators"; then tcg=yes; fi
fi
emit tcg "$tcg"
