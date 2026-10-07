#!/usr/bin/env bash
# ★★★★★ w396 — the many-workload CUDA battery, in the Mode-2 guest.
#
# ## ⊘ ONE PROCESS, MANY WORKLOADS — and that is the whole design
# The multi-app wedge is per DEVICE OPEN, not per workload (w394: four opens shared one
# RmInitAdapter span; the fifth tore the adapter down and the re-init failed on an event
# notification). So a battery inside ONE process gets breadth TODAY without touching that
# wall, and the re-init fix later converts it into many processes.
#
# ⊘ Runs the GPU arm and a CPU arm of the SAME battery in the same boot, so every workload is
# graded against its own oracle rather than against a remembered number.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
PY=/home/ubuntu/llmvenv/bin/python
TMO=${W396_TIMEOUT:-1800}

echo "=== ★ w396 CUDA workload battery $(date -Is) ==="
if ! $G true >/dev/null 2>&1; then
  echo "W396_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi
scp -i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    "$SRC_DIR/w396_torch_battery.py" ubuntu@192.168.77.2:/tmp/ >/dev/null 2>&1 \
  || { echo "W396_OUTCOME=(E) ⊘ UNMEASURED — could not stage the battery"; exit 0; }

for arm in cuda cpu; do
  echo "--- W396_ARM=$arm $(date -Is)"
  timeout "$TMO" $G "W396_DEVICE=$arm $PY /tmp/w396_torch_battery.py" 2>&1
  echo "W396_ARM_RC_${arm}=$?"
done
echo "=== W396_BATTERY_DONE $(date -Is) ==="
