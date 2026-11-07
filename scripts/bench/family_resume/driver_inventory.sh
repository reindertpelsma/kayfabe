#!/usr/bin/env bash
# Initial provider-image inventory precedes the planned driver installation.
# A closed module can reject Blackwell here. This is not native qualification.
record_initial_inventory() {
    local evidence=$1 rc
    if nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader \
        > "$evidence/smi_initial_query.txt" 2>&1; then
        rc=0
    else
        rc=$?
    fi
    printf '%s\n' "$rc" > "$evidence/smi_initial_query.rc"
    nvidia-smi -q > "$evidence/smi_initial.txt" 2>&1 || true
}
