#!/bin/bash
# For every tag of the driver matrix: the value of the two nvos.h event flags the raw client ORs into
# NV0005 notifyIndex. Text-only; the values are read from NVIDIA's open-gpu-kernel-modules at that tag.
cd /var/lib/kf-windows-20261005/kayfabe-ceint || exit 1
echo "# NV01_EVENT_* flag values per matrix tag (src/common/sdk/nvidia/inc/nvos.h), fetched $(date -Is)"
for t in $(grep -v '^#' tools/drivermatrix/tags.txt | grep . | sort -V); do
    h=$(curl -s -m 30 "https://raw.githubusercontent.com/NVIDIA/open-gpu-kernel-modules/$t/src/common/sdk/nvidia/inc/nvos.h")
    a=$(printf '%s\n' "$h" | grep -E '^#define NV01_EVENT_WITHOUT_EVENT_DATA\b' | awk '{print $3}')
    b=$(printf '%s\n' "$h" | grep -E '^#define NV01_EVENT_NONSTALL_INTR\b' | awk '{print $3}')
    echo "$t WITHOUT_EVENT_DATA=${a:-MISSING} NONSTALL_INTR=${b:-MISSING}"
done
