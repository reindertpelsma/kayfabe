#!/usr/bin/env bash
# Resume the controller after the native raw-suite concurrent-fuzz flake (rerun alone 3/3 clean). Same steps as controller.sh after the native baseline.
set -euo pipefail
exec >/root/prov/resume.log 2>&1
trap 'rc=$?; echo "RESUME_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "RESUME_START $(date -u -Is)"
export PATH=/root/.cargo/bin:$PATH KF_DEVICE=kf3 KF_LADDER=/root/kayfabe/target/release/kayfabe-rm-ladder
cd /root/kayfabe
bash scripts/bench/cuda_ladder.sh host sweep2c30a158_gb202 1 >/root/prov/sweep_cuda_host.log 2>&1
python3 - <<'CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_sweep2c30a158_gb202_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows),rows
print('NATIVE_CUDA4_PASS n=1')
CHECK
cargo build --release -p kf-harness --bin kf-gr-tier >/root/prov/sweep_grtier_build.log 2>&1
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
nohup bash /root/sweep_windows_media.sh </dev/null >/root/prov/sweep_windows_media_controller.log 2>&1 &
echo $! >/root/prov/sweep_windows_media.pid
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"
