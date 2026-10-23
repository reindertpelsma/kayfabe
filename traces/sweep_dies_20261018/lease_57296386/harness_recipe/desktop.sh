#!/bin/bash
set -euo pipefail
. /root/s2/common.sh
cd /root/kayfabe
python3 - <<'VERIFY'
import hashlib,json
from pathlib import Path
m=json.loads(Path('scripts/bench/windows/appmatrix/manifest.json').read_text())['image'];p=Path('/workspace/bench/kfapps.iso');assert p.stat().st_size==m['size'];h=hashlib.sha256()
with p.open('rb') as f:
 for chunk in iter(lambda:f.read(1024*1024),b''):h.update(chunk)
assert h.hexdigest()==m['sha256'];print('APPDISK_HASH_OK')
VERIFY
bash /root/s2/arm.sh on ampere_s4 /workspace/bench/kf3-bins/ac21aef9/qemu-system-x86_64 ac21aef93cc7a8bb1149fecd98f8404e6b7e20d7
