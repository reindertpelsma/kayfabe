#!/bin/bash
set -euo pipefail
exec >/root/prov/native_fuzz_build.log 2>&1
source=6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
fixture=/workspace/bw3-fuzz-fixture
cd /root/kayfabe
git worktree add --detach "$fixture" "$source"
python3 /root/sweep2/fuzz_fixture.py "$fixture/crates/kayfabe-rm-ladder/src/main.rs" \
 >/root/prov/native_fuzz_patch.json
cd "$fixture"
df -h /
CARGO_TARGET_DIR=/workspace/bw3-fuzz-target cargo build --release \
 -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
python3 - <<'PY'
from pathlib import Path
import hashlib,json
b=Path('/workspace/bw3-fuzz-target/release/kayfabe-rm-ladder')
p=Path('/root/prov/native_fuzz_patch.json')
j=json.loads(p.read_text());j.update(binary=str(b),binary_sha256=hashlib.sha256(b.read_bytes()).hexdigest(),base='6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b',product_QEMU_unchanged=True,experiment_mode='OBSERVATION',attestation=False)
Path('/root/prov/native_fuzz_fixture.json').write_text(json.dumps(j,indent=2)+'\n')
PY
echo NATIVE_FUZZ_FIXTURE_BUILD_DONE
