#!/bin/bash
set -euo pipefail
. /root/s2/common.sh
cd /root/kayfabe
bash /root/sweep_native_baseline.sh
cargo build --release -p kf-harness --bin kf-gr-tier >/root/prov/sweep_grtier_build.log 2>&1
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/native_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py native /root/prov/native_kernel_snapshot.log /workspace/bench/sweep17005433_ad102_native595_defer-liveness.log /workspace/bench/sweep17005433_ad102_native595_missing-page-fault.log >/root/prov/native_xid_baseline.json
cat /root/prov/native_xid_baseline.json
echo 'LINUX_SUITE_SKIPPED prior family native/Linux evidence; no new Linux claim'
