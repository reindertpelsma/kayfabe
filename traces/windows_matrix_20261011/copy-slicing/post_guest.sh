#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
bash "$W/pti-iommu_nogdm.sh" DMA-FQ
! pgrep qemu-system
# This campaign reserves 860..1059. Preserve all logs; reclaim only its stopped
# disposable overlays between boots so package staging cannot fill the host SSD.
python3 - <<'PY'
import json,pathlib,subprocess
w=pathlib.Path('/var/lib/kf-windows-20261005')
for n in range(860,1060):
 p=w/f'boundary-kayfabe-{n}'/'windows.qcow2'
 if not p.exists(): continue
 info=json.loads(subprocess.check_output(['qemu-img','info','--output=json',str(p)]))
 assert info.get('full-backing-filename')==str(w/'baseline/windows.qcow2')
 print('RECLAIM_STOPPED_OVERLAY',n,p.stat().st_blocks*512)
 p.unlink()
PY

# Controlled campaign checkpoint: completed verdicts stay intact. Signal only
# this campaign's driver after QEMU has stopped, DMA-FQ restored and overlays reclaimed.
if [[ -e /tmp/kf-slice-stop-after-guest ]]; then
 rm -f /tmp/kf-slice-stop-after-guest
 echo "CONTROLLED_MATRIX_CHECKPOINT_STOP pid=4041868 at=$(date -Is)"
 if [[ -r /proc/4041868/cmdline ]] && tr '\0' ' ' < /proc/4041868/cmdline | grep -q '/data/kf-slice-matrix-8037e19b/scripts/bench/windows/appmatrix/winapps.py'; then
  kill -TERM 4041868
 fi
fi
