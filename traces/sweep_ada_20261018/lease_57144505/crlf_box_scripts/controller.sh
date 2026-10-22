#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov
exec >/root/prov/s2_crlf_controller.log 2>&1
xid_watcher_pid=
controller_exit(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$xid_watcher_pid" ]; then kill -TERM "$xid_watcher_pid" 2>/dev/null; wait "$xid_watcher_pid" 2>/dev/null; fi
 echo "SWEEP_CRLF_EXIT rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap controller_exit EXIT
trap 'exit 143' TERM
export PATH=/root/.cargo/bin:$PATH KF_DEVICE=kf3 CARGO_BUILD_JOBS=8
echo "SWEEP_CRLF_START own_instance=57144505 $(date -u -Is)"
[ "$(cat /proc/sys/kernel/random/boot_id)" = 425c0e6f-5587-4b0e-a1f7-f446e46c1434 ]
cat /proc/sys/kernel/random/boot_id >/root/prov/s2_crlf_boot_id
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
cd /root/kayfabe
git fetch --depth 1 origin 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
git checkout --detach 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
[ "$(git rev-parse HEAD)" = 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0 ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git rev-parse HEAD >/root/prov/sweep_product_source
python3 -B - <<'CHECK'
import sys
sys.path.insert(0,'scripts/bench')
import native_xid_guard as g
assert not g.xid_lines(g.kernel_snapshot()),'new boot must start with zero faults; never exclude unmatched history'
CHECK
python3 -B -m unittest discover -s scripts/bench/tests -p 'test_*guard*.py' >/root/prov/s2_crlf_host_tests.log 2>&1
python3 -B -m unittest discover -s scripts/bench/tests -p test_xid_abort_tree.py >>/root/prov/s2_crlf_host_tests.log 2>&1
bash scripts/bench/host_preflight.sh >/root/prov/s2_crlf_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/s2_crlf_host_preflight.log
bash scripts/bench/v3_gates.sh /root/prov/s2_crlf_gates.log >/root/prov/s2_crlf_gates.run 2>&1
grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' /root/prov/s2_crlf_gates.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 >/root/prov/s2_crlf_build.log 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/sweep_raw_build.log 2>&1
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/sweep_fast595_build.log 2>&1
[ "$(sed -n 's/^version=//p' /workspace/bench/fastguest595/driver.STAGED)" = 595.91.07 ]
sha256sum /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
echo "SWEEP_CRLF_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
bash /root/sweep_native_baseline.sh
# Native public kernel-GR report release: real GPU payload/GET and host event, before Windows.
cargo build --release -p kf-harness --bin kf-gr-tier >/root/prov/sweep_grtier_build.log 2>&1
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
# Full serial current-revision Linux guest acceptance before Windows.
echo "LINUX_GUEST_START source=$(git rev-parse HEAD) $(date -u -Is)"
# Owner decision1: source-matched negative controls only, then freeze ALL Xid lines.
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/native_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py native /root/prov/native_kernel_snapshot.log /workspace/bench/sweep7f6f7e7a_ad102_native595_defer-liveness.log /workspace/bench/sweep7f6f7e7a_ad102_native595_missing-page-fault.log >/root/prov/native_xid_baseline.json
cat /root/prov/native_xid_baseline.json
cp /root/prov/native_xid_baseline.json /root/prov/s2_accepted_xid_history.json
python3 -B scripts/bench/guarded_fast_suite.py s2r4_guest595 /root/prov/s2_accepted_xid_history.json 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0 /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 --fastguest-dir /workspace/bench/fastguest595 >/root/prov/sweep_linux_guest595.log 2>&1
grep '^FAST_SUITE_PASS=30 ' /root/prov/sweep_linux_guest595.log
python3 - <<'FREEZE'
import json
from pathlib import Path
state=json.loads(Path('/root/prov/s2_accepted_xid_history.json').read_text())
assert len(state['approved_native_xid_lines'])==5
assert len(state['accepted_xid_lines'])==10
assert [p['accepted_faults'] for p in state['guest_proofs']]==[4,1]
Path('/root/prov/s2_frozen_xid_history.json').write_text(json.dumps({'approved_native_xid_lines':state['accepted_xid_lines']}))
FREEZE
python3 -u scripts/bench/native_xid_guard.py watch /root/prov/s2_frozen_xid_history.json $$ /root/prov/native_xid_fatal.json >/root/prov/native_xid_watch.log 2>&1 &
xid_watcher_pid=$!
kill -0 "$xid_watcher_pid"
echo "LINUX_GUEST_EXIT rc=0 $(date -u -Is)"
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"
echo 'H_AD102_ON BEFORE RUN: current frozen product supports AD102 Windows580.88 ON; falsifier Code43/crash/TDR/desktop/CUDA/DX failure. PASS needs600second zeroTDRhold and real app results.'
bash /root/s2/arm.sh on ad102_on_d1 /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
python3 /root/s2/validate_arm.py ad102_on_d1
S2_OBSERVER=1 bash /root/s2/arm.sh on ad102_obs_d1 /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
python3 /root/s2/validate_arm.py ad102_obs_d1
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/post_windows_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py check /root/prov/post_windows_kernel_snapshot.log /root/prov/s2_frozen_xid_history.json
echo "SWEEP_WINDOWS_ON_COMPLETE $(date -u -Is)"
