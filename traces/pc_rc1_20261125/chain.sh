#!/bin/bash
# short linear chain of the Windows arms (W-default x3, W-display-off x2, W-fill-on x2, W-display-off windows-support=off x1), arms interleaved,
# no new boot after the deadline (HHMM, UTC). The pc lock is held by the session (no flock here).
set -u
R=/workspace/bench/pc_rc1_20261125
VMROOT=/workspace/pc1/winvm/pc_rc1_20261125
REV=237349e0
QEMU=/workspace/bench/kf3-bins/$REV/qemu-system-x86_64
PREP=/workspace/pc1/winvm/vfio_s3_20261124/prep.qcow2
BROKER=/workspace/bench/b2_20261119/bin/nvkvm-display-broker
GOP=/workspace/pc1/winvm/win11_base/firmware/kf-gop.signed.efi
DEADLINE=${DEADLINE:-1440}
mkdir -p "$VMROOT" "$R/out"
for step in Wdef1 Woff1 Wfill1 Wdef2 Woff2 Wfill2 Wdef3 Woffsup1; do
  arm=${step%%[0-9]*}
  if [ "$(date -u +%H%M)" -ge "$DEADLINE" ]; then echo "DEADLINE: skip $step"; continue; fi
  echo "=== $step $(date -u +%T)"
  python3 -I "$R/repo/traces/pc_rc1_20261125/displayexp.py" "$arm" "$QEMU" "$VMROOT" "$PREP" "$BROKER" "$GOP" "$R/out/$step" 300 2>&1 | tail -n 400 > "$R/out/$step.stdout"
  tail -n 3 "$R/out/$step.stdout"
  rm -rf "$VMROOT"/dispexp_*
done
echo CHAIN_DONE
