#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/sweep_20261020_provision.log 2>&1
controller_pgid=$BASHPID
guard_pid=
cleanup_controller(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$guard_pid" ]; then kill "$guard_pid" 2>/dev/null; wait "$guard_pid" 2>/dev/null; fi
 journalctl -k -b -o short-iso-precise --no-pager >/root/prov/final_kernel.log
 echo "SWEEP_PROVISION_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup_controller EXIT
echo "SWEEP_PROVISION_START own_instance=FROM_LEASE_RECEIPT $(date -u -Is)"
export KAYFABE_BRANCH=codex/bw-resume-cont-20261022 CARGO_BUILD_JOBS=8
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 8f3a9bb3a5d652b3f536ca015bf95735ec588924
git checkout --detach 8f3a9bb3a5d652b3f536ca015bf95735ec588924
[ "$(git rev-parse HEAD)" = 8f3a9bb3a5d652b3f536ca015bf95735ec588924 ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git rev-parse HEAD >/root/prov/sweep_product_source
nohup bash /root/sweep_windows_media.sh </dev/null >/root/prov/sweep_windows_media_controller.log 2>&1 &
echo $! >/root/prov/sweep_windows_media.pid

bash scripts/bench/provision_host_driver.sh >/root/prov/driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
python3 - <<'ZERO_XID'
import sys
sys.path.insert(0,'/root/kayfabe/scripts/bench/blackwell');import phase_xid_guard as G
assert not G.N.xid_lines(G.N.kernel_snapshot()),'unexpected pre-gate native Xid'
ZERO_XID
# Stock driver installation is a provisioning prerequisite; health must pass
# before any kayfabe gate, guest or timing is attributed to this box.
bash scripts/bench/host_preflight.sh >/root/prov/sweep_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/sweep_host_preflight.log
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/sweep_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/sweep-ctl-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/sweep-ctl-probe >/root/prov/sweep_host_die.log 2>&1
cat /root/prov/sweep_host_die.log
grep -Eq 'PROBE_DIE .*die=GB202' /root/prov/sweep_host_die.log
# Falsifier: a native gate or stock workload fails; do not attribute that to kayfabe.
bash scripts/bench/v3_gates.sh /root/prov/sweep_gates.log >/root/prov/sweep_gates.run 2>&1
grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' /root/prov/sweep_gates.log
bash scripts/bench/provision_bench_tree.sh >/root/prov/tree.log 2>&1
# Explicitly rebuild source-matched raw client and fast guest after the normal QEMU flow.
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/sweep_raw_build.log 2>&1
export KF_DEVICE=kf3
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/sweep_fast595_build.log 2>&1
[ "$(sed -n 's/^version=//p' /workspace/bench/fastguest595/driver.STAGED)" = 595.91.07 ]
# Pin every later workload to this per-revision binary; never pull during a timed run.
[ -x /workspace/bench/kf3-bins/8f3a9bb3/qemu-system-x86_64 ]
sha256sum /workspace/bench/kf3-bins/8f3a9bb3/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
echo "SWEEP_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
# No unexpected kernel fault may precede the intentionally marked native controls.
python3 - <<'PRE_NATIVE'
import sys,json,subprocess
from pathlib import Path
sys.path.insert(0,'/root/kayfabe/scripts/bench/blackwell');import phase_xid_guard as G
j=G.N.kernel_snapshot(); assert not G.N.xid_lines(j),'pre-native Xid is fatal'
bus=subprocess.check_output(['nvidia-smi','--query-gpu=pci.bus_id','--format=csv,noheader'],text=True).strip()
domain,busno,dev=bus.split(':');pci=f'{int(domain,16):04x}:{busno}:{dev.split(".")[0]}'
Path('/root/prov/pci.txt').write_text(pci+'\n');G.atomic_json('/root/prov/native_receipt.json',{'approved_native_xid_lines':[],'pci':pci})
PRE_NATIVE
chmod 0755 /root/sweep2/native_wrap.py
export KF_LADDER=/root/sweep2/native_wrap.py
bash scripts/fastguest/bare_metal_suite.sh capc_native595 180 >/root/prov/sweep_bare595.log 2>&1
[ ! -e /root/prov/XID_STOP ]
grep -q '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' /root/prov/sweep_bare595.log
python3 -E -s scripts/bench/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$controller_pgid" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/guard_watch.log 2>&1 &
guard_pid=$!
for i in $(seq 1 20); do [ -f /root/prov/native_receipt.json.ready.json ] && break; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/native_receipt.json.ready.json ]
kill -0 "$guard_pid"
bash scripts/bench/cuda_ladder.sh host capc_native595 1 >/root/prov/sweep_cuda_host.log 2>&1
python3 - <<'CUDA_CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_capc_native595_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows)
CUDA_CHECK
# Native public kernel-GR report release: real GPU payload/GET and host event, before Windows.
cargo build --release -p kf-harness --bin kf-gr-tier >/root/prov/sweep_grtier_build.log 2>&1
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"

# Local operator checks fresh B1 readiness and pushes N0 evidence before allowing arms.
for i in $(seq 1 600); do [ -f /root/prov/ARM_GO ] && break; [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/ARM_GO ]
. /root/sweep2/common.sh
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
for cell in 'on on_precision' 'off off_c'; do
 set -- $cell
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 echo "VALIDATION_ARM_BEGIN $2 $(date -u -Is)"
 bash /root/sweep2/arm.sh "$1" "$2" "$BIN" "$BASE"
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 bash /root/sweep2/collect_bw.sh "$2"
 echo "VALIDATION_ARM_END $2 $(date -u -Is)"
 # Pause for export/push before another arm, bounded by 5 min.
 for i in $(seq 1 300); do [ -f /root/prov/EXPORT_ACK_$2 ] && break; sleep 1; done
 [ -f /root/prov/EXPORT_ACK_$2 ]
done
echo "VALIDATION_DONE $(date -u -Is)"
