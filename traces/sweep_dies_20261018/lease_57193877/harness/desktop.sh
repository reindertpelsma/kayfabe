#!/bin/bash
set -euo pipefail
. /root/s2/common.sh
cd /root/kayfabe
python3 - <<'VERIFY'
import hashlib,json
from pathlib import Path
m=json.loads(Path('scripts/bench/windows/appmatrix/manifest.json').read_text())['image'];p=Path('/workspace/bench/kfapps.iso');assert p.stat().st_size==m['size'];assert hashlib.file_digest(p.open('rb'),'sha256').hexdigest()==m['sha256'];print('APPDISK_HASH_OK')
VERIFY
bash /root/s2/arm.sh on ad102_s4 /workspace/bench/kf3-bins/17005433/qemu-system-x86_64 1700543393c34f93adc0611430e3d49ab38eca3a
