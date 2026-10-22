#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/sweep_20261020_provision.log 2>&1
xid_watcher_pid=
controller_exit(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$xid_watcher_pid" ]; then kill -TERM "$xid_watcher_pid" 2>/dev/null; wait "$xid_watcher_pid" 2>/dev/null; fi
 echo "SWEEP_PROVISION_EXIT rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap controller_exit EXIT
trap 'exit 143' TERM
echo "SWEEP_PROVISION_START own_instance=57138158 $(date -u -Is)"
export KAYFABE_BRANCH=codex/sweep-ada-20261022 CARGO_BUILD_JOBS=8
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 35ff2820e5573199072ab2d274f5f0b91a1826a8
git checkout --detach 35ff2820e5573199072ab2d274f5f0b91a1826a8
[ "$(git rev-parse HEAD)" = 35ff2820e5573199072ab2d274f5f0b91a1826a8 ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git rev-parse HEAD >/root/prov/sweep_product_source
nohup bash /root/sweep_windows_media.sh </dev/null >/root/prov/sweep_windows_media_controller.log 2>&1 &
echo $! >/root/prov/sweep_windows_media.pid
bash scripts/bench/provision_host_driver.sh >/root/prov/driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
# Stock driver installation is a provisioning prerequisite; health must pass
# before any kayfabe gate, guest or timing is attributed to this box.
bash scripts/bench/host_preflight.sh >/root/prov/sweep_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/sweep_host_preflight.log
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/sweep_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/sweep-ctl-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/sweep-ctl-probe >/root/prov/sweep_host_die.log 2>&1
cat /root/prov/sweep_host_die.log
grep -Eq 'PROBE_DIE .*die=AD102' /root/prov/sweep_host_die.log
# Falsifier: a native gate or stock workload fails; do not attribute that to kayfabe.
bash scripts/bench/v3_gates.sh /root/prov/sweep_gates.log >/root/prov/sweep_gates.run 2>&1
grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' /root/prov/sweep_gates.log
# Public, opt-in shared observer support, compiled before the frozen binary.
mkdir -p /workspace/bench
curl -fsSL -o /workspace/bench/qemu-10.2.4.tar.xz https://download.qemu.org/qemu-10.2.4.tar.xz
tar -C /workspace/bench -xf /workspace/bench/qemu-10.2.4.tar.xz
python3 tools/vfio-gsp-observer/apply.py /workspace/bench/qemu-10.2.4 --check >/root/prov/observer_source_check.log 2>&1
python3 tools/vfio-gsp-observer/apply.py /workspace/bench/qemu-10.2.4 >/root/prov/observer_source_apply.log 2>&1
bash scripts/bench/provision_bench_tree.sh >/root/prov/tree.log 2>&1
# Explicitly rebuild source-matched raw client and fast guest after the normal QEMU flow.
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/sweep_raw_build.log 2>&1
export KF_DEVICE=kf3
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/sweep_fast595_build.log 2>&1
[ "$(sed -n 's/^version=//p' /workspace/bench/fastguest595/driver.STAGED)" = 595.91.07 ]
# Pin every later workload to this per-revision binary; never pull during a timed run.
[ -x /workspace/bench/kf3-bins/35ff2820/qemu-system-x86_64 ]
sha256sum /workspace/bench/kf3-bins/35ff2820/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
echo "SWEEP_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
bash /root/sweep_native_baseline.sh
# Native public kernel-GR report release: real GPU payload/GET and host event, before Windows.
cargo build --release -p kf-harness --bin kf-gr-tier >/root/prov/sweep_grtier_build.log 2>&1
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
# Full serial current-revision Linux guest acceptance before Windows.
echo "LINUX_GUEST_START source=$(git rev-parse HEAD) $(date -u -Is)"
# Owner decision1: source-matched negative controls only, then freeze ALL Xid lines.
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/native_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py native /root/prov/native_kernel_snapshot.log /workspace/bench/sweep35ff2820_ad102_native595_defer-liveness.log /workspace/bench/sweep35ff2820_ad102_native595_missing-page-fault.log >/root/prov/native_xid_baseline.json
cat /root/prov/native_xid_baseline.json
cp /root/prov/native_xid_baseline.json /root/prov/s2_accepted_xid_history.json
python3 -B scripts/bench/guarded_fast_suite.py s2_guest595 /root/prov/s2_accepted_xid_history.json 35ff2820e5573199072ab2d274f5f0b91a1826a8 /workspace/bench/kf3-bins/35ff2820/qemu-system-x86_64 --fastguest-dir /workspace/bench/fastguest595 >/root/prov/sweep_linux_guest595.log 2>&1
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
bash /root/s2/arm.sh on ad102_on_d1 /workspace/bench/kf3-bins/35ff2820/qemu-system-x86_64 35ff2820e5573199072ab2d274f5f0b91a1826a8
python3 /root/s2/validate_arm.py ad102_on_d1
S2_OBSERVER=1 bash /root/s2/arm.sh on ad102_obs_d1 /workspace/bench/kf3-bins/35ff2820/qemu-system-x86_64 35ff2820e5573199072ab2d274f5f0b91a1826a8
python3 /root/s2/validate_arm.py ad102_obs_d1
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/post_windows_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py check /root/prov/post_windows_kernel_snapshot.log /root/prov/s2_frozen_xid_history.json
echo "SWEEP_WINDOWS_ON_COMPLETE $(date -u -Is)"
