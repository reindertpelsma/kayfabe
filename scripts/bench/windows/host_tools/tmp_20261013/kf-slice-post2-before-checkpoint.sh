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
