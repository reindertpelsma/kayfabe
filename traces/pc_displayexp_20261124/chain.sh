#!/bin/bash
# short linear chain of the display experiment: arms interleaved, hard stop for new boots at 20:50Z
set -u
R=/workspace/bench/displayexp_20261124
VMROOT=/workspace/pc1/winvm/displayexp_20261124
QEMU=/workspace/bench/kf3-bins/63e46069/qemu-system-x86_64
PREP=/workspace/pc1/winvm/vfio_s3_20261124/prep.qcow2
BROKER=/workspace/bench/b2_20261119/bin/nvkvm-display-broker
GOP=/workspace/pc1/winvm/win11_base/firmware/kf-gop.signed.efi
mkdir -p "$VMROOT" "$R/out"
exec 9>/root/prov/pc_gpu.lock
flock -n 9 || { echo "LOCK BUSY"; exit 3; }
echo "$(date -u +%FT%TZ) displayexp START (kf3 no bind, 63e46069 display experiment, until 21:00Z)" >> /root/prov/PC_ACTIVE.log
for step in A1 B1 A2 B2 A3 B3 Boff1 Boff2 Boff3; do
  arm=${step%%[0-9]*}
  if [ "$(date -u +%H%M)" -ge 2050 ]; then echo "DEADLINE: skip $step"; continue; fi
  echo "=== $step $(date -u +%T)"
  python3 -I "$R/traces/pc_displayexp_20261124/displayexp.py" "$arm" "$QEMU" "$VMROOT" "$PREP" "$BROKER" "$GOP" "$R/out/$step" 300 2>&1 | tail -n 400 > "$R/out/$step.stdout"
  tail -n 3 "$R/out/$step.stdout"
  pkill -u root -f "$VMROOT" 2>/dev/null; sleep 2
  rm -rf "$VMROOT"/dispexp_*
done
echo "$(date -u +%FT%TZ) displayexp END" >> /root/prov/PC_ACTIVE.log
echo CHAIN_DONE
